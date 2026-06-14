//! `ZR007 empty-test` — flag tests that do nothing.
//!
//! # Rule
//!
//! A test whose body is a placeholder (`pass`, `...`, or just a
//! docstring) passes unconditionally and communicates nothing. It either
//! needs a real assertion or a `pytest.skip` marker that says so
//! explicitly. Leaving empty tests around rots the suite — they
//! contribute to the green bar without exercising anything.
//!
//! This rule fires **once per test function** whose body contains only
//! placeholder statements. A statement counts as a placeholder if it is
//! one of:
//!
//! - `pass`
//! - a bare ellipsis expression (`...`)
//! - a bare string expression (a docstring)
//!
//! Any non-placeholder statement — an assignment, call, assert, raise,
//! return, control-flow construct, etc. — disqualifies the test.
//!
//! ## Runtime-skip awareness
//!
//! In addition to `@pytest.mark.skip` / `@pytest.mark.xfail` decorators,
//! this rule also skips tests whose first real body statement (after any
//! leading docstring or comments) is a call to `self.skipTest(...)`,
//! `self.skip(...)`, or `pytest.skip(...)`. Such a body is not "empty"
//! in the harmful sense — the test is unconditionally skipped at runtime
//! and its placeholder-shaped body is intentional.
//!
//! ## Examples
//!
//! Positive — `pass`-only body:
//!
//! ```python
//! def test_todo():
//!     pass                # ZR007 fires at the def line
//! ```
//!
//! Positive — docstring-only body:
//!
//! ```python
//! def test_describes_intent():
//!     """This test will verify that ..."""
//! ```
//!
//! Negative — body has a real statement:
//!
//! ```python
//! def test_real():
//!     assert 1 == 1
//! ```

use tree_sitter::Node;

use crate::ast::{self, iter_test_functions};
use crate::report::{Finding, Severity};
use crate::rules::{Context, Rule};

/// Canonical finding message for ZR007.
pub const MESSAGE: &str = "test body is empty (pass/ellipsis/docstring only)";

/// The registered ZR007 rule instance.
pub static ZR007_EMPTY_TEST: EmptyTestRule = EmptyTestRule;

/// Zero-sized rule struct implementing [`Rule`] for ZR007.
pub struct EmptyTestRule;

impl Rule for EmptyTestRule {
    fn code(&self) -> &'static str {
        "ZR007"
    }

    fn name(&self) -> &'static str {
        "empty-test"
    }

    fn doc(&self) -> &'static str {
        include_str!("../../../../docs/rules/ZR007.md")
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        for test_fn in iter_test_functions(ctx.tree, ctx.source) {
            let Some(body) = test_fn.child_by_field_name("body") else {
                continue;
            };
            if ast::test_has_skip_or_xfail_decorator(test_fn, ctx.source) {
                continue;
            }
            // Skip tests whose first real body statement is a runtime skip
            // call — `self.skipTest(...)`, `self.skip(...)`, or
            // `pytest.skip(...)`. A body with only a runtime skip call as its
            // first statement is intentionally placeholder-shaped.
            if ast::test_has_runtime_skip_call(test_fn, ctx.source) {
                continue;
            }
            if body_is_empty(body) {
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

/// Is `body` a [`block`] whose named statement children are all
/// placeholders (`pass`, `...`, or a bare docstring)?
fn body_is_empty(body: Node<'_>) -> bool {
    // A `function_definition`'s body is a `block` node. Walk its *named*
    // children only: tree-sitter exposes colons and comments as unnamed
    // nodes that would otherwise need to be filtered.
    let mut cursor = body.walk();
    let mut any_stmt = false;
    for child in body.named_children(&mut cursor) {
        if child.kind() == "comment" {
            continue;
        }
        any_stmt = true;
        if !is_placeholder_statement(child) {
            return false;
        }
    }
    any_stmt
}

fn is_placeholder_statement(node: Node<'_>) -> bool {
    match node.kind() {
        "pass_statement" => true,
        "expression_statement" => {
            // Exactly one named child, and it is either an ellipsis or a
            // bare string. Multiple expressions (tuple) or anything else
            // disqualifies.
            let mut cursor = node.walk();
            let mut named = node.named_children(&mut cursor);
            let Some(first) = named.next() else {
                return false;
            };
            if named.next().is_some() {
                return false;
            }
            matches!(first.kind(), "ellipsis" | "string")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use crate::rules::RuleConfig;
    use crate::suppress::Suppressions;
    use std::path::Path;

    fn run(src: &str) -> Vec<Finding> {
        let tree = parse(src).unwrap();
        let suppressions = Suppressions::empty();
        let config = RuleConfig::default();
        let ctx = Context {
            file: Path::new("example.py"),
            source: src,
            tree: &tree,
            config: &config,
            suppressions: &suppressions,
        };
        let mut out = Vec::new();
        ZR007_EMPTY_TEST.check(&ctx, &mut out);
        out
    }

    #[test]
    fn fires_on_pass_only_body() {
        let src = "\
def test_todo():
    pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR007");
        assert_eq!(out[0].message, MESSAGE);
        assert_eq!(out[0].line, 1);
        assert_eq!(out[0].column, 1);
    }

    #[test]
    fn fires_on_ellipsis_only_body() {
        let src = "\
def test_todo():
    ...
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn fires_on_docstring_only_body() {
        let src = "\
def test_todo():
    \"\"\"Will verify the happy path.\"\"\"
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn fires_on_docstring_plus_pass() {
        let src = "\
def test_todo():
    \"\"\"Explain.\"\"\"
    pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn does_not_fire_on_real_assertion() {
        let src = "\
def test_real():
    assert 1 == 1
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_body_with_call() {
        let src = "\
def test_calls():
    do_work()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_docstring_plus_real_statement() {
        let src = "\
def test_mixed():
    \"\"\"Doc.\"\"\"
    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_each_of_multiple_empty_tests() {
        let src = "\
def test_a():
    pass


def test_b():
    ...
";
        let out = run(src);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].line, 1);
        assert_eq!(out[1].line, 5);
    }

    #[test]
    fn does_not_fire_on_empty_non_test_function() {
        let src = "\
def _helper():
    pass


def test_real():
    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_method_of_test_class() {
        let src = "\
class TestThing:
    def test_empty(self):
        pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn does_not_fire_on_async_test_with_await() {
        let src = "\
async def test_awaits():
    await something()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_skip() {
        let src = "\
@pytest.mark.skip
def test_x():
    pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_xfail() {
        let src = "\
@pytest.mark.xfail
def test_x():
    pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_skip_call_form_with_reason() {
        let src = "\
@pytest.mark.skip(reason=\"todo\")
def test_x():
    pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_xfail_call_form_strict() {
        let src = "\
@pytest.mark.xfail(strict=True)
def test_x():
    pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_pytest_mark_skipif() {
        // `skipif` is conditional — the body is expected to do real work
        // on the non-skipped path, so an empty body still flags.
        let src = "\
@pytest.mark.skipif(True, reason=\"x\")
def test_x():
    pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR007");
    }

    #[test]
    fn fires_on_user_skip_decorator_not_under_pytest_mark() {
        // A bare `@skip` decorator (one segment) does NOT match the
        // three-segment `pytest.mark.skip` pattern — fire as usual.
        let src = "\
@skip
def test_x():
    pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR007");
    }

    #[test]
    fn fires_on_four_segment_decorator_chain_ending_in_skip() {
        // `@some.pkg.pytest.mark.skip` is a 5-segment chain that happens
        // to end in `skip`. The gate is exact-length-3 against
        // `["pytest", "mark", "skip"]`, so the rule must still fire.
        // Locks in the brief's "deeper chains must NOT match" requirement.
        let src = "\
@some.pytest.mark.skip
def test_x():
    pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR007");
    }

    #[test]
    fn does_not_fire_on_pytest_mark_skip_among_other_decorators() {
        // When a stack contains both an unrelated decorator and a
        // `@pytest.mark.skip`, the skip still short-circuits.
        let src = "\
@pytest.mark.slow
@pytest.mark.skip(reason=\"todo\")
def test_x():
    pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_method_of_class_decorated_with_pytest_mark_skip() {
        // Class-level `@pytest.mark.skip` should suppress ZR007 on every
        // method body inside it — even if the method itself has no
        // decorator. This is the common placeholder-stub pattern in
        // ~/git/esl.
        let src = "\
@pytest.mark.skip
class TestX:
    def test_y(self):
        pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_method_of_class_decorated_with_pytest_mark_xfail() {
        let src = "\
@pytest.mark.xfail
class TestX:
    def test_y(self):
        pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_method_of_class_decorated_with_pytest_mark_skip_with_reason() {
        let src = "\
@pytest.mark.skip(reason=\"todo\")
class TestX:
    def test_y(self):
        pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_method_of_class_decorated_with_pytest_mark_skipif() {
        // `skipif` is conditional — empty bodies inside still flag.
        let src = "\
@pytest.mark.skipif(True, reason=\"x\")
class TestX:
    def test_y(self):
        pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR007");
    }

    #[test]
    fn does_not_fire_when_class_is_decorated_at_method_level_too() {
        // Both the class and the method carry skip-style decorators.
        // Each gate independently suppresses; the rule should not fire
        // and the lookup should not panic.
        let src = "\
@pytest.mark.skip
class TestX:
    @pytest.mark.skip
    def test_y(self):
        pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn comment_only_body_is_still_empty() {
        // A block of only comments reaches Python as a syntactically
        // invalid function (requires at least one statement). But tree
        // -sitter parses it with a block that has zero named non-comment
        // children — treat that as an empty body for this rule.
        let src = "\
def test_commented_out():
    # TODO: write this
    pass
";
        // pass is still there, so the rule fires.
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn does_not_fire_on_runtime_skip_call_as_first_statement() {
        // A test that starts with `pytest.skip(...)` is unconditionally
        // skipped at runtime — even if the rest of the body is a `pass`,
        // the body is not "empty" in the harmful sense. Mirrors the
        // decorator gate.
        //
        // NOTE: `pytest.skip(...)` is a real call statement, so
        // `body_is_empty` would return `false` anyway (it's not a
        // placeholder). The gate here ensures parity with the decorator
        // path and guards against future refactors.
        let src = "\
def test_runtime_skip_pass():
    pytest.skip(\"reason\")
    pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn still_fires_on_pass_only_body_without_runtime_skip() {
        // Guard: a plain `pass`-only body (no skip call) still fires.
        // The runtime-skip gate must not affect unrelated empty tests.
        let src = "\
def test_pass_only():
    pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR007");
    }
}
