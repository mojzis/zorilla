//! Findings and reports.
//!
//! Phase 2 grows [`Finding`] into a first-class type (rule code,
//! message, location, severity) and adds a text emitter that groups
//! findings by file before printing a trailing summary line.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// Severity of a finding.
///
/// Defaults to [`Severity::Warning`] to match PLAN.md §Core abstractions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Severity {
    #[default]
    Warning,
    Error,
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
