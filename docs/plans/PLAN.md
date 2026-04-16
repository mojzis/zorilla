# PLACEHOLDER — Rust linter for pytest test smells

> Rename everywhere once the project name is chosen. All occurrences of
> `PLACEHOLDER` / `placeholder` in code, docs, config keys, and file paths
> need to change.

## Mission

A small, fast, opinionated Rust CLI that detects syntactic test smells in
pytest codebases. Runs in pre-commit hooks and CI across the org. Composes
with `biston --tests-only` (which handles structural duplication) and with
ruff's `PT` rules (which handle pytest idiom style). Owns only the gaps
those two leave behind.

Non-goals for v0.1: semantic resolution, fixture-scope reasoning,
patch-target correctness, cross-file analysis, unittest-specific rules.

## Rules for v0.1

Seven rules, all purely syntactic, detectable from a tree-sitter AST of a
single file:

| Code | Name | What fires |
|------|------|------------|
| PT001 | `conditional-test-logic` | `if` / `for` / `while` / `try` under a test function body (direct or nested). |
| PT002 | `sleep-in-test` | Call to `time.sleep`, `asyncio.sleep`, or a bare `sleep` in test body. |
| PT003 | `no-assertion` | Test function contains no `assert`, no `pytest.raises`/`pytest.warns` context manager, and no call matching known assertion helpers. |
| PT004 | `assertion-roulette` | More than N (default 4) bare `assert` statements in one test without messages. Configurable threshold. |
| PT005 | `mystery-guest` | Hardcoded absolute path (starts with `/`, `C:\`, `~`) or bare HTTP/HTTPS URL literal in test body. Configurable allow-list. |
| PT006 | `patch-stack` | More than N (default 3) `@patch` / `@mock.patch` decorators on one test. Configurable. |
| PT007 | `empty-test` | Test function body is `pass`, an ellipsis, or only a docstring. |

"Test function" = top-level `def test_*` or a `def test_*` method on a
`class Test*`. Async test functions count.

Each rule gets a config knob under `[tool.PLACEHOLDER.rules.PTNNN]` to
disable or tune thresholds.

## Architecture — crib from biston, don't copy

The shape is the same pipeline biston uses (discovery → parse → per-rule
visit → report), minus the similarity / anti-unification stages. Keep the
seams identical so the two tools feel like a family.

```
crates/
  PLACEHOLDER-core/          # library: rules, engine, config, report
    src/
      lib.rs                 # pub fn lint(paths, config) -> Report
      config.rs              # TOML loader, [tool.PLACEHOLDER] section
      discovery.rs           # ignore-crate walker, test-file globs
      parse.rs               # tree-sitter-python wrapper
      ast.rs                 # helpers: is_test_function, iter_body, etc.
      suppress.rs            # # PLACEHOLDER: ignore[PTNNN], file-level
      report.rs              # Finding, text/JSON/SARIF emitters
      rules/
        mod.rs               # Rule trait, registry
        pt001_conditional.rs
        pt002_sleep.rs
        pt003_no_assertion.rs
        pt004_roulette.rs
        pt005_mystery_guest.rs
        pt006_patch_stack.rs
        pt007_empty_test.rs
  PLACEHOLDER-cli/           # thin clap wrapper over core
    src/main.rs
```

Workspace, not a single crate. Lets the core be embedded later (e.g. a
pytest plugin that shells out to the library via PyO3) without reshaping
anything. If that feels like overkill at v0.1 — it isn't; splitting later
is three times the work.

## Core abstractions

```rust
// rules/mod.rs
pub struct Context<'a> {
    pub file: &'a Path,
    pub source: &'a str,
    pub tree: &'a tree_sitter::Tree,
    pub config: &'a RuleConfig,
    pub suppressions: &'a Suppressions,
}

pub struct Finding {
    pub code: &'static str,       // "PT001"
    pub message: String,
    pub file: PathBuf,
    pub line: usize,              // 1-indexed
    pub column: usize,
    pub severity: Severity,       // Warning by default
}

pub trait Rule: Sync {
    fn code(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn default_enabled(&self) -> bool { true }
    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>);
}
```

Rules are zero-state singletons registered in `rules::all()`. The engine
iterates registered rules, filters by config, runs each against each
parsed file. Parallelism via `rayon::par_iter` over files — one parse per
file, then every rule shares that tree.

## Tree-sitter queries vs. manual traversal

Use tree-sitter queries where the pattern is declarative and small
(PT002, PT005, PT006, PT007). Use manual cursor traversal where you need
counting or nested-scope logic (PT001, PT003, PT004). Don't force one
style; the maintenance cost differs by rule.

Example query for PT002 (sleep call):

```scheme
(call
  function: [
    (attribute attribute: (identifier) @name (#match? @name "^sleep$"))
    (identifier) @name (#eq? @name "sleep")
  ]) @call
```

Example query for PT006 (patch stack) — count decorators matching
`patch` / `mock.patch` on a test function. Can be done in one query with
a predicate, but cleaner as manual traversal.

## Identifying test functions

One helper, used by every rule:

```rust
pub fn is_test_function(node: tree_sitter::Node, source: &str) -> bool {
    // function_definition whose name starts with "test_"
    // parent class (if any) starts with "Test"
    // do not match nested inner functions
}
```

A rule receives a list of test function nodes to iterate, not the whole
tree. Avoid every rule rediscovering "what's a test."

## Configuration

`pyproject.toml` under `[tool.PLACEHOLDER]`:

```toml
[tool.PLACEHOLDER]
include = ["tests/**/*.py", "**/test_*.py", "**/*_test.py", "**/conftest.py"]
exclude = ["**/fixtures/**"]

[tool.PLACEHOLDER.rules]
# disable a rule
PT005 = { enabled = false }
# tune a threshold
PT004 = { max_asserts = 6 }
PT006 = { max_patches = 4 }

# per-rule allow-lists
[tool.PLACEHOLDER.rules.PT005]
allowed_prefixes = ["http://localhost", "https://127.0.0.1"]
```

Standalone `PLACEHOLDER.toml` is also accepted (scan upward from CWD
until `pyproject.toml` or `PLACEHOLDER.toml` found, first wins). CLI
flags override config.

## Suppression

Two mechanisms, both cribbed from biston:

- File-level: `# PLACEHOLDER: ignore-file` comment anywhere in the file
  disables all rules for that file.
- Line-level: `# PLACEHOLDER: ignore` (all rules) or
  `# PLACEHOLDER: ignore[PT001,PT003]` (specific) on the same line as a
  finding. The suppression applies to any finding whose reported line
  equals the comment line.

Implementation: pre-scan comments once per file, build a map
`HashMap<usize, SuppressionSet>`. Rules emit findings; the engine drops
suppressed ones just before reporting.

## CLI surface

```
PLACEHOLDER check [PATHS...]           # lint, exit 1 on findings
PLACEHOLDER check --format json
PLACEHOLDER check --format sarif
PLACEHOLDER check --files-from -       # read paths from stdin (pre-commit)
PLACEHOLDER list-rules                 # print all rules with codes and status
PLACEHOLDER explain PT003              # print the rule's long description
```

Exit codes: 0 = clean, 1 = findings, 2 = configuration/parse error.

## Output formats

Three from day one. SARIF later is annoying; do it now.

- `text` (default): `path:line:col: CODE message`, one per line, colored
  when stdout is a TTY. Group by file.
- `json`: array of `Finding` objects, schema documented in README.
- `sarif`: SARIF 2.1.0, schema matches what GitHub code-scanning expects.
  Biston's SARIF emitter is a good reference.

## Testing strategy

Three layers:

1. **Unit tests per rule.** Each rule module has a `#[cfg(test)]` block
   with 5–10 small Python snippets as string literals, each with the
   expected findings. Use `insta` for snapshot testing where the output
   is more than a line or two.
2. **Fixture-based integration tests.** A `tests/fixtures/` tree of
   small Python test files, each paired with an expected-findings JSON.
   A single integration test walks the tree and diffs.
3. **Self-dogfood.** Once the tool is green on its own fixtures, run it
   against biston's and ty-find's test suites and capture the findings
   as a baseline. Regressions show up as baseline diffs in CI.

Coverage target for v0.1: every rule has at least one positive case,
one negative case, and one suppression case. Don't chase line coverage.

## Repo scaffolding (one-time)

- `Cargo.toml` workspace, MSRV 1.75 (align with biston).
- `rust-toolchain.toml` pinned to stable.
- `.gitignore`, `LICENSE` (MIT, assuming that matches the org default —
  confirm before the first push).
- `README.md` — what it is, install, one example, link to rules page.
- `docs/rules/PT001.md` … `PT007.md` — one page per rule with rationale,
  examples, config, and suppression syntax. These become the `explain`
  command content (embed via `include_str!`).
- GitHub Actions: `ci.yml` (fmt, clippy, test on stable) and
  `release.yml` (crates.io publish on tag, mirroring biston's pattern).
- `pre-commit-hooks.yaml` in the repo root so other projects can
  consume it via the `pre-commit` framework.

## Implementation order

Ordered so each step produces something runnable. Don't merge steps.

### Step 1 — Scaffolding
- Workspace with two crates, no rules, no parsing.
- `PLACEHOLDER check PATH` prints "0 findings in N files discovered."
- Discovery walks the tree honoring `include` / `exclude` from config.
- CI green: fmt, clippy, a smoke test.

### Step 2 — Parse + rule framework
- Add tree-sitter-python. Parse every discovered file.
- Define `Rule` trait, `Context`, `Finding`, `Report`.
- Build the rule registry empty; engine loops over it.
- Add one trivial no-op rule (`PT000 hello-world` that fires on every
  test function) purely to prove the pipeline. Delete before release.

### Step 3 — First real rule (PT001, `conditional-test-logic`)
- Implement `is_test_function`.
- Implement the rule, manual cursor traversal.
- Unit tests, fixture test.
- Text output formatter.

### Step 4 — Rules PT002, PT007
- Two simple query-based rules to exercise the query path.
- Both are ~40 lines; doing them together is fine.

### Step 5 — Rules PT003, PT004
- PT003 needs an "assertion helper" list (starts with `assertEqual`,
  `assertTrue`, etc. for mixed suites; plus a user-configurable list).
- PT004 is counting; share the assert-finder with PT003.

### Step 6 — Rules PT005, PT006
- PT005 needs a URL/path regex plus the allow-list config.
- PT006 is decorator counting on function definitions.

### Step 7 — Suppression
- `# PLACEHOLDER: ignore` comment parser.
- File-level and line-level. Unit tests for the parser; integration
  tests that a suppressed rule doesn't fire.

### Step 8 — JSON and SARIF output
- JSON first — trivial with `serde_json`.
- SARIF next — use biston's emitter as a template. Validate against
  the SARIF 2.1.0 schema with one fixture.

### Step 9 — `list-rules` and `explain`
- Embed `docs/rules/*.md` at compile time.
- `list-rules` prints a table. `explain PTNNN` prints the markdown.

### Step 10 — `--files-from` and pre-commit
- Stdin and repeatable `--files` flags. Same semantics as biston's.
- Write `.pre-commit-hooks.yaml`.
- Test it against a throwaway checkout of one of your repos.

### Step 11 — Release prep
- README with a worked example.
- Crates.io metadata, trusted publishing via GHA.
- Tag v0.1.0.

Do not merge steps 7–10 before the rules are in. A tool with one
working rule and good suppression/output is more useful than seven rules
with no suppression.

## Dependencies (pin versions in `Cargo.toml`)

- `tree-sitter` + `tree-sitter-python` — parsing.
- `clap` (derive) — CLI.
- `serde` + `serde_json` + `toml` — config and JSON output.
- `ignore` — file discovery with gitignore semantics. Same crate biston
  uses.
- `rayon` — parallel file processing.
- `anyhow` + `thiserror` — error handling, thiserror for the library
  errors, anyhow in the CLI.
- `globset` — include/exclude matching.
- `insta` (dev) — snapshot tests.
- `colored` or `anstream` — TTY-aware colored output.

No regex crate needed for v0.1 — tree-sitter query predicates handle
what little pattern matching we need. Add `regex` only if PT005's
URL/path detection outgrows simple string tests.

## What this plan deliberately omits

- **No semantic resolution.** If a rule needs to know what `foo` means,
  it's out of scope for v0.1.
- **No cross-file analysis.** Every rule operates on one file's tree.
  This means PT003 can miss a test that delegates asserts to a helper
  in conftest.py — accepted limitation, documented.
- **No autofix.** Most of these smells don't have a safe automatic fix.
  Adding autofix later means per-rule fix routines; don't design for
  it yet.
- **No LSP server.** Editor integration goes through standard linter
  protocols (ruff-style). Not shipping an LSP at v0.1.
- **No Python distribution.** Pure Rust binary at v0.1. If org
  adoption wants `pip install PLACEHOLDER`, add a `maturin` build in
  v0.2 — it's a cargo feature flag, not a rewrite.

## Claude Code working notes

- Work one step at a time from the "Implementation order" list. Do not
  skip ahead. Each step should produce a commit (or a small stack of
  them) that leaves the tree green on CI.
- When implementing a rule, write the fixture test first, then the
  rule. This keeps the positive/negative cases honest.
- Do not invent new dependencies without asking. The list above is
  deliberately tight.
- When in doubt about pipeline shape (config, discovery, suppression,
  SARIF), read the equivalent module in biston's source and adapt.
  Don't paste — adapt. The two tools should feel like siblings, not
  clones (the irony is acknowledged).
- All occurrences of `PLACEHOLDER` are renames. Keep them consistent so
  a final sed pass works.
