//! `ZR003 no-assertion` — flag tests whose body asserts nothing.
//!
//! # Rule
//!
//! A test that never asserts never fails — it only exercises code for
//! side effects. Pytest treats the absence of an exception as success,
//! so a test with no check is green forever. This rule fires once per
//! test function whose body contains **none** of:
//!
//! - an `assert_statement` (bare or with message);
//! - a call whose function's final identifier is a known assertion
//!   helper (see `DEFAULT_ZR003_HELPERS` in [`crate::config`]) or an
//!   entry in the user's `[tool.zorilla.rules.ZR003] extra_helpers`;
//! - a `with <expr>.raises(...)` / `with <expr>.warns(...)` context
//!   manager (matched by final identifier — so `pytest.raises(...)`,
//!   `self.assertRaises(...)`, and any custom `foo.raises(...)` all
//!   count). The final-identifier match is deliberately permissive: it
//!   mirrors how assertion-helper matching works and keeps user-defined
//!   exception testers from producing false positives.
//!
//! The walk descends into nested inline helpers — if the test calls a
//! nested `def` that contains an assert, the test counts as asserting.
//! This matches ZR001 / ZR002 / ZR007.
//!
//! **Reported location**: the test function's `def` start position (the
//! start of `function_definition`). The intent is "this whole test is
//! assertion-free"; pointing anywhere inside is misleading.
//!
//! ## Examples
//!
//! Positive — no assertion at all:
//!
//! ```python
//! def test_does_nothing():       # ZR003 fires here
//!     setup()
//!     act()
//! ```
//!
//! Negative — plain assert:
//!
//! ```python
//! def test_ok():
//!     assert value() == 1
//! ```
//!
//! Negative — `pytest.raises` context manager counts as an assertion:
//!
//! ```python
//! import pytest
//!
//! def test_raises():
//!     with pytest.raises(ValueError):
//!         boom()
//! ```

use crate::ast::{has_any_assert, iter_test_functions};
use crate::report::{Finding, Severity};
use crate::rules::{Context, Rule};

/// Canonical finding message for ZR003.
pub const MESSAGE: &str = "test has no assertion";

/// The registered ZR003 rule instance.
pub static ZR003_NO_ASSERTION: NoAssertionRule = NoAssertionRule;

/// Zero-sized rule struct implementing [`Rule`] for ZR003.
pub struct NoAssertionRule;

impl Rule for NoAssertionRule {
    fn code(&self) -> &'static str {
        "ZR003"
    }

    fn name(&self) -> &'static str {
        "no-assertion"
    }

    fn doc(&self) -> &'static str {
        include_str!("../../../../docs/rules/ZR003.md")
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        let helpers = &ctx.config.zr003.helpers;
        for test_fn in iter_test_functions(ctx.tree, ctx.source) {
            let Some(body) = test_fn.child_by_field_name("body") else {
                continue;
            };
            if !has_any_assert(body, ctx.source, helpers) {
                let start = test_fn.start_position();
                out.push(Finding {
                    code: self.code(),
                    message: MESSAGE.to_string(),
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
        ZR003_NO_ASSERTION.check(&ctx, &mut out);
        out
    }

    #[test]
    fn fires_on_test_with_no_assertion() {
        let src = "\
def test_nothing():
    do_something()
    do_more()
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR003");
        assert_eq!(out[0].message, MESSAGE);
        assert_eq!(out[0].line, 1);
        assert_eq!(out[0].column, 1);
    }

    #[test]
    fn does_not_fire_on_bare_assert() {
        let src = "\
def test_ok():
    assert value() == 1
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_message_assert() {
        let src = "\
def test_ok():
    assert value() == 1, \"should be 1\"
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_raises_context_manager() {
        let src = "\
import pytest

def test_raises():
    with pytest.raises(ValueError):
        boom()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_warns_context_manager() {
        let src = "\
import pytest

def test_warns():
    with pytest.warns(DeprecationWarning):
        legacy()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_custom_raises_context_manager() {
        // Documented behavior: the `raises`/`warns` match is by final
        // identifier, not by `pytest.` prefix. A user-defined
        // `foo.raises(...)` context manager still suppresses ZR003.
        let src = "\
def test_custom():
    with foo.raises(ValueError):
        boom()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_builtin_assertion_helper_call() {
        let src = "\
class TestX:
    def test_uses_helper(self):
        self.assertEqual(1, 1)
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_assert_inside_nested_helper() {
        // Nested inline helper with an assert — the enclosing test
        // still counts as asserting, per ZR001/ZR002 convention.
        let src = "\
def test_with_helper():
    def inner():
        assert True
    inner()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_method_of_test_class() {
        let src = "\
class TestNothing:
    def test_method(self):
        do_work(self)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn does_not_fire_on_non_test_function() {
        let src = "\
def _helper():
    do_work()

def test_ok():
    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn extra_helpers_suppress_finding() {
        // With `expect` registered as an extra helper, the test looks
        // asserting and should not fire.
        let src = "\
def test_custom():
    expect(value()).to_be(1)
";
        let mut cfg = Config::default().rule_config();
        cfg.zr003.helpers.insert("to_be".to_string());
        assert!(run_with(src, &cfg).is_empty());
    }

    #[test]
    fn multiple_test_functions_each_get_their_own_finding() {
        let src = "\
def test_a():
    do()

def test_b():
    assert True

def test_c():
    log()
";
        let out = run(src);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].line, 1);
        assert_eq!(out[1].line, 7);
    }
}
