# Setup

zorilla is not configured in this repository yet. Run everything below at the
repository root, in this order.

**1. Install.** Add it as a dev dependency: `uv add --dev zorilla`.
Alternatives: `pip install zorilla`, or `uvx zorilla check .` for a one-off
look. Prefer the dev dependency; `uvx` on every run pins no version.

**2. Configure.** The defaults already find pytest's idiomatic layouts and skip
fixture trees. Keep them until you have a clean baseline. Setting either glob
*replaces* the default list rather than extending it, so write out every pattern
you want, in `pyproject.toml`:

```toml
[tool.zorilla]
include = ["tests/**/*.py", "**/test_*.py", "**/*_test.py", "**/conftest.py"]
exclude = ["**/fixtures/**", "**/vendor/**"]
```

A `zorilla.toml` at the root is the alternative; it wins over `pyproject.toml`.
Leave the per-rule knobs alone for now -- `zorilla guide tune` covers them once
you have numbers to tune against.

**3. Baseline before you gate.** Run `zorilla check .` and resolve or suppress
every finding it reports before wiring zorilla into any gate; run
`zorilla guide triage` for how. `zorilla overview .` shows which files carry the
weight, and `zorilla stats .` shows which rules do. Adding a linter to a dirty
repository's gate gets the linter removed, not the smells.

**4. Integrate.** Add `zorilla check .` to the check aggregator this repository
already has. The aggregator runs the full tree and has no diff context. With
poethepoet:

```toml
[tool.poe.tasks]
smells = "zorilla check ."
check = ["lint", "typecheck", "smells"]
```

`just`, `make` and `nox` are the same shape: one recipe, one command. `check`
exits 1 on findings, 0 when clean and 2 when zorilla could not run. `stats` and
`overview` always exit 0, so gate on `check` and nothing else.

For a commit hook, with pre-commit or prek:

```yaml
- repo: https://github.com/mojzis/zorilla
  rev: v0.1.4
  hooks:
    - id: zorilla
```

The hook passes the staged files as positional arguments, which bypass
`include` -- though `exclude` still applies, so a staged file under an excluded
tree is skipped. Do not hand-roll a `git diff | zorilla check --files-from -`
hook: an empty list is indistinguishable from no argument, and zorilla falls
back to scanning the whole tree.

next: run `zorilla check .`
