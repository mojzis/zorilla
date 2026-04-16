//! `ZR006 patch-stack` — flag tests with too many stacked `@patch` decorators.
//!
//! # Rule
//!
//! Stacking several `@patch`/`@mock.patch` decorators on a single test is
//! a reliable signal the test is doing too much setup inline. Each
//! decorator adds a positional argument to the test, reversed relative to
//! the decorator order, and the test body becomes a maze of mock wiring.
//! The usual fix is a fixture that builds the whole mock graph once.
//!
//! This rule fires **once per test function** whose stacked decorator
//! list contains more than `[tool.zorilla.rules.ZR006] max_patches`
//! (default `3`) `@patch` calls. The count looks only at decorators whose
//! final identifier is `patch` — so `@patch`, `@mock.patch`, and
//! `@unittest.mock.patch` all count; `@pytest.mark.foo` and
//! `@patch.object` do not.
//!
//! Threshold is **exclusive**: the default fires at 4 or more `@patch`
//! decorators.
//!
//! **Reported location**: the first decorator's `start_position()`.
//! That's the visual top of the stack and the most actionable target.
//!
//! ## Examples
//!
//! Positive — four stacked patches:
//!
//! ```python
//! from unittest.mock import patch
//!
//! @patch("a")
//! @patch("b")
//! @patch("c")
//! @patch("d")         # ZR006 fires at the first decorator
//! def test_many(d, c, b, a):
//!     assert True
//! ```
//!
//! Negative — three patches sit at the default threshold:
//!
//! ```python
//! @patch("a")
//! @patch("b")
//! @patch("c")
//! def test_ok(c, b, a):
//!     assert True
//! ```

use tree_sitter::Node;

use crate::ast::iter_test_functions;
use crate::report::{Finding, Severity};
use crate::rules::{Context, Rule};

/// The registered ZR006 rule instance.
pub static ZR006_PATCH_STACK: PatchStackRule = PatchStackRule;

/// Zero-sized rule struct implementing [`Rule`] for ZR006.
pub struct PatchStackRule;

impl Rule for PatchStackRule {
    fn code(&self) -> &'static str {
        "ZR006"
    }

    fn name(&self) -> &'static str {
        "patch-stack"
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        let max_patches = ctx.config.zr006.max_patches;
        for test_fn in iter_test_functions(ctx.tree, ctx.source) {
            let Some(parent) = test_fn.parent() else {
                continue;
            };
            if parent.kind() != "decorated_definition" {
                continue;
            }

            let decorators = collect_decorators(parent);
            let patch_count =
                decorators.iter().filter(|d| decorator_is_patch(**d, ctx.source)).count();

            if patch_count > max_patches {
                // At least one decorator exists (otherwise count == 0),
                // so first_decorator is Some.
                let Some(first) = decorators.first().copied() else {
                    continue;
                };
                let start = first.start_position();
                out.push(Finding {
                    code: self.code(),
                    message: format!(
                        "test has too many @patch decorators ({patch_count} > {max_patches}); consider a single fixture"
                    ),
                    file: ctx.file.to_path_buf(),
                    line: start.row + 1,
                    column: start.column + 1,
                    severity: Severity::Warning,
                });
            }
        }
    }
}

/// Collect every `decorator` named child of a `decorated_definition`
/// node, in source order.
fn collect_decorators(decorated: Node<'_>) -> Vec<Node<'_>> {
    let mut out = Vec::new();
    let mut cursor = decorated.walk();
    for child in decorated.named_children(&mut cursor) {
        if child.kind() == "decorator" {
            out.push(child);
        }
    }
    out
}

/// Does `decorator_node`'s body ultimately resolve to the identifier
/// `patch`?
///
/// The decorator's first named child is either a `call` (`@patch("x")`)
/// or an identifier/attribute (`@patch`). We reduce to the final
/// identifier of whatever that expression names.
fn decorator_is_patch(decorator_node: Node<'_>, source: &str) -> bool {
    let mut cursor = decorator_node.walk();
    let Some(inner) = decorator_node.named_children(&mut cursor).next() else {
        return false;
    };
    let name_node = match inner.kind() {
        "call" => {
            let Some(func) = inner.child_by_field_name("function") else {
                return false;
            };
            func
        }
        _ => inner,
    };
    final_identifier(name_node, source).is_some_and(|n| n == "patch")
}

/// Walk an `identifier` / `attribute` expression to its final identifier
/// name.
///
/// Returns `Some("patch")` for `patch`, `mock.patch`, `unittest.mock.patch`.
/// Returns `Some("object")` for `patch.object` — caller filters.
fn final_identifier<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    match node.kind() {
        "identifier" => node.utf8_text(source.as_bytes()).ok(),
        "attribute" => {
            let attr = node.child_by_field_name("attribute")?;
            attr.utf8_text(source.as_bytes()).ok()
        }
        _ => None,
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
        ZR006_PATCH_STACK.check(&ctx, &mut out);
        out
    }

    #[test]
    fn fires_on_four_stacked_patch_calls() {
        let src = "\
@patch(\"a\")
@patch(\"b\")
@patch(\"c\")
@patch(\"d\")
def test_many(d, c, b, a):
    assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR006");
        // Location is the first decorator.
        assert_eq!(out[0].line, 1);
        assert_eq!(out[0].column, 1);
        assert!(out[0].message.contains("4 > 3"));
    }

    #[test]
    fn does_not_fire_on_three_stacked_patches() {
        let src = "\
@patch(\"a\")
@patch(\"b\")
@patch(\"c\")
def test_ok(c, b, a):
    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn counts_mock_patch_variant() {
        let src = "\
@mock.patch(\"a\")
@mock.patch(\"b\")
@mock.patch(\"c\")
@mock.patch(\"d\")
def test_many(d, c, b, a):
    assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn counts_unittest_mock_patch_variant() {
        let src = "\
@unittest.mock.patch(\"a\")
@unittest.mock.patch(\"b\")
@unittest.mock.patch(\"c\")
@unittest.mock.patch(\"d\")
def test_many(d, c, b, a):
    assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn does_not_count_pytest_mark_foo() {
        let src = "\
@pytest.mark.foo
@patch(\"a\")
@patch(\"b\")
@patch(\"c\")
def test_ok(c, b, a):
    assert True
";
        // 3 patches + 1 mark = at threshold, no fire.
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_count_patch_object() {
        // `@patch.object(...)` has final identifier `object`, not `patch`.
        let src = "\
@patch.object(Foo, \"a\")
@patch.object(Foo, \"b\")
@patch.object(Foo, \"c\")
@patch.object(Foo, \"d\")
def test_many(d, c, b, a):
    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_with_configured_lower_threshold() {
        let src = "\
@patch(\"a\")
@patch(\"b\")
@patch(\"c\")
def test_three(c, b, a):
    assert True
";
        let mut cfg = Config::default().rule_config();
        cfg.zr006.max_patches = 2;
        let out = run_with(src, &cfg);
        assert_eq!(out.len(), 1);
        assert!(out[0].message.contains("3 > 2"));
    }

    #[test]
    fn does_not_fire_on_decorator_on_non_test_helper() {
        let src = "\
@patch(\"a\")
@patch(\"b\")
@patch(\"c\")
@patch(\"d\")
def _helper(d, c, b, a):
    return 1
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_method_of_test_class() {
        let src = "\
class TestThing:
    @patch(\"a\")
    @patch(\"b\")
    @patch(\"c\")
    @patch(\"d\")
    def test_method(self, d, c, b, a):
        assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn handles_bare_patch_identifier_without_call() {
        // `@patch` (no call) — rare, but a valid decorator form. Treat it
        // the same as `@patch(...)`.
        let src = "\
@patch
@patch
@patch
@patch
def test_many():
    assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn does_not_fire_on_test_without_decorators() {
        let src = "\
def test_plain():
    assert True
";
        assert!(run(src).is_empty());
    }
}
