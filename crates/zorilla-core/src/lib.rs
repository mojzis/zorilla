//! `zorilla-core` — library crate that powers the `zorilla` CLI.
//!
//! Phase 3 surface: configuration loading, file discovery, tree-sitter
//! parsing, a rule framework with the first real rule
//! (`ZR001 conditional-test-logic`), and a text emitter that renders a
//! grouped [`Report`].

pub mod ast;
pub mod config;
pub mod discovery;
pub mod parse;
pub mod report;
pub mod rules;
pub mod suppress;

use std::path::Path;

use rayon::prelude::*;

pub use config::Config;
pub use discovery::{discover, DiscoveryError};
pub use parse::ParseError;
pub use report::{Finding, Report, Severity};
pub use rules::{Context, Rule, RuleConfig};
pub use suppress::Suppressions;

/// Error surfaced to callers of [`lint`].
#[derive(Debug, thiserror::Error)]
pub enum LintError {
    /// File discovery failed (e.g. glob compilation, walker IO).
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
}

/// Run the linter against `paths` using `config`.
///
/// Pipeline:
/// 1. discover every Python file under `paths`;
/// 2. in parallel (`rayon`), read + parse each file and run every
///    enabled rule in [`rules::registry::all`];
/// 3. merge the per-file findings into a single [`Report`], sorted by
///    `(file, line, column)` so the ordering is stable across runs,
///    platforms, and rayon thread counts.
///
/// Files that fail to read or parse are silently skipped in Phase 2 —
/// PLAN.md Step 5 introduces diagnostics for these cases.
pub fn lint<P: AsRef<Path>>(paths: &[P], config: &Config) -> Result<Report, LintError> {
    let files = discover(paths, config)?;
    let files_discovered = files.len();

    let rule_config = RuleConfig;
    let registry = rules::registry::all();

    let mut findings: Vec<Finding> =
        files.par_iter().flat_map(|file| lint_one_file(file, registry, rule_config)).collect();
    // `discover` returns files in OS-dependent directory-walk order, so
    // the rayon output — though order-preserving w.r.t. its input — is
    // not canonical. Sort here so every consumer of `Report.findings`
    // (text emitter, future JSON emitter, tests) sees the same order.
    findings.sort_by(|a, b| {
        a.file.cmp(&b.file).then(a.line.cmp(&b.line)).then(a.column.cmp(&b.column))
    });

    Ok(Report { findings, files_discovered })
}

fn lint_one_file(
    file: &Path,
    registry: &[&'static dyn Rule],
    rule_config: RuleConfig,
) -> Vec<Finding> {
    let Ok(source) = std::fs::read_to_string(file) else {
        return Vec::new();
    };
    let Ok(tree) = parse::parse(&source) else {
        return Vec::new();
    };
    let suppressions = Suppressions::empty();

    let ctx = Context {
        file,
        source: &source,
        tree: &tree,
        config: &rule_config,
        suppressions: &suppressions,
    };

    let mut out = Vec::new();
    for rule in registry {
        if !rule.default_enabled() {
            continue;
        }
        rule.check(&ctx, &mut out);
    }
    out
}

/// Look up a rule name by its code. Used by the text emitter to render
/// `ZR001 conditional-test-logic` style lines without forcing the
/// `Report` type to hold a rule-name slice per finding.
#[must_use]
pub fn rule_name_for(code: &str) -> &'static str {
    rules::registry::all().iter().find(|r| r.code() == code).map_or("unknown", |r| r.name())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn lint_fires_zr001_on_test_functions_with_conditionals() {
        let tmp = TempDir::new().unwrap();
        let tests = tmp.path().join("tests");
        std::fs::create_dir_all(&tests).unwrap();
        // Test function with a conditional — should produce one finding.
        std::fs::write(
            tests.join("test_a.py"),
            "def test_one():\n    if True:\n        assert True\n",
        )
        .unwrap();
        // Test function without conditionals — no finding.
        std::fs::write(tests.join("test_b.py"), "def test_two():\n    assert True\n").unwrap();

        let config = Config::default();
        let report = lint(&[tmp.path()], &config).unwrap();
        assert_eq!(report.files_discovered, 2);
        assert_eq!(report.findings.len(), 1);
        assert_eq!(report.findings[0].code, "ZR001");
        assert!(report.findings[0].file.ends_with("test_a.py"));
    }

    #[test]
    fn rule_name_for_known_code_works() {
        assert_eq!(rule_name_for("ZR001"), "conditional-test-logic");
        assert_eq!(rule_name_for("ZR002"), "sleep-in-test");
        assert_eq!(rule_name_for("ZR007"), "empty-test");
        assert_eq!(rule_name_for("ZRwhatever"), "unknown");
    }

    #[test]
    fn findings_are_sorted_by_file_then_line() {
        // Multiple files, each with conditional-bearing test functions at
        // different lines. After `lint()` we expect findings sorted by
        // file path, then by line, then by column — regardless of how
        // discovery / rayon happened to order them internally.
        let tmp = TempDir::new().unwrap();
        let tests = tmp.path().join("tests");
        std::fs::create_dir_all(&tests).unwrap();
        // Two test functions with conditionals at offending-statement
        // lines 2 and 6.
        std::fs::write(
            tests.join("test_b.py"),
            "def test_b1():\n    if True:\n        pass\n\n\
             def test_b2():\n    for i in (1,):\n        pass\n",
        )
        .unwrap();
        // One test function with a conditional on line 2.
        std::fs::write(tests.join("test_a.py"), "def test_a():\n    if True:\n        pass\n")
            .unwrap();

        let config = Config::default();
        let report = lint(&[tmp.path()], &config).unwrap();
        let keys: Vec<(String, usize)> = report
            .findings
            .iter()
            .map(|f| (f.file.file_name().unwrap().to_string_lossy().into_owned(), f.line))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("test_a.py".to_string(), 2),
                ("test_b.py".to_string(), 2),
                ("test_b.py".to_string(), 6),
            ]
        );
    }
}
