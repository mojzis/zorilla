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
fn check_files_from_stdin_lints_only_listed_files() {
    // Piping one path via `--files-from -` must lint exactly that file
    // — and produce the same text output as invoking `check PATH`
    // directly on it. This pins the "stdin paths bypass include/exclude
    // globs the same way explicit file args do" invariant.
    let tmp = TempDir::new().unwrap();
    // Non-test-prefixed name ensures it would NOT match the default
    // include globs on a directory walk; only the explicit-path bypass
    // lets it be linted here.
    let file = tmp.path().join("scratch.py");
    std::fs::write(&file, "def test_x():\n    if True:\n        assert True\n").unwrap();

    let baseline = Command::cargo_bin("zorilla").unwrap().arg("check").arg(&file).assert().code(1);
    let baseline_stdout = String::from_utf8(baseline.get_output().stdout.clone()).unwrap();
    // Sanity-anchor the baseline so a silent degradation (both sides
    // producing empty output, for instance) fails the test instead of
    // the equality assertion below quietly holding on `""`.
    assert!(baseline_stdout.contains("ZR001"), "baseline missing ZR001:\n{baseline_stdout}");

    let piped = Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg("--files-from")
        .arg("-")
        .write_stdin(format!("{}\n", file.display()))
        .assert()
        .code(1);
    let piped_stdout = String::from_utf8(piped.get_output().stdout.clone()).unwrap();

    assert_eq!(baseline_stdout, piped_stdout);
}

#[test]
fn check_files_from_stdin_skips_comments_and_blank_lines() {
    // Blank lines and `#` comments must be ignored — pre-commit and
    // shell pipelines sometimes emit them, and the user's list file
    // may contain TODO annotations.
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("scratch.py");
    std::fs::write(&file, "def test_x():\n    if True:\n        assert True\n").unwrap();

    let baseline = Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg("--files-from")
        .arg("-")
        .write_stdin(format!("{}\n", file.display()))
        .assert()
        .code(1);
    let baseline_stdout = String::from_utf8(baseline.get_output().stdout.clone()).unwrap();
    assert!(baseline_stdout.contains("ZR001"), "baseline missing ZR001:\n{baseline_stdout}");

    let with_noise = Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg("--files-from")
        .arg("-")
        .write_stdin(format!("# a comment\n\n{}\n   \n", file.display()))
        .assert()
        .code(1);
    let with_noise_stdout = String::from_utf8(with_noise.get_output().stdout.clone()).unwrap();

    assert_eq!(baseline_stdout, with_noise_stdout);
}

#[test]
fn check_files_from_empty_input_reports_zero_findings() {
    // Empty stdin after filtering yields no paths and no findings —
    // NOT an error. This is the "pre-commit ran but no Python files
    // changed" path.
    Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg("--files-from")
        .arg("-")
        .write_stdin("")
        .assert()
        .success()
        .stdout(contains("0 findings in 0 files discovered."));
}

#[test]
fn check_files_flag_is_repeatable() {
    // `--files PATH1 --files PATH2` must lint both paths. Use two
    // files with different rule violations so the combined run's
    // finding count is strictly greater than either alone.
    let tmp = TempDir::new().unwrap();
    let a = tmp.path().join("a.py");
    std::fs::write(&a, "def test_x():\n    if True:\n        assert True\n").unwrap();
    let b = tmp.path().join("b.py");
    std::fs::write(&b, "import time\n\ndef test_y():\n    time.sleep(1)\n    assert True\n")
        .unwrap();

    let assert = Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg("--files")
        .arg(&a)
        .arg("--files")
        .arg(&b)
        .assert()
        .code(1);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    assert!(stdout.contains("ZR001"), "expected ZR001 from a.py in:\n{stdout}");
    assert!(stdout.contains("ZR002"), "expected ZR002 from b.py in:\n{stdout}");
    assert!(
        stdout.contains("2 findings in 2 files discovered."),
        "expected combined summary in:\n{stdout}"
    );
}

#[test]
fn check_files_from_file_path() {
    // Passing `--files-from LIST_FILE` must produce the same output as
    // piping the same content to `--files-from -`. Pins parity between
    // the two delivery modes.
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("scratch.py");
    std::fs::write(&file, "def test_x():\n    if True:\n        assert True\n").unwrap();

    let list_file = tmp.path().join("paths.txt");
    let list_content = format!("# header comment\n\n{}\n", file.display());
    std::fs::write(&list_file, &list_content).unwrap();

    let from_file = Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg("--files-from")
        .arg(&list_file)
        .assert()
        .code(1);
    let from_file_stdout = String::from_utf8(from_file.get_output().stdout.clone()).unwrap();
    assert!(from_file_stdout.contains("ZR001"), "baseline missing ZR001:\n{from_file_stdout}");

    let from_stdin = Command::cargo_bin("zorilla")
        .unwrap()
        .arg("check")
        .arg("--files-from")
        .arg("-")
        .write_stdin(list_content)
        .assert()
        .code(1);
    let from_stdin_stdout = String::from_utf8(from_stdin.get_output().stdout.clone()).unwrap();

    assert_eq!(from_file_stdout, from_stdin_stdout);
}

#[test]
fn stats_help_mentions_format_and_files_flags() {
    // Advertises the full surface: the `--format` summary flag and the
    // two input-plumbing flags `stats` inherits from `check`.
    let assert =
        Command::cargo_bin("zorilla").unwrap().arg("stats").arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("--format"), "stats --help missing --format:\n{stdout}");
    assert!(stdout.contains("--files-from"), "stats --help missing --files-from:\n{stdout}");
    assert!(stdout.contains("--files"), "stats --help missing --files:\n{stdout}");
}

#[test]
fn stats_on_empty_directory_reports_zero_counts_and_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let assert =
        Command::cargo_bin("zorilla").unwrap().arg("stats").arg(tmp.path()).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Files scanned:"), "missing `Files scanned:` in:\n{stdout}");
    assert!(stdout.contains("Total findings:"), "missing `Total findings:` in:\n{stdout}");
    // Every summary counter should read zero on an empty directory.
    for label in ["Files scanned:", "Files with findings:", "Clean files:", "Total findings:"] {
        let line = stdout
            .lines()
            .find(|l| l.contains(label))
            .unwrap_or_else(|| panic!("label {label} not found in:\n{stdout}"));
        assert!(line.trim_end().ends_with('0'), "expected {label} to end in 0, got: {line}");
    }
}

#[test]
fn stats_on_tree_with_findings_reports_nonzero_breakdown_and_exits_zero() {
    let tmp = TempDir::new().unwrap();
    let tests = tmp.path().join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    // Two test functions with conditionals — two ZR001 findings.
    std::fs::write(tests.join("test_a.py"), "def test_x():\n    if True:\n        assert True\n")
        .unwrap();
    std::fs::write(
        tests.join("test_b.py"),
        "def test_y():\n    for i in (1,):\n        assert i\n",
    )
    .unwrap();

    let assert =
        Command::cargo_bin("zorilla").unwrap().arg("stats").arg(tmp.path()).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("Files scanned:"), "missing `Files scanned:` in:\n{stdout}");

    // At least one rule row must have a non-zero count. We know ZR001
    // fired twice above; locate that row and assert its count > 0.
    let zr001_line = stdout
        .lines()
        .find(|l| l.contains("ZR001") && l.contains("conditional-test-logic"))
        .unwrap_or_else(|| panic!("missing ZR001 row in:\n{stdout}"));
    let count: usize = zr001_line
        .split_whitespace()
        .next_back()
        .and_then(|t| t.parse().ok())
        .unwrap_or_else(|| panic!("could not parse count from: {zr001_line}"));
    assert!(count > 0, "expected non-zero ZR001 count in {zr001_line}");
}

#[test]
fn stats_files_from_stdin_actually_lints_listed_paths() {
    // Regression guard for `stats`'s --files-from plumbing. Mirrors
    // `check_files_from_stdin_lints_only_listed_files` in spirit: prove
    // that a path delivered via stdin is actually linted, not silently
    // dropped. One test covers the invariant without duplicating the
    // three-mode matrix (`--files`, `--files-from FILE`, `--files-from
    // -`) the `check` tests pin.
    let tmp = TempDir::new().unwrap();
    // Non-test-prefixed so include globs would reject it on a dir walk;
    // only the explicit-path bypass gets it into the scan.
    let file = tmp.path().join("scratch.py");
    std::fs::write(&file, "def test_x():\n    if True:\n        assert True\n").unwrap();

    let assert = Command::cargo_bin("zorilla")
        .unwrap()
        .arg("stats")
        .arg("--format")
        .arg("json")
        .arg("--files-from")
        .arg("-")
        .write_stdin(format!("{}\n", file.display()))
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stats --format json stdout parses as JSON");
    // Evidence: the stdin-supplied path was actually linted — the file
    // contains a ZR001 violation, so the breakdown for ZR001 must read
    // non-zero. A regression where `stats` ignored `--files-from` would
    // show ZR001 = 0 here.
    assert_eq!(
        parsed["breakdown"]["ZR001"].as_u64(),
        Some(1),
        "expected ZR001 count of 1 in breakdown, got:\n{stdout}"
    );
    assert_eq!(parsed["total_findings"].as_u64(), Some(1), "unexpected total_findings:\n{stdout}");
}

#[test]
fn stats_format_json_produces_parseable_output_with_required_keys() {
    let tmp = TempDir::new().unwrap();
    let tests = tmp.path().join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("test_a.py"), "def test_x():\n    if True:\n        assert True\n")
        .unwrap();

    let assert = Command::cargo_bin("zorilla")
        .unwrap()
        .arg("stats")
        .arg(tmp.path())
        .arg("--format")
        .arg("json")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stats --format json stdout parses as JSON");
    for key in
        ["files_scanned", "files_with_findings", "clean_files", "total_findings", "breakdown"]
    {
        assert!(parsed.get(key).is_some(), "missing key {key} in JSON output:\n{stdout}");
    }
    // Breakdown must be an object; engine ran at least once, so at
    // minimum the registered rule codes should be present as keys.
    let breakdown = parsed["breakdown"].as_object().expect("breakdown is an object");
    for code in ["ZR001", "ZR002", "ZR003", "ZR004", "ZR005", "ZR006", "ZR007"] {
        assert!(breakdown.contains_key(code), "missing {code} in breakdown:\n{stdout}");
    }
}

#[test]
fn overview_help_mentions_format_and_files_flags() {
    // `overview --help` advertises the full surface: `--format` and the
    // two input-plumbing flags it inherits from `check`.
    let assert =
        Command::cargo_bin("zorilla").unwrap().arg("overview").arg("--help").assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("--format"), "overview --help missing --format:\n{stdout}");
    assert!(stdout.contains("--files-from"), "overview --help missing --files-from:\n{stdout}");
    assert!(stdout.contains("--files"), "overview --help missing --files:\n{stdout}");
}

#[test]
fn overview_on_tree_with_findings_prints_filename_findings_and_bullet() {
    // `NO_COLOR=1` is set in the child env so the assertions below do
    // not have to cope with ANSI codes — the smoke test pins the
    // uncoloured bullet character (unicode bullet ●).
    let tmp = TempDir::new().unwrap();
    let tests = tmp.path().join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("test_a.py"), "def test_x():\n    if True:\n        assert True\n")
        .unwrap();

    let assert = Command::cargo_bin("zorilla")
        .unwrap()
        .env("NO_COLOR", "1")
        .arg("overview")
        .arg(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // Locate the file's header line — it must contain the filename and
    // the per-file finding-count fragment. Match on the singular form
    // ("N finding" rather than "N findings") since the fixture has a
    // single finding and the renderer pluralises based on count.
    let header = stdout
        .lines()
        .find(|l| l.contains("test_a.py") && l.contains("finding"))
        .unwrap_or_else(|| panic!("missing file header in overview output:\n{stdout}"));
    // Per-file header pluralises "finding" based on the count — one
    // finding stays singular.
    assert!(
        header.contains("1 finding") && !header.contains("1 findings"),
        "expected singular '1 finding' on file header, got: {header:?}\nfull:\n{stdout}"
    );
    // Plain bullet (unicode ●) appears on at least one finding line.
    assert!(stdout.contains('\u{25CF}'), "missing bullet character in:\n{stdout}");
    // No ANSI escape bytes because NO_COLOR is set.
    assert!(!stdout.contains('\x1b'), "unexpected ANSI codes despite NO_COLOR:\n{stdout:?}");
}

#[test]
fn overview_format_json_parses_with_required_keys() {
    let tmp = TempDir::new().unwrap();
    let tests = tmp.path().join("tests");
    std::fs::create_dir_all(&tests).unwrap();
    std::fs::write(tests.join("test_a.py"), "def test_x():\n    if True:\n        assert True\n")
        .unwrap();

    let assert = Command::cargo_bin("zorilla")
        .unwrap()
        .env("NO_COLOR", "1")
        .arg("overview")
        .arg(tmp.path())
        .arg("--format")
        .arg("json")
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("overview --format json stdout parses as JSON");
    for key in ["summary", "files", "clean_files"] {
        assert!(parsed.get(key).is_some(), "missing key {key} in JSON output:\n{stdout}");
    }
    // Files array must carry at least one entry pointing at test_a.py.
    let files = parsed["files"].as_array().expect("files is an array");
    assert!(!files.is_empty(), "expected non-empty files array in:\n{stdout}");
    assert!(
        files[0]["path"].as_str().unwrap_or("").contains("test_a.py"),
        "expected first file to be test_a.py, got: {stdout}"
    );
    assert_eq!(files[0]["finding_count"], 1);
}

#[test]
fn overview_on_empty_directory_is_clean_and_exits_zero() {
    let tmp = TempDir::new().unwrap();

    let assert = Command::cargo_bin("zorilla")
        .unwrap()
        .env("NO_COLOR", "1")
        .arg("overview")
        .arg(tmp.path())
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // No bullets on a clean run — the per-file sections are skipped.
    assert!(!stdout.contains('\u{25CF}'), "unexpected bullet on empty run:\n{stdout}");
    // Header still prints with zero counts.
    assert!(
        stdout.contains("Overview: 0 files, 0 findings in 0 files"),
        "missing zero-counts header in:\n{stdout}"
    );
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
