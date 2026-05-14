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
/// Two complementary checks:
///
/// 1. **Exact match** against the caller-supplied `helpers` set. This is
///    how the built-in case-sensitive unittest helpers (`assertEqual`,
///    `assertTrue`, …) and any user-extended names are recognized.
/// 2. **Snake-case prefix** — names starting with `assert_` or
///    `_assert_` (the latter covering private helpers like
///    `_assert_invariant`). This branch is universal Python testing
///    convention (`assert_called_with`, `assert_css_class_present`, …).
///
/// Both branches are gated on `helpers.is_some()`. Callers that pass
/// `None` (e.g. ZR004's bare-assert counter) get a hard `false`: they
/// only care about `assert_statement` nodes, not helper calls, and must
/// remain unaffected by helper-name heuristics.
fn is_assertion_helper_name(
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
    match func.kind() {
        "identifier" => func.utf8_text(source.as_bytes()).ok(),
        "attribute" => {
            let attr = func.child_by_field_name("attribute")?;
            attr.utf8_text(source.as_bytes()).ok()
        }
        _ => None,
    }
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
}
