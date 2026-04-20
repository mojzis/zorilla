//! `ZR002 sleep-in-test` — flag `sleep` calls inside a test.
//!
//! # Rule
//!
//! Real time in tests is a race-condition generator: `time.sleep` /
//! `asyncio.sleep` either wastes the suite's wall-clock budget or
//! occasionally fails under load. The fix is almost always to wait on a
//! condition (polling, event, mocked clock), not on a fixed duration.
//!
//! This rule fires **once per `sleep` call** inside a test function body
//! (direct or nested in inline helpers). A call matches when its function
//! expression is:
//!
//! - `sleep` — a bare identifier, e.g. `from time import sleep; sleep(1)`;
//! - `*.sleep` — any attribute access whose final segment is `sleep`,
//!   which covers `time.sleep`, `asyncio.sleep`, `trio.sleep`, etc.
//!
//! ## Examples
//!
//! Positive — `time.sleep` in the test body:
//!
//! ```python
//! import time
//!
//! def test_waits():
//!     trigger()
//!     time.sleep(1)       # ZR002 fires here
//!     assert done()
//! ```
//!
//! Positive — bare `sleep` import:
//!
//! ```python
//! from time import sleep
//!
//! def test_waits():
//!     sleep(0.5)          # ZR002 fires here
//! ```
//!
//! Negative — `sleep` outside any test function is ignored.

use std::ops::ControlFlow;

use tree_sitter::Node;

use crate::ast::{iter_test_functions, walk_descendants};
use crate::report::{Finding, Severity};
use crate::rules::{Context, Rule};

/// Canonical finding message for ZR002.
pub const MESSAGE: &str = "test calls sleep — wait on a condition instead";

/// The registered ZR002 rule instance.
pub static ZR002_SLEEP: SleepRule = SleepRule;

/// Zero-sized rule struct implementing [`Rule`] for ZR002.
pub struct SleepRule;

impl Rule for SleepRule {
    fn code(&self) -> &'static str {
        "ZR002"
    }

    fn name(&self) -> &'static str {
        "sleep-in-test"
    }

    fn doc(&self) -> &'static str {
        include_str!("../../../../docs/rules/ZR002.md")
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        for test_fn in iter_test_functions(ctx.tree, ctx.source) {
            let Some(body) = test_fn.child_by_field_name("body") else {
                continue;
            };
            let _ = walk_descendants::<()>(body, |node| {
                if node.kind() == "call" && call_is_sleep(node, ctx.source) {
                    let start = node.start_position();
                    out.push(Finding {
                        code: self.code(),
                        message: MESSAGE.to_string(),
                        file: ctx.file.to_path_buf(),
                        line: start.row + 1,
                        column: start.column + 1,
                        severity: Severity::Warning,
                    });
                }
                ControlFlow::Continue(())
            });
        }
    }
}

/// Is `call_node` a call whose function expression resolves (syntactically)
/// to `sleep` or `<something>.sleep`?
fn call_is_sleep(call_node: Node<'_>, source: &str) -> bool {
    let Some(func) = call_node.child_by_field_name("function") else {
        return false;
    };
    match func.kind() {
        "identifier" => identifier_text(func, source) == Some("sleep"),
        "attribute" => func
            .child_by_field_name("attribute")
            .and_then(|a| identifier_text(a, source))
            .is_some_and(|n| n == "sleep"),
        _ => false,
    }
}

fn identifier_text<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    node.utf8_text(source.as_bytes()).ok()
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
        ZR002_SLEEP.check(&ctx, &mut out);
        out
    }

    #[test]
    fn fires_on_time_sleep_call() {
        let src = "\
import time

def test_waits():
    time.sleep(1)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR002");
        assert_eq!(out[0].message, MESSAGE);
        assert_eq!(out[0].line, 4);
        assert_eq!(out[0].column, 5);
    }

    #[test]
    fn fires_on_asyncio_sleep_call() {
        let src = "\
import asyncio

async def test_waits():
    await asyncio.sleep(0.1)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 4);
    }

    #[test]
    fn fires_on_bare_sleep_call() {
        let src = "\
from time import sleep

def test_waits():
    sleep(2)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 4);
        assert_eq!(out[0].column, 5);
    }

    #[test]
    fn fires_on_each_sleep_call_in_test() {
        let src = "\
import time

def test_many():
    time.sleep(1)
    time.sleep(2)
";
        let out = run(src);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].line, 4);
        assert_eq!(out[1].line, 5);
    }

    #[test]
    fn fires_on_sleep_inside_nested_helper() {
        let src = "\
import time

def test_with_helper():
    def inner():
        time.sleep(1)
    inner()
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 5);
    }

    #[test]
    fn does_not_fire_on_non_sleep_calls() {
        let src = "\
def test_plain():
    do_something()
    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_sleep_outside_any_test() {
        let src = "\
import time

def _helper():
    time.sleep(1)

def test_clean():
    assert True
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_attribute_ending_in_other_name() {
        // `time.monotonic()` is not a sleep even though it's an attribute
        // call on `time`.
        let src = "\
import time

def test_mono():
    t = time.monotonic()
    assert t > 0
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_variable_named_sleep_not_called() {
        // A bare `sleep` identifier that isn't in call position is ignored.
        let src = "\
def test_refs():
    sleep = 1
    assert sleep == 1
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_method_of_test_class() {
        let src = "\
import time

class TestClock:
    def test_method(self):
        time.sleep(1)
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 5);
    }
}
