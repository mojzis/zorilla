//! Findings and reports.
//!
//! Phase 2 grows [`Finding`] into a first-class type (rule code,
//! message, location, severity) and adds a text emitter that groups
//! findings by file before printing a trailing summary line.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Severity of a finding.
///
/// Defaults to [`Severity::Warning`] to match PLAN.md §Core abstractions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Severity {
    #[default]
    Warning,
    Error,
}

impl Severity {
    /// Lowercase string representation used by JSON and SARIF emitters.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }
}

/// A single lint finding.
#[derive(Debug, Clone)]
pub struct Finding {
    /// Rule identifier, e.g. `"ZR001"`.
    pub code: &'static str,
    /// Human-readable message.
    pub message: String,
    /// File the finding applies to.
    pub file: PathBuf,
    /// 1-indexed line number.
    pub line: usize,
    /// 1-indexed column number.
    pub column: usize,
    /// Severity level.
    pub severity: Severity,
}

/// Aggregated output of a lint run.
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub files_discovered: usize,
    /// Full list of files discovered by `lint()`, sorted by path. Used by
    /// the `overview` subcommand to enumerate clean files. `len()`
    /// matches `files_discovered`; callers that only need the count
    /// continue to read `files_discovered` for backward compatibility.
    pub discovered_files: Vec<PathBuf>,
}

impl Report {
    /// Render the trailing summary line.
    ///
    /// Shape: `"N findings in M files discovered."`.
    #[must_use]
    pub fn summary_line(&self) -> String {
        format!("{} findings in {} files discovered.", self.findings.len(), self.files_discovered)
    }

    /// Process exit code: `0` clean, `1` findings. `2` (error) is set by
    /// the CLI when it can't produce a `Report` at all.
    #[must_use]
    pub fn exit_code(&self) -> u8 {
        u8::from(!self.findings.is_empty())
    }

    /// Render the text report.
    ///
    /// Findings are grouped by file (stable, lexicographic file order),
    /// each file's findings are printed in source order
    /// (line-then-column), and the trailing summary line is always
    /// emitted. Rule names (`"conditional-test-logic"` for `ZR001`) are
    /// looked up via `rule_name`.
    ///
    /// `base` is the user-supplied root path used to shorten file paths
    /// in the output — typically the argument to `check`. Pass an empty
    /// path to skip shortening.
    pub fn render_text(&self, base: &Path, rule_name: impl Fn(&str) -> &'static str) -> String {
        let mut buf = String::new();

        // Stable group-by-file ordering: BTreeMap keyed by display path.
        let mut by_file: BTreeMap<PathBuf, Vec<&Finding>> = BTreeMap::new();
        for finding in &self.findings {
            by_file.entry(finding.file.clone()).or_default().push(finding);
        }

        for (file, mut group) in by_file {
            group.sort_by_key(|f| (f.line, f.column));
            for f in group {
                let display = display_path(&file, base);
                // Format matches the Phase 2 spec:
                //   path:line:col: CODE name: message
                let _ = writeln!(
                    buf,
                    "{}:{}:{}: {} {}: {}",
                    display.display(),
                    f.line,
                    f.column,
                    f.code,
                    rule_name(f.code),
                    f.message,
                );
            }
        }

        buf.push_str(&self.summary_line());
        buf.push('\n');
        // Only when there is something to act on: a clean run has nothing to
        // triage, and a footer under `0 findings` is noise on every green
        // build. Text only — JSON and SARIF go to parsers.
        if !self.findings.is_empty() {
            crate::guide::push_report_footer(&mut buf);
        }
        buf
    }

    /// Render the findings as a JSON array.
    ///
    /// One object per finding, in the same order as
    /// [`Report::render_text`] (sorted by file, then line, then column,
    /// just like `Report.findings`). Paths are shortened relative to
    /// `base` — the same shape text emits. Output ends with a trailing
    /// newline to match [`Report::render_text`] — callers can use
    /// `print!` for all formats without double-newline surprises.
    ///
    /// No trailing summary line — callers that want the summary should
    /// use `render_text`.
    #[must_use]
    pub fn render_json(&self, base: &Path) -> String {
        let items: Vec<FindingJson<'_>> = self
            .findings
            .iter()
            .map(|f| FindingJson {
                code: f.code,
                message: &f.message,
                file: display_path(&f.file, base).display().to_string(),
                line: f.line,
                column: f.column,
                severity: f.severity.as_str(),
            })
            .collect();
        // `serde_json::to_string_pretty` over a `Vec<FindingJson>` cannot
        // fail at runtime — every field is `&str` / `usize` / owned
        // `String` with no custom `Serialize` impl, no non-string map
        // key, and no float. If this ever does fail, we'd rather abort
        // than silently emit `[]` (which would lie about the finding
        // count and mask a serious bug).
        let mut out = serde_json::to_string_pretty(&items)
            .expect("FindingJson serializes infallibly; see Report::render_json invariants");
        out.push('\n');
        out
    }

    /// Render the findings as a SARIF 2.1.0 log document.
    ///
    /// Paths are shortened relative to `base` — the same shape text
    /// emits. Output ends with a trailing newline to match
    /// [`Report::render_text`]. No trailing summary line.
    ///
    /// `runs[0].tool.driver.rules` is populated from
    /// [`crate::rules::registry::all`] so SARIF consumers (GitHub
    /// code-scanning, Sonar, …) can display each rule's human name and
    /// long-form description on hover. Each result emits a `ruleIndex`
    /// pointing into this array; unknown codes (shouldn't happen for
    /// engine-produced findings, but keep the mapping defensive) fall
    /// back to emitting no `ruleIndex` rather than panicking.
    #[must_use]
    pub fn render_sarif(&self, base: &Path) -> String {
        // Build the reportingDescriptor catalogue once per run, and a
        // parallel `code -> index` map for per-result lookup.
        let registry = crate::rules::registry::all();
        let rules_meta: Vec<SarifReportingDescriptor<'_>> = registry
            .iter()
            .map(|r| SarifReportingDescriptor {
                id: r.code(),
                name: r.name(),
                short_description: SarifMessage { text: r.name() },
                full_description: SarifMessage { text: r.doc() },
            })
            .collect();
        // Linear scan over 7 rules is faster than building a HashMap;
        // revisit only if the registry grows past a couple dozen.
        let index_of =
            |code: &str| -> Option<usize> { registry.iter().position(|r| r.code() == code) };

        let results: Vec<SarifResult<'_>> = self
            .findings
            .iter()
            .map(|f| SarifResult {
                rule_id: f.code,
                rule_index: index_of(f.code),
                level: f.severity.as_str(),
                message: SarifMessage { text: &f.message },
                locations: vec![SarifLocation {
                    physical_location: SarifPhysicalLocation {
                        artifact_location: SarifArtifactLocation {
                            uri: display_path(&f.file, base).display().to_string(),
                        },
                        region: SarifRegion { start_line: f.line, start_column: f.column },
                    },
                }],
            })
            .collect();

        let log = SarifLog {
            schema: "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
            version: "2.1.0",
            runs: vec![SarifRun {
                tool: SarifTool {
                    driver: SarifDriver {
                        name: "zorilla",
                        version: env!("CARGO_PKG_VERSION"),
                        information_uri: "https://github.com/mojzis/zorilla",
                        rules: rules_meta,
                    },
                },
                results,
            }],
        };
        // See `render_json` for why we `expect` — same infallibility
        // argument holds for `SarifLog`. Silent fallback to `"{}"` would
        // hide regressions far worse than a panic.
        let mut out = serde_json::to_string_pretty(&log)
            .expect("SarifLog serializes infallibly; see Report::render_sarif invariants");
        out.push('\n');
        out
    }
}

#[derive(Serialize)]
struct FindingJson<'a> {
    code: &'a str,
    message: &'a str,
    file: String,
    line: usize,
    column: usize,
    severity: &'a str,
}

#[derive(Serialize)]
struct SarifLog<'a> {
    #[serde(rename = "$schema")]
    schema: &'a str,
    version: &'a str,
    runs: Vec<SarifRun<'a>>,
}

#[derive(Serialize)]
struct SarifRun<'a> {
    tool: SarifTool<'a>,
    results: Vec<SarifResult<'a>>,
}

#[derive(Serialize)]
struct SarifTool<'a> {
    driver: SarifDriver<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifDriver<'a> {
    name: &'a str,
    version: &'a str,
    information_uri: &'a str,
    /// Rule catalogue; see `Report::render_sarif` for construction.
    rules: Vec<SarifReportingDescriptor<'a>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifReportingDescriptor<'a> {
    /// Rule code (matches `result.ruleId`), e.g. `"ZR001"`.
    id: &'a str,
    /// Kebab-case human name, e.g. `"conditional-test-logic"`.
    name: &'a str,
    /// Short summary — SARIF viewers show this inline next to the id.
    short_description: SarifMessage<'a>,
    /// Long-form markdown (the embedded `docs/rules/ZR00N.md`) — shown
    /// on rule hover / in the rule catalogue. Markdown is permitted per
    /// SARIF §3.49.6.
    full_description: SarifMessage<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult<'a> {
    rule_id: &'a str,
    /// Index into `runs[0].tool.driver.rules`, when the rule code is
    /// known. `None` (elided from JSON) for findings whose code is
    /// missing from the registry — defensive, not expected in practice.
    #[serde(skip_serializing_if = "Option::is_none")]
    rule_index: Option<usize>,
    level: &'a str,
    message: SarifMessage<'a>,
    locations: Vec<SarifLocation>,
}

#[derive(Serialize)]
struct SarifMessage<'a> {
    text: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLocation {
    physical_location: SarifPhysicalLocation,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifPhysicalLocation {
    artifact_location: SarifArtifactLocation,
    region: SarifRegion,
}

#[derive(Serialize)]
struct SarifArtifactLocation {
    uri: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRegion {
    start_line: usize,
    start_column: usize,
}

fn display_path(file: &Path, base: &Path) -> PathBuf {
    if base.as_os_str().is_empty() {
        return file.to_path_buf();
    }
    // `file.strip_prefix(base)` returns an empty path when `file == base`
    // (the CLI passed a single file as the only argument). An empty
    // display path renders as `":line:col: ..."` which is useless — fall
    // back to the file's basename, or to the full `file` if there's no
    // file name component (e.g. `/`).
    match file.strip_prefix(base) {
        Ok(rel) if rel.as_os_str().is_empty() => {
            file.file_name().map_or_else(|| file.to_path_buf(), PathBuf::from)
        }
        Ok(rel) => rel.to_path_buf(),
        Err(_) => file.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_lookup(code: &str) -> &'static str {
        match code {
            "ZR001" => "conditional-test-logic",
            "ZR002" => "sleep-in-test",
            "ZR007" => "empty-test",
            _ => "unknown",
        }
    }

    #[test]
    fn summary_line_shape_is_stable() {
        let report =
            Report { findings: Vec::new(), files_discovered: 2, discovered_files: Vec::new() };
        assert_eq!(report.summary_line(), "0 findings in 2 files discovered.");
    }

    #[test]
    fn zero_findings_exits_zero() {
        let report = Report::default();
        assert_eq!(report.exit_code(), 0);
    }

    #[test]
    fn findings_exit_one() {
        let report = Report {
            findings: vec![Finding {
                code: "ZR001",
                message: "placeholder".into(),
                file: PathBuf::from("x.py"),
                line: 1,
                column: 1,
                severity: Severity::Warning,
            }],
            files_discovered: 1,
            discovered_files: Vec::new(),
        };
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn severity_defaults_to_warning() {
        assert_eq!(Severity::default(), Severity::Warning);
    }

    #[test]
    fn severity_as_str_maps_both_variants() {
        // Locks the lowercase strings consumed by the SARIF `level`
        // enumeration (`warning` / `error`) and the JSON emitter. If a
        // future variant is added, this assertion forces the author to
        // update emitter tests too.
        assert_eq!(Severity::Warning.as_str(), "warning");
        assert_eq!(Severity::Error.as_str(), "error");
    }

    #[test]
    fn json_and_sarif_emit_error_severity_end_to_end() {
        // Regression guard: no rule currently emits `Severity::Error`,
        // so without this test the `error` arm of `Severity::as_str`
        // would never round-trip through the JSON / SARIF emitters.
        let report = Report {
            findings: vec![Finding {
                code: "ZR099",
                message: "boom".into(),
                file: PathBuf::from("/tmp/root/tests/test_err.py"),
                line: 1,
                column: 1,
                severity: Severity::Error,
            }],
            files_discovered: 1,
            discovered_files: Vec::new(),
        };

        let json = report.render_json(Path::new("/tmp/root"));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["severity"], "error");

        let sarif = report.render_sarif(Path::new("/tmp/root"));
        let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();
        assert_eq!(parsed["runs"][0]["results"][0]["level"], "error");
    }

    #[test]
    fn json_and_sarif_end_with_single_trailing_newline() {
        // Keeps the three renderers consistent so CLI callers can use
        // `print!` uniformly without double-newline surprises.
        let report =
            Report { findings: Vec::new(), files_discovered: 0, discovered_files: Vec::new() };
        let json = report.render_json(Path::new(""));
        assert!(json.ends_with('\n'), "json should end with \\n, got {json:?}");
        assert!(!json.ends_with("\n\n"), "json should not end with double \\n, got {json:?}");
        let sarif = report.render_sarif(Path::new(""));
        assert!(sarif.ends_with('\n'), "sarif should end with \\n, got {sarif:?}");
        assert!(!sarif.ends_with("\n\n"), "sarif should not end with double \\n, got {sarif:?}");
    }

    #[test]
    fn text_output_groups_by_file_and_trailing_summary() {
        let report = Report {
            findings: vec![
                Finding {
                    code: "ZR001",
                    message: "test function has conditional logic (if/for/while/try)".into(),
                    file: PathBuf::from("/tmp/root/tests/test_b.py"),
                    line: 3,
                    column: 1,
                    severity: Severity::Warning,
                },
                Finding {
                    code: "ZR001",
                    message: "test function has conditional logic (if/for/while/try)".into(),
                    file: PathBuf::from("/tmp/root/tests/test_a.py"),
                    line: 1,
                    column: 1,
                    severity: Severity::Warning,
                },
                Finding {
                    code: "ZR001",
                    message: "test function has conditional logic (if/for/while/try)".into(),
                    file: PathBuf::from("/tmp/root/tests/test_a.py"),
                    line: 5,
                    column: 1,
                    severity: Severity::Warning,
                },
            ],
            files_discovered: 2,
            discovered_files: Vec::new(),
        };
        let out = report.render_text(Path::new("/tmp/root"), name_lookup);
        // The footer is composed from `guide::report_footer` rather than
        // spelled out, so rewording it stays a one-file change.
        let expected = format!(
            "\
tests/test_a.py:1:1: ZR001 conditional-test-logic: test function has conditional logic (if/for/while/try)
tests/test_a.py:5:1: ZR001 conditional-test-logic: test function has conditional logic (if/for/while/try)
tests/test_b.py:3:1: ZR001 conditional-test-logic: test function has conditional logic (if/for/while/try)
3 findings in 2 files discovered.

{}
",
            crate::guide::report_footer(),
        );
        assert_eq!(out, expected);
    }

    #[test]
    fn text_output_empty_prints_only_summary() {
        let report =
            Report { findings: Vec::new(), files_discovered: 0, discovered_files: Vec::new() };
        let out = report.render_text(Path::new(""), name_lookup);
        assert_eq!(out, "0 findings in 0 files discovered.\n");
    }

    #[test]
    fn text_output_with_findings_points_at_the_triage_guide() {
        // A failed gate captures stdout; the footer is the only breadcrumb
        // from that captured text to the instructions for acting on it.
        let report = Report {
            findings: vec![Finding {
                code: "ZR001",
                message: "test function has conditional logic".into(),
                file: PathBuf::from("tests/test_a.py"),
                line: 2,
                column: 5,
                severity: Severity::Warning,
            }],
            files_discovered: 1,
            discovered_files: Vec::new(),
        };
        let out = report.render_text(Path::new(""), name_lookup);
        assert!(
            out.contains("zorilla guide triage"),
            "footer should point at the triage guide, got:\n{out}"
        );
    }

    #[test]
    fn json_and_sarif_omit_the_triage_footer() {
        // Both formats feed parsers. A trailing prose line would break them.
        let report = Report {
            findings: vec![Finding {
                code: "ZR001",
                message: "test function has conditional logic".into(),
                file: PathBuf::from("tests/test_a.py"),
                line: 2,
                column: 5,
                severity: Severity::Warning,
            }],
            files_discovered: 1,
            discovered_files: Vec::new(),
        };
        for rendered in [report.render_json(Path::new("")), report.render_sarif(Path::new(""))] {
            assert!(
                !rendered.contains("zorilla guide triage"),
                "machine formats carry no footer, got:\n{rendered}"
            );
        }
    }

    #[test]
    fn json_output_has_one_object_per_finding_and_shares_text_paths() {
        let report = Report {
            findings: vec![
                Finding {
                    code: "ZR001",
                    message: "test function has conditional logic".into(),
                    file: PathBuf::from("/tmp/root/tests/test_a.py"),
                    line: 3,
                    column: 5,
                    severity: Severity::Warning,
                },
                Finding {
                    code: "ZR002",
                    message: "sleep in test".into(),
                    file: PathBuf::from("/tmp/root/tests/test_b.py"),
                    line: 7,
                    column: 9,
                    severity: Severity::Warning,
                },
            ],
            files_discovered: 2,
            discovered_files: Vec::new(),
        };
        let json = report.render_json(Path::new("/tmp/root"));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["code"], "ZR001");
        assert_eq!(arr[0]["file"], "tests/test_a.py");
        assert_eq!(arr[0]["line"], 3);
        assert_eq!(arr[0]["column"], 5);
        assert_eq!(arr[0]["severity"], "warning");
        assert_eq!(arr[1]["code"], "ZR002");
        assert_eq!(arr[1]["file"], "tests/test_b.py");
    }

    #[test]
    fn json_output_empty_is_empty_array() {
        let report =
            Report { findings: Vec::new(), files_discovered: 0, discovered_files: Vec::new() };
        let json = report.render_json(Path::new(""));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.as_array().unwrap().is_empty());
    }

    #[test]
    fn sarif_output_has_expected_shape() {
        let report = Report {
            findings: vec![Finding {
                code: "ZR001",
                message: "test function has conditional logic".into(),
                file: PathBuf::from("/tmp/root/tests/test_a.py"),
                line: 3,
                column: 5,
                severity: Severity::Warning,
            }],
            files_discovered: 1,
            discovered_files: Vec::new(),
        };
        let sarif = report.render_sarif(Path::new("/tmp/root"));
        let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();
        assert_eq!(parsed["version"], "2.1.0");
        assert_eq!(
            parsed["$schema"],
            "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json"
        );
        let runs = parsed["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["tool"]["driver"]["name"], "zorilla");
        assert_eq!(
            runs[0]["tool"]["driver"]["informationUri"],
            "https://github.com/mojzis/zorilla"
        );
        let results = runs[0]["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["ruleId"], "ZR001");
        // ZR001 is the first entry in the registry, so its index is 0.
        assert_eq!(results[0]["ruleIndex"], 0);
        assert_eq!(results[0]["level"], "warning");
        assert_eq!(results[0]["message"]["text"], "test function has conditional logic");
        let loc = &results[0]["locations"][0]["physicalLocation"];
        assert_eq!(loc["artifactLocation"]["uri"], "tests/test_a.py");
        assert_eq!(loc["region"]["startLine"], 3);
        assert_eq!(loc["region"]["startColumn"], 5);
    }

    #[test]
    fn sarif_driver_rules_lists_every_registered_rule() {
        // Proof that `tool.driver.rules` is populated with the full
        // registry — GitHub code-scanning / Sonar consume this to render
        // human-readable titles and long-form descriptions on hover.
        let report =
            Report { findings: Vec::new(), files_discovered: 0, discovered_files: Vec::new() };
        let sarif = report.render_sarif(Path::new(""));
        let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();
        let rules = parsed["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap();
        let codes: Vec<&str> =
            rules.iter().map(|r| r["id"].as_str().expect("id is string")).collect();
        assert_eq!(
            codes,
            vec!["ZR001", "ZR002", "ZR003", "ZR004", "ZR005", "ZR006", "ZR007", "ZR008"]
        );
        // Every descriptor must carry a non-empty short + full
        // description; otherwise SARIF viewers fall back to the bare id,
        // defeating the point of populating the catalogue.
        for r in rules {
            assert!(!r["name"].as_str().unwrap().is_empty());
            assert!(!r["shortDescription"]["text"].as_str().unwrap().is_empty());
            let full = r["fullDescription"]["text"].as_str().unwrap();
            assert!(
                full.starts_with("# ZR"),
                "fullDescription must be the embedded doc, got: {full:.40}"
            );
        }
    }

    #[test]
    fn sarif_result_rule_index_is_elided_for_unknown_codes() {
        // Defensive: a hand-constructed Finding with a code not in the
        // registry must not break serialization and must not ship a
        // nonsense `ruleIndex`. Engine-produced findings always have
        // registered codes, but this guards against test / programmatic
        // construction.
        let report = Report {
            findings: vec![Finding {
                code: "ZR999",
                message: "synthetic".into(),
                file: PathBuf::from("/tmp/root/tests/test_a.py"),
                line: 1,
                column: 1,
                severity: Severity::Warning,
            }],
            files_discovered: 1,
            discovered_files: Vec::new(),
        };
        let sarif = report.render_sarif(Path::new("/tmp/root"));
        let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();
        let result = &parsed["runs"][0]["results"][0];
        assert_eq!(result["ruleId"], "ZR999");
        assert!(result.get("ruleIndex").is_none(), "ruleIndex should be elided for unknown codes");
    }

    #[test]
    fn sarif_output_empty_has_no_results() {
        let report =
            Report { findings: Vec::new(), files_discovered: 0, discovered_files: Vec::new() };
        let sarif = report.render_sarif(Path::new(""));
        let parsed: serde_json::Value = serde_json::from_str(&sarif).unwrap();
        assert!(parsed["runs"][0]["results"].as_array().unwrap().is_empty());
    }

    #[test]
    fn text_output_when_base_equals_file_falls_back_to_basename() {
        // Regression: when the CLI is invoked as `zorilla check
        // /tmp/test_bug.py`, `base` and `file` point to the same path, so
        // `strip_prefix` returns an empty path. Previously that rendered
        // as `:2:5: ZR001 …`; it should render as `test_bug.py:2:5: …`.
        let report = Report {
            findings: vec![Finding {
                code: "ZR001",
                message: "test function has conditional logic (if/for/while/try)".into(),
                file: PathBuf::from("/tmp/test_bug.py"),
                line: 2,
                column: 5,
                severity: Severity::Warning,
            }],
            files_discovered: 1,
            discovered_files: Vec::new(),
        };
        let out = report.render_text(Path::new("/tmp/test_bug.py"), name_lookup);
        assert!(
            out.starts_with("test_bug.py:2:5: "),
            "expected leading 'test_bug.py:2:5:' in {out:?}"
        );
    }
}
