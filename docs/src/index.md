# zorilla

zorilla is a small, fast Rust CLI that detects syntactic test smells in pytest
codebases: tests that assert nothing, sleep, branch, or hide behind a stack of
patches. It composes with [biston](https://github.com/mojzis/biston)
(structural test duplication) and ruff's `PT` rules, and owns only the gaps
those two leave behind.

It is meant for every commit: it says yes or no, then gets out of the way. For
the occasional full audit that ranks every test by suspicion, see
[pycoati](https://mojzis.github.io/pycoati/).

## Install

```sh
uvx zorilla@latest guide      # agent-oriented instructions
uvx zorilla@latest check .    # lint the current project
```

As a pre-commit hook:

```yaml
repos:
  - repo: https://github.com/mojzis/zorilla
    rev: v0.2.6
    hooks:
      - id: zorilla
```

## What it does

```sh
zorilla check path/to/tests            # exit 0 clean, 1 findings, 2 error
zorilla check --format json .          # one object per finding
zorilla check --format sarif .         # for code-scanning tools
zorilla overview .                     # findings grouped by file
zorilla stats .                        # counters and per-rule breakdown
zorilla list-rules                     # the eight rules
zorilla explain ZR003                  # a rule's page, from the terminal
```

Configuration lives in `zorilla.toml` or `[tool.zorilla]` in `pyproject.toml`:
include/exclude globs and per-rule knobs. Suppression is per line
(`# zorilla: ignore[ZR001] -- reason`) or per file (`# zorilla: ignore-file`).
See [Tune suppression and knobs](guide/tune.md).

## The guide and the rule pages

`zorilla guide [setup|triage|tune]` prints the pages in the *Agent guide*
section; `zorilla explain ZR###` prints the pages in the *Rules* section. Both
are embedded in the binary at compile time, so the site and the CLI serve the
same bytes. Tests feed every command a guide shows through the real CLI parser,
and fail the build when a guide names a rule or config key that doesn't exist.

## For your CLAUDE.md

<!-- BEGIN SHARED:claude-snippet -->
```markdown
### `zorilla` — test-smell gate

- Runs on every commit (pre-commit hook `zorilla check`). Findings are not
  verdicts: run `zorilla guide triage` for the remedy ladder.
- Fix the test before suppressing. A legitimate exception gets
  `# zorilla: ignore[ZR###] -- <reason>` on the line; never a bare ignore.
- `zorilla explain ZR###` says why a rule fires and which line it reports on.
```
<!-- END SHARED:claude-snippet -->
