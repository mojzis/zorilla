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
    // engine before any rule runs: a ZR001-firing test body in a file
    // with the directive at the top must produce zero findings, exit 0.
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("test_ignored.py");
    std::fs::write(
        &file,
        "# zorilla: ignore-file\ndef test_x():\n    if True:\n        assert True\n",
    )
    .unwrap();

    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg(&file)
        .assert()
        .success()
        .stdout(contains("0 findings in 1 files discovered."));
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
