//! Per-file view of a [`Report`].
//!
//! Post-processes the output of [`crate::lint`] into a [`OverviewReport`]
//! with findings grouped by file (sorted by finding count desc, then
//! path asc) and a separate list of clean files (those discovered by
//! `lint()` that produced no findings). Renders as text (with optional
//! ANSI color) or pretty-printed JSON.
//!
//! Used by the `zorilla overview` subcommand; it is a report, not a
//! gate — callers always exit 0.
//!
//! Both renderers finish with a trailing newline so callers can use
//! `print!` uniformly with the other renderers in [`Report`] and
//! [`crate::stats`].

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::PathBuf;

use serde::Serialize;

use crate::report::{Finding, Report, Severity};
use crate::rules::{self, Rule};

/// Aggregate summary counters surfaced in every overview.
#[derive(Debug, Clone, Serialize)]
pub struct OverviewSummary {
    pub files_scanned: usize,
    pub files_with_findings: usize,
    pub total_findings: usize,
}

/// One entry in [`OverviewReport::files`] — a file that produced at
/// least one finding, with its findings in source order.
#[derive(Debug, Clone, Serialize)]
pub struct FileOverview {
    pub path: PathBuf,
    pub finding_count: usize,
    #[serde(serialize_with = "serialize_findings")]
    pub findings: Vec<Finding>,
}

/// Full per-file overview of a scan. `clean_files` lists every
/// discovered file that produced no findings — sorted by path so the
/// output is stable across platforms.
#[derive(Debug, Clone, Serialize)]
pub struct OverviewReport {
    pub summary: OverviewSummary,
    pub files: Vec<FileOverview>,
    pub clean_files: Vec<PathBuf>,
}

/// Group `report`'s findings by file, returning an [`OverviewReport`].
///
/// - `files` is sorted by `finding_count` descending, then `path`
///   ascending (ties broken deterministically).
/// - Within each `FileOverview`, findings preserve the `(line, column)`
///   order that `lint()` already emits — i.e. we keep the slice order
///   from `report.findings`, which is already sorted.
/// - `clean_files` is the set of paths in `report.discovered_files`
///   that produced zero findings, sorted ascending.
#[must_use]
pub fn compute_overview(report: &Report) -> OverviewReport {
    // Group findings by file, preserving their existing order. `lint()`
    // already sorts findings by (file, line, column) so the Vec we push
    // into is already in source order for every file.
    let mut by_file: BTreeMap<PathBuf, Vec<Finding>> = BTreeMap::new();
    for finding in &report.findings {
        by_file.entry(finding.file.clone()).or_default().push(finding.clone());
    }

    let files_with_findings = by_file.len();
    let total_findings = report.findings.len();

    // `clean_files` = discovered − files-with-findings. `by_file`
    // already carries the "has-findings" key set, so we use it for the
    // membership test before consuming it into `files` below. This
    // avoids a second pass over `report.findings` plus a throwaway
    // `BTreeSet`. `report.discovered_files` is sorted ascending by
    // `lint()` (see `zorilla_core::lint`), so the filtered output is
    // already in the order we want — no explicit sort needed.
    let clean_files: Vec<PathBuf> = report
        .discovered_files
        .iter()
        .filter(|p| !by_file.contains_key(p.as_path()))
        .cloned()
        .collect();

    let mut files: Vec<FileOverview> = by_file
        .into_iter()
        .map(|(path, findings)| FileOverview { path, finding_count: findings.len(), findings })
        .collect();
    // Stable sort: primary key is finding_count descending, secondary
    // key is path ascending. `sort_by` is stable on Rust's default
    // implementation; ties collapse to path order.
    files.sort_by(|a, b| b.finding_count.cmp(&a.finding_count).then(a.path.cmp(&b.path)));

    OverviewReport {
        summary: OverviewSummary {
            files_scanned: report.files_discovered,
            files_with_findings,
            total_findings,
        },
        files,
        clean_files,
    }
}

/// Render [`OverviewReport`] as a text block.
///
/// Shape:
/// ```text
/// Overview: {files_scanned} files, {total_findings} findings in {files_with_findings} files
///
/// {path}                                     {n} findings
///   ● {line}:{col}  ZRNNN rule-name          message
///   ...
///
/// {N} clean files not shown.
/// ```
///
/// When `use_color = true`, the bullet (`●`) is wrapped in ANSI escape
/// codes: red for [`Severity::Error`], yellow for [`Severity::Warning`].
/// Callers should pass `false` when stdout is not a terminal, or when
/// `NO_COLOR` is set.
#[must_use]
pub fn format_overview_text(overview: &OverviewReport, use_color: bool) -> String {
    let mut out = String::new();

    // Header line — matches context.md's "Overview: 12 files, 7 findings
    // in 3 files" shape.
    let _ = writeln!(
        out,
        "Overview: {} files, {} findings in {} files",
        overview.summary.files_scanned,
        overview.summary.total_findings,
        overview.summary.files_with_findings,
    );

    // Column widths for the per-finding lines. Each file's findings are
    // aligned locally: position width = widest "line:col" in that file,
    // code column is fixed (5 chars, e.g. "ZR001"), name column widens
    // to the file's widest rule name.
    //
    // Precompute rows once — this resolves each finding's rule name
    // (avoiding a second `rules::registry::find` pass per finding) and
    // formats its `line:col` position string a single time. The width
    // loop then runs over the prepared rows rather than re-walking the
    // raw findings with fresh allocations.
    for file in &overview.files {
        out.push('\n');
        // File header: "path    N findings". Keep the two columns
        // separated by at least two spaces — pure cosmetic. Pluralise
        // "finding" when there is exactly one so the output reads as
        // English.
        let path_str = file.path.display().to_string();
        let _ = writeln!(
            out,
            "{}  {} {}",
            path_str,
            file.finding_count,
            plural_findings(file.finding_count),
        );

        let rows: Vec<FindingRow<'_>> = file
            .findings
            .iter()
            .map(|f| FindingRow {
                bullet: render_bullet(f.severity, use_color),
                pos: format!("{}:{}", f.line, f.column),
                code: f.code,
                name: rules::registry::find(f.code).map_or("unknown", Rule::name),
                message: &f.message,
            })
            .collect();
        let pos_width = rows.iter().map(|r| r.pos.len()).max().unwrap_or(0);
        let name_width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);

        for row in &rows {
            let _ = writeln!(
                out,
                "  {bullet} {pos:<pos_width$}  {code} {name:<name_width$}  {message}",
                bullet = row.bullet,
                pos = row.pos,
                code = row.code,
                name = row.name,
                message = row.message,
            );
        }
    }

    if !overview.clean_files.is_empty() {
        out.push('\n');
        let _ = writeln!(out, "{} clean files not shown.", overview.clean_files.len());
    }

    // Same rule as the `check` text report: a footer only where there is
    // something to act on. Never coloured — it has to survive being piped.
    if overview.summary.total_findings > 0 {
        crate::guide::push_report_footer(&mut out);
    }

    out
}

/// One finding's pre-resolved render fragments. Built once per file so
/// the width pass and render pass share the same `line:col` string and
/// rule-name lookup.
struct FindingRow<'a> {
    bullet: &'static str,
    pos: String,
    code: &'a str,
    name: &'static str,
    message: &'a str,
}

/// English pluralisation for "finding" — "1 finding" but "0/2/… findings".
fn plural_findings(count: usize) -> &'static str {
    if count == 1 {
        "finding"
    } else {
        "findings"
    }
}

/// Render [`OverviewReport`] as pretty-printed JSON with a trailing newline.
///
/// Shape matches `cf/.../context.md`:
/// ```json
/// {
///   "summary": { "files_scanned": N, "files_with_findings": N, "total_findings": N },
///   "files": [
///     { "path": "...", "finding_count": N,
///       "findings": [ { "code": "ZRNNN", "line": N, "column": N,
///                        "severity": "warning", "message": "..." }, ... ] }
///   ],
///   "clean_files": [ "...", ... ]
/// }
/// ```
///
/// `serde_json::to_string_pretty` over [`OverviewReport`] cannot fail
/// at runtime — every field is a primitive, `String`, `PathBuf`
/// (serialized as string), or `Vec` thereof, with no non-string map
/// key and no float. Returns `String` (not `Result<String>`) to match
/// the sibling renderers in [`Report`] and [`crate::stats`].
#[must_use]
pub fn format_overview_json(overview: &OverviewReport) -> String {
    let mut out = serde_json::to_string_pretty(overview)
        .expect("OverviewReport serializes infallibly; see format_overview_json invariants");
    out.push('\n');
    out
}

/// Rendered bullet strings. `BULLET_PLAIN` is the unstyled Unicode
/// bullet; the colored variants wrap it with SGR codes (red 31,
/// yellow 33, reset 0). Returning `&'static str` from
/// [`render_bullet`] saves one allocation per finding in the text
/// renderer, so the common `NO_COLOR` path emits zero bullet
/// allocations.
const BULLET_PLAIN: &str = "\u{25CF}";
const BULLET_RED: &str = "\x1b[31m\u{25CF}\x1b[0m";
const BULLET_YELLOW: &str = "\x1b[33m\u{25CF}\x1b[0m";

fn render_bullet(severity: Severity, use_color: bool) -> &'static str {
    if !use_color {
        return BULLET_PLAIN;
    }
    match severity {
        Severity::Error => BULLET_RED,
        Severity::Warning => BULLET_YELLOW,
    }
}

/// Custom serializer for the `Vec<Finding>` field so we emit the shape
/// promised in context.md (just the fields the overview cares about —
/// no redundant `file`, matching biston's per-file view).
fn serialize_findings<S>(findings: &[Finding], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeSeq as _;
    let mut seq = serializer.serialize_seq(Some(findings.len()))?;
    for f in findings {
        seq.serialize_element(&FindingOverviewJson {
            code: f.code,
            line: f.line,
            column: f.column,
            severity: f.severity.as_str(),
            message: &f.message,
        })?;
    }
    seq.end()
}

#[derive(Serialize)]
struct FindingOverviewJson<'a> {
    code: &'a str,
    line: usize,
    column: usize,
    severity: &'a str,
    message: &'a str,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(code: &'static str, file: &str, line: usize, column: usize) -> Finding {
        Finding {
            code,
            message: "msg".into(),
            file: PathBuf::from(file),
            line,
            column,
            severity: Severity::Warning,
        }
    }

    #[test]
    fn compute_overview_on_empty_report_produces_empty_files_and_clean_files() {
        let report =
            Report { findings: Vec::new(), files_discovered: 0, discovered_files: Vec::new() };
        let overview = compute_overview(&report);
        assert_eq!(overview.summary.files_scanned, 0);
        assert_eq!(overview.summary.files_with_findings, 0);
        assert_eq!(overview.summary.total_findings, 0);
        assert!(overview.files.is_empty());
        assert!(overview.clean_files.is_empty());
    }

    #[test]
    fn compute_overview_sorts_files_by_count_desc_then_path_asc() {
        // Three files:
        //   test_a.py  — 1 finding (few)
        //   test_b.py  — 3 findings (most)
        //   test_c.py  — 3 findings (tie with test_b.py)
        // Expected order: test_b.py, test_c.py, test_a.py
        let report = Report {
            findings: vec![
                finding("ZR001", "test_a.py", 1, 1),
                finding("ZR001", "test_b.py", 5, 1),
                finding("ZR002", "test_b.py", 10, 1),
                finding("ZR003", "test_b.py", 15, 1),
                finding("ZR001", "test_c.py", 2, 1),
                finding("ZR002", "test_c.py", 4, 1),
                finding("ZR003", "test_c.py", 6, 1),
            ],
            files_discovered: 3,
            discovered_files: vec![
                PathBuf::from("test_a.py"),
                PathBuf::from("test_b.py"),
                PathBuf::from("test_c.py"),
            ],
        };
        let overview = compute_overview(&report);
        let paths: Vec<_> = overview.files.iter().map(|f| f.path.display().to_string()).collect();
        assert_eq!(paths, vec!["test_b.py", "test_c.py", "test_a.py"]);
        // Counts carry over correctly.
        let counts: Vec<_> = overview.files.iter().map(|f| f.finding_count).collect();
        assert_eq!(counts, vec![3, 3, 1]);
    }

    #[test]
    fn compute_overview_preserves_findings_order_within_each_file() {
        // Two findings in the same file at (line 5, col 9) and (line
        // 12, col 3). After grouping they must come back in source
        // order: (5,9) then (12,3).
        let report = Report {
            findings: vec![
                finding("ZR001", "test_a.py", 5, 9),
                finding("ZR002", "test_a.py", 12, 3),
            ],
            files_discovered: 1,
            discovered_files: vec![PathBuf::from("test_a.py")],
        };
        let overview = compute_overview(&report);
        let locs: Vec<(usize, usize)> =
            overview.files[0].findings.iter().map(|f| (f.line, f.column)).collect();
        assert_eq!(locs, vec![(5, 9), (12, 3)]);
    }

    #[test]
    fn compute_overview_marks_discovered_files_without_findings_as_clean() {
        // Four discovered files; two carry findings, two are clean.
        let report = Report {
            findings: vec![
                finding("ZR001", "tests/test_a.py", 1, 1),
                finding("ZR001", "tests/test_b.py", 1, 1),
            ],
            files_discovered: 4,
            discovered_files: vec![
                PathBuf::from("tests/test_a.py"),
                PathBuf::from("tests/test_b.py"),
                PathBuf::from("tests/test_clean_x.py"),
                PathBuf::from("tests/test_clean_y.py"),
            ],
        };
        let overview = compute_overview(&report);
        assert_eq!(overview.summary.files_scanned, 4);
        assert_eq!(overview.summary.files_with_findings, 2);
        let cleans: Vec<_> = overview.clean_files.iter().map(|p| p.display().to_string()).collect();
        assert_eq!(cleans, vec!["tests/test_clean_x.py", "tests/test_clean_y.py"]);
    }

    #[test]
    fn overview_text_with_findings_points_at_the_triage_guide() {
        let report = Report {
            findings: vec![finding("ZR001", "tests/test_a.py", 12, 5)],
            files_discovered: 1,
            discovered_files: vec![PathBuf::from("tests/test_a.py")],
        };
        let text = format_overview_text(&compute_overview(&report), false);
        assert!(
            text.contains("zorilla guide triage"),
            "footer should point at the triage guide, got:\n{text}"
        );
    }

    #[test]
    fn overview_text_without_findings_omits_the_triage_footer() {
        let report = Report {
            findings: Vec::new(),
            files_discovered: 1,
            discovered_files: vec![PathBuf::from("tests/test_clean.py")],
        };
        let text = format_overview_text(&compute_overview(&report), false);
        assert!(
            !text.contains("zorilla guide triage"),
            "a clean run has nothing to triage, got:\n{text}"
        );
    }

    #[test]
    fn format_overview_text_without_color_has_no_ansi_and_shows_expected_fields() {
        let report = Report {
            findings: vec![finding("ZR001", "tests/test_a.py", 12, 5)],
            files_discovered: 2,
            discovered_files: vec![
                PathBuf::from("tests/test_a.py"),
                PathBuf::from("tests/test_clean.py"),
            ],
        };
        let overview = compute_overview(&report);
        let text = format_overview_text(&overview, false);
        // No ANSI escape bytes when color is off.
        assert!(!text.contains('\x1b'), "expected no ANSI escapes in:\n{text}");
        // Bullet character is present (plain, unwrapped).
        assert!(text.contains('\u{25CF}'), "missing bullet in:\n{text}");
        // Path, finding_count, and the line:col + code + name + message
        // all show up.
        assert!(text.contains("tests/test_a.py"), "missing path in:\n{text}");
        // Per-file header uses singular form when count == 1.
        // Anchored on the full phrase so we don't match the header line
        // ("Overview: 2 files, 1 findings in 1 files") incidentally.
        assert!(
            text.contains("tests/test_a.py  1 finding\n"),
            "expected singular `1 finding` on per-file header, got:\n{text}",
        );
        assert!(text.contains("12:5"), "missing line:column in:\n{text}");
        assert!(text.contains("ZR001"), "missing rule code in:\n{text}");
        assert!(text.contains("conditional-test-logic"), "missing rule name in:\n{text}");
        assert!(text.contains("msg"), "missing message in:\n{text}");
        // Clean-files footer mentions the count.
        assert!(
            text.contains("1 clean files not shown."),
            "missing clean-files footer in:\n{text}"
        );
        // Header line shape.
        assert!(
            text.contains("Overview: 2 files, 1 findings in 1 files"),
            "missing overview header in:\n{text}",
        );
    }

    #[test]
    fn format_overview_text_with_color_emits_ansi_around_bullet() {
        let report = Report {
            findings: vec![finding("ZR001", "tests/test_a.py", 1, 1)],
            files_discovered: 1,
            discovered_files: vec![PathBuf::from("tests/test_a.py")],
        };
        let overview = compute_overview(&report);
        let text = format_overview_text(&overview, true);
        // ANSI escape byte present (0x1b); reset sequence present.
        assert!(text.contains('\x1b'), "expected ANSI escapes in:\n{text:?}");
        assert!(text.contains("\x1b[0m"), "missing ANSI reset in:\n{text:?}");
        // Warning severity → yellow (33).
        assert!(text.contains("\x1b[33m"), "missing yellow ANSI code in:\n{text:?}");
    }

    #[test]
    fn format_overview_text_with_color_uses_red_for_error_severity() {
        // Synthetic: hand-build a finding with Error severity.
        let report = Report {
            findings: vec![Finding {
                code: "ZR099",
                message: "boom".into(),
                file: PathBuf::from("tests/test_err.py"),
                line: 1,
                column: 1,
                severity: Severity::Error,
            }],
            files_discovered: 1,
            discovered_files: vec![PathBuf::from("tests/test_err.py")],
        };
        let overview = compute_overview(&report);
        let text = format_overview_text(&overview, true);
        assert!(text.contains("\x1b[31m"), "missing red ANSI code in:\n{text:?}");
    }

    #[test]
    fn format_overview_json_round_trips_with_expected_shape() {
        let report = Report {
            findings: vec![
                finding("ZR001", "tests/test_a.py", 12, 5),
                finding("ZR003", "tests/test_a.py", 20, 1),
                finding("ZR002", "tests/test_b.py", 7, 9),
            ],
            files_discovered: 3,
            discovered_files: vec![
                PathBuf::from("tests/test_a.py"),
                PathBuf::from("tests/test_b.py"),
                PathBuf::from("tests/test_clean.py"),
            ],
        };
        let overview = compute_overview(&report);
        let json = format_overview_json(&overview);
        assert!(json.ends_with('\n'), "json must end with newline, got {json:?}");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Summary block.
        assert_eq!(parsed["summary"]["files_scanned"], 3);
        assert_eq!(parsed["summary"]["files_with_findings"], 2);
        assert_eq!(parsed["summary"]["total_findings"], 3);

        // `files` array sorted by count desc (test_a.py=2 before
        // test_b.py=1), each entry carrying path, finding_count, findings.
        let files = parsed["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["path"], "tests/test_a.py");
        assert_eq!(files[0]["finding_count"], 2);
        let first_findings = files[0]["findings"].as_array().unwrap();
        assert_eq!(first_findings.len(), 2);
        assert_eq!(first_findings[0]["code"], "ZR001");
        assert_eq!(first_findings[0]["line"], 12);
        assert_eq!(first_findings[0]["column"], 5);
        assert_eq!(first_findings[0]["severity"], "warning");
        assert_eq!(first_findings[0]["message"], "msg");
        assert_eq!(files[1]["path"], "tests/test_b.py");
        assert_eq!(files[1]["finding_count"], 1);

        // `clean_files` is an array of strings.
        let clean = parsed["clean_files"].as_array().unwrap();
        assert_eq!(clean.len(), 1);
        assert_eq!(clean[0], "tests/test_clean.py");
    }

    #[test]
    fn format_overview_text_empty_has_no_bullets_and_no_footer() {
        let report =
            Report { findings: Vec::new(), files_discovered: 0, discovered_files: Vec::new() };
        let overview = compute_overview(&report);
        let text = format_overview_text(&overview, false);
        // No bullet character on a clean run.
        assert!(!text.contains('\u{25CF}'), "unexpected bullet in:\n{text}");
        // Footer is only emitted when there are clean files — skipped here.
        assert!(!text.contains("clean files not shown."), "unexpected footer in:\n{text}");
        // Header still present.
        assert!(text.contains("Overview: 0 files, 0 findings in 0 files"));
    }
}
