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

use tree_sitter::{Node, Tree, TreeCursor};

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
    let mut cursor = tree.walk();
    collect_test_functions(&mut cursor, source, &mut out);
    out.into_iter()
}

fn collect_test_functions<'tree>(
    cursor: &mut TreeCursor<'tree>,
    source: &str,
    out: &mut Vec<Node<'tree>>,
) {
    let node = cursor.node();
    if node.kind() == "function_definition" && is_test_function(node, source) {
        out.push(node);
    }

    if cursor.goto_first_child() {
        loop {
            collect_test_functions(cursor, source, out);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
        cursor.goto_parent();
    }
}

fn function_name<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    let name = node.child_by_field_name("name")?;
    name.utf8_text(source.as_bytes()).ok()
}

fn class_name<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    let name = node.child_by_field_name("name")?;
    name.utf8_text(source.as_bytes()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;

    fn all_function_defs(tree: &Tree) -> Vec<Node<'_>> {
        fn walk<'a>(cursor: &mut TreeCursor<'a>, out: &mut Vec<Node<'a>>) {
            let node = cursor.node();
            if node.kind() == "function_definition" {
                out.push(node);
            }
            if cursor.goto_first_child() {
                loop {
                    walk(cursor, out);
                    if !cursor.goto_next_sibling() {
                        break;
                    }
                }
                cursor.goto_parent();
            }
        }
        let mut out = Vec::new();
        let mut cursor = tree.walk();
        walk(&mut cursor, &mut out);
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
