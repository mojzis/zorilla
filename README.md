# zorilla

A small, fast, opinionated Rust CLI that detects syntactic test smells in
pytest codebases. Built to compose with
[biston](https://github.com/mojzis/biston) (structural test duplication)
and with ruff's `PT` rules — zorilla owns only the gaps those two leave
behind.

> **Status:** v0.1 ready. Ships eight rules (ZR001–ZR008), inline and
> file-level suppression comments, `text` / `json` / `sarif` output
> formats, a `list-rules` / `explain` pair, and a pre-commit hook.

## Installation

zorilla is distributed as a Python wheel built with
[maturin](https://github.com/PyO3/maturin).

```bash
# once published
pip install zorilla
# or, for a local checkout (requires an activated venv)
maturin develop
```

This installs a `zorilla` binary on your `PATH`.

> `maturin develop` needs an activated Python virtualenv — either
> `source .venv/bin/activate` first, or export `VIRTUAL_ENV=/path/to/venv`.
> Having the venv's `bin/` on `PATH` is not sufficient.

## Usage

```bash
zorilla check path/to/tests
```

Exit codes: `0` clean, `1` findings reported, `2` error.

`--files PATH` (repeatable) and `--files-from FILE` (use `-` for stdin) accept explicit file lists, bypassing the configured include globs — useful for CI pipelines.

### Rules

| Code   | Name                    | Summary                                                  |
| ------ | ----------------------- | -------------------------------------------------------- |
| ZR001  | conditional-test-logic  | `if` / `for` / `while` / `try` in a test body            |
| ZR002  | sleep-in-test           | `time.sleep` / `asyncio.sleep` inside a test             |
| ZR003  | no-assertion            | Test function with no assertion or `pytest.raises`       |
| ZR004  | assertion-roulette      | Too many bare (message-less) asserts in one test         |
| ZR005  | mystery-guest           | Absolute path, URL, or `~`-path literal inside a test    |
| ZR006  | patch-stack             | Too many stacked `@patch` / `@mock.patch` decorators     |
| ZR007  | empty-test              | Test body is empty (`pass`, `...`, docstring-only)       |
| ZR008  | context-patch-stack     | Too many `with patch(...)` context managers in one test   |

Long-form docs (motivation, positive/negative examples, config knobs,
suppression syntax) live under [`docs/rules/`](./docs/rules/). You can
also print them inline with `zorilla explain ZR###` — see below.

### Worked example

Save as `tests/test_demo.py`:

```python
import time

def test_branch_and_wait():
    if ready():
        time.sleep(1)
        assert done()
```

Run:

```
$ zorilla check .
tests/test_demo.py:4:5: ZR001 conditional-test-logic: test function has conditional logic (if/for/while/try)
tests/test_demo.py:5:9: ZR002 sleep-in-test: test calls sleep — wait on a condition instead
2 findings in 1 files discovered.

Findings are not verdicts. Run `zorilla guide triage` for the remedy ladder, including when `# zorilla: ignore -- <reason>` is the right answer.
```

zorilla exits with status `1` because findings were reported. A clean
run (or an empty directory) exits `0`; an internal error exits `2`.

### Output formats

```bash
zorilla check --format json .
```

emits a JSON array, one object per finding — suitable for piping into
`jq` or a CI aggregator:

```json
[
  {
    "code": "ZR001",
    "message": "test function has conditional logic (if/for/while/try)",
    "file": "tests/test_demo.py",
    "line": 4,
    "column": 5,
    "severity": "warning"
  }
]
```

```bash
zorilla check --format sarif .
```

emits a SARIF 2.1.0 document that most code-scanning tools (GitHub code
scanning, SonarQube, etc.) ingest directly:

```json
{
  "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
  "version": "2.1.0",
  "runs": [
    {
      "tool": { "driver": { "name": "zorilla", "version": "0.1.0" } },
      "results": [
        {
          "ruleId": "ZR001",
          "level": "warning",
          "message": { "text": "test function has conditional logic (if/for/while/try)" },
          "locations": [ /* ...physicalLocation... */ ]
        }
      ]
    }
  ]
}
```

JSON and SARIF output both omit the trailing human-readable summary
line so stdout parses cleanly.

### Scan statistics

`zorilla stats` post-processes a scan into aggregate counters and a
per-rule breakdown. Use it for CI dashboards, quick health checks, or a
high-level snapshot of how many tests are flagged and by which rules.
Unlike `check`, it always exits `0` — it is a report, not a gate.

```
$ zorilla stats path/to/tests
Scan statistics:
  Files scanned:        12
  Files with findings:  3
  Clean files:          9
  Total findings:       7

Breakdown by rule:
  ZR001 conditional-test-logic:  2
  ZR002 sleep-in-test:           1
  ZR003 no-assertion:            4
  ZR004 assertion-roulette:      0
  ZR005 mystery-guest:           0
  ZR006 patch-stack:             0
  ZR007 empty-test:              0
  ZR008 context-patch-stack:     0
```

Add `--format json` to emit a parseable summary (flat counters plus a
`breakdown` object keyed by rule code). `--files` and `--files-from`
work the same way they do for `check`.

### Per-file overview

`zorilla overview` groups findings by file. Files are sorted by finding
count (most-flagged first), and files that produced no findings are
rolled up into a trailing count — useful when triaging a large repo.
Like `stats`, it always exits `0` and accepts `--files` / `--files-from`.

```
$ zorilla overview path/to/tests
Overview: 12 files, 7 findings in 3 files

tests/test_orders.py  3 findings
  ● 12:5  ZR003 no-assertion           test has no assertion
  ● 25:5  ZR001 conditional-test-logic test function has conditional logic (if/for/while/try)
  ● 25:9  ZR002 sleep-in-test          test calls sleep — wait on a condition instead

tests/test_returns.py  2 findings
  ● 7:5   ZR003 no-assertion           test has no assertion
  ● 18:5  ZR003 no-assertion           test has no assertion

9 clean files not shown.

Findings are not verdicts. Run `zorilla guide triage` for the remedy ladder, including when `# zorilla: ignore -- <reason>` is the right answer.
```

The bullet (`●`) is coloured by severity — yellow for warnings, red for
errors — when stdout is a terminal. `overview` honours the [`NO_COLOR`
convention](https://no-color.org/), and emits plain bullets when output
is piped or redirected. Add `--format json` to get the same data as a
structured document (`summary`, `files`, `clean_files`) suitable for
dashboards or scripting.

### Guide

`zorilla guide` prints short, imperative instructions for the three moments
someone — or a coding agent — meets zorilla. With no topic it picks between
`setup` and `triage` by looking at the current directory and every directory
above it, stopping at the repository root:

```
$ zorilla guide
# zorilla guide: configured via pyproject.toml [tool.zorilla] -> triage
...
```

| Topic | For |
| ----- | --- |
| `setup` | zorilla is not wired into this repository yet |
| `triage` | a check run reported findings; what to do about each code |
| `tune` | suppression syntax, scope globs, per-rule knobs, precedence |

`tune` is a reference and is never auto-selected. The prose lives in
[`docs/guide/`](./docs/guide/) and is compiled into the binary with
`include_str!`, so the files and the CLI serve the same bytes.

Any `check` or `overview` text report that has findings ends with a one-line
footer pointing back at `zorilla guide triage` — the breadcrumb from a failed
gate to the instructions for clearing it. JSON and SARIF never carry it.

### List and explain

```
$ zorilla list-rules
CODE  NAME                      DEFAULT
ZR001 conditional-test-logic    on
ZR002 sleep-in-test             on
ZR003 no-assertion              on
ZR004 assertion-roulette        on
ZR005 mystery-guest             on
ZR006 patch-stack               on
ZR007 empty-test                on
ZR008 context-patch-stack       on
```

```
$ zorilla explain ZR003
# ZR003 — no-assertion
…
```

`explain` accepts the rule code in either case (`ZR003` or `zr003`) and
prints the bundled markdown. Unknown codes exit `2`.

### Pre-commit integration

Add zorilla to your project's `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/mojzis/zorilla
    rev: v0.1.4
    hooks:
      - id: zorilla
```

The hook entry point is `zorilla check`; pre-commit appends only the
staged Python files as positional arguments, so zorilla lints each one
directly (bypassing the `include` globs the way any explicit file path
does). Replace the `rev:` with whichever tag is current when you wire the
hook up.

## Configuration

zorilla searches upward from the working directory for either
`zorilla.toml` or a `pyproject.toml` containing `[tool.zorilla]`. The
first match wins.

```toml
# pyproject.toml
[tool.zorilla]
include = ["tests/**/*.py", "**/test_*.py", "**/*_test.py", "**/conftest.py"]
exclude = ["**/fixtures/**"]

[tool.zorilla.rules.ZR004]
max_asserts = 4

[tool.zorilla.rules.ZR006]
max_patches = 3
```

Per-rule sections (`[tool.zorilla.rules.ZRNNN]`) accept `enabled = false`
to disable the rule and any rule-specific knobs (`max_asserts` for
ZR004, `max_patches` for ZR006, `extra_helpers` for ZR003,
`allowed_prefixes` for ZR005). See [`docs/rules/`](./docs/rules/) for
the exhaustive list.

`zorilla guide tune` prints the whole reference — knobs, scope globs,
suppression syntax and precedence — without leaving the terminal.

Suppression comments work per-line and per-file:

```python
# zorilla: ignore-file                              <- silences the whole file
def test_x():
    if cond:  # zorilla: ignore[ZR001]              <- silences just this line
        ...
```

## Developing

```bash
# Pre-commit gate
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace

# Maturin develop build
maturin develop
```

Workspace layout:

```
crates/
  zorilla-core/   # library
  zorilla-cli/    # binary (`zorilla`)
```

See `CLAUDE.md` for the development workflow and `docs/plans/PLAN.md` for
the design doc driving the rule set.

## License

MIT. See [LICENSE](./LICENSE).
