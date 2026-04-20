# zorilla

A small, fast, opinionated Rust CLI that detects syntactic test smells in
pytest codebases. Built to compose with
[biston](https://github.com/mojzis/biston) (structural test duplication)
and with ruff's `PT` rules — zorilla owns only the gaps those two leave
behind.

> **Status:** v0.1 ready. Ships seven rules (ZR001–ZR007), inline and
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
$ zorilla check tests/
tests/test_demo.py:4:5: ZR001 conditional-test-logic — test function has conditional logic
tests/test_demo.py:5:9: ZR002 sleep-in-test — sleep() call inside a test

2 findings in 1 files discovered.
```

zorilla exits with status `1` because findings were reported. A clean
run (or an empty directory) exits `0`; an internal error exits `2`.

### Output formats

```bash
zorilla check --format json tests/
```

emits a JSON array, one object per finding — suitable for piping into
`jq` or a CI aggregator:

```json
[
  {
    "code": "ZR001",
    "message": "test function has conditional logic",
    "file": "tests/test_demo.py",
    "line": 4,
    "column": 5,
    "severity": "warning"
  }
]
```

```bash
zorilla check --format sarif tests/
```

emits a SARIF 2.1.0 document that most code-scanning tools (GitHub code
scanning, SonarQube, etc.) ingest directly:

```json
{
  "$schema": "https://docs.oasis-open.org/sarif/sarif/v2.1.0/errata01/os/schemas/sarif-schema-2.1.0.json",
  "version": "2.1.0",
  "runs": [
    {
      "tool": { "driver": { "name": "zorilla", "version": "0.1.0" } },
      "results": [
        {
          "ruleId": "ZR001",
          "level": "warning",
          "message": { "text": "test function has conditional logic" },
          "locations": [ /* ...physicalLocation... */ ]
        }
      ]
    }
  ]
}
```

JSON and SARIF output both omit the trailing human-readable summary
line so stdout parses cleanly.

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
    rev: v0.1.0
    hooks:
      - id: zorilla
```

The hook entry point is `zorilla check --files-from -`, so pre-commit
feeds only the staged Python files to zorilla via stdin. `v0.1.0` is
the target release tag — replace it with whichever tag is current when
you wire the hook up.

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
