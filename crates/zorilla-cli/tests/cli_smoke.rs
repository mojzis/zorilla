//! End-to-end smoke test for the `zorilla` binary.
//!
//! Phase 2 shape: `zorilla check <dir>` runs the rule pipeline. On an
//! empty tree it exits 0 and prints a zero-findings summary; on a tree
//! with test functions it exits 1 and prints one `ZR000 hello-world`
//! finding per test function plus the trailing summary line.

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
fn check_on_tree_with_test_files_emits_hello_world_findings() {
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
        .code(1)
        .stdout(contains("ZR000 hello-world"))
        .stdout(contains("hello world (test function detected)"))
        .stdout(contains("tests/test_a.py:1:1:"))
        .stdout(contains("tests/test_b.py:1:1:"))
        .stdout(contains("2 findings in 2 files discovered."));
}
