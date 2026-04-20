//! Integration test for the SARIF emitter.
//!
//! Runs `lint()` on a single offending Python file under
//! `tests/fixtures/sarif/`, serialises via `render_sarif`, parses back
//! with `serde_json`, and spot-checks the shape against the SARIF 2.1.0
//! schema requirements. Also diffs against a checked-in expected
//! document to catch silent drifts in the emitted JSON.

use std::path::{Path, PathBuf};

use zorilla_core::{lint, Config};

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("sarif")
}

#[test]
fn render_sarif_matches_schema_and_expected_document() {
    let dir = fixture_dir();
    let file = dir.join("test_positive.py");

    // The default exclude glob `**/fixtures/**` would filter the fixture
    // out — drop it for this test so the linter actually sees the file.
    let mut config = Config::default();
    config.exclude.clear();
    let report = lint(&[&file], &config).expect("lint runs");
    assert!(
        !report.findings.is_empty(),
        "fixture should produce at least one finding; got {:?}",
        report.findings
    );

    let sarif = report.render_sarif(&file);
    let parsed: serde_json::Value =
        serde_json::from_str(&sarif).expect("render_sarif emits parseable JSON");

    assert_eq!(parsed["version"], "2.1.0");
    let runs = parsed["runs"].as_array().expect("runs is array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["tool"]["driver"]["name"], "zorilla");

    let results = runs[0]["results"].as_array().expect("results is array");
    assert_eq!(results.len(), report.findings.len());
    let first = &results[0];
    assert_eq!(first["ruleId"], report.findings[0].code);
    assert_eq!(first["level"], "warning");
    let uri = first["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
        .as_str()
        .expect("uri is string");
    assert!(!uri.is_empty(), "artifactLocation.uri must be non-empty");

    // Full-document diff against the checked-in expected file. This
    // locks the shape end-to-end so a future refactor can't silently
    // drop, rename, or reorder a required field.
    //
    // Two fields are normalised before the diff:
    //
    // - `tool.driver.version` interpolates `env!("CARGO_PKG_VERSION")`
    //   at build time. Embedding the literal in the fixture would break
    //   the test on every version bump with a misleading "SARIF output
    //   diverged" message. Overwrite both sides with the build-time
    //   value so the diff stays version-agnostic.
    // - `tool.driver.rules[*].fullDescription.text` contains the full
    //   embedded `docs/rules/ZR00N.md` body (hundreds of lines across
    //   seven rules). The registry-wide catalogue shape is covered by
    //   report.rs unit tests; here we only lock the *shape* (codes +
    //   names) while normalising the markdown bodies to avoid a fragile
    //   fixture that regenerates on every doc edit.
    let expected_path = dir.join("test_positive.sarif.json");
    let expected_text =
        std::fs::read_to_string(&expected_path).expect("expected sarif fixture readable");
    let mut expected: serde_json::Value =
        serde_json::from_str(&expected_text).expect("expected sarif parses");
    let mut actual = parsed.clone();
    normalise(&mut expected);
    normalise(&mut actual);
    assert_eq!(actual, expected, "SARIF output diverged from fixture");
}

/// Strip version and full-description markdown before the fixture diff.
/// See `render_sarif_matches_schema_and_expected_document` for why.
fn normalise(v: &mut serde_json::Value) {
    if let Some(driver) = v.pointer_mut("/runs/0/tool/driver") {
        if let Some(obj) = driver.as_object_mut() {
            obj.insert("version".into(), serde_json::Value::String("<normalised>".into()));
        }
        if let Some(rules) = driver.pointer_mut("/rules").and_then(|r| r.as_array_mut()) {
            for rule in rules {
                if let Some(full) = rule.pointer_mut("/fullDescription/text") {
                    *full = serde_json::Value::String("<normalised>".into());
                }
            }
        }
    }
}
