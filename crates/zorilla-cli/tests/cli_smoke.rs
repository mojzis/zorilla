//! End-to-end smoke test for the `zorilla` binary.
//!
//! Phase 3 shape: `zorilla check <dir>` runs the rule pipeline. On an
//! empty tree it exits 0 and prints a zero-findings summary; on a tree
//! with a conditional-bearing test function it exits 1 and prints the
//! `ZR001 conditional-test-logic` finding plus the trailing summary line.

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

#[test]
fn check_on_empty_directory_reports_zero_findings_and_exits_zero() {
    let tmp = TempDir::new().unwrap();

    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(contains("0 findings in 0 files discovered."));
}

#[test]
fn check_on_tree_with_plain_tests_reports_zero_findings_and_exits_zero() {
    // Phase 2's temporary pipeline-proof rule fired on every test
    // function. With that rule retired and ZR001 only firing on
    // conditionals, a plain test body produces no findings.
    let tmp = TempDir::new().unwrap();
    let tests = tmp.path().join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("test_plain.py"), "def test_x():\n    assert True\n").unwrap();

    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(contains("0 findings in 1 files discovered."));
}

#[test]
fn check_on_single_file_argument_renders_basename_not_empty_path() {
    // Regression: `zorilla check /tmp/test_bug.py` used to emit
    // `:2:5: ZR001 ...` because `file.strip_prefix(base)` returns an
    // empty path when base == file. Now the path falls back to the
    // basename.
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("test_single.py");
    // Keep the body assertion-bearing so only ZR001 fires (not ZR003).
    std::fs::write(&file, "def test_x():\n    if True:\n        assert True\n").unwrap();

    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg(&file)
        .assert()
        .code(1)
        .stdout(contains("test_single.py:2:5: ZR001 "))
        .stdout(contains("1 findings in 1 files discovered."));
}

#[test]
fn check_on_explicit_non_test_prefixed_file_still_lints_it() {
    // Regression: users should not have to name their one-off scripts
    // `test_*.py` to lint them explicitly. The include globs apply to
    // directory walks but are bypassed for explicit file arguments.
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("scratch.py");
    std::fs::write(&file, "def test_x():\n    if True:\n        assert True\n").unwrap();

    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg(&file)
        .assert()
        .code(1)
        .stdout(contains("scratch.py:2:5: ZR001 "))
        .stdout(contains("1 findings in 1 files discovered."));
}

#[test]
fn check_discovers_config_from_target_path_not_cwd() {
    // Regression for M1: running `zorilla check /some/other/project`
    // should pick up that project's zorilla.toml, not walk upward from
    // the current working directory. The project here disables ZR001,
    // so the conditional-bearing test produces no finding.
    let tmp = TempDir::new().unwrap();
    let tests = tmp.path().join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("test_a.py"), "def test_x():\n    if True:\n        assert True\n")
        .unwrap();
    std::fs::write(tmp.path().join("zorilla.toml"), "[rules.ZR001]\nenabled = false\n").unwrap();

    // Invoke from an unrelated cwd (the binary's own target dir), with
    // the project as an absolute arg — config discovery should start
    // from the target, not the cwd.
    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(contains("0 findings in 1 files discovered."));
}

#[test]
fn check_on_file_with_ignore_file_directive_reports_zero_findings() {
    // End-to-end proof that `# zorilla: ignore-file` short-circuits the
    // engine before any rule runs. We pair the suppressed run with a
    // control: the same body without the directive must fire ZR001 and
    // exit non-zero. Without this paired control, a regression that
    // accidentally always returns zero findings on single-file mode
    // would still pass the suppression assertion.
    let body = "def test_x():\n    if True:\n        assert True\n";

    // Control: no directive — ZR001 fires, exit 1.
    let control_dir = TempDir::new().unwrap();
    let control_file = control_dir.path().join("test_control.py");
    std::fs::write(&control_file, body).unwrap();
    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg(&control_file)
        .assert()
        .code(1)
        .stdout(contains("ZR001 conditional-test-logic"))
        .stdout(contains("1 findings in 1 files discovered."));

    // Suppressed: same body prefixed with the directive — zero findings, exit 0.
    let suppressed_dir = TempDir::new().unwrap();
    let suppressed_file = suppressed_dir.path().join("test_ignored.py");
    std::fs::write(&suppressed_file, format!("# zorilla: ignore-file\n{body}")).unwrap();
    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg(&suppressed_file)
        .assert()
        .success()
        .stdout(contains("0 findings in 1 files discovered."));
}

#[test]
fn check_format_json_emits_json_array_matching_findings() {
    let tmp = TempDir::new().unwrap();
    let tests = tmp.path().join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("test_a.py"), "def test_x():\n    if True:\n        assert True\n")
        .unwrap();

    let assert = Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg("--format")
        .arg("json")
        .arg(tmp.path())
        .assert()
        .code(1);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // JSON output must NOT include the text summary line.
    assert!(
        !stdout.contains("findings in"),
        "JSON output should not include summary line, got: {stdout}"
    );
    let arr: Vec<serde_json::Value> =
        serde_json::from_str(&stdout).expect("stdout parses as JSON array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["code"], "ZR001");
    assert_eq!(arr[0]["severity"], "warning");
}

#[test]
fn check_format_sarif_emits_sarif_document() {
    let tmp = TempDir::new().unwrap();
    let tests = tmp.path().join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("test_a.py"), "def test_x():\n    if True:\n        assert True\n")
        .unwrap();

    let assert = Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg("--format")
        .arg("sarif")
        .arg(tmp.path())
        .assert()
        .code(1);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        !stdout.contains("findings in"),
        "SARIF output should not include summary line, got: {stdout}"
    );
    let v: serde_json::Value = serde_json::from_str(&stdout).expect("stdout parses as JSON object");
    assert!(v.get("$schema").is_some(), "missing $schema: {v}");
    assert!(v.get("version").is_some(), "missing version: {v}");
    assert!(v.get("runs").is_some(), "missing runs: {v}");
    assert_eq!(v["version"], "2.1.0");
    assert_eq!(v["runs"][0]["tool"]["driver"]["name"], "zorilla");
}

#[test]
fn list_rules_lists_all_seven_rules() {
    let assert = Command::cargo_bin("zorilla").unwrap().arg("list-rules").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    for code in ["ZR001", "ZR002", "ZR003", "ZR004", "ZR005", "ZR006", "ZR007"] {
        assert!(stdout.contains(code), "missing {code} in list-rules output:\n{stdout}");
    }
    for name in [
        "conditional-test-logic",
        "sleep-in-test",
        "no-assertion",
        "assertion-roulette",
        "mystery-guest",
        "patch-stack",
        "empty-test",
    ] {
        assert!(stdout.contains(name), "missing {name} in list-rules output:\n{stdout}");
    }
    assert!(stdout.contains("CODE"), "missing CODE header in:\n{stdout}");
    assert!(stdout.contains("DEFAULT"), "missing DEFAULT header in:\n{stdout}");
    // No row may end with trailing whitespace — the DEFAULT column is
    // the last one and must be unpadded. Regression guard: a prior
    // implementation padded it with `{:<7}` and trailing spaces leaked
    // into every row.
    for line in stdout.lines() {
        assert_eq!(
            line,
            line.trim_end(),
            "row has trailing whitespace: {line:?}\nfull output:\n{stdout}"
        );
    }
}

#[test]
fn explain_zr001_prints_embedded_markdown() {
    let assert =
        Command::cargo_bin("zorilla").unwrap().arg("explain").arg("ZR001").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("conditional"), "missing 'conditional' in:\n{stdout}");
    assert!(stdout.contains("Positive example"), "missing 'Positive example' in:\n{stdout}");
}

#[test]
fn explain_accepts_lowercase_rule_code() {
    // Upper-case baseline.
    let upper =
        Command::cargo_bin("zorilla").unwrap().arg("explain").arg("ZR001").assert().success();
    let upper_stdout = String::from_utf8(upper.get_output().stdout.clone()).unwrap();

    // Lower-case must produce byte-identical output.
    let lower =
        Command::cargo_bin("zorilla").unwrap().arg("explain").arg("zr001").assert().success();
    let lower_stdout = String::from_utf8(lower.get_output().stdout.clone()).unwrap();

    assert_eq!(upper_stdout, lower_stdout);
}

#[test]
fn explain_unknown_code_exits_two() {
    let assert =
        Command::cargo_bin("zorilla").unwrap().arg("explain").arg("BOGUS").assert().code(2);
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(stderr.contains("unknown rule: BOGUS"), "missing error message in stderr:\n{stderr}");
}

#[test]
fn check_on_tree_with_conditional_test_emits_zr001_finding() {
    let tmp = TempDir::new().unwrap();
    let tests = tmp.path().join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("test_a.py"), "def test_x():\n    if True:\n        assert True\n")
        .unwrap();

    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg(tmp.path())
        .assert()
        .code(1)
        .stdout(contains("ZR001 conditional-test-logic"))
        .stdout(contains("test function has conditional logic"))
        .stdout(contains("tests/test_a.py:2:5:"))
        .stdout(contains("1 findings in 1 files discovered."));
}
