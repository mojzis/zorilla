//! End-to-end smoke test for the `zorilla` binary.
//!
//! Phase 1 shape: `zorilla check <dir>` must exit 0 on a clean tree and
//! print the Phase 1 summary line. The test is deliberately minimal —
//! later phases will add finding-level assertions once real rules land.

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
fn check_on_tree_with_test_files_counts_them() {
    let tmp = TempDir::new().unwrap();
    let tests = tmp.path().join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("test_a.py"), "def test_x():\n    pass\n").unwrap();
    std::fs::write(tests.join("test_b.py"), "def test_y():\n    pass\n").unwrap();

    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg(tmp.path())
        .assert()
        .success()
        .stdout(contains("0 findings in 2 files discovered."));
}
