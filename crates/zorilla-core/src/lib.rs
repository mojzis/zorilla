//! `zorilla-core` — library crate that powers the `zorilla` CLI.
//!
//! Phase 2 surface: configuration loading, file discovery, tree-sitter
//! parsing, a rule framework with a single temporary rule
//! (`ZR000 hello-world`), and a text emitter that renders a grouped
//! [`Report`].

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
/// 3. merge the per-file findings into a single [`Report`].
///
/// Files that fail to read or parse are silently skipped in Phase 2 —
/// PLAN.md Step 5 introduces diagnostics for these cases.
pub fn lint<P: AsRef<Path>>(paths: &[P], config: &Config) -> Result<Report, LintError> {
    let files = discover(paths, config)?;
    let files_discovered = files.len();

    let rule_config = RuleConfig;
    let registry = rules::registry::all();

    let findings: Vec<Finding> =
        files.par_iter().flat_map(|file| lint_one_file(file, registry, rule_config)).collect();

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
/// `ZR000 hello-world` style lines without forcing the `Report` type to
/// hold a rule-name slice per finding.
#[must_use]
pub fn rule_name_for(code: &str) -> &'static str {
    rules::registry::all().iter().find(|r| r.code() == code).map_or("unknown", |r| r.name())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn lint_fires_zr000_on_test_functions_in_discovered_files() {
        let tmp = TempDir::new().unwrap();
        let tests = tmp.path().join("tests");
        std::fs::create_dir_all(&tests).unwrap();
        std::fs::write(
            tests.join("test_a.py"),
            "def test_one():\n    pass\n\ndef test_two():\n    pass\n",
        )
        .unwrap();
        std::fs::write(tests.join("test_b.py"), "def _helper():\n    pass\n").unwrap();

        let config = Config::default();
        let report = lint(&[tmp.path()], &config).unwrap();
        assert_eq!(report.files_discovered, 2);
        assert_eq!(report.findings.len(), 2);
        assert!(report.findings.iter().all(|f| f.code == "ZR000"));
    }

    #[test]
    fn rule_name_for_known_code_works() {
        assert_eq!(rule_name_for("ZR000"), "hello-world");
        assert_eq!(rule_name_for("ZRwhatever"), "unknown");
    }
}
