//! Integration test: ZR001 against the `tests/fixtures/zr001/` tree.
//!
//! For every `.py` file in the fixtures directory, read the paired
//! `<file>.expected.json` and diff it against the findings the real
//! engine (parse + rule) produces. This is the layer that catches
//! regressions the per-module unit tests miss — most importantly, that
//! the rule participates correctly in [`zorilla_core::lint`]'s pipeline.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use zorilla_core::parse::parse;
use zorilla_core::rules::zr001_conditional::ZR001_CONDITIONAL;
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

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("zr001")
}

fn run_zr001_on_file(path: &Path) -> Vec<Finding> {
    let source = std::fs::read_to_string(path).expect("fixture read");
    let tree = parse(&source).expect("fixture parse");
    let suppressions = Suppressions::empty();
    let config = RuleConfig;
    let ctx = Context {
        file: path,
        source: &source,
        tree: &tree,
        config: &config,
        suppressions: &suppressions,
    };
    let mut out = Vec::new();
    ZR001_CONDITIONAL.check(&ctx, &mut out);
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

fn collect_py_fixtures() -> Vec<PathBuf> {
    let dir = fixtures_dir();
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("fixtures dir") {
        let entry = entry.expect("dir entry");
        let p = entry.path();
        if p.extension().is_some_and(|e| e == "py") {
            out.push(p);
        }
    }
    out.sort();
    out
}

#[test]
fn every_zr001_fixture_matches_its_expected_json() {
    let fixtures = collect_py_fixtures();
    assert!(!fixtures.is_empty(), "no .py fixtures found in {:?}", fixtures_dir());

    // Collect mismatches and report all of them at once — easier to
    // debug than failing on the first divergence.
    let mut mismatches: BTreeMap<PathBuf, (Vec<ExpectedFinding>, Vec<ExpectedFinding>)> =
        BTreeMap::new();

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
            run_zr001_on_file(py).iter().map(ExpectedFinding::from_finding).collect();
        actual.sort();

        if actual != expected {
            mismatches.insert(py.clone(), (expected, actual));
        }
    }

    assert!(
        mismatches.is_empty(),
        "ZR001 fixture diffs:\n{}",
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
