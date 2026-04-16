//! `ZR001 conditional-test-logic` — flag control-flow inside a test.
//!
//! # Rule
//!
//! A pytest test function should read as a linear "arrange / act / assert".
//! Control-flow constructs (`if`, `for`, `while`, `try`) hide intent,
//! invite coverage holes, and often signal that two scenarios are being
//! smuggled into one test. Extract the branches into parametrized cases
//! (`@pytest.mark.parametrize`) or separate tests instead.
//!
//! This rule fires **once per test function** whose body contains any
//! `if_statement`, `for_statement`, `while_statement`, or `try_statement`
//! — directly or nested inside helpers defined within the test. The
//! reported location is the first offending statement in source order.
//!
//! ## Examples
//!
//! Positive — a branch inside the test:
//!
//! ```python
//! def test_branch():
//!     x = compute()
//!     if x > 0:           # ZR001 fires here
//!         assert x == 1
//!     else:
//!         assert x == -1
//! ```
//!
//! Positive — conditional inside a helper nested inside the test:
//!
//! ```python
//! def test_with_helper():
//!     def inner(x):
//!         if x:           # ZR001 fires here (still "inside" the test)
//!             return 1
//!         return 0
//!     assert inner(True) == 1
//! ```
//!
//! Negative — `if __name__ == "__main__":` at module scope is not inside
//! any test function and is left alone.

use std::ops::ControlFlow;

use tree_sitter::Node;

use crate::ast::{iter_test_functions, walk_descendants};
use crate::report::{Finding, Severity};
use crate::rules::{Context, Rule};

/// Canonical finding message for ZR001.
pub const MESSAGE: &str = "test function has conditional logic (if/for/while/try)";

/// The registered ZR001 rule instance.
pub static ZR001_CONDITIONAL: ConditionalRule = ConditionalRule;

/// Zero-sized rule struct implementing [`Rule`] for ZR001.
pub struct ConditionalRule;

impl Rule for ConditionalRule {
    fn code(&self) -> &'static str {
        "ZR001"
    }

    fn name(&self) -> &'static str {
        "conditional-test-logic"
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        for test_fn in iter_test_functions(ctx.tree, ctx.source) {
            let Some(body) = test_fn.child_by_field_name("body") else {
                continue;
            };
            if let Some(offender) = find_first_conditional(body) {
                let start = offender.start_position();
                out.push(Finding {
                    code: self.code(),
                    message: MESSAGE.to_string(),
                    file: ctx.file.to_path_buf(),
                    // tree-sitter points are 0-indexed; findings are 1-indexed.
                    line: start.row + 1,
                    column: start.column + 1,
                    severity: Severity::Warning,
                });
            }
        }
    }
}

/// Does `kind` name one of the four offending statement kinds?
fn is_conditional_kind(kind: &str) -> bool {
    matches!(kind, "if_statement" | "for_statement" | "while_statement" | "try_statement")
}

/// Walk `body` depth-first (source order) and return the first node whose
/// kind is one of the conditional kinds. Descends into nested
/// `function_definition` / `class_definition` children too — PLAN.md says
/// "direct or nested".
fn find_first_conditional(body: Node<'_>) -> Option<Node<'_>> {
    match walk_descendants(body, |node| {
        if is_conditional_kind(node.kind()) {
            ControlFlow::Break(node)
        } else {
            ControlFlow::Continue(())
        }
    }) {
        ControlFlow::Break(node) => Some(node),
        ControlFlow::Continue(()) => None,
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
        ZR001_CONDITIONAL.check(&ctx, &mut out);
        out
    }

    #[test]
    fn fires_once_on_test_with_if() {
        let src = "\
def test_has_if():
    x = 1
    if x:
        assert x
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR001");
        assert_eq!(out[0].message, MESSAGE);
        assert_eq!(out[0].line, 3);
        assert_eq!(out[0].column, 5);
    }

    #[test]
    fn fires_on_test_with_for_loop() {
        let src = "\
def test_has_for():
    total = 0
    for i in range(3):
        total += i
    assert total == 3
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn fires_on_test_with_try() {
        let src = "\
def test_has_try():
    try:
        do()
    except Exception:
        pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
        assert_eq!(out[0].column, 5);
    }

    #[test]
    fn fires_on_conditional_inside_nested_helper() {
        // The helper is defined inside the test, and the conditional is
        // inside the helper. PLAN.md says "direct or nested" — this must
        // still fire.
        let src = "\
def test_with_helper():
    def inner(x):
        if x:
            return 1
        return 0
    assert inner(True) == 1
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        // Reported at the `if` inside `inner`, not at `def inner`.
        assert_eq!(out[0].line, 3);
        assert_eq!(out[0].column, 9);
    }

    #[test]
    fn does_not_fire_on_plain_test() {
        let src = "\
def test_plain():
    x = 1 + 1
    assert x == 2
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_if_name_main_at_module_scope() {
        // `if __name__ == "__main__":` at module scope is outside every
        // test function, so ZR001 must leave it alone.
        let src = "\
def test_ok():
    assert True


if __name__ == \"__main__\":
    test_ok()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_conditional_in_nonnested_helper_outside_test() {
        // Sanity: a conditional inside a top-level helper that is merely
        // *called* from a test function doesn't count — the rule only
        // looks inside test function bodies.
        let src = "\
def _helper(x):
    if x > 0:
        return x
    return -x


def test_uses_helper():
    assert _helper(-1) == 1
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_once_per_test_function_even_with_multiple_conditionals() {
        // Two conditionals in one test — still only one finding, at the
        // first one.
        let src = "\
def test_many():
    x = 1
    if x:
        pass
    for i in range(3):
        pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn fires_on_each_of_multiple_test_functions() {
        let src = "\
def test_a():
    if True:
        pass


def test_b():
    while True:
        break
";
        let out = run(src);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].line, 2);
        assert_eq!(out[1].line, 7);
    }

    #[test]
    fn fires_on_method_of_test_class() {
        let src = "\
class TestSomething:
    def test_method(self):
        if self:
            pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
    }
}
