//! `ZR004 assertion-roulette` — flag tests with too many bare asserts
//! about more than one subject.
//!
//! ## Runtime-skip awareness
//!
//! A test whose first real body statement (after any leading docstring or
//! comments) is `self.skipTest(...)`, `self.skip(...)`, or
//! `pytest.skip(...)` is unconditionally skipped at runtime. Such tests
//! are treated the same as `@pytest.mark.skip`-decorated tests: their
//! bare-assert count is academic and ZR004 does not fire on them.
//!
//! # Rule
//!
//! A test littered with bare `assert x` statements about several things
//! fails obscurely: when one of them trips, the reader has to reconstruct
//! which of the test's claims broke, and the asserts after it never ran
//! at all — the "assertion roulette" smell, the eager-test smell's usual
//! symptom. Splitting the test so each one has a single subject, adding a
//! message to each assert (`assert x, "explain"`), or replacing a wall of
//! asserts with a single rich check cures this.
//!
//! This rule fires **once per test function** whose count of **bare**
//! `assert_statement` nodes strictly exceeds
//! `[tool.zorilla.rules.ZR004] max_asserts` (default `4`). A bare
//! `assert_statement` has exactly one named child — the expression. An
//! `assert x, "msg"` carries a second named child (the message) and is
//! **not** counted.
//!
//! ## Single-subject contract exemption
//!
//! A run of bare asserts that are all about **one subject** is a contract
//! check, not roulette: `assert record.name == "demo"`, `assert
//! record.count == 2`, ... each fail on a line that names its field, and
//! the only alternatives — a message repeating the field name, or six
//! one-assert tests — cost maintenance without adding information. Such
//! tests never fire, whatever their count; `max_asserts` measures sprawl
//! across subjects, not length ([issue #23]).
//!
//! The subject of a bare assert is the name at the root of the operand
//! under test, found structurally by [`assert_subject`]:
//!
//! - the left operand of a comparison, or the **right** one for `in` /
//!   `not in` (the container is what is being checked); a Yoda
//!   comparison (`assert 1 == record.a`) therefore has no subject;
//! - through `not`, unary minus, parentheses, `await`, and the left side
//!   of `and` / `or`;
//! - through attribute access, subscripts and method calls down to the
//!   root name (`record.tags[0].lower()` is about `record`);
//! - into the first positional argument of a free-function call
//!   (`len(row)`, `float(row[4])`, `isinstance(row, tuple)` are about
//!   `row`);
//! - `self` and `cls` are namespaces, not subjects: `self.result.x` is
//!   about `self.result`, and `self.a` / `self.b` are two subjects.
//!
//! A literal, a comprehension, or a call with no positional argument has
//! no subject, so a table of `assert f(1) == 1` cases is not a contract
//! (it is `@pytest.mark.parametrize` material) and the threshold applies.
//! One assert about a second subject is enough to withdraw the exemption:
//! intent is never guessed from a name, and a test that checks two things
//! is two tests.
//!
//! The walk descends into nested inline helpers, consistent with ZR001 /
//! ZR002 / ZR003.
//!
//! **Reported location**: the first bare `assert_statement` in source
//! order inside the offending test. Pointing at the test's `def` would
//! hide where the wall-of-asserts begins. A suppression therefore goes on
//! that first assert (or any line of it, if it spans several).
//!
//! [issue #23]: https://github.com/mojzis/zorilla/issues/23
//!
//! ## Examples
//!
//! Positive — five bare asserts about four subjects, one over the default
//! threshold of 4:
//!
//! ```python
//! def test_sprawl():
//!     result = run()
//!     assert result.ok            # ZR004 fires here
//!     assert result.code == 0
//!     assert mock.call_count == 1
//!     assert log.lines == []
//!     assert db.rows == []
//! ```
//!
//! Negative — six bare asserts, all about `record`:
//!
//! ```python
//! def test_record_contract():
//!     record = load()
//!     assert record.name == "demo"
//!     assert record.enabled is True
//!     assert record.count == 2
//!     assert record.tags == []
//!     assert record.mode == "safe"
//!     assert record.owner is None
//! ```
//!
//! Negative — the sprawling test with a message on each assert:
//!
//! ```python
//! def test_sprawl_explained():
//!     result = run()
//!     assert result.ok, "run succeeds"
//!     assert result.code == 0, "exit code"
//!     assert mock.call_count == 1, "one upstream call"
//!     assert log.lines == [], "nothing logged"
//!     assert db.rows == [], "nothing persisted"
//! ```

use tree_sitter::Node;

use crate::ast::{self, collect_bare_asserts, iter_test_functions};
use crate::report::{Finding, Severity};
use crate::rules::{Context, Rule};

/// The registered ZR004 rule instance.
pub static ZR004_ASSERTION_ROULETTE: AssertionRouletteRule = AssertionRouletteRule;

/// Zero-sized rule struct implementing [`Rule`] for ZR004.
pub struct AssertionRouletteRule;

impl Rule for AssertionRouletteRule {
    fn code(&self) -> &'static str {
        "ZR004"
    }

    fn name(&self) -> &'static str {
        "assertion-roulette"
    }

    fn doc(&self) -> &'static str {
        include_str!("../../../../docs/rules/ZR004.md")
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        let max_asserts = ctx.config.zr004.max_asserts;
        for test_fn in iter_test_functions(ctx.tree, ctx.source) {
            let Some(body) = test_fn.child_by_field_name("body") else {
                continue;
            };
            // Skip tests marked `@pytest.mark.skip` / `@pytest.mark.xfail`
            // (function-level or class-level). The body isn't executed,
            // so its assert count is academic — mirrors ZR007's gate.
            // `skipif` is conditional and intentionally not matched.
            if ast::test_has_skip_or_xfail_decorator(test_fn, ctx.source) {
                continue;
            }
            // Skip tests whose first real body statement is a runtime skip
            // call — `self.skipTest(...)`, `self.skip(...)`, or
            // `pytest.skip(...)`. The body never executes, so the
            // bare-assert count is academic.
            if ast::test_has_runtime_skip_call(test_fn, ctx.source) {
                continue;
            }
            let asserts = collect_bare_asserts(body, ctx.source);
            let count = asserts.len();
            if count > max_asserts {
                // `count > max_asserts` with `max_asserts: usize` implies
                // `count >= 1`, so `first` is guaranteed `Some`.
                let Some(first) = asserts.first() else { continue };
                // A contract check about one subject is not roulette —
                // see the module docs. Decided only past the threshold,
                // so the common case never pays for the subject walk.
                if shares_one_subject(&asserts, ctx.source) {
                    continue;
                }
                let start = first.start_position();
                out.push(Finding {
                    code: self.code(),
                    message: format!(
                        "test has too many bare assertions about more than one subject ({count} > {max_asserts})"
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

/// Do all of `asserts` have a subject, and the same one?
///
/// Empty input is vacuously single-subject, but callers only ask past the
/// threshold, so it never is.
fn shares_one_subject(asserts: &[Node<'_>], source: &str) -> bool {
    let mut subjects = asserts.iter().map(|node| assert_subject(*node, source));
    let Some(Some(first)) = subjects.next() else {
        return false;
    };
    subjects.all(|subject| subject == Some(first))
}

/// The subject of a bare `assert_statement`: the root name of the operand
/// the assert is about, or `None` when there is no name at the root.
///
/// See the module docs for the descent rules. Returned as the dotted
/// root borrowed from `source` — `record` or, past a `self` / `cls`
/// namespace, `self.result` — so two subjects compare by string equality.
fn assert_subject<'src>(assert_node: Node<'_>, source: &'src str) -> Option<&'src str> {
    let expression = first_named_non_comment(assert_node)?;
    let root = root_name_node(expression, source)?;
    let text = root.utf8_text(source.as_bytes()).ok()?;
    // `root_name_node` stops at an attribute only past a namespace, so a
    // bare identifier that is itself the namespace (`assert self`) names
    // nothing under test.
    if root.kind() == "identifier" && is_namespace(text) {
        return None;
    }
    Some(text)
}

/// The first named child that is not a `comment`. tree-sitter emits
/// comments as named extras anywhere, so `assert (\n    # why\n    x\n)`
/// would otherwise hand the descent a comment and lose the subject.
fn first_named_non_comment(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    let mut children = node.named_children(&mut cursor);
    children.find(|child| child.kind() != "comment")
}

/// `self` and `cls` are the namespaces a test method reaches its subjects
/// through, not subjects themselves.
fn is_namespace(name: &str) -> bool {
    matches!(name, "self" | "cls")
}

/// Descend from an assert's expression to the node that names its
/// subject: an `identifier`, or an `attribute` whose object is `self` /
/// `cls`. `None` when the descent bottoms out in something unnamed — a
/// literal, a comprehension, a lambda, a call with no positional argument.
fn root_name_node<'tree>(node: Node<'tree>, source: &str) -> Option<Node<'tree>> {
    match node.kind() {
        "identifier" => Some(node),
        "attribute" => {
            let object = node.child_by_field_name("object")?;
            let object_is_namespace = object.kind() == "identifier"
                && object.utf8_text(source.as_bytes()).is_ok_and(is_namespace);
            if object_is_namespace {
                return Some(node);
            }
            root_name_node(object, source)
        }
        "subscript" => root_name_node(node.child_by_field_name("value")?, source),
        "call" => {
            let function = node.child_by_field_name("function")?;
            if function.kind() == "identifier" {
                // A free function (`len`, `float`, `isinstance`, a
                // project helper) is applied to its subject: look at
                // the first positional argument. A keyword argument or
                // a generator in that slot names nothing.
                let arguments = node.child_by_field_name("arguments")?;
                // `all(r.ok for r in rows)`: tree-sitter makes the bare
                // generator the `arguments` node itself.
                if arguments.kind() == "generator_expression" {
                    return None;
                }
                let first = first_named_non_comment(arguments)?;
                if first.kind() == "keyword_argument" {
                    return None;
                }
                return root_name_node(first, source);
            }
            // A method call is about its receiver: `record.get("x")`,
            // `row.count(None)`.
            root_name_node(function, source)
        }
        "comparison_operator" => {
            // `x in container` / `x not in container` is about the
            // container; every other comparison is about its left side.
            // `operators` is the first operator token of the chain.
            let operator = node.child_by_field_name("operators").map(|op| op.kind());
            let index = u32::from(matches!(operator, Some("in" | "not in")));
            root_name_node(node.named_child(index)?, source)
        }
        "not_operator" | "unary_operator" => {
            root_name_node(node.child_by_field_name("argument")?, source)
        }
        "boolean_operator" => root_name_node(node.child_by_field_name("left")?, source),
        "parenthesized_expression" | "await" => {
            root_name_node(first_named_non_comment(node)?, source)
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
        ZR004_ASSERTION_ROULETTE.check(&ctx, &mut out);
        out
    }

    #[test]
    fn fires_on_five_bare_asserts_at_default_threshold() {
        let src = "\
def test_many():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR004");
        // Reports at the first bare assert.
        assert_eq!(out[0].line, 2);
        assert_eq!(out[0].column, 5);
        assert!(out[0].message.contains('5'));
        assert!(out[0].message.contains('4'));
    }

    #[test]
    fn does_not_fire_on_four_bare_asserts() {
        let src = "\
def test_ok():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_six_message_carrying_asserts() {
        let src = "\
def test_many_with_messages():
    assert 1 == 1, \"a\"
    assert 2 == 2, \"b\"
    assert 3 == 3, \"c\"
    assert 4 == 4, \"d\"
    assert 5 == 5, \"e\"
    assert 6 == 6, \"f\"
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_test_with_no_asserts() {
        let src = "\
def test_empty_ish():
    do_work()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn counts_asserts_inside_nested_helpers() {
        // Asserts inside an inline helper count toward the enclosing
        // test's total. The subjects are deliberately mixed (`a`, `b`,
        // `log`) so the single-subject contract exemption does not apply
        // and the test keeps pinning the counting behaviour.
        let src = "\
def test_with_helper():
    def check_all(a, b):
        assert a.x == 1
        assert b.y == 2
        assert a.z == 3
    a, b = f()
    assert log.lines == []
    assert a.ok
    check_all(a, b)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    // --- Single-subject contract exemption (issue #23) ---

    #[test]
    fn does_not_fire_on_six_bare_asserts_about_one_subject() {
        // The reproduction from issue #23: one record, six fields. Every
        // failing line names its field, so there is nothing to
        // reconstruct — not roulette.
        let src = "\
def test_record_contract():
    record = SimpleNamespace(name=\"demo\", enabled=True, count=2, tags=[], mode=\"safe\", owner=None)
    assert record.name == \"demo\"
    assert record.enabled is True
    assert record.count == 2
    assert record.tags == []
    assert record.mode == \"safe\"
    assert record.owner is None
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_six_bare_asserts_about_several_subjects() {
        // The same count spread over the result, the mock, the log and
        // the database is a sprawling test: split it.
        let src = "\
def test_sprawl():
    result = run()
    assert result.ok
    assert result.code == 0
    assert mock.call_count == 1
    assert log.lines == []
    assert db.rows == []
    assert clock.now == 0
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
        assert!(out[0].message.contains("6 > 4"));
    }

    #[test]
    fn one_stray_subject_among_many_still_fires() {
        let src = "\
def test_mostly_one_record():
    assert record.a == 1
    assert record.b == 2
    assert record.c == 3
    assert record.d == 4
    assert other.e == 5
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2, "reported at the first bare assert");
    }

    #[test]
    fn membership_asserts_take_the_container_as_subject() {
        // `x in result`: the subject is what is being searched, on the
        // right-hand side. Twelve substring checks against one rendered
        // report are one contract.
        let src = "\
def test_report_contract():
    assert \"--- Tokens ---\" in result
    assert \"Cost: $0.18\" in result
    assert \"output $0.06\" in result
    assert \"cache read\" in result
    assert \"Tokens: 118,500\" in result
    assert \"noise\" not in result
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn subscripts_calls_and_conversions_keep_the_subject() {
        // `row[0]`, `float(row[4])`, `len(row)`, `row.count(1)` and
        // `isinstance(row, tuple)` are all about `row`.
        let src = "\
def test_rollup_contract():
    assert row[0] == verdict.n_requests
    assert row[1] == verdict.n_gaps
    assert float(row[4]) == pytest.approx(verdict.cost)
    assert len(row) == 7
    assert row.count(None) == 0
    assert isinstance(row, tuple)
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn negation_parentheses_and_boolean_operators_keep_the_subject() {
        let src = "\
def test_flags_contract():
    assert not record.deleted
    assert (record.active)
    assert record.a and record.b
    assert record.c or record.d
    assert -record.balance == 0
    assert record
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn a_table_of_calls_on_literals_has_no_subject_and_fires() {
        // `f(1) == 1`, `f(2) == 2`, ...: nothing named is under test in a
        // way the reader can identify from one line. That is
        // `@pytest.mark.parametrize` territory, so the threshold applies.
        let src = "\
def test_table():
    assert f(1) == 1
    assert f(2) == 2
    assert f(3) == 3
    assert f(4) == 4
    assert f(5) == 5
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn self_and_cls_are_not_subjects_but_the_attribute_after_them_is() {
        // `self.result.*` is one subject: the test case instance is a
        // namespace, not a thing under test.
        let one_subject = "\
class TestThing:
    def test_result_contract(self):
        assert self.result.a == 1
        assert self.result.b == 2
        assert self.result.c == 3
        assert self.result.d == 4
        assert self.result.e == 5
";
        assert!(run(one_subject).is_empty());
        // `self.a`, `self.b`, ...: five different attributes of the
        // instance are five subjects.
        let many_subjects = "\
class TestThing:
    def test_attributes(self):
        assert self.a == 1
        assert self.b == 2
        assert self.c == 3
        assert self.d == 4
        assert self.e == 5
";
        let out = run(many_subjects);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn a_bare_self_has_no_subject() {
        let src = "\
class TestThing:
    def test_self(self):
        assert self
        assert self
        assert self
        assert self
        assert self
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn exemption_is_not_capped_by_the_threshold() {
        // A long contract is still one contract. Length is not this
        // rule's smell; identifiability is.
        use std::fmt::Write as _;
        let mut src = String::from("def test_long_contract():\n");
        for i in 0..12 {
            writeln!(src, "    assert record.f{i} == {i}").expect("writing to a String");
        }
        assert!(run(&src).is_empty());
    }

    #[test]
    fn message_asserts_do_not_take_part_in_the_subject_set() {
        // Message-carrying asserts are not counted, so they cannot break
        // an otherwise single-subject run either.
        let src = "\
def test_contract_with_one_explained_check():
    assert record.a == 1
    assert record.b == 2
    assert record.c == 3
    assert record.d == 4
    assert record.e == 5
    assert other.ready, \"other must be ready before the record is read\"
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn keyword_argument_to_a_free_function_is_not_a_subject() {
        // `assert all(x for x in rows)` and `assert sorted(rows, key=f)`:
        // only a positional first argument names a subject; a generator
        // or a keyword-only call has none, so the threshold applies.
        let src = "\
def test_generators():
    assert all(r.ok for r in rows)
    assert all(r.ok for r in rows)
    assert all(r.ok for r in rows)
    assert all(r.ok for r in rows)
    assert all(r.ok for r in rows)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn await_keeps_the_subject() {
        let src = "\
async def test_client_contract(client):
    assert await client.ping()
    assert await client.status() == 200
    assert (await client.body()) == \"ok\"
    assert await client.headers()
    assert not await client.closed()
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn cls_is_a_namespace_like_self() {
        let src = "\
class TestThing:
    def test_shared_result_contract(cls):
        assert cls.result.a == 1
        assert cls.result.b == 2
        assert cls.result.c == 3
        assert cls.result.d == 4
        assert cls.result.e == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn one_subjectless_assert_withdraws_the_exemption() {
        // Four asserts about `record` plus a bare `assert True`: the
        // `None` in the tail is what breaks the run, not a second name.
        let src = "\
def test_record_then_true():
    assert record.a == 1
    assert record.b == 2
    assert record.c == 3
    assert record.d == 4
    assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
    }

    #[test]
    fn a_leading_comment_inside_the_assert_does_not_hide_the_subject() {
        let src = "\
def test_commented_contract():
    assert (
        # the name is the display name
        record.name == \"demo\"
    )
    assert len(
        # tags are a list
        record.tags,
    ) == 0
    assert record.c == 3
    assert record.d == 4
    assert record.e == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn respects_configured_max_asserts() {
        // With max_asserts = 2, three bare asserts fire.
        let src = "\
def test_three():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
";
        let mut cfg = Config::default().rule_config();
        cfg.zr004.max_asserts = 2;
        let out = run_with(src, &cfg);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 2);
        assert!(out[0].message.contains("3 > 2"));
    }

    #[test]
    fn fires_on_method_of_test_class() {
        let src = "\
class TestThing:
    def test_method(self):
        assert 1 == 1
        assert 2 == 2
        assert 3 == 3
        assert 4 == 4
        assert 5 == 5
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn does_not_fire_on_pytest_mark_skip() {
        // Skipped tests don't execute — their bare-assert count is
        // academic. Mirrors ZR007's gate.
        let src = "\
@pytest.mark.skip
def test_x():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_skip_with_reason() {
        let src = "\
@pytest.mark.skip(reason=\"todo\")
def test_x():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_pytest_mark_xfail() {
        let src = "\
@pytest.mark.xfail
def test_x():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_method_of_class_decorated_with_pytest_mark_skip() {
        let src = "\
@pytest.mark.skip
class TestX:
    def test_y(self):
        assert 1 == 1
        assert 2 == 2
        assert 3 == 3
        assert 4 == 4
        assert 5 == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_pytest_mark_skipif() {
        // `skipif` is conditional — body runs on the non-skipped path,
        // so the bare-assert count still counts. Matches ZR007.
        let src = "\
@pytest.mark.skipif(True, reason=\"x\")
def test_x():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR004");
    }

    #[test]
    fn does_not_fire_on_non_test_function_with_many_asserts() {
        let src = "\
def _helper():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5

def test_ok():
    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_runtime_skip_call_as_first_statement() {
        // A test whose first statement is `self.skipTest(...)` is
        // unconditionally skipped — its bare-assert count is academic.
        // Mirrors the decorator-based skip gate.
        let src = "\
class TestThing:
    def test_skipped_many_asserts(self):
        self.skipTest(\"not ready\")
        assert 1 == 1
        assert 2 == 2
        assert 3 == 3
        assert 4 == 4
        assert 5 == 5
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_when_runtime_skip_is_not_first_statement() {
        // The skip call is after real asserts; the test still fires.
        let src = "\
def test_asserts_then_skip():
    assert 1 == 1
    assert 2 == 2
    assert 3 == 3
    assert 4 == 4
    assert 5 == 5
    pytest.skip(\"reason\")
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR004");
    }
}
