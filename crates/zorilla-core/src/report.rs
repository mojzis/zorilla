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
        buf
    }

    /// Render the findings as a JSON array.
    ///
    /// One object per finding, in the same order as
    /// [`Report::render_text`] (sorted by file, then line, then column,
    /// just like `Report.findings`). Paths are shortened relative to
    /// `base` — the same shape text emits.
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
        // fail — all fields serialize infallibly.
        serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
    }

    /// Render the findings as a SARIF 2.1.0 log document.
    ///
    /// Paths are shortened relative to `base` — the same shape text
    /// emits. No trailing summary line.
    #[must_use]
    pub fn render_sarif(&self, base: &Path) -> String {
        let results: Vec<SarifResult<'_>> = self
            .findings
            .iter()
            .map(|f| SarifResult {
                rule_id: f.code,
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
                    },
                },
                results,
            }],
        };
        serde_json::to_string_pretty(&log).unwrap_or_else(|_| "{}".to_string())
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult<'a> {
    rule_id: &'a str,
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
        let report = Report { findings: Vec::new(), files_discovered: 2 };
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
        };
        assert_eq!(report.exit_code(), 1);
    }

    #[test]
    fn severity_defaults_to_warning() {
        assert_eq!(Severity::default(), Severity::Warning);
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
        };
        let out = report.render_text(Path::new("/tmp/root"), name_lookup);
        let expected = "\
tests/test_a.py:1:1: ZR001 conditional-test-logic: test function has conditional logic (if/for/while/try)
tests/test_a.py:5:1: ZR001 conditional-test-logic: test function has conditional logic (if/for/while/try)
tests/test_b.py:3:1: ZR001 conditional-test-logic: test function has conditional logic (if/for/while/try)
3 findings in 2 files discovered.
";
        assert_eq!(out, expected);
    }

    #[test]
    fn text_output_empty_prints_only_summary() {
        let report = Report { findings: Vec::new(), files_discovered: 0 };
        let out = report.render_text(Path::new(""), name_lookup);
        assert_eq!(out, "0 findings in 0 files discovered.\n");
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
        let report = Report { findings: Vec::new(), files_discovered: 0 };
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
        assert_eq!(results[0]["level"], "warning");
        assert_eq!(results[0]["message"]["text"], "test function has conditional logic");
        let loc = &results[0]["locations"][0]["physicalLocation"];
        assert_eq!(loc["artifactLocation"]["uri"], "tests/test_a.py");
        assert_eq!(loc["region"]["startLine"], 3);
        assert_eq!(loc["region"]["startColumn"], 5);
    }

    #[test]
    fn sarif_output_empty_has_no_results() {
        let report = Report { findings: Vec::new(), files_discovered: 0 };
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
        };
        let out = report.render_text(Path::new("/tmp/test_bug.py"), name_lookup);
        assert!(
            out.starts_with("test_bug.py:2:5: "),
            "expected leading 'test_bug.py:2:5:' in {out:?}"
        );
    }
}
