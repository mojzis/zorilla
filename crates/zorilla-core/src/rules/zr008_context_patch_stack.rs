//! `ZR008 context-patch-stack` — flag tests with too many stacked
//! `patch(...)` context managers.
//!
//! # Rule
//!
//! The spiritual sibling of [`crate::rules::zr006_patch_stack`]: ZR006
//! catches stacked `@patch` *decorators*, this rule catches the same
//! problem expressed with `with patch(...)` *context managers*. Each
//! extra `patch` adds one more piece of production internals the test
//! is bound to by string-path, and the test body becomes a maze of
//! mock wiring whether the wiring sits above the `def` or inside it.
//!
//! This rule fires **once per test function** whose body contains more
//! than `[tool.zorilla.rules.ZR008] max_patches` (default `3`)
//! `patch(...)` context-manager items. Two stack-forms count
//! equivalently:
//!
//! - sequential nested `with patch(...):` blocks (vertical stack), and
//! - comma-separated items in a single `with patch(), patch(), ...:`
//!   clause (horizontal stack).
//!
//! A test that mixes the two — say, two `with patch(), patch():` items
//! inside another `with patch():` — sums the items the same way: each
//! `with_item` contributes one to the count regardless of which `with`
//! it lives under.
//!
//! Like ZR006, the count looks only at `with_item`s whose value
//! expression is a `call` whose function's final identifier is
//! `patch` — so `patch`, `mock.patch`, and `unittest.mock.patch` all
//! count; `patch.object(...)` does not (its final identifier is
//! `object`), `with open(...)` does not, neither does a bare
//! `with manager:` that isn't a call.
//!
//! Threshold is **exclusive**: the default fires at 4 or more `patch`
//! context-manager items.
//!
//! Nested `function_definition`s and `lambda`s inside the test body are
//! pruned during the walk — context managers inside an inline helper
//! don't count toward the outer test, mirroring the
//! "outer-test-scope-only" convention shared by ZR001.
//!
//! **Reported location**: the first qualifying `patch` `with_item`'s
//! `start_position()`. That's the visual top of the patch stack and the
//! most actionable target — pointing at unrelated `with` items would
//! mislead the reader.
//!
//! ## Examples
//!
//! Positive — four sequential `with patch(...)` blocks:
//!
//! ```python
//! from unittest.mock import patch
//!
//! def test_many():
//!     with patch("a"):
//!         with patch("b"):
//!             with patch("c"):
//!                 with patch("d"):   # ZR008 fires at the first patch
//!                     assert run()
//! ```
//!
//! Positive — four items in one `with` clause:
//!
//! ```python
//! def test_many():
//!     with patch("a"), patch("b"), patch("c"), patch("d"):  # ZR008
//!         assert run()
//! ```
//!
//! Negative — three patches sit at the default threshold:
//!
//! ```python
//! def test_ok():
//!     with patch("a"), patch("b"), patch("c"):
//!         assert run()
//! ```

use std::ops::ControlFlow;

use tree_sitter::Node;

use crate::ast::{iter_test_functions, walk_descendants_pruned};
use crate::report::{Finding, Severity};
use crate::rules::{Context, Rule};

/// The registered ZR008 rule instance.
pub static ZR008_CONTEXT_PATCH_STACK: ContextPatchStackRule = ContextPatchStackRule;

/// Zero-sized rule struct implementing [`Rule`] for ZR008.
pub struct ContextPatchStackRule;

impl Rule for ContextPatchStackRule {
    fn code(&self) -> &'static str {
        "ZR008"
    }

    fn name(&self) -> &'static str {
        "context-patch-stack"
    }

    fn doc(&self) -> &'static str {
        include_str!("../../../../docs/rules/ZR008.md")
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        let max_patches = ctx.config.zr008.max_patches;
        for test_fn in iter_test_functions(ctx.tree, ctx.source) {
            let Some(body) = test_fn.child_by_field_name("body") else {
                continue;
            };
            let patches = collect_patch_with_items(body, ctx.source);
            let patch_count = patches.len();

            if patch_count > max_patches {
                // Point at the first qualifying `patch` with_item — not
                // the first `with_statement` overall — so the location
                // matches the offense (e.g. a leading `with open(...)`
                // must not be reported).
                let Some(first_patch) = patches.first().copied() else {
                    continue;
                };
                let start = first_patch.start_position();
                out.push(Finding {
                    code: self.code(),
                    message: format!(
                        "test has too many `with patch(...)` context managers ({patch_count} > {max_patches}); consider a single fixture"
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

/// Walk the test body in source order, collecting every `with_item`
/// whose value resolves to a call of `patch` (final identifier).
///
/// The walk prunes `function_definition` and `lambda` subtrees — only
/// the outer test's context managers count. A helper defined inline
/// can use however many `with patch(...)`es it needs.
fn collect_patch_with_items<'tree>(body: Node<'tree>, source: &str) -> Vec<Node<'tree>> {
    let mut out = Vec::new();
    let _ = walk_descendants_pruned::<()>(
        body,
        |n| !matches!(n.kind(), "function_definition" | "lambda"),
        |node| {
            if node.kind() == "with_statement" {
                collect_patch_items_in_with(node, source, &mut out);
            }
            ControlFlow::Continue(())
        },
    );
    out
}

/// Append every `with_item` that names a `patch` call (in source order)
/// to `out`. Iterates the `with_clause`'s named children — a single
/// `with patch(), patch(), patch():` carries multiple `with_item`s on
/// one clause, and each contributes independently.
fn collect_patch_items_in_with<'tree>(
    with_node: Node<'tree>,
    source: &str,
    out: &mut Vec<Node<'tree>>,
) {
    // `with_statement` → `with_clause` (single named child holding the
    // items) → one or more `with_item`s. The grammar names the clause
    // type `with_clause`; iterate named children defensively in case
    // future grammar versions inline the items.
    let mut cursor = with_node.walk();
    for child in with_node.named_children(&mut cursor) {
        match child.kind() {
            "with_clause" => {
                let mut inner_cursor = child.walk();
                for item in child.named_children(&mut inner_cursor) {
                    if item.kind() == "with_item" && with_item_is_patch_call(item, source) {
                        out.push(item);
                    }
                }
            }
            // Some grammar variants might surface `with_item` as a
            // direct child of `with_statement`. Handle that path too.
            "with_item" => {
                if with_item_is_patch_call(child, source) {
                    out.push(child);
                }
            }
            _ => {}
        }
    }
}

/// Does `with_item`'s value expression resolve to a call of `patch`
/// (rightmost identifier)?
///
/// The value of a `with_item` may sit under a `value` field directly,
/// or may be wrapped in an `as_pattern` (`with patch(...) as p:`) whose
/// own value field carries the call. We peel one `as_pattern` layer
/// and then look at the final-identifier shape of the call's function.
fn with_item_is_patch_call(with_item: Node<'_>, source: &str) -> bool {
    let Some(value) = with_item_value(with_item) else {
        return false;
    };
    // `with patch(...) as p:` — peel one `as_pattern` layer.
    let inner = if value.kind() == "as_pattern" {
        match as_pattern_value(value) {
            Some(v) => v,
            None => return false,
        }
    } else {
        value
    };
    if inner.kind() != "call" {
        return false;
    }
    let Some(func) = inner.child_by_field_name("function") else {
        return false;
    };
    final_identifier(func, source) == Some("patch")
}

/// Locate the value expression of a `with_item`.
///
/// Prefers the `value` field (newer grammars expose it) and falls back
/// to the first named child for older / variant grammars. Either way
/// the returned node is the expression we want to classify.
fn with_item_value(with_item: Node<'_>) -> Option<Node<'_>> {
    if let Some(v) = with_item.child_by_field_name("value") {
        return Some(v);
    }
    // Bind the iterator's first element before `cursor` is dropped —
    // returning the call expression directly would let the cursor
    // borrow outlive the function frame.
    let mut cursor = with_item.walk();
    let first = with_item.named_children(&mut cursor).next();
    first
}

/// Pull the value out of an `as_pattern` — `expr as name`. Newer
/// grammars expose it as the first named child (the `as_pattern_target`
/// sits second); fall back to that when no field is available.
fn as_pattern_value(as_pattern: Node<'_>) -> Option<Node<'_>> {
    if let Some(v) = as_pattern.child_by_field_name("value") {
        return Some(v);
    }
    // Bind before `cursor` is dropped — see `with_item_value` for the
    // same reason.
    let mut cursor = as_pattern.walk();
    let first = as_pattern.named_children(&mut cursor).next();
    first
}

/// Walk an `identifier` / `attribute` expression to its final identifier
/// name.
///
/// Returns `Some("patch")` for `patch`, `mock.patch`, `unittest.mock.patch`.
/// Returns `Some("object")` for `patch.object` — caller filters.
/// Mirrors the helper of the same name inside `zr006_patch_stack.rs`;
/// kept local to avoid a public-API hoist that no other rule needs yet.
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
        ZR008_CONTEXT_PATCH_STACK.check(&ctx, &mut out);
        out
    }

    #[test]
    fn fires_on_four_sequential() {
        let src = "\
def test_many():
    with patch(\"a\"):
        with patch(\"b\"):
            with patch(\"c\"):
                with patch(\"d\"):
                    assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR008");
        assert!(out[0].message.contains("4 > 3"));
    }

    #[test]
    fn fires_on_single_with_four_patches() {
        let src = "\
def test_many():
    with patch(\"a\"), patch(\"b\"), patch(\"c\"), patch(\"d\"):
        assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR008");
    }

    #[test]
    fn does_not_fire_on_three() {
        let src = "\
def test_ok():
    with patch(\"a\"), patch(\"b\"), patch(\"c\"):
        assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn counts_mock_patch_variant() {
        let src = "\
def test_many():
    with mock.patch(\"a\"), mock.patch(\"b\"), mock.patch(\"c\"), mock.patch(\"d\"):
        assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn does_not_count_patch_object() {
        // `patch.object(...)` has final identifier `object`, not `patch`.
        let src = "\
def test_many():
    with patch.object(F, \"a\"):
        with patch.object(F, \"b\"):
            with patch.object(F, \"c\"):
                with patch.object(F, \"d\"):
                    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn respects_max_patches_config() {
        let src = "\
def test_two():
    with patch(\"a\"), patch(\"b\"):
        assert True
";
        let mut cfg = Config::default().rule_config();
        cfg.zr008.max_patches = 1;
        let out = run_with(src, &cfg);
        assert_eq!(out.len(), 1);
        assert!(out[0].message.contains("2 > 1"));
    }

    #[test]
    fn fires_on_method_of_test_class() {
        let src = "\
class TestThing:
    def test_method(self):
        with patch(\"a\"), patch(\"b\"), patch(\"c\"), patch(\"d\"):
            assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn does_not_fire_on_helper_function() {
        // Five `with patch(...)`es, but they live in a non-test helper.
        // `iter_test_functions` excludes `_helper`, so the rule sees
        // nothing.
        let src = "\
def _helper():
    with patch(\"a\"):
        with patch(\"b\"):
            with patch(\"c\"):
                with patch(\"d\"):
                    with patch(\"e\"):
                        pass
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn points_at_first_patch_with_item() {
        // The first patch context-manager item is on line 3, column 10
        // (the `patch("a")` after `    with `). The finding must point
        // there, not at the `with_statement` start.
        let src = "\
def test_many():
    with patch(\"a\"), patch(\"b\"), patch(\"c\"), patch(\"d\"):
        assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        // `    with patch("a")` — `patch("a")` starts at column 10 (1-indexed).
        assert_eq!(out[0].line, 2);
        assert_eq!(out[0].column, 10);
    }

    #[test]
    fn does_not_count_with_patch_in_nested_def() {
        // Outer test has 1 `with patch()`; nested helper def has 3
        // more. The pruned walk must skip the nested def, so total
        // counted = 1, well below the default threshold.
        let src = "\
def test_outer():
    with patch(\"a\"):
        pass

    def helper():
        with patch(\"b\"):
            with patch(\"c\"):
                with patch(\"d\"):
                    pass
    helper()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn handles_with_patch_as_alias() {
        // `with patch("a") as p:` — `as_pattern` wraps the call. The
        // four items together must still trip the rule.
        let src = "\
def test_many():
    with patch(\"a\") as a, patch(\"b\") as b, patch(\"c\") as c, patch(\"d\") as d:
        assert (a, b, c, d)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn does_not_fire_on_with_open() {
        // `with open(...)` is not `patch`. Even four nested opens stay
        // silent.
        let src = "\
def test_reads():
    with open(\"a\") as a:
        with open(\"b\") as b:
            with open(\"c\") as c:
                with open(\"d\") as d:
                    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn mixed_with_open_and_patch_counts_only_patch() {
        // One `with open(...)` followed by four `patch`es. The finding
        // points at the first `patch`, not at the `open`.
        let src = "\
def test_many():
    with open(\"f\") as f:
        with patch(\"a\"):
            with patch(\"b\"):
                with patch(\"c\"):
                    with patch(\"d\"):
                        assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        // Pointed at the first `patch("a")` line, not at `with open`.
        assert_eq!(out[0].line, 3);
    }
}
