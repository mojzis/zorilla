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
//! at the outer scope. The reported location is the first offending
//! statement in source order.
//!
//! ## Refinements
//!
//! Three patterns look like control flow to a naive walk but read as
//! linear test code in practice and are therefore exempted:
//!
//! - **`try` / `finally` without `except`** — pure cleanup, not
//!   branching. Only `try` blocks that contain at least one
//!   `except_clause` fire.
//! - **`for` over bare or helper-only asserts** — a parametrize-by-loop
//!   pattern (`for case in cases: assert case.ok`) where the loop body's
//!   top-level statements are all `assert_statement` or assertion-helper
//!   calls. Empty / non-pure for-loops still fire.
//! - **Control flow inside a nested `def` or `lambda` within the test** —
//!   the outer test's control flow is the only concern. A helper
//!   defined inline can have whatever structure it needs.
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
//! Negative — `if __name__ == "__main__":` at module scope is not inside
//! any test function and is left alone.
//!
//! Negative — `try` / `finally` cleanup with no `except` clause is not
//! flagged.
//!
//! Negative — a `for` loop whose body is only bare or helper-only
//! `assert`s is a parametrize-substitute and is not flagged.

use std::collections::HashSet;
use std::ops::ControlFlow;

use tree_sitter::Node;

use crate::ast::{call_final_name, iter_test_functions, walk_descendants_pruned};
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

    fn doc(&self) -> &'static str {
        include_str!("../../../../docs/rules/ZR001.md")
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        let helpers = &ctx.config.zr003.helpers;
        for test_fn in iter_test_functions(ctx.tree, ctx.source) {
            let Some(body) = test_fn.child_by_field_name("body") else {
                continue;
            };
            if let Some(offender) = find_first_conditional(body, ctx.source, helpers) {
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

/// Walk `body` depth-first (source order, pre-order) and return the
/// first node whose kind names a conditional construct that actually
/// counts. The walk skips nested `function_definition` / `lambda`
/// subtrees — the outer test's control flow is what the rule cares
/// about; an inline helper's internal structure isn't.
///
/// Three refinements rule out look-alikes:
/// - `try_statement` only fires when it has at least one `except_clause`
///   ([`has_except_clause`]). A `try` / `finally` with no `except` is
///   cleanup, not branching.
/// - `for_statement` is skipped when its body's top-level statements
///   are all `assert_statement` or known assertion-helper calls
///   ([`for_body_is_only_asserts`]). That's a parametrize-substitute,
///   not branching.
/// - `if_statement` and `while_statement` always fire when found at the
///   outer scope.
fn find_first_conditional<'tree>(
    body: Node<'tree>,
    source: &str,
    helpers: &HashSet<String>,
) -> Option<Node<'tree>> {
    let result = walk_descendants_pruned(
        body,
        |n| !matches!(n.kind(), "function_definition" | "lambda"),
        |node| match node.kind() {
            "if_statement" | "while_statement" => ControlFlow::Break(node),
            "try_statement" => {
                if has_except_clause(node) {
                    ControlFlow::Break(node)
                } else {
                    ControlFlow::Continue(())
                }
            }
            "for_statement" => {
                if for_body_is_only_asserts(node, source, helpers) {
                    ControlFlow::Continue(())
                } else {
                    ControlFlow::Break(node)
                }
            }
            _ => ControlFlow::Continue(()),
        },
    );
    match result {
        ControlFlow::Break(node) => Some(node),
        ControlFlow::Continue(()) => None,
    }
}

/// Does `try_node` (a `try_statement`) carry at least one
/// `except_clause` named child? `try` / `finally` without an `except`
/// is cleanup, not branching.
fn has_except_clause(try_node: Node<'_>) -> bool {
    let mut cursor = try_node.walk();
    let found = try_node.named_children(&mut cursor).any(|child| child.kind() == "except_clause");
    found
}

/// Does the body of `for_node` consist solely of bare/message
/// `assert_statement`s or `expression_statement`s wrapping a call
/// whose final identifier is in `helpers`?
///
/// This is the parametrize-by-loop pattern (`for case in cases: assert
/// case.ok`) — semantically the same as a parametrized test. The walk
/// only inspects the loop's **top-level** statements; a nested
/// conditional inside an assert expression would not appear here as a
/// statement-kind child and is not considered.
///
/// Returns `false` for empty bodies (no named children). An empty
/// loop is still a loop and should still fire — the brief is
/// explicit on this point.
fn for_body_is_only_asserts(for_node: Node<'_>, source: &str, helpers: &HashSet<String>) -> bool {
    let Some(body) = for_node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    let mut any = false;
    let all_asserts_or_helpers = body.named_children(&mut cursor).all(|child| {
        any = true;
        match child.kind() {
            "assert_statement" => true,
            "expression_statement" => {
                // An `expression_statement` wraps exactly one expression.
                // We want it to be a `call` whose final identifier is a
                // known assertion helper.
                let Some(inner) = child.named_child(0) else {
                    return false;
                };
                if inner.kind() != "call" {
                    return false;
                }
                let Some(name) = call_final_name(inner, source) else {
                    return false;
                };
                helpers.contains(name)
            }
            _ => false,
        }
    });
    any && all_asserts_or_helpers
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
    fn does_not_fire_on_try_finally_only() {
        // A `try` with only a `finally` clause is cleanup, not branching
        // test logic — it's the test setup/teardown idiom, not a branch
        // between scenarios. Per the rollout summary it must not fire.
        let src = "\
def test_cleanup():
    try:
        do()
    finally:
        cleanup()
";
        assert!(run(src).is_empty());
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
    fn does_not_fire_on_if_inside_nested_def() {
        // SEMANTICS CHANGE: previously `fires_on_conditional_inside_nested_helper`
        // asserted that the inner `if` fires. The rollout summary
        // identified that nested-def control flow is a systematic false
        // positive — helpers defined inline can have whatever structure
        // they need. The outer test's control flow is the only concern.
        let src = "\
def test_with_helper():
    def inner(x):
        if x:
            return 1
        return 0
    assert inner(True) == 1
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_if_inside_lambda() {
        // Same reasoning as nested-def: a `lambda` body is its own
        // scope; whatever conditional expression it carries belongs to
        // the helper, not the test.
        let src = "\
def test_with_lambda():
    pick = lambda x: 1 if x else 0
    assert pick(True) == 1
";
        assert!(run(src).is_empty());
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

    #[test]
    fn does_not_fire_on_for_loop_of_only_bare_asserts() {
        // Parametrize-by-loop pattern: each iteration is a single
        // assert. Semantically the same as a parametrized test —
        // exempt from ZR001.
        let src = "\
def test_each_case():
    for case in CASES:
        assert case.ok
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_for_loop_of_only_helper_asserts() {
        // `assertEqual` is in the built-in helper set; an
        // `expression_statement` wrapping a call to it is the same kind
        // of pattern as a bare assert.
        let src = "\
def test_each_case():
    for case in CASES:
        self.assertEqual(case.actual, case.expected)
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_for_loop_with_real_logic_alongside_asserts() {
        // A non-assert top-level statement (`y = ...` assignment) means
        // the loop is doing real work, not just iterating through
        // assertions. Still fires.
        let src = "\
def test_each_case():
    for case in CASES:
        y = transform(case)
        assert y
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn fires_on_for_loop_calling_unknown_helper() {
        // A call to something that isn't in the helpers set is not a
        // recognised assertion form, so the body isn't "only asserts".
        let src = "\
def test_each_case():
    for case in CASES:
        process(case)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn fires_on_for_loop_with_empty_body() {
        // `for x in xs: pass` is still a loop. Empty / placeholder
        // bodies are explicitly NOT exempted by the for-of-asserts
        // refinement — only loops whose bodies actually consist of
        // asserts qualify.
        let src = "\
def test_empty_loop():
    for case in CASES:
        pass
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
    }
}
