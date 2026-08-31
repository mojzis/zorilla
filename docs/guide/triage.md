# Triage

A check run reported findings. Work them one at a time.

**Read the finding.** The shape is `path:line:col: CODE name: message`; line and
column are 1-indexed. `zorilla explain ZR003` prints the long-form rule
documentation, with examples of what does and does not fire.

**Apply the remedy ladder.** Take the first entry that matches the code you are
holding; the order is roughly cheapest fix first.

1. ZR007 empty-test: the body is `pass`, `...` or a docstring. Write the test or
   delete it. An empty test is a false green, not a placeholder.
2. ZR003 no-assertion: add the assertion the test is missing. If the call under
   test *is* the assertion because it raises on failure, say so with
   `pytest.raises`; if it is a project-local assertion helper, register its name
   in `ZR003.extra_helpers` rather than editing the test.
3. ZR002 sleep-in-test: wait on the condition the test is actually waiting for,
   or inject a fake clock. A sleep is flake, wasted wall-clock, or both.
4. ZR001 conditional-test-logic: a branch means the test does not know what it
   asserts. A loop over cases becomes `@pytest.mark.parametrize`; an `if` over
   environments becomes two tests or a skip marker; a `try`/`except` becomes
   `pytest.raises`.
5. ZR004 assertion-roulette: give the bare asserts messages, or split the test
   so each one has a single subject. A failure should name itself.
6. ZR006 patch-stack and ZR008 context-patch-stack: too much mocking for one
   test. Extract a seam and build a fake once, or move the setup into a
   fixture.
7. ZR005 mystery-guest: replace the path or URL with `tmp_path`, a fixture, or a
   local double. If the literal is genuinely local and safe, add its prefix to
   `ZR005.allowed_prefixes`.
8. The smell is deliberate, and only then: suppress it with
   `# zorilla: ignore[ZR001] -- <reason>` on the offending line. The reason is
   required, and the bracketed code keeps the suppression narrow.

**Around every edit.** Run the test suite before you touch anything. Refactor.
Run the test suite again. Then re-check the file you edited, by name:

```bash
zorilla check tests/test_orders.py
```

A named file is linted directly, so the answer comes back in milliseconds
instead of a whole-tree scan. The finding must be gone and no new finding may
appear. If either fails, revert the edit.

**Do not:**

- Do not raise `ZR004.max_asserts` or `ZR006.max_patches`, and do not set
  `enabled = false`, to make a finding disappear. Those are repository-wide
  policy, decided by humans, and changing one hides every other finding it
  covers. See `zorilla guide tune`.
- Do not add `# zorilla: ignore` without a reason.
- Do not suppress a finding to pass a gate.
- Do not weaken an assertion to satisfy a rule.

next: run `zorilla check` on the file you edited
