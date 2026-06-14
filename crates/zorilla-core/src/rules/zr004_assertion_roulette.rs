//! `ZR004 assertion-roulette` — flag tests with too many bare asserts.
//!
//! ## Runtime-skip awareness
//!
//! A test whose first real body statement (after any leading docstring or
//! comments) is `self.skipTest(...)`, `self.skip(...)`, or
//! `pytest.skip(...)` is unconditionally skipped at runtime. Such tests
//! are treated the same as `@pytest.mark.skip`-decorated tests: their
//! bare-assert count is academic and ZR004 does not fire on them.
//!
//! # Rule
//!
//! A test littered with bare `assert x` statements fails obscurely: when
//! one of them trips, the pytest traceback shows the line number but
//! nothing else, and the reader has to reconstruct which check
//! broke — the "assertion roulette" smell. Adding a message to each
//! assert (`assert x, "explain"`) or replacing a wall of asserts with a
//! single rich check cures this.
//!
//! This rule fires **once per test function** whose count of **bare**
//! `assert_statement` nodes strictly exceeds
//! `[tool.zorilla.rules.ZR004] max_asserts` (default `4`). A bare
//! `assert_statement` has exactly one named child — the expression. An
//! `assert x, "msg"` carries a second named child (the message) and is
//! **not** counted.
//!
//! The walk descends into nested inline helpers, consistent with ZR001 /
//! ZR002 / ZR003.
//!
//! **Reported location**: the first bare `assert_statement` in source
//! order inside the offending test. Pointing at the test's `def` would
//! hide where the wall-of-asserts begins.
//!
//! ## Examples
//!
//! Positive — five bare asserts, one over the default threshold of 4:
//!
//! ```python
//! def test_many():
//!     a = f()
//!     assert a.x == 1   # ZR004 fires here
//!     assert a.y == 2
//!     assert a.z == 3
//!     assert a.w == 4
//!     assert a.v == 5
//! ```
//!
//! Negative — same five asserts but each with a message:
//!
//! ```python
//! def test_many_with_messages():
//!     a = f()
//!     assert a.x == 1, "x"
//!     assert a.y == 2, "y"
//!     assert a.z == 3, "z"
//!     assert a.w == 4, "w"
//!     assert a.v == 5, "v"
//! ```

use crate::ast::{self, count_bare_asserts_with_first, iter_test_functions};
use crate::report::{Finding, Severity};
use crate::rules::{Context, Rule};

/// The registered ZR004 rule instance.
pub static ZR004_ASSERTION_ROULETTE: AssertionRouletteRule = AssertionRouletteRule;

/// Zero-sized rule struct implementing [`Rule`] for ZR004.
pub struct AssertionRouletteRule;

impl Rule for AssertionRouletteRule {
    fn code(&self) -> &'static str {
        "ZR004"
    }

    fn name(&self) -> &'static str {
        "assertion-roulette"
    }

    fn doc(&self) -> &'static str {
        include_str!("../../../../docs/rules/ZR004.md")
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        let max_asserts = ctx.config.zr004.max_asserts;
        for test_fn in iter_test_functions(ctx.tree, ctx.source) {
            let Some(body) = test_fn.child_by_field_name("body") else {
                continue;
            };
            // Skip tests marked `@pytest.mark.skip` / `@pytest.mark.xfail`
            // (function-level or class-level). The body isn't executed,
            // so its assert count is academic — mirrors ZR007's gate.
            // `skipif` is conditional and intentionally not matched.
            if ast::test_has_skip_or_xfail_decorator(test_fn, ctx.source) {
                continue;
            }
            // Skip tests whose first real body statement is a runtime skip
            // call — `self.skipTest(...)`, `self.skip(...)`, or
            // `pytest.skip(...)`. The body never executes, so the
            // bare-assert count is academic.
            if ast::test_has_runtime_skip_call(test_fn, ctx.source) {
                continue;
            }
            let (count, first) = count_bare_asserts_with_first(body, ctx.source);
            if count > max_asserts {
                // `count > max_asserts` with `max_asserts: usize` implies
                // `count >= 1`, so `first` is guaranteed `Some`.
                let Some(first) = first else { continue };
                let start = first.start_position();
                out.push(Finding {
                    code: self.code(),
                    message: format!("test has too many bare assertions ({count} > {max_asserts})"),
                    file: ctx.file.to_path_buf(),
                    line: start.row + 1,
                    column: start.column + 1,
                    severity: Severity::Warning,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::parse::parse;
    use crate::rules::RuleConfig;
    use crate::suppress::Suppressions;
    use std::path::Path;

    fn run(src: &str) -> Vec<Finding> {
        run_with(src, &Config::default().rule_config())
    }

    fn run_with(src: &str, config: &RuleConfig) -> Vec<Finding> {
        let tree = parse(src).unwrap();
        let suppressions = Suppressions::empty();
        let ctx = Context {
            file: Path::new("example.py"),
            source: src,
            tree: &tree,
            config,
            suppressions: &suppressions,
        };
        let mut out = Vec::new();
        ZR004_ASSERTION_ROULETTE.check(&ctx, &mut out);
        out
    }

    #[test]
    fn fires_on_five_bare_asserts_at_default_threshold() {
        let src = "\
def test_many():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR004");
        // Reports at the first bare assert.
        assert_eq!(out[0].line, 2);
        assert_eq!(out[0].column, 5);
        assert!(out[0].message.contains('5'));
        assert!(out[0].message.contains('4'));
    }

    #[test]
    fn does_not_fire_on_four_bare_asserts() {
        let src = "\
def test_ok():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_six_message_carrying_asserts() {
        let src = "\
def test_many_with_messages():
    assert 1 == 1, \"a\"
    assert 2 == 2, \"b\"
    assert 3 == 3, \"c\"
    assert 4 == 4, \"d\"
    assert 5 == 5, \"e\"
    assert 6 == 6, \"f\"
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_test_with_no_asserts() {
        let src = "\
def test_empty_ish():
    do_work()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn counts_asserts_inside_nested_helpers() {
        // Asserts inside an inline helper count toward the enclosing
        // test's total.
        let src = "\
def test_with_helper():
    def check_all(a):
        assert a.x == 1
        assert a.y == 2
        assert a.z == 3
    a = f()
    assert a.kind == \"x\"
    assert a.ok
    check_all(a)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn respects_configured_max_asserts() {
        // With max_asserts = 2, three bare asserts fire.
        let src = "\
def test_three():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
";
        let mut cfg = Config::default().rule_config();
        cfg.zr004.max_asserts = 2;
        let out = run_with(src, &cfg);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
        assert!(out[0].message.contains("3 > 2"));
    }

    #[test]
    fn fires_on_method_of_test_class() {
        let src = "\
class TestThing:
    def test_method(self):
        assert 1 == 1
        assert 2 == 2
        assert 3 == 3
        assert 4 == 4
        assert 5 == 5
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn does_not_fire_on_pytest_mark_skip() {
        // Skipped tests don't execute — their bare-assert count is
        // academic. Mirrors ZR007's gate.
        let src = "\
@pytest.mark.skip
def test_x():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_skip_with_reason() {
        let src = "\
@pytest.mark.skip(reason=\"todo\")
def test_x():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_xfail() {
        let src = "\
@pytest.mark.xfail
def test_x():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_method_of_class_decorated_with_pytest_mark_skip() {
        let src = "\
@pytest.mark.skip
class TestX:
    def test_y(self):
        assert 1 == 1
        assert 2 == 2
        assert 3 == 3
        assert 4 == 4
        assert 5 == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_pytest_mark_skipif() {
        // `skipif` is conditional — body runs on the non-skipped path,
        // so the bare-assert count still counts. Matches ZR007.
        let src = "\
@pytest.mark.skipif(True, reason=\"x\")
def test_x():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR004");
    }

    #[test]
    fn does_not_fire_on_non_test_function_with_many_asserts() {
        let src = "\
def _helper():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5

def test_ok():
    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_runtime_skip_call_as_first_statement() {
        // A test whose first statement is `self.skipTest(...)` is
        // unconditionally skipped — its bare-assert count is academic.
        // Mirrors the decorator-based skip gate.
        let src = "\
class TestThing:
    def test_skipped_many_asserts(self):
        self.skipTest(\"not ready\")
        assert 1 == 1
        assert 2 == 2
        assert 3 == 3
        assert 4 == 4
        assert 5 == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_when_runtime_skip_is_not_first_statement() {
        // The skip call is after real asserts; the test still fires.
        let src = "\
def test_asserts_then_skip():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
    pytest.skip(\"reason\")
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR004");
    }
}
