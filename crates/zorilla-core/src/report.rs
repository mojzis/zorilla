//! Findings and reports.
//!
//! Phase 1 keeps the types here minimal — enough for the CLI to print a
//! summary. Phase 2 will grow the `Finding` side (rule code, message,
//! location) and add file-grouped text / JSON / SARIF emitters.

use std::path::PathBuf;

/// Severity of a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// A single lint finding. Reserved for Phase 2 — no rules produce
/// findings in Phase 1, but the type is here so the rest of the API can
/// be shaped correctly.
#[derive(Debug, Clone)]
pub struct Finding {
    pub code: &'static str,
    pub message: String,
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub severity: Severity,
}

/// Aggregated output of a lint run.
#[derive(Debug, Default, Clone)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub files_discovered: usize,
}

impl Report {
    /// Render the Phase 1 summary line.
    ///
    /// Shape: `"0 findings in 2 files discovered."` — deliberately matches
    /// the acceptance criterion in the Phase 1 brief.
    pub fn summary_line(&self) -> String {
        format!("{} findings in {} files discovered.", self.findings.len(), self.files_discovered)
    }

    /// Process exit code: `0` clean, `1` findings. `2` (error) is set by
    /// the CLI when it can't produce a `Report` at all.
    pub fn exit_code(&self) -> u8 {
        u8::from(!self.findings.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                code: "ZR000",
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
}
