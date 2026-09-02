# Tune

Reference for suppression and policy. Every knob here is repository-wide:
changing one changes what zorilla reports everywhere, so change it deliberately.

**Suppression.** Directives are `# zorilla:` comments. Text after the directive
is your reason and is ignored by the parser, so always write one.

- `# zorilla: ignore -- <reason>` drops every finding **on that same line**.
  Strictly that line: a directive above a statement does not reach it.
- `# zorilla: ignore[ZR001, ZR003] -- <reason>` drops only the listed codes on
  that line. Prefer it; a bare `ignore` also hides the next rule to fire there.
- `# zorilla: ignore-file -- <reason>` drops everything in the file, from
  anywhere in it. `# zorilla: ignore-file[ZR005]` narrows it to the listed codes.
- Codes are case-insensitive. An unrecognised directive is silently an ordinary
  comment, so check it against `zorilla list-rules`.
- The parser locks onto the first `#` on a line without reasoning about string
  literals, so `# zorilla: ignore` inside a docstring is read as a directive.

**Scope.** `include` and `exclude` decide which files are read at all. Both
*replace* the defaults rather than extending them, so list every pattern you
want. Explicit paths -- positional arguments, `--files`, `--files-from` --
bypass `include`, but `exclude` still applies: a hook lints every file it was
handed that is not excluded, and silently skips the rest.

```toml
[tool.zorilla]
include = ["tests/**/*.py", "**/test_*.py", "**/*_test.py", "**/conftest.py"]
exclude = ["**/fixtures/**"]
```

**Per-rule knobs.** Every rule takes `enabled = false`; five take more.

| Key | Default | Change it when |
|---|---|---|
| `ZR003.extra_helpers` | `[]` | a project-local assertion helper reads as no assertion |
| `ZR004.max_asserts` | `4` | bare asserts are conventional here and drown real findings |
| `ZR005.allowed_prefixes` | `[]` | a literal path or URL is genuinely local and safe |
| `ZR006.max_patches` | `3` | stacked `@patch` decorators are irreducible in this suite |
| `ZR008.max_patches` | `3` | the same, for `with patch(...)` context managers |

```toml
[tool.zorilla.rules.ZR004]
max_asserts = 6

[tool.zorilla.rules.ZR002]
enabled = false
```

Rule table keys are case-sensitive and uppercase: `[rules.zr004]` is silently
ignored, `[rules.ZR004]` is not.

**Precedence.** `zorilla.toml` > `pyproject.toml [tool.zorilla]` > defaults,
applied per directory. Discovery walks upward from the first path you name; in
each directory it checks `zorilla.toml`, then `pyproject.toml [tool.zorilla]`,
and the first directory holding either wins outright. A `pyproject.toml` with no
`[tool.zorilla]` table is skipped and the walk continues. Files are never
merged: a nested config replaces the one above it wholesale.

next: run `zorilla list-rules`
