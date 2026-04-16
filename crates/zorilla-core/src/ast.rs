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
        match node.kind() {
            "assert_statement" => {
                let count = node.named_child_count();
                if count <= 1 {
                    out.push(AssertHit::BareAssert(node));
                } else {
                    out.push(AssertHit::MessageAssert(node));
                }
            }
            "call" => {
                if let Some(name) = call_final_name(node, source) {
                    if helpers.contains(name) {
                        out.push(AssertHit::HelperCall(node));
                    }
                }
            }
            "with_statement" => {
                if with_statement_is_raises_or_warns(node, source) {
                    out.push(AssertHit::RaisesContext(node));
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    });
    out
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
