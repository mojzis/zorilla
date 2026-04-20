//! `ZR005 mystery-guest` — flag string literals that reach outside the test.
//!
//! # Rule
//!
//! "Mystery guest" is the test-smell name for resources a test depends on
//! whose identity isn't apparent from the test itself: an absolute path
//! on disk, a hard-coded URL, a file under the user's home. These
//! literals make tests environment-dependent and hostile to CI — the fix
//! is a fixture, a temporary directory, or a parametrized constant.
//!
//! This rule fires **once per offending string literal** inside a test
//! function body. A literal is offending when, after stripping surrounding
//! quotes, its content starts with one of:
//!
//! - `/` — absolute Unix path (excludes relative paths like `./foo`);
//! - a Windows drive letter followed by `:\` — `C:\Users\…`;
//! - `~/` or `~\` — home-anchored path;
//! - `http://` or `https://` — bare HTTP(S) URL.
//!
//! F-strings / interpolated strings are skipped in v0.1 — only plain
//! `string` nodes fire. Strings used as an `assert x, "msg"` message are
//! explicitly skipped to avoid flagging failure explanations.
//!
//! Users can silence specific literals via
//! `[tool.zorilla.rules.ZR005] allowed_prefixes = [...]`: any literal
//! whose content starts with a listed prefix is not flagged.
//!
//! **Reported location**: the `string` node's `start_position()`. Pointing
//! at the literal itself is the most useful signal — the reader sees
//! exactly which argument is the mystery guest.
//!
//! ## Examples
//!
//! Positive — an absolute path inside a test:
//!
//! ```python
//! def test_reads_config():
//!     data = open("/etc/myapp/config.yml").read()   # ZR005 fires here
//!     assert data
//! ```
//!
//! Positive — a hard-coded URL:
//!
//! ```python
//! def test_fetches():
//!     resp = requests.get("https://api.example.com/v1")  # ZR005 fires here
//!     assert resp.ok
//! ```
//!
//! Negative — a relative path:
//!
//! ```python
//! def test_reads_fixture():
//!     data = open("fixtures/sample.json").read()
//!     assert data
//! ```
//!
//! Negative — the literal is the assert message:
//!
//! ```python
//! def test_path_exists():
//!     assert exists(p), "/etc/hosts should always be present"
//! ```

use std::ops::ControlFlow;

use tree_sitter::Node;

use crate::ast::{iter_test_functions, walk_descendants};
use crate::report::{Finding, Severity};
use crate::rules::{Context, Rule};

/// Maximum number of characters from the offending literal we paste into a
/// finding message. A pathological test pasting a multi-kilobyte URL or
/// path otherwise produces a multi-kilobyte finding that clobbers the
/// terminal and any future JSON consumer.
const MESSAGE_LITERAL_MAX: usize = 80;

/// The registered ZR005 rule instance.
pub static ZR005_MYSTERY_GUEST: MysteryGuestRule = MysteryGuestRule;

/// Zero-sized rule struct implementing [`Rule`] for ZR005.
pub struct MysteryGuestRule;

impl Rule for MysteryGuestRule {
    fn code(&self) -> &'static str {
        "ZR005"
    }

    fn name(&self) -> &'static str {
        "mystery-guest"
    }

    fn doc(&self) -> &'static str {
        include_str!("../../../../docs/rules/ZR005.md")
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        let allowed = &ctx.config.zr005.allowed_prefixes;
        for test_fn in iter_test_functions(ctx.tree, ctx.source) {
            let Some(body) = test_fn.child_by_field_name("body") else {
                continue;
            };
            let _ = walk_descendants::<()>(body, |node| {
                if node.kind() == "string" && !is_assert_message(node) {
                    if let Some(literal) = string_content(node, ctx.source) {
                        if is_mystery_guest(literal)
                            && !allowed
                                .iter()
                                .any(|p| !p.is_empty() && literal.starts_with(p.as_str()))
                        {
                            let start = node.start_position();
                            let display = truncate_for_message(literal, MESSAGE_LITERAL_MAX);
                            out.push(Finding {
                                code: self.code(),
                                message: format!(
                                    "test contains hardcoded external resource: {display:?}"
                                ),
                                file: ctx.file.to_path_buf(),
                                line: start.row + 1,
                                column: start.column + 1,
                                severity: Severity::Warning,
                            });
                        }
                    }
                }
                ControlFlow::Continue(())
            });
        }
    }
}

/// Extract the *content* of a tree-sitter `string` node — the text inside
/// the surrounding quotes. Prefers the `string_content` named child when
/// the grammar exposes one; falls back to stripping leading/trailing
/// quote characters from the full node text.
///
/// Returns `None` if the node is an f-string or otherwise carries
/// interpolation children we refuse to interpret in v0.1.
fn string_content<'a>(node: Node<'_>, source: &'a str) -> Option<&'a str> {
    // Reject f-strings: a plain string node has no `interpolation` child.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "interpolation" {
            return None;
        }
    }

    // Prefer `string_content` child when present (modern tree-sitter-python).
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "string_content" {
            return child.utf8_text(source.as_bytes()).ok();
        }
    }

    // Fallback: strip one layer of quote characters off the raw text. This
    // is OK because we only care about the leading characters for prefix
    // detection; trailing quotes don't affect `starts_with`.
    let raw = node.utf8_text(source.as_bytes()).ok()?;
    Some(strip_quotes(raw))
}

/// Strip the leading/trailing quote delimiter from a raw `string` node's
/// text. Handles `"..."`, `'...'`, `"""..."""`, `'''...'''`, and string
/// prefixes (`r`, `b`, `rb`, `u`) case-insensitively. Returns the raw
/// text unchanged if it doesn't look like a quoted string.
fn strip_quotes(raw: &str) -> &str {
    // Drop optional prefix letters (r, b, u, rb, br, …).
    let mut rest = raw;
    while let Some(ch) = rest.chars().next() {
        if ch.is_ascii_alphabetic() {
            rest = &rest[ch.len_utf8()..];
        } else {
            break;
        }
    }
    for delim in ["\"\"\"", "'''", "\"", "'"] {
        if let Some(inner) = rest.strip_prefix(delim) {
            return inner.strip_suffix(delim).unwrap_or(inner);
        }
    }
    raw
}

/// Does `literal` (string content, no surrounding quotes) look like a
/// mystery-guest external resource per the rule's criteria?
fn is_mystery_guest(literal: &str) -> bool {
    if literal.starts_with('/') {
        return true;
    }
    if literal.starts_with("http://") || literal.starts_with("https://") {
        return true;
    }
    if literal.starts_with("~/") || literal.starts_with("~\\") {
        return true;
    }
    if is_windows_drive_path(literal) {
        return true;
    }
    false
}

/// `^[A-Za-z]:\\` — a Windows absolute path like `C:\Users\alice`.
fn is_windows_drive_path(literal: &str) -> bool {
    let mut chars = literal.chars();
    let Some(drive) = chars.next() else {
        return false;
    };
    if !drive.is_ascii_alphabetic() {
        return false;
    }
    chars.next() == Some(':') && chars.next() == Some('\\')
}

/// Is `string_node` the **message** argument of an `assert x, "msg"`
/// statement? That is the string's immediate enclosing statement is an
/// `assert_statement` and the string is its second named child.
///
/// Walks up through any `parenthesized_expression` wrappers first so that
/// `assert exists(p), ("/etc/hosts should be there")` is treated the same
/// as the unparenthesized form.
fn is_assert_message(string_node: Node<'_>) -> bool {
    // The literal we care about is the one nested inside the message slot.
    // Climb out of any `parenthesized_expression` wrappers, tracking the
    // outermost expression so we can identify it as the assert's second
    // named child.
    let mut current = string_node;
    while let Some(parent) = current.parent() {
        if parent.kind() == "parenthesized_expression" {
            current = parent;
        } else {
            break;
        }
    }
    let Some(parent) = current.parent() else {
        return false;
    };
    if parent.kind() != "assert_statement" {
        return false;
    }
    // Named children of `assert_statement`: index 0 = asserted expression,
    // index 1 = message (when present).
    let mut cursor = parent.walk();
    let named: Vec<Node<'_>> = parent.named_children(&mut cursor).collect();
    named.get(1).is_some_and(|n| n.id() == current.id())
}

/// Truncate `s` to at most `max` characters, appending `...` when we cut.
/// Counts characters (not bytes) so multi-byte literals don't slice on a
/// UTF-8 boundary.
fn truncate_for_message(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("...");
    out
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
        ZR005_MYSTERY_GUEST.check(&ctx, &mut out);
        out
    }

    #[test]
    fn fires_on_absolute_unix_path() {
        let src = "\
def test_reads():
    data = open(\"/etc/myapp/config\").read()
    assert data
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, "ZR005");
        assert_eq!(out[0].line, 2);
        assert!(out[0].message.contains("/etc/myapp/config"));
    }

    #[test]
    fn fires_on_https_url() {
        let src = "\
def test_fetches():
    r = get(\"https://api.example.com/v1\")
    assert r
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert!(out[0].message.contains("https://api.example.com/v1"));
    }

    #[test]
    fn fires_on_home_path() {
        let src = "\
def test_reads_home():
    data = open(\"~/data.txt\").read()
    assert data
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn fires_on_windows_drive_path() {
        // Raw string so we don't have to escape backslashes in the Python
        // literal; tree-sitter-python still parses this as a `string`.
        let src = "\
def test_windows():
    data = open(r\"C:\\Users\\alice\\data.txt\").read()
    assert data
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn fires_once_per_literal_in_same_test() {
        let src = "\
def test_two():
    a = open(\"/etc/a\").read()
    b = get(\"https://api.example.com\")
    assert a and b
";
        let out = run(src);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].line, 2);
        assert_eq!(out[1].line, 3);
    }

    #[test]
    fn does_not_fire_on_relative_path() {
        let src = "\
def test_relative():
    data = open(\"fixtures/sample.json\").read()
    assert data
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_assert_message_string() {
        let src = "\
def test_msg():
    assert exists(p), \"/etc/hosts should be there\"
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn allowed_prefix_silences_matching_literal() {
        let src = "\
def test_local():
    r = get(\"http://localhost:8080/v1\")
    assert r
";
        let mut cfg = Config::default().rule_config();
        cfg.zr005.allowed_prefixes = vec!["http://localhost".to_string()];
        assert!(run_with(src, &cfg).is_empty());
    }

    #[test]
    fn allowed_prefix_does_not_silence_other_literals() {
        let src = "\
def test_mixed():
    r = get(\"http://localhost/\")
    s = get(\"https://api.example.com/\")
    assert r and s
";
        let mut cfg = Config::default().rule_config();
        cfg.zr005.allowed_prefixes = vec!["http://localhost".to_string()];
        let out = run_with(src, &cfg);
        assert_eq!(out.len(), 1);
        assert!(out[0].message.contains("https://api.example.com"));
    }

    #[test]
    fn does_not_fire_on_fstring_literal() {
        // f-strings are deliberately out of scope in v0.1.
        let src = "\
def test_fstring():
    base = \"example.com\"
    r = get(f\"https://{base}/v1\")
    assert r
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn fires_on_method_of_test_class() {
        let src = "\
class TestThing:
    def test_method(self):
        data = open(\"/etc/hosts\").read()
        assert data
";
        let out = run(src);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].line, 3);
    }

    #[test]
    fn does_not_fire_on_mystery_literal_outside_any_test() {
        let src = "\
def _helper():
    return open(\"/etc/hosts\").read()


def test_ok():
    data = _helper()
    assert data
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn does_not_fire_on_parenthesized_assert_message() {
        // `assert cond, ("msg")` — the literal is still the failure
        // message, even though the grammar wraps it in a
        // `parenthesized_expression`.
        let src = "\
def test_msg():
    assert exists(p), (\"/etc/hosts should be there\")
";
        assert!(run(src).is_empty());
    }

    #[test]
    fn truncates_long_literal_in_message() {
        // A pathological long literal should not produce a multi-kilobyte
        // finding message.
        let long_path = "/".to_string() + &"a".repeat(500);
        let src = format!("def test_long():\n    open(\"{long_path}\")\n    assert True\n");
        let out = run(&src);
        assert_eq!(out.len(), 1);
        // Message length stays bounded — the literal contribution is
        // capped at MESSAGE_LITERAL_MAX (+ ellipsis + format prefix).
        assert!(
            out[0].message.len() < 200,
            "ZR005 message should be bounded; got {} chars",
            out[0].message.len()
        );
        assert!(out[0].message.contains("..."), "expected truncation marker in message");
    }

    #[test]
    fn empty_allowed_prefix_does_not_silence_findings() {
        // Regression guard: an empty-string prefix would trivially match
        // every literal via `starts_with("")`. Config layer strips empty
        // entries; here we verify the rule code is safe even if such an
        // entry leaks through.
        let src = "\
def test_url():
    r = get(\"https://api.example.com\")
    assert r
";
        let mut cfg = Config::default().rule_config();
        cfg.zr005.allowed_prefixes = vec![String::new()];
        // The rule must NOT silence the finding just because the empty
        // string is technically a prefix of every literal.
        let out = run_with(src, &cfg);
        assert_eq!(out.len(), 1, "empty prefix must not silence ZR005");
    }

    #[test]
    fn fires_on_logger_call_with_url() {
        // Strings used as call arguments to non-assert functions still
        // count — v0.1 is deliberately opinionated.
        let src = "\
def test_logs():
    logger.info(\"https://api.example.com\")
    assert True
";
        let out = run(src);
        assert_eq!(out.len(), 1);
    }
}
