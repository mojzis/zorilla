### `zorilla` — test-smell gate

- Runs on every commit (pre-commit hook `zorilla check`). Findings are not
  verdicts: run `zorilla guide triage` for the remedy ladder.
- Fix the test before suppressing. A legitimate exception gets
  `# zorilla: ignore[ZR###] -- <reason>` on the line; never a bare ignore.
- `zorilla explain ZR###` says why a rule fires and which line it reports on.
