# zorilla

A small, fast, opinionated Rust CLI that detects syntactic test smells in
pytest codebases. Built to compose with
[biston](https://github.com/mojzis/biston) (structural test duplication)
and with ruff's `PT` rules — zorilla owns only the gaps those two leave
behind.

> **Status:** Phase 1 scaffolding. The `check` subcommand walks the
> filesystem and reports file counts. Real rules arrive in Phase 2.

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

## Configuration

zorilla searches upward from the working directory for either
`zorilla.toml` or a `pyproject.toml` containing `[tool.zorilla]`. The
first match wins.

```toml
# pyproject.toml
[tool.zorilla]
include = ["tests/**/*.py", "**/test_*.py", "**/*_test.py", "**/conftest.py"]
exclude = ["**/fixtures/**"]
```

Rule-level configuration (`[tool.zorilla.rules.ZR001]`) will be honored
starting in Phase 2.

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
