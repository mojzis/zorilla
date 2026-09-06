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
Leave the per-rule knobs alone until you have numbers: `zorilla guide tune`.

**3. Baseline before you gate.** Run `zorilla check .` and resolve or suppress
every finding it reports before wiring zorilla into any gate; run
`zorilla guide triage` for how. `zorilla overview .` shows which files carry the
weight, and `zorilla stats .` shows which rules do. Adding a linter to a dirty
repository's gate gets the linter removed, not the smells.

**4. Integrate.** Add `zorilla check .` to the check aggregator this repository
already has - poethepoet, `just`, `make`, `nox`: one recipe, one command. It
runs the full tree and has no diff context. `check` exits 1 on findings, 0
when clean and 2 when zorilla could not run. `stats` and `overview` always
exit 0, so gate on `check` and nothing else.

For a commit hook, as a madoqua step in `pyproject.toml`:

```toml
[tool.madoqua]
extend_check = [{ name = "zorilla", cmd = "zorilla check" }]
```

madoqua appends the staged Python files, so the step lints exactly those.
Any path list is valid - `zorilla check tests a.py b.py` lints the directory
and both files - and named paths bypass `include`, though `exclude` still
applies. With pre-commit or prek the framework passes the staged files the
same way:

```yaml
- repo: https://github.com/mojzis/zorilla
  rev: v0.2.0
  hooks:
    - id: zorilla
```

Do not hand-roll a `git diff | zorilla check --files-from -` hook: an empty
list is indistinguishable from no argument, and zorilla falls back to
scanning the whole tree.

next: run `zorilla check .`
