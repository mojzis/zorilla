//! Integration test: every rule against its `tests/fixtures/<code>/` tree.
//!
//! For every subdirectory of `tests/fixtures/` named after a rule code
//! (lowercase, e.g. `zr001`, `zr002`, `zr007`), walk every `.py` file and
//! diff the findings produced by just that rule against the paired
//! `<file>.expected.json`. Scoping to a single rule per fixture dir keeps
//! expectations focused — a ZR002 positive fixture that happens to be
//! syntactically empty shouldn't need to list ZR007 in its expectations.
//!
//! The special directory `zr_suppress/` exercises the suppression parser
//! end-to-end: every registered rule runs on each `.py` file (mirroring
//! `lint_one_file` — short-circuit on `# zorilla: ignore-file`, run all
//! enabled rules, then filter findings via `Suppressions::is_suppressed`).
//! This is the only fixture dir whose `.expected.json` may legitimately
//! enumerate findings from multiple rules.
//!
//! This harness is what catches regressions the per-module unit tests
//! miss: it proves each rule participates correctly in
//! [`zorilla_core::parse`] + `Rule::check` on real file contents.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use zorilla_core::parse::parse;
use zorilla_core::rules::registry;
use zorilla_core::rules::{Context, Rule, RuleConfig};
use zorilla_core::suppress::Suppressions;
use zorilla_core::Finding;

#[derive(Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct ExpectedFinding {
    code: String,
    line: usize,
    column: usize,
}

impl ExpectedFinding {
    fn from_finding(f: &Finding) -> Self {
        Self { code: f.code.to_string(), line: f.line, column: f.column }
    }
}

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

fn rule_by_code(code: &str) -> &'static dyn Rule {
    registry::find(code)
        .unwrap_or_else(|| panic!("fixture dir {code} has no matching rule in registry"))
}

fn run_single_rule(rule: &dyn Rule, path: &Path) -> Vec<Finding> {
    let source = std::fs::read_to_string(path).expect("fixture read");
    let tree = parse(&source).expect("fixture parse");
    let suppressions = Suppressions::empty();
    let config = RuleConfig::default();
    let ctx = Context {
        file: path,
        source: &source,
        tree: &tree,
        config: &config,
        suppressions: &suppressions,
    };
    let mut out = Vec::new();
    rule.check(&ctx, &mut out);
    out
}

/// Mirror of `zorilla_core::lint_one_file` for fixture-level
/// integration: parse suppressions, short-circuit per code via
/// `suppresses_code`, run every registered rule (modulo
/// `RuleConfig.disabled` and `default_enabled`), then filter by
/// suppressions. Used only by the `zr_suppress/` fixture dir, where
/// expectations cover findings produced by *any* rule.
fn run_all_rules_with_suppressions(path: &Path) -> Vec<Finding> {
    let source = std::fs::read_to_string(path).expect("fixture read");
    let tree = parse(&source).expect("fixture parse");
    let suppressions = Suppressions::from_source(&source);
    let config = RuleConfig::default();
    let ctx = Context {
        file: path,
        source: &source,
        tree: &tree,
        config: &config,
        suppressions: &suppressions,
    };
    let mut out = Vec::new();
    for rule in registry::all() {
        // Mirror the engine's filters in the same order as
        // `lint_one_file`: per-rule disable first, then default-enabled,
        // then file-scope `ignore-file[<code>]` short-circuit.
        if config.disabled.contains(rule.code()) {
            continue;
        }
        if !rule.default_enabled() {
            continue;
        }
        if suppressions.suppresses_code(rule.code()) {
            continue;
        }
        rule.check(&ctx, &mut out);
    }
    out.retain(|f| !suppressions.is_suppressed(f.line, f.code));
    out
}

fn load_expected(path: &Path) -> Vec<ExpectedFinding> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    let mut parsed: Vec<ExpectedFinding> =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()));
    parsed.sort();
    parsed
}

fn collect_py_fixtures(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("fixture dir") {
        let entry = entry.expect("dir entry");
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "py") {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn collect_rule_dirs() -> Vec<(String, PathBuf)> {
    let root = fixtures_root();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root).expect("fixtures root") {
        let entry = entry.expect("dir entry");
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        // Only rule-scoped fixture dirs (`zrNNN`) and the suppression
        // harness dir (`zr_suppress`) participate in the per-rule loop.
        // Other dirs under `tests/fixtures/` — e.g. `sarif/` used by the
        // emitter integration test — are skipped here.
        if !name.to_ascii_lowercase().starts_with("zr") {
            continue;
        }
        out.push((name.to_ascii_uppercase(), p));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[test]
fn every_fixture_matches_its_expected_json() {
    let dirs = collect_rule_dirs();
    assert!(!dirs.is_empty(), "no rule fixture dirs under {:?}", fixtures_root());

    let mut mismatches: BTreeMap<PathBuf, (Vec<ExpectedFinding>, Vec<ExpectedFinding>)> =
        BTreeMap::new();

    for (code, dir) in &dirs {
        let fixtures = collect_py_fixtures(dir);
        assert!(!fixtures.is_empty(), "no .py fixtures in {}", dir.display());

        // The `zr_suppress` dir is special: it covers the suppression
        // parser and short-circuit, so every registered rule must run.
        // Every other dir is named after a single rule code.
        let run: Box<dyn Fn(&Path) -> Vec<Finding>> = if code == "ZR_SUPPRESS" {
            Box::new(run_all_rules_with_suppressions)
        } else {
            let rule = rule_by_code(code);
            Box::new(move |p: &Path| run_single_rule(rule, p))
        };

        for py in &fixtures {
            let expected_path = py.with_file_name(format!(
                "{}.expected.json",
                py.file_name().expect("file name").to_string_lossy()
            ));
            assert!(
                expected_path.is_file(),
                "missing expected JSON for {}: looked for {}",
                py.display(),
                expected_path.display()
            );

            let expected = load_expected(&expected_path);
            let mut actual: Vec<ExpectedFinding> =
                run(py).iter().map(ExpectedFinding::from_finding).collect();
            actual.sort();

            if actual != expected {
                mismatches.insert(py.clone(), (expected, actual));
            }
        }
    }

    assert!(
        mismatches.is_empty(),
        "fixture diffs:\n{}",
        mismatches
            .iter()
            .map(|(path, (expected, actual))| format!(
                "  {}:\n    expected: {expected:?}\n    actual:   {actual:?}",
                path.display()
            ))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
