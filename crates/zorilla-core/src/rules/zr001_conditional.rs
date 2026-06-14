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
//! `if_statement`, `for_statement`, `while_statement`, `try_statement`,
//! or `match_statement` at the outer scope. The reported location is
//! the first offending statement in source order.
//!
//! ## Refinements
//!
//! Four patterns look like control flow to a naive walk but read as
//! linear test code in practice and are therefore exempted:
//!
//! - **`try` / `finally` without `except`** — pure cleanup, not
//!   branching. Only `try` blocks that contain at least one
//!   `except_clause` fire.
//! - **`for` over asserts (with optional intervening assignments,
//!   comments, and `with` wrappers)** — a parametrize-by-loop pattern
//!   (`for case in cases: assert case.ok`, `for m in months: s =
//!   compute(m); assert s == 0`, or `for case in cases: with
//!   self.subTest(...): self.assertX(...)`) where the loop body has at
//!   least one assert-bearing statement, plain `assignment` /
//!   `augmented_assignment` and inline `# comments` interleaved,
//!   optionally wrapped in `with` blocks (whose bodies recursively
//!   satisfy the same shape), and no control flow anywhere inside.
//!   Helper-call asserts (`self.assertEqual(...)`) count alongside bare
//!   `assert`s.
//! - **`for` of pure side-effect calls inside an assertless test** — a
//!   mis-named fixture (`def test_env(): ...; for k in KEYS:
//!   os.environ.pop(k)`) is doing cleanup, not branching. When the
//!   enclosing test body has zero asserts anywhere and the loop body
//!   contains only call statements or assignments, the loop is exempt.
//! - **Control flow inside a nested `def` or `lambda` within the test** —
//!   the outer test's control flow is the only concern. A helper
//!   defined inline can have whatever structure it needs.
//! - **Runtime skip call as the first real body statement** — a test
//!   whose very first (non-docstring, non-comment) statement is a call
//!   to `self.skipTest(...)`, `self.skip(...)`, or `pytest.skip(...)`
//!   is unconditionally skipped at runtime and is treated identically
//!   to a `@pytest.mark.skip`-decorated test: its body is not executed,
//!   so structural smells are irrelevant.
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

use crate::ast::{self, call_final_name, iter_test_functions, walk_descendants_pruned};
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
            // Skip tests marked `@pytest.mark.skip` / `@pytest.mark.xfail`
            // (function-level or class-level). The body isn't executed,
            // so its structural smells are irrelevant — flagging them
            // produces a noisy false positive across placeholder stubs
            // (esl's 50 `@pytest.mark.skip` class-decorated methods, etc.).
            // Mirrors ZR007's skip-decorator gate. `skipif` is conditional
            // and intentionally not matched.
            if ast::test_has_skip_or_xfail_decorator(test_fn, ctx.source) {
                continue;
            }
            // Skip tests whose first real body statement is a runtime skip
            // call — `self.skipTest(...)`, `self.skip(...)`, or
            // `pytest.skip(...)`. Unconditional runtime skips are equivalent
            // to decorator-based skips: the body never executes and its
            // structural smells are irrelevant.
            if ast::test_has_runtime_skip_call(test_fn, ctx.source) {
                continue;
            }
            // Pre-compute whether the enclosing test function body contains
            // *any* assert-shaped construct anywhere — but only at the
            // outer test's scope. The cleanup-loop downgrade
            // ([`for_body_is_pure_cleanup`]) only applies when this is
            // `false`: a `for` over plain calls is exempt only inside a
            // mis-named "fixture" test that does no asserting at all
            // (e.g. alma's `def test_env(): ...; for k in KEYS:
            // os.environ.pop(k)`). The walk skips nested
            // `function_definition` / `lambda` subtrees to match the rest
            // of the rule's posture — an assert inside an inline helper
            // does not invalidate the cleanup shape any more than control
            // flow inside that helper triggers the rule.
            let test_body_has_asserts = outer_test_body_has_assert(body, ctx.source, helpers);
            if let Some(offender) =
                find_first_conditional(body, ctx.source, helpers, test_body_has_asserts)
            {
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
/// Four refinements rule out look-alikes:
/// - `try_statement` only fires when it has at least one `except_clause`
///   ([`has_except_clause`]). A `try` / `finally` with no `except` is
///   cleanup, not branching.
/// - `for_statement` is skipped when its body matches
///   [`for_body_is_parametrize_substitute`]: at least one
///   assert-bearing statement, with only plain assignments interleaved
///   and no control flow at any nesting level. That's a
///   parametrize-substitute, not branching.
/// - `for_statement` is also skipped — only when the enclosing test
///   body contains no asserts anywhere — by
///   [`for_body_is_pure_cleanup`]: a loop of plain call statements /
///   assignments inside a mis-named fixture-shaped test. Caller passes
///   `test_body_has_asserts` so this downgrade is disabled the moment
///   any assert appears in the enclosing test.
/// - `if_statement`, `while_statement`, and `match_statement` always
///   fire when found at the outer scope. (PEP 634 structural pattern
///   matching is multi-branch flow by definition.)
fn find_first_conditional<'tree>(
    body: Node<'tree>,
    source: &str,
    helpers: &HashSet<String>,
    test_body_has_asserts: bool,
) -> Option<Node<'tree>> {
    let result = walk_descendants_pruned(
        body,
        |n| !matches!(n.kind(), "function_definition" | "lambda"),
        |node| match node.kind() {
            "if_statement" | "while_statement" | "match_statement" => ControlFlow::Break(node),
            "try_statement" => {
                if has_except_clause(node) {
                    ControlFlow::Break(node)
                } else {
                    ControlFlow::Continue(())
                }
            }
            "for_statement" => {
                if for_body_is_parametrize_substitute(node, source, helpers) {
                    ControlFlow::Continue(())
                } else if !test_body_has_asserts && for_body_is_pure_cleanup(node) {
                    // The enclosing test function has no asserts anywhere —
                    // it's a mis-named fixture using a loop purely to set
                    // up / tear down state. Treat the loop as cleanup, not
                    // branching. Scope is deliberately narrow: any assert
                    // in the test body disables this downgrade.
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

/// Like [`crate::ast::has_any_assert`] but pruned: nested
/// `function_definition` and `lambda` subtrees are skipped during the
/// walk. Used by the cleanup-loop downgrade so an assert *inside an
/// inline helper* doesn't count as the test asserting anything —
/// consistent with `find_first_conditional`, which already treats those
/// subtrees as opaque.
fn outer_test_body_has_assert(body: Node<'_>, source: &str, helpers: &HashSet<String>) -> bool {
    walk_descendants_pruned(
        body,
        |n| !matches!(n.kind(), "function_definition" | "lambda"),
        |node| {
            if is_assert_shaped(node, source, helpers) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        },
    )
    .is_break()
}

/// Is `node` one of the three assert-shaped node kinds the rest of
/// `zorilla` recognises — a bare/message `assert_statement`, a `call`
/// to a known assertion helper, or a `with` over `pytest.raises` /
/// `assertRaises` / `assertWarns` / `pytest.warns` (or their *Regex
/// variants)? Mirrors `crate::ast::classify_assert_hit` but inline so
/// the pruned walk above can short-circuit on the first hit.
fn is_assert_shaped(node: Node<'_>, source: &str, helpers: &HashSet<String>) -> bool {
    match node.kind() {
        "assert_statement" => true,
        "call" => call_final_name(node, source)
            .is_some_and(|name| helpers.contains(name) || is_helper_prefix(name)),
        "with_statement" => with_item_is_assertion_context_manager(node, source),
        _ => false,
    }
}

/// Universal-Python-testing-convention prefix check, mirroring
/// `ast::is_assertion_helper_name`'s second branch. Catches the
/// `assert_called_with` / `_assert_invariant` family that the built-in
/// helper set doesn't enumerate explicitly.
fn is_helper_prefix(name: &str) -> bool {
    name.starts_with("assert_") || name.starts_with("_assert_")
}

/// Does `try_node` (a `try_statement`) carry at least one
/// `except_clause` named child? `try` / `finally` without an `except`
/// is cleanup, not branching.
fn has_except_clause(try_node: Node<'_>) -> bool {
    let mut cursor = try_node.walk();
    // The `let found = …; found` shape is deliberate: `named_children`
    // borrows `cursor`, and a bare tail expression here would drop the
    // cursor while the iterator's destructor still references it.
    let found = try_node.named_children(&mut cursor).any(|child| child.kind() == "except_clause");
    found
}

/// Does the body of `for_node` consist of assertion-bearing statements
/// — bare/message `assert_statement`s, or assertion-helper calls — with
/// **simple assignments and comments allowed in between**, optionally
/// wrapped in transparent `with` blocks, and no control-flow nodes?
/// (The name reflects the *semantic* shape: a loop that substitutes
/// for `@pytest.mark.parametrize`. The exact set of accepted body
/// statements is the union enumerated below.)
///
/// This is the parametrize-by-loop pattern (`for case in cases: assert
/// case.ok`) and its close relatives where the body computes a local
/// before asserting on it (`for m in months: s = compute(m); assert s
/// == 0`), wraps each iteration in a `with self.subTest(...)` or
/// `with self.assertRaises(...)`, or carries an inline comment
/// explaining the assertion. Semantically still "every iteration is
/// one logical case" — not branching.
///
/// Rules (all must hold):
/// - body has at least one named child (empty bodies still fire),
/// - body contains at least one assertion-bearing statement,
/// - every named child is either an assertion-bearing statement, a
///   plain `assignment` / `augmented_assignment` (wrapped in an
///   `expression_statement` by tree-sitter Python), a `comment`, or a
///   `with_statement` whose body recursively satisfies the same rules,
/// - body contains no control-flow node (`if_statement`,
///   `while_statement`, nested `for_statement`, `try_statement`) at any
///   nesting level.
///
/// The control-flow ban is enforced via a second pass over the body's
/// descendants because a top-level-only check would miss things like a
/// nested `if` inside an inner expression-statement that wraps an
/// assignment.
fn for_body_is_parametrize_substitute(
    for_node: Node<'_>,
    source: &str,
    helpers: &HashSet<String>,
) -> bool {
    let Some(body) = for_node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    let children: Vec<Node<'_>> = body.named_children(&mut cursor).collect();
    // An empty body is not exempt — `for x in xs: pass` is still a loop.
    if children.is_empty() {
        return false;
    }

    // Reject if any nested statement is control flow. The walk descends
    // through everything *except* nested function/lambda bodies — a
    // helper defined inline can have whatever structure it needs.
    if loop_body_has_control_flow(body) {
        return false;
    }

    let mut has_assertion = false;
    for child in &children {
        match classify_for_body_statement(*child, source, helpers) {
            BodyStatement::Assertion => has_assertion = true,
            BodyStatement::Assignment | BodyStatement::Comment => {}
            BodyStatement::Other => return false,
        }
    }
    has_assertion
}

/// Does the body of `for_node` look like pure cleanup — only call
/// expressions or assignments, no asserts, no control flow?
///
/// Only valid when the *enclosing test function* contains no asserts
/// anywhere (caller's responsibility — see `find_first_conditional`).
/// The combination "test-named function with zero asserts + loop of
/// plain side-effect calls" is the mis-named-fixture shape (alma
/// `tests/conftest.py:25`'s `def test_env(): ...; for k in KEYS:
/// os.environ.pop(k)`), not branching logic.
///
/// Rules:
/// - body has at least one named child,
/// - every named child is an `expression_statement` wrapping a `call`,
///   or an `assignment` / `augmented_assignment`,
/// - body contains no control-flow node anywhere.
fn for_body_is_pure_cleanup(for_node: Node<'_>) -> bool {
    let Some(body) = for_node.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    let children: Vec<Node<'_>> = body.named_children(&mut cursor).collect();
    if children.is_empty() {
        return false;
    }
    if loop_body_has_control_flow(body) {
        return false;
    }
    children.iter().all(|child| match child.kind() {
        "expression_statement" => {
            let Some(inner) = child.named_child(0) else {
                return false;
            };
            matches!(inner.kind(), "call" | "assignment" | "augmented_assignment")
        }
        // Bare `assignment` / `augmented_assignment` at statement position
        // already lives inside an `expression_statement` in tree-sitter
        // Python, but accept the unwrapped form for robustness.
        "assignment" | "augmented_assignment" => true,
        // Anything else (including `assert_statement` — though this
        // helper is only invoked when the enclosing test has zero
        // asserts) disqualifies the loop from the cleanup downgrade.
        _ => false,
    })
}

/// Classification used by [`for_body_is_parametrize_substitute`] for
/// each top-level statement of the loop body.
enum BodyStatement {
    /// Bare/message `assert_statement`, `expression_statement` wrapping
    /// a call to a known assertion helper, or a `with_statement` whose
    /// body recursively contains at least one assertion-bearing
    /// statement and no disqualifying content.
    Assertion,
    /// `expression_statement` wrapping `assignment` / `augmented_assignment`
    /// (or, defensively, the unwrapped form), or a `with_statement`
    /// whose body is entirely assignments and/or comments. Transparent
    /// for the for-of-asserts shape: the loop still needs at least one
    /// `Assertion` somewhere in its body, but Assignments don't count
    /// against it.
    Assignment,
    /// Tree-sitter exposes `# ...` as a named `comment` child. Treated
    /// as transparent — neither asserts nor disqualifies — so the
    /// surrounding loop body still qualifies for the for-of-asserts
    /// downgrade when comments are interleaved with real statements.
    Comment,
    /// Anything else — fails the "only asserts/assignments" check.
    Other,
}

fn classify_for_body_statement(
    node: Node<'_>,
    source: &str,
    helpers: &HashSet<String>,
) -> BodyStatement {
    match node.kind() {
        "assert_statement" => BodyStatement::Assertion,
        "expression_statement" => {
            let Some(inner) = node.named_child(0) else {
                return BodyStatement::Other;
            };
            match inner.kind() {
                "call" => {
                    let Some(name) = call_final_name(inner, source) else {
                        return BodyStatement::Other;
                    };
                    if helpers.contains(name) {
                        BodyStatement::Assertion
                    } else {
                        BodyStatement::Other
                    }
                }
                "assignment" | "augmented_assignment" => BodyStatement::Assignment,
                _ => BodyStatement::Other,
            }
        }
        "assignment" | "augmented_assignment" => BodyStatement::Assignment,
        "comment" => BodyStatement::Comment,
        "with_statement" => classify_with_statement(node, source, helpers),
        _ => BodyStatement::Other,
    }
}

/// Classify a `with_statement` for [`for_body_is_parametrize_substitute`].
///
/// A `with` block is treated as transparent for the for-of-asserts
/// downgrade in two situations:
///
/// 1. The context manager is *itself* an assertion — `assertRaises`,
///    `assertWarns`, `assertRaisesRegex`, `assertWarnsRegex`, or
///    `pytest.raises` / `pytest.warns`. In that case the `with` block
///    is the assertion (it asserts that its body raises / warns), and
///    its body is treated as opaque setup. The whole `with_statement`
///    classifies as `Assertion`.
/// 2. The context manager is structural noise around inner statements
///    that *would themselves qualify* — the classic cteni shape `for
///    case in cases: with self.subTest(...): self.assertX(...)`. In
///    that case the body's classification (recursively) wins.
///
/// Recursion: nested `with` (`with a: with b: assert x`) is handled
/// because classifying the inner `with` re-enters this function via
/// [`classify_for_body_statement`].
///
/// Returns:
/// - `Assertion` if the with-item is assertion-shaped, OR if every
///   body statement is Assertion / Assignment / Comment **and** at
///   least one body statement is Assertion;
/// - `Assignment` if every body statement is Assignment / Comment (no
///   asserts inside — the `with` still doesn't break the loop, but the
///   loop still needs at least one assert *somewhere* in its body);
/// - `Other` otherwise (empty body, an unrecognized statement kind, or
///   anything else that disqualifies the for-of-asserts shape).
fn classify_with_statement(
    with_node: Node<'_>,
    source: &str,
    helpers: &HashSet<String>,
) -> BodyStatement {
    if with_item_is_assertion_context_manager(with_node, source) {
        // `with self.assertRaises(...):` / `with pytest.raises(...):`
        // is itself an assertion — the body can be whatever the test
        // wants it to raise. Treat the whole statement as one assert.
        return BodyStatement::Assertion;
    }
    let Some(body) = with_node.child_by_field_name("body") else {
        return BodyStatement::Other;
    };
    let mut cursor = body.walk();
    let children: Vec<Node<'_>> = body.named_children(&mut cursor).collect();
    if children.is_empty() {
        return BodyStatement::Other;
    }
    let mut has_assertion = false;
    let mut has_non_comment = false;
    for child in &children {
        match classify_for_body_statement(*child, source, helpers) {
            BodyStatement::Assertion => {
                has_assertion = true;
                has_non_comment = true;
            }
            BodyStatement::Assignment => {
                has_non_comment = true;
            }
            BodyStatement::Comment => {}
            BodyStatement::Other => return BodyStatement::Other,
        }
    }
    if !has_non_comment {
        // Body is comments only — that's not a recognized shape.
        return BodyStatement::Other;
    }
    if has_assertion {
        BodyStatement::Assertion
    } else {
        BodyStatement::Assignment
    }
}

/// Does `with_node` carry a `with_item` whose value is a call to an
/// assertion-shaped context manager — `assertRaises`, `assertWarns`,
/// `assertRaisesRegex`, `assertWarnsRegex`, or `pytest.raises` /
/// `pytest.warns` (matched on the trailing identifier `raises` /
/// `warns`)?
///
/// Used by [`classify_with_statement`] to treat such `with` blocks as
/// a single assertion for the for-of-asserts downgrade — the context
/// manager is itself the assertion, regardless of body content.
///
/// Implementation note: only `with_node`'s direct `with_clause` /
/// `with_item` children are inspected. A nested
/// `with self.assertRaises(...):` inside the body must not promote the
/// outer `with` to "is an assertion context manager" — that would be a
/// classification error (and would mask disqualifying content inside
/// the nested `with`'s body from the body-recursion path).
fn with_item_is_assertion_context_manager(with_node: Node<'_>, source: &str) -> bool {
    let mut cursor = with_node.walk();
    for clause in with_node.named_children(&mut cursor) {
        // tree-sitter Python exposes the with-clause as either a
        // `with_clause` wrapping one or more `with_item`s, or — in some
        // grammars — `with_item`s directly under the `with_statement`.
        // Handle both shapes.
        match clause.kind() {
            "with_clause" => {
                let mut inner_cursor = clause.walk();
                for item in clause.named_children(&mut inner_cursor) {
                    if item.kind() == "with_item"
                        && with_item_value_is_raises_or_warns(item, source)
                    {
                        return true;
                    }
                }
            }
            "with_item" => {
                if with_item_value_is_raises_or_warns(clause, source) {
                    return true;
                }
            }
            // The `body` field and any other named children are
            // explicitly skipped.
            _ => {}
        }
    }
    false
}

/// Does `with_item`'s value (the expression after `with` / `as`) call
/// one of the assertion-shaped context-manager names?
fn with_item_value_is_raises_or_warns(with_item: Node<'_>, source: &str) -> bool {
    let value = with_item.child_by_field_name("value").unwrap_or_else(|| {
        // Some grammar revisions expose the call directly as the first
        // (and only) named child of `with_item` rather than via a
        // `value` field. Fall back to that.
        with_item.named_child(0).unwrap_or(with_item)
    });
    if value.kind() != "call" {
        return false;
    }
    let Some(name) = call_final_name(value, source) else {
        return false;
    };
    matches!(
        name,
        "assertRaises"
            | "assertWarns"
            | "assertRaisesRegex"
            | "assertWarnsRegex"
            | "raises"
            | "warns"
    )
}

/// Does any descendant of `body` (skipping nested `function_definition`
/// / `lambda` subtrees) carry a control-flow kind name? Used by both
/// for-body downgrades to refuse loops with `if` / `while` / `for` /
/// `try` / `match` inside them.
fn loop_body_has_control_flow(body: Node<'_>) -> bool {
    walk_descendants_pruned(
        body,
        |n| !matches!(n.kind(), "function_definition" | "lambda"),
        |node| match node.kind() {
            "if_statement" | "while_statement" | "for_statement" | "try_statement"
            | "match_statement" => ControlFlow::Break(()),
            _ => ControlFlow::Continue(()),
        },
    )
    .is_break()
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
    fn does_not_fire_on_for_loop_with_assignment_then_assert() {
        // SEMANTICS CHANGE: previously
        // `fires_on_for_loop_with_real_logic_alongside_asserts` asserted
        // that an assignment between iterations broke the
        // for-of-asserts shape and re-fired. Run-2 against real
        // corpora flagged this as a false positive (klice
        // `test_backtest.py:32`, biston `test_injector.py:167`,
        // …): `for m in months: s = compute(m); assert s == 0` is
        // still a parametrize-by-loop — every iteration computes one
        // local and asserts on it. As long as the body has at least
        // one assert-bearing statement, no control flow, and only
        // assignments interleaved, it stays exempt.
        let src = "\
def test_each_case():
    for case in CASES:
        y = transform(case)
        assert y
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_for_loop_with_control_flow_inside() {
        // The broadened for-of-asserts downgrade is gated on the loop
        // body containing no control flow at any nesting level. A
        // nested `if` inside the body disqualifies the loop — the rule
        // reports the `for_statement` itself (which is what's wrong
        // here), not the nested `if`.
        let src = "\
def test_each_case():
    for case in CASES:
        if case:
            assert case == 1
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn does_not_fire_on_for_loop_calling_unknown_helper_in_assertless_test() {
        // SEMANTICS CHANGE: previously
        // `fires_on_for_loop_calling_unknown_helper` asserted that a
        // loop of plain calls fires. The cleanup-loop downgrade now
        // exempts loops of plain calls *when the enclosing test
        // function has no asserts anywhere* — that's the
        // mis-named-fixture shape (alma `tests/conftest.py:25`'s
        // `def test_env(): ...; for k in KEYS: os.environ.pop(k)`).
        let src = "\
def test_each_case():
    for case in CASES:
        process(case)
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_for_loop_calling_unknown_helper_in_test_with_asserts() {
        // Counter-case to `does_not_fire_on_for_loop_calling_unknown_helper_in_assertless_test`:
        // the cleanup downgrade is disabled the moment any assert
        // appears in the test body. A loop of plain calls is still a
        // for_statement with non-assertion content — fires.
        let src = "\
def test_does_real_work():
    assert True
    for case in CASES:
        process(case)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn does_not_fire_on_for_loop_with_subtest_wrapper() {
        // cteni's dominant shape: every iteration wraps the assertion in
        // `with self.subTest(...)`. The `with` is structural noise — the
        // body is still a single helper-call assertion, so the loop is
        // still parametrize-by-loop.
        let src = "\
class TestSomething:
    def test_each_case(self):
        for case in self.cases:
            with self.subTest(case=case):
                self.assertEqual(case, case)
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_for_loop_with_assert_raises_wrapper() {
        // `with self.assertRaises(...)` is itself the assertion. The
        // body can be any call — it's what the test expects to raise.
        let src = "\
class TestSomething:
    def test_each_case(self):
        for case in self.cases:
            with self.assertRaises(ValueError):
                do_something(case)
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_for_loop_with_pytest_raises_wrapper() {
        // The same shape with `pytest.raises` instead of unittest's
        // `assertRaises`. `call_final_name` reduces both to the
        // trailing identifier `raises` / `assertRaises`.
        let src = "\
import pytest


def test_each_case():
    for case in CASES:
        with pytest.raises(ValueError):
            do_something(case)
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_for_loop_with_comment_between_assignment_and_assert() {
        // Tree-sitter exposes `# comment` lines as named `comment`
        // children of the loop body. They must be treated as
        // transparent — neither asserts nor disqualifies the
        // for-of-asserts shape.
        let src = "\
def test_each_case():
    for case in CASES:
        result = transform(case)
        # explanation
        assert result in CASES
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_for_loop_with_branching_inside_with_wrapper() {
        // Counter-case to `does_not_fire_on_for_loop_with_subtest_wrapper`:
        // a control-flow construct inside the `with` body still
        // disqualifies the loop. `loop_body_has_control_flow` walks the
        // whole subtree, so an `if` inside a `with` inside a `for` is
        // caught and the outer `for_statement` fires.
        let src = "\
class TestSomething:
    def test_each_case(self):
        for case in self.cases:
            with self.subTest(case=case):
                if case > 0:
                    self.assertEqual(case, case)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        // First conditional in the body is the `for_statement`.
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn does_not_fire_on_for_loop_with_nested_with_wrapper() {
        // Defensive: `with a: with b: assert x` recurses through
        // `classify_with_statement` and still classifies as Assertion.
        let src = "\
class TestSomething:
    def test_each_case(self):
        for case in self.cases:
            with self.subTest(case=case):
                with self.subTest(inner=True):
                    self.assertEqual(case, case)
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_test_with_match_statement() {
        // PEP 634 structural pattern matching is multi-branch flow by
        // definition. tree-sitter Python exposes it as `match_statement`.
        // ZR001's stated intent ("test should be linear arrange / act /
        // assert") applies as much to `match` as to `if`.
        let src = "\
def test_top_match():
    match x:
        case 1:
            assert x == 1
        case _:
            assert False
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR001");
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn fires_on_for_loop_with_match_statement_inside_with() {
        // The for-of-asserts downgrade's `loop_body_has_control_flow`
        // walk must also catch `match_statement` — otherwise a `match`
        // inside an asserting `with` body would qualify the loop.
        let src = "\
class TestSomething:
    def test_each_case(self):
        for case in self.cases:
            with self.subTest(case=case):
                match case:
                    case 1:
                        self.assertEqual(case, 1)
                    case _:
                        self.fail()
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        // The outer for_statement fires (first conditional in the body).
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn cleanup_downgrade_ignores_assert_inside_nested_helper() {
        // An assert *inside an inline helper* shouldn't count toward
        // the enclosing test's assertion content — the outer test is
        // still doing pure cleanup. Consistent with the rule's posture
        // that nested-def subtrees are opaque to ZR001.
        let src = "\
def test_env():
    def _check():
        assert True

    for k in KEYS:
        cleanup(k)
";
        // No control flow at the outer scope: nested def is pruned, and
        // the cleanup loop qualifies because the outer test has no
        // assert at the outer scope.
        assert!(run(src).is_empty());
    }

    #[test]
    fn outer_non_assertion_with_around_inner_assertion_with_is_classified_correctly() {
        // Defensive: the outer `with somecontext():` is *not* itself
        // an assertion context manager, so it must be classified by
        // recursing into its body. The previous implementation walked
        // the entire subtree from `with_node` and would incorrectly
        // see the *inner* `with self.assertRaises(...)` as the outer
        // with-item, masking the body. The current direct-child walk
        // gets this right — and because the inner with body is a
        // single call, the for loop is still a parametrize-substitute.
        let src = "\
class TestSomething:
    def test_each_case(self):
        for case in self.cases:
            with somecontext():
                with self.assertRaises(ValueError):
                    do(case)
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_skip() {
        // Skipped tests don't execute — their structural smells are
        // suppressed. Mirrors ZR007's gate.
        let src = "\
@pytest.mark.skip
def test_x():
    if True:
        assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_skip_with_reason() {
        let src = "\
@pytest.mark.skip(reason=\"todo\")
def test_x():
    if True:
        assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_xfail() {
        let src = "\
@pytest.mark.xfail
def test_x():
    for case in CASES:
        if case:
            assert case
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_method_of_class_decorated_with_pytest_mark_skip() {
        // Class-level `@pytest.mark.skip` suppresses every method body,
        // mirroring ZR007. esl's 50 placeholder-stub pattern.
        let src = "\
@pytest.mark.skip
class TestX:
    def test_y(self):
        if True:
            assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_pytest_mark_skipif() {
        // `skipif` is conditional — body still runs on the non-skipped
        // path, so structural smells still flag. Matches ZR007's
        // documented decision.
        let src = "\
@pytest.mark.skipif(True, reason=\"x\")
def test_x():
    if True:
        assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR001");
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

    #[test]
    fn does_not_fire_on_runtime_skip_call_as_first_statement() {
        // A test whose first statement is `self.skipTest(...)` is
        // unconditionally skipped at runtime — its structural smells are
        // irrelevant, just like a `@pytest.mark.skip`-decorated test.
        // This covers the cteni unittest pattern (~28 FP findings).
        let src = "\
class TestThing:
    def test_conditional_but_skipped(self):
        self.skipTest(\"not ready\")
        if self.condition:
            assert self.value == 1
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_when_runtime_skip_is_not_first_statement() {
        // A skip call that is NOT the first real statement does not
        // suppress ZR001 — the test body may still execute before it.
        let src = "\
def test_has_if_then_skip():
    if True:
        pytest.skip(\"reason\")
    assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR001");
    }
}
