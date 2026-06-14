//! AST helpers built on top of tree-sitter-python.
//!
//! The single public helper right now is [`is_test_function`], which
//! encodes the pytest-style naming rules for identifying test functions:
//!
//! - top-level `def test_*` → yes;
//! - method `def test_*` inside `class Test*` → yes;
//! - `async def test_*` counts (tree-sitter models it as the same
//!   `function_definition` kind with an `async` keyword child);
//! - a `def test_*` nested inside another function is **not** a test.
//!
//! [`iter_test_functions`] walks the tree and yields every
//! `function_definition` node that passes [`is_test_function`].
//!
//! [`collect_asserts`] walks a test body and yields every assert-shaped
//! construct (bare + message-carrying `assert_statement`s, calls to known
//! assertion helpers, and `with pytest.raises/warns(...)` context
//! managers). ZR003 and ZR004 share this walk to avoid duplicated descent
//! logic.
//!
//! [`test_has_runtime_skip_call`] complements the decorator-based
//! [`test_has_skip_or_xfail_decorator`] gate: it recognises tests whose
//! **first real body statement** is a runtime skip call —
//! `self.skipTest(...)`, `self.skip(...)`, or `pytest.skip(...)` — and
//! allows the four skip-aware rules (ZR001, ZR003, ZR004, ZR007) to
//! suppress findings on those tests just as they do for decorator-skipped
//! tests.

use std::ops::ControlFlow;

use tree_sitter::{Node, Tree, TreeCursor};

/// Visit every descendant of `root` in source order (pre-order DFS).
///
/// The visitor returns a [`ControlFlow`] — `Continue(())` keeps walking,
/// `Break(value)` short-circuits and returns that value. This one helper
/// replaces hand-written cursor loops inside every rule: "find the first
/// X" becomes `Break(node)`, "collect all X" stays `Continue(())` and
/// mutates a captured `Vec`.
///
/// Always descends into every child. When a rule wants to skip certain
/// subtrees (e.g. "the outer test body's control flow only — ignore
/// nested function definitions"), use [`walk_descendants_pruned`].
pub fn walk_descendants<'tree, B>(
    root: Node<'tree>,
    mut visit: impl FnMut(Node<'tree>) -> ControlFlow<B>,
) -> ControlFlow<B> {
    let mut cursor = root.walk();
    recurse(&mut cursor, root, &mut visit)
}

fn recurse<'tree, B>(
    cursor: &mut TreeCursor<'tree>,
    node: Node<'tree>,
    visit: &mut dyn FnMut(Node<'tree>) -> ControlFlow<B>,
) -> ControlFlow<B> {
    visit(node)?;
    if cursor.goto_first_child() {
        let result = (|| -> ControlFlow<B> {
            loop {
                let child = cursor.node();
                recurse(cursor, child, visit)?;
                if !cursor.goto_next_sibling() {
                    return ControlFlow::Continue(());
                }
            }
        })();
        cursor.goto_parent();
        result?;
    }
    ControlFlow::Continue(())
}

/// Like [`walk_descendants`] but consults `should_descend(child)` before
/// recursing into each child. The visitor still runs on the start node
/// and on every node that survives the descent filter.
///
/// Use this when a rule wants to prune whole subtrees during a walk —
/// e.g. ZR001 skipping `function_definition` / `lambda` children so the
/// outer test's control flow is the only concern. Pruned nodes are
/// never passed to `visit` and their children are never visited either.
///
/// Contrast with [`walk_descendants`]: that helper always descends; its
/// visitor closure cannot prevent descent without short-circuiting the
/// entire walk via `ControlFlow::Break`.
pub fn walk_descendants_pruned<'tree, B>(
    root: Node<'tree>,
    mut should_descend: impl FnMut(Node<'tree>) -> bool,
    mut visit: impl FnMut(Node<'tree>) -> ControlFlow<B>,
) -> ControlFlow<B> {
    let mut cursor = root.walk();
    recurse_pruned(&mut cursor, root, &mut should_descend, &mut visit)
}

fn recurse_pruned<'tree, B>(
    cursor: &mut TreeCursor<'tree>,
    node: Node<'tree>,
    should_descend: &mut dyn FnMut(Node<'tree>) -> bool,
    visit: &mut dyn FnMut(Node<'tree>) -> ControlFlow<B>,
) -> ControlFlow<B> {
    visit(node)?;
    if cursor.goto_first_child() {
        let result = (|| -> ControlFlow<B> {
            loop {
                let child = cursor.node();
                if should_descend(child) {
                    recurse_pruned(cursor, child, should_descend, visit)?;
                }
                if !cursor.goto_next_sibling() {
                    return ControlFlow::Continue(());
                }
            }
        })();
        cursor.goto_parent();
        result?;
    }
    ControlFlow::Continue(())
}

/// Return `true` when `node` is a tree-sitter Python function that
/// qualifies as a pytest test function.
///
/// The rules match `context.md` (PLAN.md §Identifying test functions):
/// name must start with `test_`; if the function is a method, its
/// enclosing class name must start with `Test`; nested inner functions do
/// not count.
#[must_use]
pub fn is_test_function(node: Node<'_>, source: &str) -> bool {
    if node.kind() != "function_definition" {
        return false;
    }

    let Some(name) = function_name(node, source) else {
        return false;
    };
    if !name.starts_with("test_") {
        return false;
    }

    // Walk ancestors. The function is a test iff:
    // - no enclosing function_definition (not nested), and
    // - any enclosing class_definition has a name starting with "Test".
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        match parent.kind() {
            "function_definition" => return false,
            "class_definition" => {
                let Some(class_name) = class_name(parent, source) else {
                    return false;
                };
                if !class_name.starts_with("Test") {
                    return false;
                }
                // Keep walking — a class inside a function still disqualifies.
            }
            _ => {}
        }
        ancestor = parent.parent();
    }

    true
}

/// Walk `tree` and yield every `function_definition` node that
/// [`is_test_function`] considers a test.
///
/// Results are produced in source order. The iterator owns a
/// pre-collected `Vec` of nodes to sidestep the self-referential borrow
/// between a cursor and the tree it walks.
pub fn iter_test_functions<'tree>(
    tree: &'tree Tree,
    source: &str,
) -> impl Iterator<Item = Node<'tree>> {
    let mut out = Vec::new();
    let _ = walk_descendants::<()>(tree.root_node(), |node| {
        if node.kind() == "function_definition" && is_test_function(node, source) {
            out.push(node);
        }
        ControlFlow::Continue(())
    });
    out.into_iter()
}

fn function_name<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    let name = node.child_by_field_name("name")?;
    name.utf8_text(source.as_bytes()).ok()
}

fn class_name<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    let name = node.child_by_field_name("name")?;
    name.utf8_text(source.as_bytes()).ok()
}

/// An assert-shaped construct found inside a test body.
///
/// ZR003 checks "is there any of these?" and ZR004 counts the bare form.
/// Centralizing the walk here (rather than duplicating descent logic in
/// each rule) keeps both rules consistent on edge cases like asserts
/// nested inside inline helpers.
#[derive(Debug, Clone, Copy)]
pub enum AssertHit<'tree> {
    /// A bare `assert x` statement: `assert_statement` with exactly one
    /// named child.
    BareAssert(Node<'tree>),
    /// An `assert x, "msg"` statement: `assert_statement` with a second
    /// named child (the message).
    MessageAssert(Node<'tree>),
    /// A call whose function's final identifier is in the caller-supplied
    /// helper set (e.g. `self.assertEqual(...)`, `fail()`).
    HelperCall(Node<'tree>),
    /// A `with_statement` whose `with_item` value is a call to something
    /// ending in `raises` or `warns` — `pytest.raises(...)`,
    /// `pytest.warns(...)`, `self.assertRaises(...)` etc.
    RaisesContext(Node<'tree>),
}

/// Walk `body` pre-order and collect every [`AssertHit`] found.
///
/// Descends into nested function/class definitions so asserts inside
/// inline helpers count — matches the `walk_descendants` semantics ZR001
/// and ZR002 already use.
///
/// `helpers` is the set of identifier names (the "final segment" of the
/// call target) that should count as assertion helpers. Callers pass
/// their merged (built-in + user-extra) set.
pub fn collect_asserts<'tree>(
    body: Node<'tree>,
    source: &str,
    helpers: &std::collections::HashSet<String>,
) -> Vec<AssertHit<'tree>> {
    let mut out = Vec::new();
    let _ = walk_descendants::<()>(body, |node| {
        if let Some(hit) = classify_assert_hit(node, source, Some(helpers)) {
            out.push(hit);
        }
        ControlFlow::Continue(())
    });
    out
}

/// Fast path for ZR003: does `body` contain **any** assert-shaped
/// construct? Returns `true` on the first hit and stops walking.
///
/// Semantically equivalent to `!collect_asserts(body, source,
/// helpers).is_empty()` but avoids the `Vec` allocation and walks only
/// as far as needed.
#[must_use]
pub fn has_any_assert(
    body: Node<'_>,
    source: &str,
    helpers: &std::collections::HashSet<String>,
) -> bool {
    walk_descendants(body, |node| {
        if classify_assert_hit(node, source, Some(helpers)).is_some() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    })
    .is_break()
}

/// Count every bare `assert_statement` under `body` and return the first
/// one in source order, if any.
///
/// Used by ZR004, which cares only about the bare form — not helper
/// calls and not `with pytest.raises(...)`. Avoids allocating a `Vec`
/// and avoids constructing a throwaway helpers `HashSet`.
#[must_use]
pub fn count_bare_asserts_with_first<'tree>(
    body: Node<'tree>,
    source: &str,
) -> (usize, Option<Node<'tree>>) {
    let mut count = 0;
    let mut first: Option<Node<'tree>> = None;
    let _ = walk_descendants::<()>(body, |node| {
        if let Some(AssertHit::BareAssert(n)) = classify_assert_hit(node, source, None) {
            count += 1;
            if first.is_none() {
                first = Some(n);
            }
        }
        ControlFlow::Continue(())
    });
    (count, first)
}

/// Classify a single node as an [`AssertHit`] variant, if it is one.
///
/// `helpers = None` disables `HelperCall` detection (the caller doesn't
/// care about helper calls, e.g. ZR004).
fn classify_assert_hit<'tree>(
    node: Node<'tree>,
    source: &str,
    helpers: Option<&std::collections::HashSet<String>>,
) -> Option<AssertHit<'tree>> {
    match node.kind() {
        "assert_statement" => {
            if node.named_child_count() <= 1 {
                Some(AssertHit::BareAssert(node))
            } else {
                Some(AssertHit::MessageAssert(node))
            }
        }
        "call" => {
            let name = call_final_name(node, source)?;
            is_assertion_helper_name(name, helpers).then_some(AssertHit::HelperCall(node))
        }
        "with_statement" => with_statement_is_raises_or_warns(node, source)
            .then_some(AssertHit::RaisesContext(node)),
        _ => None,
    }
}

/// Does `name` look like an assertion-helper identifier?
///
/// Internal helper for [`classify_assert_hit`]; `pub(crate)` so the
/// rules layer can reuse it in tests / future helper-call rules
/// without re-exporting it publicly.
///
/// Two complementary checks:
///
/// 1. **Exact match** against the caller-supplied `helpers` set. This is
///    how the built-in case-sensitive unittest helpers (`assertEqual`,
///    `assertTrue`, …) and any user-extended names are recognized.
/// 2. **Snake-case prefix** — names starting with `assert_` or
///    `_assert_` (the latter covering private helpers like
///    `_assert_invariant`). This branch is universal Python testing
///    convention (`assert_called_with`, `assert_css_class_present`, …).
///    The check is exactly `name.starts_with("assert_") ||
///    name.starts_with("_assert_")` — the trailing underscore is
///    deliberate so we don't catch unrelated identifiers like
///    `assertion` or `assertively`.
///
/// Both branches are gated on `helpers.is_some()`. Callers that pass
/// `None` (e.g. ZR004's bare-assert counter via
/// [`count_bare_asserts_with_first`]) get a hard `false`: they only care
/// about `assert_statement` nodes, not helper calls, and must remain
/// unaffected by helper-name heuristics. The gate is what preserves
/// ZR004's semantics across this extension.
pub(crate) fn is_assertion_helper_name(
    name: &str,
    helpers: Option<&std::collections::HashSet<String>>,
) -> bool {
    let Some(helpers) = helpers else {
        return false;
    };
    helpers.contains(name) || name.starts_with("assert_") || name.starts_with("_assert_")
}

/// The final identifier of a `call` node's `function` expression.
///
/// Returns `Some("foo")` for `foo(...)`, `a.b.foo(...)`, `self.foo(...)`.
/// Returns `None` for calls whose target is a more exotic expression
/// (e.g. `(lambda: ...)(...)`, `f[0](...)`).
pub fn call_final_name<'a>(call_node: Node<'_>, source: &'a str) -> Option<&'a str> {
    let func = call_node.child_by_field_name("function")?;
    identifier_or_attribute_tail(func, source)
}

/// Walk an `identifier` / `attribute` expression to its final identifier
/// name.
///
/// Returns `Some("patch")` for `patch`, `mock.patch`,
/// `unittest.mock.patch`. Returns `Some("object")` for `patch.object` —
/// the caller decides whether `object` matters. Returns `None` for
/// anything more exotic (subscripts, parenthesised expressions, lambdas).
///
/// `pub(crate)` so ZR006 (decorator form) and ZR008 (with-item call form)
/// can share the helper without re-exporting it on the public API. Both
/// rules need the same "what's the last name in this chain?" lookup but
/// neither operates on a `call` node directly, which is why this is a
/// sibling of [`call_final_name`] rather than its only caller.
pub(crate) fn identifier_or_attribute_tail<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    match node.kind() {
        "identifier" => node.utf8_text(source.as_bytes()).ok(),
        "attribute" => {
            let attr = node.child_by_field_name("attribute")?;
            attr.utf8_text(source.as_bytes()).ok()
        }
        _ => None,
    }
}

/// Return the dotted attribute-chain segments of a decorator expression.
///
/// A Python decorator is a `decorator` node whose first named child is
/// the expression that names the decorator. That expression is one of:
///
/// - an `identifier` (`@skip` → `["skip"]`)
/// - an `attribute` chain (`@pytest.mark.skip` → `["pytest", "mark", "skip"]`)
/// - a `call` whose `function` is one of the above
///   (`@pytest.mark.skip(reason="todo")` → `["pytest", "mark", "skip"]`)
///
/// Anything more exotic (subscripts, lambdas, parenthesized expressions)
/// returns an empty `Vec` — callers that need a specific decorator shape
/// should match on the returned segment count.
///
/// Borrowing: the returned segments are slices into `source`. The helper
/// is generic across rules — Phase 4 uses it for the ZR007 skip/xfail
/// gate, future rules can match on any other decorator chain.
#[must_use]
pub fn decorator_chain_segments<'a>(decorator_node: Node<'_>, source: &'a str) -> Vec<&'a str> {
    let mut cursor = decorator_node.walk();
    let Some(inner) = decorator_node.named_children(&mut cursor).next() else {
        return Vec::new();
    };
    let expr = match inner.kind() {
        "call" => match inner.child_by_field_name("function") {
            Some(func) => func,
            None => return Vec::new(),
        },
        _ => inner,
    };
    let mut out = Vec::new();
    if !collect_chain_into(expr, source, &mut out) {
        return Vec::new();
    }
    out
}

/// Walk an `identifier`/`attribute` chain left-to-right, appending each
/// segment name to `out`. Returns `false` if the chain contains any
/// non-attribute/non-identifier node (e.g. a subscript) — caller treats
/// that as "not a recognizable chain" and clears the result.
fn collect_chain_into<'a>(node: Node<'_>, source: &'a str, out: &mut Vec<&'a str>) -> bool {
    match node.kind() {
        "identifier" => {
            let Ok(text) = node.utf8_text(source.as_bytes()) else {
                return false;
            };
            out.push(text);
            true
        }
        "attribute" => {
            // `attribute` has `object` (left side, recursive) and
            // `attribute` (the rightmost identifier). Walk object first
            // so segments land in source order.
            let Some(object) = node.child_by_field_name("object") else {
                return false;
            };
            if !collect_chain_into(object, source, out) {
                return false;
            }
            let Some(attr) = node.child_by_field_name("attribute") else {
                return false;
            };
            let Ok(text) = attr.utf8_text(source.as_bytes()) else {
                return false;
            };
            out.push(text);
            true
        }
        _ => false,
    }
}

/// Does the test function carry a `@pytest.mark.skip` or
/// `@pytest.mark.xfail` decorator (call form with kwargs accepted)?
///
/// When the test function is decorated, tree-sitter parses the construct
/// as `decorated_definition` whose children are one or more `decorator`
/// nodes followed by the `function_definition`. Walk each decorator and
/// check via [`is_skip_or_xfail_decorator`].
///
/// Method-on-class case: pytest treats a class-level `@pytest.mark.skip`
/// as suppressing every method inside the class. ~/git/esl's 50
/// placeholder stubs all use this pattern. So when the test function is
/// a method on a `class Test*`, also check the enclosing class's
/// decorators.
///
/// `@pytest.mark.skipif(...)` is **not** matched here — `skipif` is
/// conditional, so the body is expected to do real work on the
/// non-skipped path. The check is exact: segments must be exactly
/// `["pytest", "mark", "skip"]` or `["pytest", "mark", "xfail"]`.
#[must_use]
pub fn test_has_skip_or_xfail_decorator(test_fn: Node<'_>, source: &str) -> bool {
    if decorated_definition_has_skip_or_xfail(test_fn.parent(), source) {
        return true;
    }
    if let Some(class) = enclosing_class(test_fn) {
        if decorated_definition_has_skip_or_xfail(class.parent(), source) {
            return true;
        }
    }
    false
}

/// If `maybe_decorated` is a `decorated_definition`, scan its decorators
/// for any `@pytest.mark.skip` or `@pytest.mark.xfail`.
///
/// `pub(crate)` — internal helper for
/// [`test_has_skip_or_xfail_decorator`]. External rules go through that
/// entry point so the method-vs-class-vs-`skipif` semantics stay in one
/// place.
#[must_use]
pub(crate) fn decorated_definition_has_skip_or_xfail(
    maybe_decorated: Option<Node<'_>>,
    source: &str,
) -> bool {
    let Some(parent) = maybe_decorated else {
        return false;
    };
    if parent.kind() != "decorated_definition" {
        return false;
    }
    let mut cursor = parent.walk();
    for child in parent.named_children(&mut cursor) {
        if child.kind() == "decorator" && is_skip_or_xfail_decorator(child, source) {
            return true;
        }
    }
    false
}

/// Walk ancestors of a (test) function and return the nearest enclosing
/// `class_definition`, if any.
///
/// The method-level decorator gate uses the function's immediate parent,
/// but a class-level decorator sits on the `class_definition`'s parent
/// (`decorated_definition`), so we need a handle on the class node
/// itself.
///
/// `pub(crate)` — generic-enough that future rules may want it, but no
/// external caller today.
#[must_use]
pub(crate) fn enclosing_class(node: Node<'_>) -> Option<Node<'_>> {
    let mut ancestor = node.parent();
    while let Some(parent) = ancestor {
        if parent.kind() == "class_definition" {
            return Some(parent);
        }
        ancestor = parent.parent();
    }
    None
}

/// Is `decorator` a `@pytest.mark.skip` / `@pytest.mark.xfail` decorator
/// (bare or call form)?
///
/// Matches exactly the three-segment chain `pytest.mark.skip` or
/// `pytest.mark.xfail`. `@pytest.mark.skipif`, bare `@skip`,
/// `@unittest.skip`, etc. all return `false`.
///
/// `pub(crate)` — internal helper for
/// [`decorated_definition_has_skip_or_xfail`].
#[must_use]
pub(crate) fn is_skip_or_xfail_decorator(decorator: Node<'_>, source: &str) -> bool {
    let segments = decorator_chain_segments(decorator, source);
    segments.len() == 3
        && segments[0] == "pytest"
        && segments[1] == "mark"
        && (segments[2] == "skip" || segments[2] == "xfail")
}

/// Climb past any `parenthesized_expression` wrappers around `node`,
/// returning the outermost equivalent expression.
///
/// Useful for rule predicates that need to recognise a syntactic position
/// regardless of paren-wrapping — `assert exists(p), ("/x")` and
/// `mock.return_value = ("/x")` should be treated the same as their
/// unparenthesised forms. Returns `node` unchanged when it has no
/// `parenthesized_expression` parent.
#[must_use]
pub fn climb_past_parens(node: Node<'_>) -> Node<'_> {
    let mut current = node;
    while let Some(parent) = current.parent() {
        if parent.kind() == "parenthesized_expression" {
            current = parent;
        } else {
            break;
        }
    }
    current
}

/// Does the test function's **first real body statement** call one of the
/// runtime-skip idioms — `self.skipTest(...)`, `self.skip(...)`, or
/// `pytest.skip(...)`?
///
/// A test whose very first action is an unconditional runtime skip is
/// effectively always-skipped — exactly like a decorator-based
/// `@pytest.mark.skip`. This helper lets ZR001, ZR003, ZR004, and ZR007
/// suppress findings on those tests, reducing false positives from
/// `unittest`-style suites that write `self.skipTest("reason")` as the
/// first body line.
///
/// **Matching rules**
///
/// - The call's `function` must be an `attribute` node with:
///     - `object` = identifier `self` or `pytest`
///     - `attribute` = identifier `skipTest` or `skip`
///
///   Only the exact pairs (`self.skipTest`, `self.skip`, `pytest.skip`)
///   are accepted. `self.skipif`, bare `skip(...)`, and any other receiver
///   are deliberately excluded.
/// - "First real statement" skips a leading docstring
///   (`expression_statement` wrapping a `string`) and any leading
///   `comment` named children.
/// - Only the first real statement is inspected. A skip call as the
///   *second* or later statement does **not** trigger this helper.
#[must_use]
pub fn test_has_runtime_skip_call(test_fn: Node<'_>, source: &str) -> bool {
    let Some(body) = test_fn.child_by_field_name("body") else {
        return false;
    };
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        let kind = child.kind();
        // Skip leading comments (tree-sitter exposes `# ...` as named
        // `comment` children of the block).
        if kind == "comment" {
            continue;
        }
        // Skip a leading docstring: `expression_statement` whose single
        // named child is a `string`.
        if kind == "expression_statement" {
            if let Some(inner) = child.named_child(0) {
                if inner.kind() == "string" && child.named_child_count() == 1 {
                    continue;
                }
            }
        }
        // This is the first real statement. Check whether it is an
        // `expression_statement` wrapping a skip call.
        if kind != "expression_statement" {
            return false;
        }
        let Some(inner) = child.named_child(0) else {
            return false;
        };
        if inner.kind() != "call" {
            return false;
        }
        return call_is_runtime_skip(inner, source);
    }
    false
}

/// Is `call_node` a call to one of the accepted runtime-skip targets:
/// `self.skipTest(...)`, `self.skip(...)`, or `pytest.skip(...)`?
///
/// Inspects the `function` field of the `call` node directly. It must be
/// an `attribute` node with an `object` identifier equal to `self` or
/// `pytest` and an `attribute` identifier equal to `skipTest` or `skip`.
/// The pairing is further constrained: `pytest.skipTest` is NOT accepted
/// (not a real pytest API) and `self.skip` is only accepted when the
/// object is `self`.
fn call_is_runtime_skip(call_node: Node<'_>, source: &str) -> bool {
    let Some(func) = call_node.child_by_field_name("function") else {
        return false;
    };
    if func.kind() != "attribute" {
        return false;
    }
    let Some(object) = func.child_by_field_name("object") else {
        return false;
    };
    if object.kind() != "identifier" {
        return false;
    }
    let Ok(obj_name) = object.utf8_text(source.as_bytes()) else {
        return false;
    };
    let Some(attr) = func.child_by_field_name("attribute") else {
        return false;
    };
    let Ok(attr_name) = attr.utf8_text(source.as_bytes()) else {
        return false;
    };
    matches!((obj_name, attr_name), ("self", "skipTest" | "skip") | ("pytest", "skip"))
}

/// Does `with_node` carry a `with_item` whose value is a call whose
/// function's final name is `raises` or `warns`?
fn with_statement_is_raises_or_warns(with_node: Node<'_>, source: &str) -> bool {
    // `with_statement` -> `with_clause` -> one or more `with_item`s.
    // The value expression sits under `with_item` as either a named
    // `value` field (newer grammar) or as the sole named child
    // (depending on grammar version). Walk descendants to cover both.
    let mut found = false;
    let _ = walk_descendants::<()>(with_node, |n| {
        if n.kind() == "with_item" {
            // A with_item may contain a call (optionally wrapped in an
            // `as` pattern). Look for a `call` descendant whose final
            // name is `raises`/`warns`.
            let _ = walk_descendants::<()>(n, |inner| {
                if inner.kind() == "call" {
                    if let Some(name) = call_final_name(inner, source) {
                        if name == "raises" || name == "warns" {
                            found = true;
                            return ControlFlow::Break(());
                        }
                    }
                }
                ControlFlow::Continue(())
            });
            if found {
                return ControlFlow::Break(());
            }
        }
        ControlFlow::Continue(())
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn all_function_defs(tree: &Tree) -> Vec<Node<'_>> {
        let mut out = Vec::new();
        let _ = walk_descendants::<()>(tree.root_node(), |node| {
            if node.kind() == "function_definition" {
                out.push(node);
            }
            ControlFlow::Continue(())
        });
        out
    }

    fn find_fn<'a>(tree: &'a Tree, source: &str, name: &str) -> Node<'a> {
        all_function_defs(tree)
            .into_iter()
            .find(|n| function_name(*n, source) == Some(name))
            .expect("function not found")
    }

    #[test]
    fn top_level_test_is_a_test() {
        let src = "def test_foo():\n    pass\n";
        let tree = parse(src).unwrap();
        assert!(is_test_function(find_fn(&tree, src, "test_foo"), src));
    }

    #[test]
    fn method_on_test_class_is_a_test() {
        let src = "class TestThing:\n    def test_foo(self):\n        pass\n";
        let tree = parse(src).unwrap();
        assert!(is_test_function(find_fn(&tree, src, "test_foo"), src));
    }

    #[test]
    fn async_top_level_test_is_a_test() {
        let src = "async def test_foo():\n    pass\n";
        let tree = parse(src).unwrap();
        assert!(is_test_function(find_fn(&tree, src, "test_foo"), src));
    }

    #[test]
    fn nested_inner_def_is_not_a_test() {
        let src = "def outer():\n    def test_foo():\n        pass\n";
        let tree = parse(src).unwrap();
        assert!(!is_test_function(find_fn(&tree, src, "test_foo"), src));
    }

    #[test]
    fn module_level_non_test_helper_is_not_a_test() {
        let src = "def _helper():\n    pass\n";
        let tree = parse(src).unwrap();
        assert!(!is_test_function(find_fn(&tree, src, "_helper"), src));
    }

    #[test]
    fn method_on_non_test_class_is_not_a_test() {
        let src = "class OtherThing:\n    def test_foo(self):\n        pass\n";
        let tree = parse(src).unwrap();
        assert!(!is_test_function(find_fn(&tree, src, "test_foo"), src));
    }

    #[test]
    fn walk_descendants_pruned_skips_nested_function_definition_subtrees() {
        // The outer body contains a top-level `if_statement`, plus a
        // nested `def helper()` whose own body contains another
        // `if_statement`. Pruning on `function_definition` must visit
        // the outer `if` but not the inner one — confirming that the
        // subtree under the pruned node never reaches the visitor.
        let src = "\
def test_outer():
    if True:
        pass
    def helper():
        if False:
            pass
    helper()
";
        let tree = parse(src).unwrap();
        let test_fn = find_fn(&tree, src, "test_outer");
        let body = test_fn.child_by_field_name("body").unwrap();

        // Baseline: the unpruned walk sees both `if_statement`s.
        let mut unpruned = Vec::new();
        let _ = walk_descendants::<()>(body, |n| {
            if n.kind() == "if_statement" {
                unpruned.push(n.start_position().row);
            }
            ControlFlow::Continue(())
        });
        assert_eq!(unpruned.len(), 2, "sanity: source has two if_statements");

        // Pruning on `function_definition` keeps only the outer one.
        let mut pruned = Vec::new();
        let _ = walk_descendants_pruned::<()>(
            body,
            |n| !matches!(n.kind(), "function_definition" | "lambda"),
            |n| {
                if n.kind() == "if_statement" {
                    pruned.push(n.start_position().row);
                }
                ControlFlow::Continue(())
            },
        );
        assert_eq!(pruned.len(), 1);
        // The outer `if True:` is on the second line of the source
        // (0-indexed row 1).
        assert_eq!(pruned[0], 1);
    }

    #[test]
    fn walk_descendants_pruned_visits_root_even_if_should_descend_rejects_root_kind() {
        // The closure is only consulted for *children* — the root node
        // is always visited. This makes "skip nested defs while
        // walking a body" work even when the body itself happens to be
        // (or contain) a function_definition shape.
        let src = "def f():\n    pass\n";
        let tree = parse(src).unwrap();
        let root = tree.root_node();
        let mut visited_root = false;
        let _ = walk_descendants_pruned::<()>(
            root,
            |_| false,
            |n| {
                if n == root {
                    visited_root = true;
                }
                ControlFlow::Continue(())
            },
        );
        assert!(visited_root, "the start node is always visited");
    }

    #[test]
    fn is_assertion_helper_name_accepts_underscore_prefix() {
        use std::collections::HashSet;

        let helpers: HashSet<String> = std::iter::once("assertEqual".to_string()).collect();

        // Exact match against the configured set still works.
        assert!(is_assertion_helper_name("assertEqual", Some(&helpers)));

        // Snake-case prefix `assert_…` is accepted when helpers are
        // active — this is the new behaviour.
        assert!(is_assertion_helper_name("assert_css_class_present", Some(&helpers)));

        // Private-helper prefix `_assert_…` is also accepted.
        assert!(is_assertion_helper_name("_assert_invariant", Some(&helpers)));

        // A bare `assertion` (no underscore) must NOT match — the prefix
        // check requires the underscore so we don't catch unrelated
        // identifiers like `assertion`, `assertively`, etc.
        assert!(!is_assertion_helper_name("assertion", Some(&helpers)));

        // The prefix branch is gated on `helpers.is_some()`. With
        // `None`, only the (absent) exact-match branch would fire — so
        // an `assert_*` name must NOT match. This preserves ZR004's
        // bare-assert counter semantics.
        assert!(!is_assertion_helper_name("assert_css_class_present", None));
        assert!(!is_assertion_helper_name("_assert_invariant", None));
        assert!(!is_assertion_helper_name("assertEqual", None));
    }

    fn first_decorator(tree: &Tree) -> Node<'_> {
        let mut found: Option<Node<'_>> = None;
        let _ = walk_descendants::<()>(tree.root_node(), |n| {
            if n.kind() == "decorator" {
                found = Some(n);
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });
        found.expect("source must contain a decorator")
    }

    #[test]
    fn decorator_chain_segments_handles_pytest_mark_skip() {
        let src = "\
@pytest.mark.skip
def test_x():
    pass
";
        let tree = parse(src).unwrap();
        let decorator = first_decorator(&tree);
        assert_eq!(decorator_chain_segments(decorator, src), vec!["pytest", "mark", "skip"]);
    }

    #[test]
    fn decorator_chain_segments_handles_pytest_mark_skip_call_form_with_reason() {
        let src = "\
@pytest.mark.skip(reason=\"todo\")
def test_x():
    pass
";
        let tree = parse(src).unwrap();
        let decorator = first_decorator(&tree);
        assert_eq!(decorator_chain_segments(decorator, src), vec!["pytest", "mark", "skip"]);
    }

    #[test]
    fn decorator_chain_segments_handles_bare_identifier() {
        let src = "\
@skip
def test_x():
    pass
";
        let tree = parse(src).unwrap();
        let decorator = first_decorator(&tree);
        assert_eq!(decorator_chain_segments(decorator, src), vec!["skip"]);
    }

    #[test]
    fn decorator_chain_segments_handles_bare_identifier_call_form() {
        // `@skip()` — call whose function is a bare identifier.
        let src = "\
@skip()
def test_x():
    pass
";
        let tree = parse(src).unwrap();
        let decorator = first_decorator(&tree);
        assert_eq!(decorator_chain_segments(decorator, src), vec!["skip"]);
    }

    #[test]
    fn decorator_chain_segments_handles_two_segment_chain() {
        // `@mock.patch` — two-segment attribute chain.
        let src = "\
@mock.patch
def test_x():
    pass
";
        let tree = parse(src).unwrap();
        let decorator = first_decorator(&tree);
        assert_eq!(decorator_chain_segments(decorator, src), vec!["mock", "patch"]);
    }

    #[test]
    fn decorator_chain_segments_handles_four_segment_chain() {
        // `@a.b.c.d` — four segments. ZR007's gate matches length-3 only,
        // so this MUST come back as a four-element Vec so the caller's
        // exact-length match correctly rejects it. Locks in the
        // "any deeper chain is preserved verbatim, not truncated" promise.
        let src = "\
@a.b.c.d
def test_x():
    pass
";
        let tree = parse(src).unwrap();
        let decorator = first_decorator(&tree);
        assert_eq!(decorator_chain_segments(decorator, src), vec!["a", "b", "c", "d"]);
    }

    #[test]
    fn decorator_chain_segments_returns_empty_for_subscripted_decorator() {
        // `@mark[skip]` — a subscript expression is not a recognizable
        // identifier/attribute chain. The helper must return an empty
        // `Vec` (and crucially must NOT panic). This is the documented
        // contract for unrecognised decorator shapes.
        //
        // Note: tree-sitter's Python grammar may or may not accept this
        // as valid; the rule cares only that we don't crash and that
        // callers see an empty chain.
        let src = "\
@mark[skip]
def test_x():
    pass
";
        let tree = parse(src).unwrap();
        let decorator = first_decorator(&tree);
        assert!(
            decorator_chain_segments(decorator, src).is_empty(),
            "subscripted decorator must return an empty Vec, got {:?}",
            decorator_chain_segments(decorator, src)
        );
    }

    #[test]
    fn climb_past_parens_unwraps_nested_parentheses() {
        // The literal "x" sits inside `(("x"))` — two layers of
        // `parenthesized_expression`. `climb_past_parens` must walk through
        // both and stop at the outer `assignment` parent's right-hand side.
        let src = "x = ((\"value\"))\n";
        let tree = parse(src).unwrap();
        let mut string_node: Option<Node<'_>> = None;
        let _ = walk_descendants::<()>(tree.root_node(), |n| {
            if n.kind() == "string" {
                string_node = Some(n);
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });
        let string_node = string_node.expect("source has a string");
        let outer = climb_past_parens(string_node);
        // After climbing past both parens, the parent should be the
        // `assignment` node — not another `parenthesized_expression`.
        assert_eq!(outer.kind(), "parenthesized_expression");
        assert_eq!(outer.parent().map(|p| p.kind()), Some("assignment"));
    }

    #[test]
    fn climb_past_parens_returns_node_when_no_paren_parent() {
        let src = "x = \"value\"\n";
        let tree = parse(src).unwrap();
        let mut string_node: Option<Node<'_>> = None;
        let _ = walk_descendants::<()>(tree.root_node(), |n| {
            if n.kind() == "string" {
                string_node = Some(n);
                return ControlFlow::Break(());
            }
            ControlFlow::Continue(())
        });
        let string_node = string_node.expect("source has a string");
        let result = climb_past_parens(string_node);
        assert_eq!(result.id(), string_node.id());
    }

    #[test]
    fn iter_yields_every_test_function_in_source_order() {
        let src = "\
def test_a():
    pass

class TestX:
    def test_b(self):
        pass

def _helper():
    pass

async def test_c():
    pass

def outer():
    def test_inner():
        pass

class Other:
    def test_d(self):
        pass
";
        let tree = parse(src).unwrap();
        let names: Vec<_> = iter_test_functions(&tree, src)
            .map(|n| function_name(n, src).unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["test_a", "test_b", "test_c"]);
    }

    // --- test_has_runtime_skip_call tests ---

    #[test]
    fn runtime_skip_self_skip_test_as_first_statement_returns_true() {
        let src = "\
class TestX:
    def test_foo(self):
        self.skipTest(\"not ready\")
        assert True
";
        let tree = parse(src).unwrap();
        let test_fn = find_fn(&tree, src, "test_foo");
        assert!(test_has_runtime_skip_call(test_fn, src));
    }

    #[test]
    fn runtime_skip_self_skip_as_first_statement_returns_true() {
        let src = "\
class TestX:
    def test_foo(self):
        self.skip(\"reason\")
";
        let tree = parse(src).unwrap();
        let test_fn = find_fn(&tree, src, "test_foo");
        assert!(test_has_runtime_skip_call(test_fn, src));
    }

    #[test]
    fn runtime_skip_pytest_skip_as_first_statement_returns_true() {
        let src = "\
def test_foo():
    pytest.skip(\"reason\")
    assert True
";
        let tree = parse(src).unwrap();
        let test_fn = find_fn(&tree, src, "test_foo");
        assert!(test_has_runtime_skip_call(test_fn, src));
    }

    #[test]
    fn runtime_skip_after_docstring_returns_true() {
        // A leading docstring is transparent — the skip call right after
        // is still the "first real statement".
        let src = "\
def test_foo():
    \"\"\"This test is skipped.\"\"\"
    pytest.skip(\"reason\")
    assert True
";
        let tree = parse(src).unwrap();
        let test_fn = find_fn(&tree, src, "test_foo");
        assert!(test_has_runtime_skip_call(test_fn, src));
    }

    #[test]
    fn runtime_skip_as_second_statement_returns_false() {
        // A skip that is NOT first does not suppress the test.
        let src = "\
def test_foo():
    setup()
    pytest.skip(\"reason\")
";
        let tree = parse(src).unwrap();
        let test_fn = find_fn(&tree, src, "test_foo");
        assert!(!test_has_runtime_skip_call(test_fn, src));
    }

    #[test]
    fn runtime_skip_bare_skip_call_returns_false() {
        // `skip(...)` without a receiver is NOT accepted — the brief
        // explicitly requires a two-segment attribute.
        let src = "\
def test_foo():
    skip(\"reason\")
";
        let tree = parse(src).unwrap();
        let test_fn = find_fn(&tree, src, "test_foo");
        assert!(!test_has_runtime_skip_call(test_fn, src));
    }

    #[test]
    fn runtime_skip_self_skipif_returns_false() {
        // `skipif` is conditional and must NOT match.
        let src = "\
class TestX:
    def test_foo(self):
        self.skipTest_someother(\"reason\")
";
        let tree = parse(src).unwrap();
        let test_fn = find_fn(&tree, src, "test_foo");
        assert!(!test_has_runtime_skip_call(test_fn, src));
    }

    #[test]
    fn runtime_skip_plain_test_returns_false() {
        let src = "\
def test_foo():
    assert 1 == 1
";
        let tree = parse(src).unwrap();
        let test_fn = find_fn(&tree, src, "test_foo");
        assert!(!test_has_runtime_skip_call(test_fn, src));
    }

    #[test]
    fn runtime_skip_other_receiver_returns_false() {
        // `something_else.skip(...)` — not `self` or `pytest`.
        let src = "\
def test_foo():
    manager.skip(\"reason\")
";
        let tree = parse(src).unwrap();
        let test_fn = find_fn(&tree, src, "test_foo");
        assert!(!test_has_runtime_skip_call(test_fn, src));
    }
}
