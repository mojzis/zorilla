//! Aggregate-statistics view of a [`Report`].
//!
//! Post-processes the output of [`crate::lint`] into a single
//! [`ScanStats`] value and renders it as text or pretty-printed JSON.
//! Used by the `zorilla stats` subcommand; zorilla stays focused on the
//! per-finding rendering in [`Report`] for the primary `check` flow.
//!
//! The breakdown always lists every rule returned by
//! [`crate::rules::registry::all`] so consumers see `ZR004: 0` rows even
//! when the scan produced no findings for that rule — stable columns
//! across runs are more useful than a sparse map.
//!
//! Both renderers finish with a trailing newline so callers can use
//! `print!` uniformly with the other renderers in [`Report`].

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::Serialize;

use crate::report::Report;
use crate::rules::{self, Rule};

/// Aggregate counters derived from a single [`Report`].
///
/// `breakdown` is a `BTreeMap` so serialization and text output both
/// emit rule codes in stable ascending order.
#[derive(Debug, Clone, Serialize)]
pub struct ScanStats {
    pub files_scanned: usize,
    pub files_with_findings: usize,
    pub clean_files: usize,
    pub total_findings: usize,
    pub breakdown: BTreeMap<String, usize>,
}

/// Compute summary counters from a [`Report`].
///
/// The breakdown is seeded with every registered rule at `0`, then
/// incremented per finding. Files-with-findings is computed by counting
/// the set of distinct paths carrying at least one finding.
#[must_use]
pub fn compute_stats(report: &Report) -> ScanStats {
    let mut breakdown: BTreeMap<String, usize> = BTreeMap::new();
    for rule in rules::registry::all() {
        breakdown.insert(rule.code().to_string(), 0);
    }

    let mut files_with_findings: BTreeSet<&std::path::Path> = BTreeSet::new();
    for finding in &report.findings {
        *breakdown.entry(finding.code.to_string()).or_insert(0) += 1;
        files_with_findings.insert(finding.file.as_path());
    }

    let files_scanned = report.files_discovered;
    let files_with_findings_count = files_with_findings.len();
    // `clean_files` is the non-negative difference; clamp at zero in
    // case some future code path produces findings for files not counted
    // in `files_discovered` (e.g. explicit file args bypassing the walk).
    let clean_files = files_scanned.saturating_sub(files_with_findings_count);

    ScanStats {
        files_scanned,
        files_with_findings: files_with_findings_count,
        clean_files,
        total_findings: report.findings.len(),
        breakdown,
    }
}

/// Render [`ScanStats`] as an aligned text block.
///
/// Columns are aligned so the numeric column lines up — rule rows pad
/// the `  ZRxxx rule-name:` portion to the widest label so counts
/// stack vertically. Shape matches `cf/.../context.md`.
#[must_use]
pub fn format_stats_text(stats: &ScanStats) -> String {
    let mut out = String::new();
    out.push_str("Scan statistics:\n");
    // Top block: four summary counters. Pad labels so the numeric
    // column lines up. "Files with findings:" (20 chars incl. colon) is
    // the longest of the four — pin to that width explicitly so a
    // future renamed label doesn't silently lose alignment.
    let label_width = "Files with findings:".len();
    // Writing to `String` is infallible; `let _ = writeln!(...)` mirrors
    // the idiom used in `list_rules`/`Report::render_text`.
    let mut write_row = |label: &str, count: usize| {
        let _ = writeln!(out, "  {label:<label_width$}  {count}");
    };
    write_row("Files scanned:", stats.files_scanned);
    write_row("Files with findings:", stats.files_with_findings);
    write_row("Clean files:", stats.clean_files);
    write_row("Total findings:", stats.total_findings);

    out.push('\n');
    out.push_str("Breakdown by rule:\n");

    // Build "CODE name:" labels first so we can pad them to the widest
    // one. Unknown codes (shouldn't happen for engine-produced reports,
    // but breakdown is a plain map) fall back to "unknown".
    let labels: Vec<(String, usize)> = stats
        .breakdown
        .iter()
        .map(|(code, count)| {
            let name = rules::registry::find(code).map_or("unknown", Rule::name);
            (format!("{code} {name}:"), *count)
        })
        .collect();
    let rule_label_width = labels.iter().map(|(label, _)| label.len()).max().unwrap_or(0);
    for (label, count) in labels {
        let _ = writeln!(out, "  {label:<rule_label_width$}  {count}");
    }

    out
}

/// Render [`ScanStats`] as pretty-printed JSON with a trailing newline.
///
/// The output shape matches `cf/.../context.md`: flat keys for the four
/// counters and an ordered `breakdown` object. `serde_json` preserves
/// the `BTreeMap` ordering so rule codes appear in ascending order.
///
/// `serde_json::to_string_pretty` over [`ScanStats`] cannot fail at
/// runtime — every field is `usize` / `String` / `BTreeMap<String,
/// usize>` with no custom `Serialize`, no non-string map key, and no
/// float. Returns `String` (not `Result<String>`) to match the sibling
/// [`Report::render_json`] / [`Report::render_sarif`] renderers, which
/// solve the same infallibility argument the same way.
///
/// [`Report::render_json`]: crate::report::Report::render_json
/// [`Report::render_sarif`]: crate::report::Report::render_sarif
#[must_use]
pub fn format_stats_json(stats: &ScanStats) -> String {
    let mut out = serde_json::to_string_pretty(stats)
        .expect("ScanStats serializes infallibly; see format_stats_json invariants");
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::report::{Finding, Severity};

    fn finding(code: &'static str, file: &str, line: usize) -> Finding {
        Finding {
            code,
            message: "msg".into(),
            file: PathBuf::from(file),
            line,
            column: 1,
            severity: Severity::Warning,
        }
    }

    #[test]
    fn empty_report_produces_zero_stats_with_every_rule_seeded() {
        let report = Report { findings: Vec::new(), files_discovered: 0 };
        let stats = compute_stats(&report);
        assert_eq!(stats.files_scanned, 0);
        assert_eq!(stats.files_with_findings, 0);
        assert_eq!(stats.clean_files, 0);
        assert_eq!(stats.total_findings, 0);

        // Every registered rule must appear in the breakdown at 0, even
        // when no findings were produced — stable columns across runs.
        for rule in rules::registry::all() {
            assert_eq!(
                stats.breakdown.get(rule.code()).copied(),
                Some(0),
                "missing or non-zero breakdown entry for {}",
                rule.code()
            );
        }
    }

    #[test]
    fn compute_stats_tallies_findings_across_files_and_rules() {
        // 3 files discovered; 2 of them carry findings spanning 3
        // rule codes with varying counts.
        let report = Report {
            findings: vec![
                finding("ZR001", "tests/test_a.py", 2),
                finding("ZR003", "tests/test_a.py", 6),
                finding("ZR003", "tests/test_a.py", 11),
                finding("ZR002", "tests/test_b.py", 3),
            ],
            files_discovered: 3,
        };
        let stats = compute_stats(&report);

        assert_eq!(stats.files_scanned, 3);
        assert_eq!(stats.files_with_findings, 2);
        assert_eq!(stats.clean_files, 1);
        assert_eq!(stats.total_findings, 4);

        assert_eq!(stats.breakdown.get("ZR001").copied(), Some(1));
        assert_eq!(stats.breakdown.get("ZR002").copied(), Some(1));
        assert_eq!(stats.breakdown.get("ZR003").copied(), Some(2));
        // Rules that had no findings are still present at 0.
        assert_eq!(stats.breakdown.get("ZR004").copied(), Some(0));
        assert_eq!(stats.breakdown.get("ZR007").copied(), Some(0));
    }

    #[test]
    fn format_stats_text_contains_expected_labels_and_rule_codes() {
        let report = Report {
            findings: vec![
                finding("ZR001", "tests/test_a.py", 2),
                finding("ZR003", "tests/test_a.py", 6),
            ],
            files_discovered: 2,
        };
        let stats = compute_stats(&report);
        let text = format_stats_text(&stats);

        assert!(text.starts_with("Scan statistics:"));
        assert!(text.contains("Files scanned:"), "missing `Files scanned:` label:\n{text}");
        assert!(
            text.contains("Files with findings:"),
            "missing `Files with findings:` label:\n{text}"
        );
        assert!(text.contains("Clean files:"), "missing `Clean files:` label:\n{text}");
        assert!(text.contains("Total findings:"), "missing `Total findings:` label:\n{text}");
        assert!(text.contains("Breakdown by rule:"), "missing breakdown header:\n{text}");

        // Every registered rule code appears — zero rows included.
        for rule in rules::registry::all() {
            assert!(
                text.contains(rule.code()),
                "missing rule code {} in output:\n{text}",
                rule.code()
            );
            assert!(
                text.contains(rule.name()),
                "missing rule name {} in output:\n{text}",
                rule.name()
            );
        }
    }

    #[test]
    fn format_stats_text_numeric_column_is_aligned() {
        // The numeric column across the four summary rows must line up.
        // Locate the count on each row (last whitespace-split token) and
        // record where it starts; all four starts must be equal.
        let report =
            Report { findings: vec![finding("ZR001", "tests/test_a.py", 2)], files_discovered: 10 };
        let stats = compute_stats(&report);
        let text = format_stats_text(&stats);
        let summary_labels =
            ["Files scanned:", "Files with findings:", "Clean files:", "Total findings:"];
        let mut positions = Vec::new();
        for label in summary_labels {
            let line = text
                .lines()
                .find(|l| l.contains(label))
                .unwrap_or_else(|| panic!("label {label} not found in:\n{text}"));
            let idx = line.rfind(|c: char| c.is_whitespace()).unwrap();
            positions.push(idx);
        }
        let first = positions[0];
        for p in &positions[1..] {
            assert_eq!(*p, first, "misaligned numeric column in:\n{text}");
        }
    }

    #[test]
    fn format_stats_json_round_trips_and_has_required_keys() {
        let report = Report {
            findings: vec![
                finding("ZR001", "tests/test_a.py", 2),
                finding("ZR003", "tests/test_b.py", 5),
                finding("ZR003", "tests/test_b.py", 9),
            ],
            files_discovered: 4,
        };
        let stats = compute_stats(&report);
        let json = format_stats_json(&stats);
        assert!(json.ends_with('\n'), "json must end in newline, got {json:?}");

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["files_scanned"], 4);
        assert_eq!(parsed["files_with_findings"], 2);
        assert_eq!(parsed["clean_files"], 2);
        assert_eq!(parsed["total_findings"], 3);

        let breakdown = parsed["breakdown"].as_object().expect("breakdown is object");
        assert_eq!(breakdown.get("ZR001").and_then(serde_json::Value::as_u64), Some(1));
        assert_eq!(breakdown.get("ZR003").and_then(serde_json::Value::as_u64), Some(2));
        // Every rule present at zero.
        for rule in rules::registry::all() {
            assert!(
                breakdown.contains_key(rule.code()),
                "missing rule {} in JSON breakdown: {json}",
                rule.code()
            );
        }
    }
}
