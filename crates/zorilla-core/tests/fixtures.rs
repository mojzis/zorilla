//! Integration test: every rule against its `tests/fixtures/<code>/` tree.
//!
//! For every subdirectory of `tests/fixtures/` named after a rule code
//! (lowercase, e.g. `zr001`, `zr002`, `zr007`), walk every `.py` file and
//! diff the findings produced by just that rule against the paired
//! `<file>.expected.json`. Scoping to a single rule per fixture dir keeps
//! expectations focused — a ZR002 positive fixture that happens to be
//! syntactically empty shouldn't need to list ZR007 in its expectations.
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
    *registry::all()
        .iter()
        .find(|r| r.code() == code)
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
        let rule = rule_by_code(code);
        let fixtures = collect_py_fixtures(dir);
        assert!(!fixtures.is_empty(), "no .py fixtures in {}", dir.display());

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
                run_single_rule(rule, py).iter().map(ExpectedFinding::from_finding).collect();
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
