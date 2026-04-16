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
