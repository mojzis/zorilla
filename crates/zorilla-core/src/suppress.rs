//! Suppression annotation parsing.
//!
//! Recognises four forms of inline directive (matching the biston dialect
//! described in `PLAN.md`):
//!
//! | Comment                              | Effect                                                |
//! | ------------------------------------ | ----------------------------------------------------- |
//! | `# zorilla: ignore-file`             | every finding in the file is suppressed               |
//! | `# zorilla: ignore-file[ZR005,ZR007]`| listed codes (case-insensitive) suppressed file-wide  |
//! | `# zorilla: ignore`                  | every finding on the same line / statement suppressed |
//! | `# zorilla: ignore[ZR001,ZR003]`     | listed codes (case-insensitive), same line / statement|
//!
//! ## Line scope: the comment's line, widened to its statement
//!
//! A line-level suppression applies to findings whose reported line equals
//! the comment's own line. When the comment sits inside a bracketed
//! multi-line statement — a call, an `assert (...)`, a `for x in (...)`
//! header — it applies to **every physical line of that statement**, from
//! the line the opening bracket sits on to the line of the closing one.
//! Two facts make that widening necessary rather than convenient:
//!
//! - a rule anchors its finding on one specific line of a statement (the
//!   `assert` keyword, the `for` keyword, the string literal itself), and
//!   a reader should not have to know which;
//! - `ruff format` and `black` move a trailing comment to the **closing
//!   bracket's line** when they split an over-long statement, so a
//!   directive written on the anchor line is carried off it by the next
//!   format run. Strict same-line matching then dropped the suppression
//!   silently, and consumers responded by writing the directive on both
//!   bracket lines (zorilla issue #26).
//!
//! The widening stops at the statement: a compound statement's body is not
//! part of its header, and a `# zorilla: ignore` on the line *above* a
//! statement still does not reach it — "next line" semantics remain
//! deliberately unimplemented. [`Suppressions::from_tree`] is the one
//! constructor: the strict per-line half of the parse is a private step
//! inside it, so no caller can reach for the un-widened view by mistake.
//!
//! ## Known limitation: `#` inside string literals
//!
//! The parser scans each source line for the first `#` character without
//! reasoning about Python string literals. A literal like
//! `"# zorilla: ignore"` inside a docstring or assignment will therefore
//! be misread as a real suppression directive. This is a v0.1 edge case;
//! a future revision can swap in a tree-sitter walk over `comment` nodes
//! to fix it. The same caveat applies to `#` inside f-strings and
//! triple-quoted strings.

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use tree_sitter::{Node, Tree};

use crate::ast::walk_descendants_pruned;

/// Suppression annotations parsed from one Python source file.
///
/// Built once per file by [`Suppressions::from_tree`] and threaded into
/// [`crate::rules::Context`]. The engine consults
/// [`Self::suppresses_code`] to short-circuit a rule before it runs, then
/// filters per-finding via [`Self::is_suppressed`] after rules have
/// produced their output.
#[derive(Debug, Default, Clone)]
pub struct Suppressions {
    file_level: FileLevel,
    /// 1-indexed source line → suppression scope on that line.
    per_line: HashMap<usize, LineSuppression>,
}

/// File-scope suppression — produced by `# zorilla: ignore-file` and
/// `# zorilla: ignore-file[ZR00X, ...]` directives.
#[derive(Debug, Clone, Default)]
enum FileLevel {
    /// No file-level directive was seen.
    #[default]
    None,
    /// `# zorilla: ignore-file` — drop every code at file scope.
    All,
    /// `# zorilla: ignore-file[ZR00X, ...]` — drop only listed codes
    /// across the whole file. Codes are stored upper-cased so lookup is
    /// `code.to_ascii_uppercase()`.
    Codes(HashSet<String>),
}

impl FileLevel {
    /// Combine two file-scope directives encountered in the same file.
    /// `All` dominates; otherwise the code sets union.
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::All, _) | (_, Self::All) => Self::All,
            (Self::None, x) | (x, Self::None) => x,
            (Self::Codes(mut a), Self::Codes(b)) => {
                a.extend(b);
                Self::Codes(a)
            }
        }
    }

    /// Does this file-scope directive suppress `code`?
    fn suppresses(&self, code: &str) -> bool {
        match self {
            Self::None => false,
            Self::All => true,
            Self::Codes(set) => set.contains(&code.to_ascii_uppercase()),
        }
    }
}

#[derive(Debug, Clone)]
enum LineSuppression {
    /// `# zorilla: ignore` — drop any finding on this line.
    All,
    /// `# zorilla: ignore[ZR00X, ...]` — drop only listed codes. Codes
    /// are stored upper-cased so lookup is `code.to_ascii_uppercase()`.
    Codes(HashSet<String>),
}

impl LineSuppression {
    /// Combine two suppressions found on the same source line. `All`
    /// dominates; otherwise unions the code sets.
    fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::All, _) | (_, Self::All) => Self::All,
            (Self::Codes(mut a), Self::Codes(b)) => {
                a.extend(b);
                Self::Codes(a)
            }
        }
    }
}

impl Suppressions {
    /// Build an empty set — no annotations, no file-level scope. Used by
    /// per-rule unit tests so they don't have to construct a synthetic
    /// source string just to satisfy the [`crate::rules::Context`].
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parse every `# zorilla:` directive in `source`, strictly per line.
    ///
    /// Iterates `source.lines()` once, locates the first `#` per line, and
    /// classifies the comment. Unknown directives (e.g. typos) are
    /// silently ignored — they're treated like any other comment.
    ///
    /// This is the per-line half of [`Self::from_tree`], which also widens
    /// each directive to the bracketed statement it sits in. Kept
    /// crate-private on purpose: a caller that reached for this name would
    /// silently reintroduce the formatter hazard the widening exists for.
    #[must_use]
    pub(crate) fn from_source(source: &str) -> Self {
        let mut out = Self::default();
        for (idx, raw_line) in source.lines().enumerate() {
            let line_no = idx + 1;
            let Some(hash_at) = raw_line.find('#') else {
                continue;
            };
            // Comment text after the `#`. We trim leading whitespace so
            // `#zorilla:`, `# zorilla:`, and `#   zorilla:` all match.
            let after_hash = raw_line[hash_at + 1..].trim_start();
            let Some(rest) = after_hash.strip_prefix("zorilla:") else {
                continue;
            };
            let directive = rest.trim();
            match parse_directive(directive) {
                Some(ParsedDirective::Line(suppression)) => {
                    merge_into(&mut out.per_line, line_no, suppression);
                }
                Some(ParsedDirective::File(file_level)) => {
                    let existing = std::mem::take(&mut out.file_level);
                    out.file_level = existing.merge(file_level);
                }
                None => {}
            }
        }
        out
    }

    /// Parse every directive in `source` and widen each line directive to
    /// the bracketed multi-line statement it sits in.
    ///
    /// `tree` must be the parse of `source`. See the module docs for why
    /// the widening exists; the short version is that rules anchor on one
    /// line of a statement and formatters move trailing comments to
    /// another. Directives on single-line statements are unaffected, and
    /// file-scope directives are never widened (they already cover
    /// everything).
    #[must_use]
    pub fn from_tree(tree: &Tree, source: &str) -> Self {
        let mut out = Self::from_source(source);
        // Most files carry no line directive at all; do not pay for a
        // token walk to widen nothing.
        if out.per_line.is_empty() {
            return out;
        }
        for (start, end) in bracket_spans(tree) {
            let merged = (start..=end)
                .filter_map(|line| out.per_line.get(&line).cloned())
                .reduce(LineSuppression::merge);
            let Some(merged) = merged else { continue };
            for line in start..=end {
                merge_into(&mut out.per_line, line, merged.clone());
            }
        }
        out
    }

    /// Whether rule `code` is suppressed at file scope — i.e. a
    /// `# zorilla: ignore-file` (all codes) or
    /// `# zorilla: ignore-file[<code>, ...]` directive appears in the
    /// file. The engine checks this before running each rule's `check()`
    /// so an entirely silenced rule never visits the AST.
    #[must_use]
    pub fn suppresses_code(&self, code: &str) -> bool {
        self.file_level.suppresses(code)
    }

    /// Whether a finding reported at `line` for rule `code` is silenced
    /// by the file's suppression annotations. Per line, widened to the
    /// enclosing bracketed statement when built by [`Self::from_tree`] —
    /// see the module-level rustdoc.
    #[must_use]
    pub fn is_suppressed(&self, line: usize, code: &str) -> bool {
        if self.file_level.suppresses(code) {
            return true;
        }
        match self.per_line.get(&line) {
            Some(LineSuppression::All) => true,
            Some(LineSuppression::Codes(set)) => set.contains(&code.to_ascii_uppercase()),
            None => false,
        }
    }
}

/// Outcome of parsing a single `# zorilla: <directive>` comment.
enum ParsedDirective {
    /// A line-scope directive (`ignore` or `ignore[...]`).
    Line(LineSuppression),
    /// A file-scope directive (`ignore-file` or `ignore-file[...]`).
    File(FileLevel),
}

/// Decode the text **after** the `zorilla:` token. Returns a
/// [`ParsedDirective`] when we recognise the form; returns `None` for
/// anything we don't (including bare `ignore-file[]` which is a noop).
///
/// Whitespace between the keyword and a bracket list is tolerated:
/// `ignore-file [ZR005]` and `ignore [ZR001]` are parsed the same way as
/// the no-space form. Without this allowance the bracketed form would
/// silently degrade to `All`, widening user-requested suppression scope.
fn parse_directive(directive: &str) -> Option<ParsedDirective> {
    // `ignore-file` must be checked before `ignore` because the latter is
    // a prefix of the former.
    if let Some(tail) = directive.strip_prefix("ignore-file") {
        // Allow optional whitespace between the keyword and a bracket
        // list — `ignore-file [ZR005]` honours the brackets instead of
        // falling through to the bare-keyword (`All`) branch.
        let after_keyword = tail.trim_start();
        if let Some(after_bracket) = after_keyword.strip_prefix('[') {
            let close = after_bracket.find(']')?;
            let inside = &after_bracket[..close];
            let codes = parse_code_list(inside);
            if codes.is_empty() {
                // `# zorilla: ignore-file[]` — no codes, no effect.
                // Don't degrade to `All` silently.
                return None;
            }
            return Some(ParsedDirective::File(FileLevel::Codes(codes)));
        }
        if tail.is_empty() || tail.starts_with(char::is_whitespace) {
            return Some(ParsedDirective::File(FileLevel::All));
        }
        // `ignore-fileXYZ` — not a directive we recognise.
        return None;
    }

    let tail = directive.strip_prefix("ignore")?;
    // Same whitespace allowance for line scope: `ignore [ZR001]` parses
    // as a bracketed directive, not bare-`ignore`-then-extra-text.
    let after_keyword = tail.trim_start();
    if let Some(after_bracket) = after_keyword.strip_prefix('[') {
        let close = after_bracket.find(']')?;
        let inside = &after_bracket[..close];
        let codes = parse_code_list(inside);
        if codes.is_empty() {
            // `# zorilla: ignore[]` — no codes, no effect. Don't degrade
            // to `All` silently.
            return None;
        }
        return Some(ParsedDirective::Line(LineSuppression::Codes(codes)));
    }

    if tail.is_empty() || tail.starts_with(char::is_whitespace) {
        return Some(ParsedDirective::Line(LineSuppression::All));
    }
    None
}

/// Parse a comma-separated list of rule codes (e.g. `ZR001, zr003`) into
/// an upper-cased set. Whitespace and empty entries are ignored.
fn parse_code_list(inside: &str) -> HashSet<String> {
    let mut codes = HashSet::new();
    for raw in inside.split(',') {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            codes.insert(trimmed.to_ascii_uppercase());
        }
    }
    codes
}

/// The 1-indexed line ranges of every statement that spans more than one
/// physical line because a bracket is open across the line break.
///
/// Walks every token of `tree` in source order and tracks bracket depth:
/// a span opens on the line where depth leaves zero and closes on the
/// line where it returns there. `string` subtrees are skipped wholesale,
/// so a bracket inside a literal — including inside an f-string
/// interpolation — never opens or closes a span. Unbalanced closers
/// (only possible in an `ERROR` tree) are ignored rather than driving the
/// depth negative.
///
/// A compound statement's body is never inside a span: its header's last
/// bracket closes before the `:`, and the indented block that follows has
/// its own statements. Backslash continuations are not brackets and are
/// not spanned; the supported formatters remove them anyway.
///
/// An unclosed bracket (an `ERROR` tree; `parse` does not fail on one)
/// leaves the depth open, so no span closes after it and every later
/// directive keeps its strict per-line effect. tree-sitter's recovery
/// folds the rest of such a file into the error node anyway, so there is
/// no statement structure left to be faithful to.
///
/// Spans that share a line (`f(\n)(\n)` closes and re-opens on one line)
/// are merged so each physical line belongs to at most one span. Sorted
/// by start line, non-overlapping.
fn bracket_spans(tree: &Tree) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut depth = 0_usize;
    let mut open_line = 0_usize;
    let _ = walk_descendants_pruned::<()>(
        tree.root_node(),
        |child| child.kind() != "string",
        |node: Node<'_>| {
            if node.child_count() != 0 {
                return ControlFlow::Continue(());
            }
            let line = node.start_position().row + 1;
            match node.kind() {
                "(" | "[" | "{" => {
                    if depth == 0 {
                        open_line = line;
                    }
                    depth += 1;
                }
                ")" | "]" | "}" => {
                    if depth == 0 {
                        return ControlFlow::Continue(());
                    }
                    depth -= 1;
                    if depth == 0 && line > open_line {
                        spans.push((open_line, line));
                    }
                }
                _ => {}
            }
            ControlFlow::Continue(())
        },
    );
    merge_spans(spans)
}

/// Merge spans that overlap or touch on a shared line. Input is already
/// in source order because the walk is.
fn merge_spans(spans: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (start, end) in spans {
        match out.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => out.push((start, end)),
        }
    }
    out
}

/// Insert `incoming` at `line`, merging with any existing entry via
/// [`LineSuppression::merge`].
///
/// In practice a single `Suppressions::from_source` pass never produces
/// two entries for the same line: Python's `#` runs to end-of-line and
/// the parser locks onto the first `#` per source line, so the second
/// directive in `# zorilla: ignore[ZR001]  # zorilla: ignore` is consumed
/// inside the bracket scan. The merge branch therefore only fires when
/// callers stitch together two `Suppressions` from independent parses,
/// which the unit tests do explicitly to lock in the merge semantics.
/// See `first_hash_wins_when_two_directives_share_a_line` and
/// `merge_all_dominates_codes` in the test module. [`Suppressions::from_tree`]
/// is the one production caller that merges: it copies a statement's
/// combined directive onto every line of that statement.
fn merge_into(
    per_line: &mut HashMap<usize, LineSuppression>,
    line: usize,
    incoming: LineSuppression,
) {
    match per_line.remove(&line) {
        Some(existing) => {
            per_line.insert(line, existing.merge(incoming));
        }
        None => {
            per_line.insert(line, incoming);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_suppresses_nothing() {
        let s = Suppressions::empty();
        assert!(!s.is_suppressed(1, "ZR001"));
        assert!(!s.is_suppressed(42, "ZR999"));
        assert!(!s.suppresses_code("ZR001"));
    }

    #[test]
    fn from_source_with_no_directives_is_empty() {
        let s = Suppressions::from_source("def test_x():\n    assert True\n# unrelated comment\n");
        assert!(!s.suppresses_code("ZR001"));
        assert!(!s.is_suppressed(1, "ZR001"));
        assert!(!s.is_suppressed(3, "ZR001"));
    }

    #[test]
    fn ignore_file_at_top_short_circuits_every_line() {
        let s = Suppressions::from_source(
            "# zorilla: ignore-file\ndef test_x():\n    if True:\n        assert True\n",
        );
        assert!(s.suppresses_code("ZR001"));
        assert!(s.is_suppressed(99, "ZR001"));
        assert!(s.is_suppressed(1, "ZR007"));
    }

    #[test]
    fn ignore_file_inline_after_code_still_counts() {
        // The directive doesn't have to be the only thing on its line.
        let s = Suppressions::from_source("import os  # zorilla: ignore-file\n");
        assert!(s.suppresses_code("ZR001"));
    }

    #[test]
    fn ignore_file_with_trailing_text_still_counts() {
        let s = Suppressions::from_source("# zorilla: ignore-file (legacy fixture)\n");
        assert!(s.suppresses_code("ZR001"));
    }

    #[test]
    fn line_ignore_drops_findings_on_that_line_only() {
        let s = Suppressions::from_source(
            "def test_x():\n    if True:  # zorilla: ignore\n        assert True\n",
        );
        assert!(s.is_suppressed(2, "ZR001"));
        assert!(!s.is_suppressed(3, "ZR001"));
        assert!(!s.is_suppressed(1, "ZR001"));
    }

    #[test]
    fn line_ignore_with_specific_code_only_drops_that_code() {
        let s = Suppressions::from_source("    if True:  # zorilla: ignore[ZR001]\n");
        assert!(s.is_suppressed(1, "ZR001"));
        assert!(!s.is_suppressed(1, "ZR002"));
    }

    #[test]
    fn line_ignore_with_multiple_codes_in_one_bracket() {
        let s = Suppressions::from_source("x = 1  # zorilla: ignore[ZR001,ZR003]\n");
        assert!(s.is_suppressed(1, "ZR001"));
        assert!(s.is_suppressed(1, "ZR003"));
        assert!(!s.is_suppressed(1, "ZR002"));
    }

    #[test]
    fn ignore_codes_are_case_insensitive_when_parsed() {
        let s = Suppressions::from_source("x = 1  # zorilla: ignore[zr001, Zr002, ZR003]\n");
        assert!(s.is_suppressed(1, "ZR001"));
        assert!(s.is_suppressed(1, "ZR002"));
        assert!(s.is_suppressed(1, "ZR003"));
        // And lookup is also case-insensitive (lower-case input gets
        // uppercased before lookup).
        assert!(s.is_suppressed(1, "zr001"));
    }

    #[test]
    fn ignore_codes_tolerate_whitespace_between_entries() {
        let s = Suppressions::from_source("x = 1  # zorilla:   ignore[  ZR001 ,   ZR003  ]\n");
        assert!(s.is_suppressed(1, "ZR001"));
        assert!(s.is_suppressed(1, "ZR003"));
        assert!(!s.is_suppressed(1, "ZR002"));
    }

    #[test]
    fn ignore_wrong_code_does_not_suppress_other_codes() {
        let s = Suppressions::from_source("if True:  # zorilla: ignore[ZR002]\n");
        assert!(s.is_suppressed(1, "ZR002"));
        assert!(!s.is_suppressed(1, "ZR001"));
    }

    #[test]
    fn first_hash_wins_when_two_directives_share_a_line() {
        // Pins the documented parser limitation: `from_source` locks onto
        // the FIRST `#` on each line and treats everything after it as one
        // comment. So `# zorilla: ignore[ZR001]  # zorilla: ignore` is
        // parsed as a bracketed directive for ZR001 only — the trailing
        // bare `ignore` is consumed inside the bracket scan and silently
        // discarded. ZR002 must therefore stay un-suppressed; if a future
        // change starts honouring the second directive this test will
        // catch it.
        let s = Suppressions::from_source("x = 1  # zorilla: ignore[ZR001]  # zorilla: ignore\n");
        assert!(s.is_suppressed(1, "ZR001"));
        assert!(!s.is_suppressed(1, "ZR002"));
    }

    #[test]
    fn duplicate_directives_on_same_logical_line_merge_codes() {
        // Construct a line whose comment text contains two `zorilla:`
        // directives separated by a non-comment delimiter — actually
        // impossible in pure Python, since `#` runs to end-of-line. The
        // realistic merge is two source lines whose findings happen to
        // both report on the SAME reported line, but suppression is
        // strictly tied to comment location.
        //
        // The merge code path is still exercised programmatically here:
        let mut s = Suppressions::from_source("x = 1  # zorilla: ignore[ZR001]\n");
        // Manually merge another suppression into the same line via
        // public surface (re-parse a single-line source with a different
        // code, then bolt it onto the existing map).
        let other = Suppressions::from_source("y = 2  # zorilla: ignore[ZR002]\n");
        // Line 1 from `other` carries ZR002. Merge it into `s`.
        for (line, sup) in other.per_line {
            merge_into(&mut s.per_line, line, sup);
        }
        assert!(s.is_suppressed(1, "ZR001"));
        assert!(s.is_suppressed(1, "ZR002"));
        assert!(!s.is_suppressed(1, "ZR003"));
    }

    #[test]
    fn merge_all_dominates_codes() {
        let mut s = Suppressions::from_source("x = 1  # zorilla: ignore[ZR001]\n");
        let other = Suppressions::from_source("y = 2  # zorilla: ignore\n");
        for (line, sup) in other.per_line {
            merge_into(&mut s.per_line, line, sup);
        }
        // After merging an `All` onto an existing `Codes(...)`, the line
        // should suppress every code, not just ZR001.
        assert!(s.is_suppressed(1, "ZR999"));
    }

    #[test]
    fn unrecognised_directives_are_ignored() {
        let s = Suppressions::from_source("# zorilla: maybe-later\n# something else\n");
        assert!(!s.suppresses_code("ZR001"));
        assert!(!s.is_suppressed(1, "ZR001"));
        assert!(!s.is_suppressed(2, "ZR001"));
    }

    #[test]
    fn empty_brackets_are_a_noop() {
        // Better to do nothing than to silently treat `ignore[]` as
        // `ignore` and suppress the whole line.
        let s = Suppressions::from_source("x = 1  # zorilla: ignore[]\n");
        assert!(!s.is_suppressed(1, "ZR001"));
    }

    #[test]
    fn bare_ignorefoo_token_does_not_match_ignore() {
        // `ignorefoo` must not be mistaken for `ignore`.
        let s = Suppressions::from_source("x = 1  # zorilla: ignorefoo\n");
        assert!(!s.is_suppressed(1, "ZR001"));
    }

    #[test]
    fn ignore_file_with_brackets_only_suppresses_listed_code() {
        let s = Suppressions::from_source("# zorilla: ignore-file[ZR005]\n");
        assert!(s.suppresses_code("ZR005"));
        assert!(s.is_suppressed(1, "ZR005"));
        assert!(s.is_suppressed(42, "ZR005"));
    }

    #[test]
    fn ignore_file_without_brackets_still_suppresses_all() {
        // Back-compat: bare `# zorilla: ignore-file` continues to drop
        // every code in the file.
        let s = Suppressions::from_source("# zorilla: ignore-file\n");
        assert!(s.suppresses_code("ZR001"));
        assert!(s.suppresses_code("ZR005"));
        assert!(s.suppresses_code("ZR999"));
    }

    #[test]
    fn ignore_file_brackets_do_not_suppress_other_codes() {
        let s = Suppressions::from_source("# zorilla: ignore-file[ZR005]\n");
        assert!(!s.suppresses_code("ZR001"));
        assert!(!s.is_suppressed(1, "ZR001"));
    }

    #[test]
    fn ignore_file_brackets_codes_are_case_insensitive() {
        let s = Suppressions::from_source("# zorilla: ignore-file[zr005, Zr007]\n");
        assert!(s.suppresses_code("ZR005"));
        assert!(s.suppresses_code("ZR007"));
        // Lookup is case-insensitive too.
        assert!(s.suppresses_code("zr005"));
        assert!(s.is_suppressed(3, "ZR005"));
        assert!(s.is_suppressed(3, "ZR007"));
        assert!(!s.is_suppressed(3, "ZR001"));
    }

    #[test]
    fn ignore_file_empty_brackets_are_a_noop() {
        // Better to do nothing than to silently treat `ignore-file[]` as
        // `ignore-file` and suppress every code.
        let s = Suppressions::from_source("# zorilla: ignore-file[]\n");
        assert!(!s.suppresses_code("ZR001"));
        assert!(!s.suppresses_code("ZR005"));
        assert!(!s.is_suppressed(1, "ZR005"));
    }

    #[test]
    fn ignore_file_with_space_before_brackets_does_not_widen_to_all() {
        // Regression guard: a user typing `# zorilla: ignore-file [ZR005]`
        // (with a space between the keyword and the bracket) was previously
        // misparsed as bare `ignore-file` — i.e. dropping every code rather
        // than only ZR005. Tolerate the whitespace and honour the brackets.
        let s = Suppressions::from_source("# zorilla: ignore-file [ZR005]\n");
        assert!(s.suppresses_code("ZR005"));
        assert!(!s.suppresses_code("ZR001"));
    }

    #[test]
    fn ignore_with_space_before_brackets_does_not_widen_to_all() {
        // Same hazard at line scope: `# zorilla: ignore [ZR001]` was
        // previously misparsed as bare `ignore`, dropping every code on the
        // line rather than only ZR001.
        let s = Suppressions::from_source("x = 1  # zorilla: ignore [ZR001]\n");
        assert!(s.is_suppressed(1, "ZR001"));
        assert!(!s.is_suppressed(1, "ZR002"));
    }

    // --- Statement scope (`from_tree`) ---

    fn from_tree(source: &str) -> Suppressions {
        let tree = crate::parse::parse(source).expect("test source should parse");
        Suppressions::from_tree(&tree, source)
    }

    #[test]
    fn from_tree_matches_from_source_on_single_line_statements() {
        let src = "def test_x():\n    if True:  # zorilla: ignore[ZR001]\n        assert True\n";
        let s = from_tree(src);
        assert!(s.is_suppressed(2, "ZR001"));
        assert!(!s.is_suppressed(2, "ZR002"));
        assert!(!s.is_suppressed(1, "ZR001"));
        assert!(!s.is_suppressed(3, "ZR001"));
    }

    #[test]
    fn directive_on_closing_bracket_line_reaches_the_statement_start() {
        // What `ruff format` / `black` produce from
        // `assert cond  # zorilla: ignore[ZR004] -- <long reason>` once the
        // line is over the width limit: the comment lands on the `)` line.
        // ZR004 reports at the `assert` keyword, one line up.
        let src = "\
def test_x():
    assert (
        \"Server ready\" in output
    )  # zorilla: ignore[ZR004] -- startup contract
    assert other
";
        let s = from_tree(src);
        assert!(s.is_suppressed(2, "ZR004"), "the `assert (` line is part of the statement");
        assert!(s.is_suppressed(3, "ZR004"));
        assert!(s.is_suppressed(4, "ZR004"));
        assert!(!s.is_suppressed(2, "ZR001"), "the bracketed code list still narrows the scope");
        assert!(!s.is_suppressed(5, "ZR004"), "the next statement is not covered");
        assert!(!s.is_suppressed(1, "ZR004"), "the `def` line is not covered");
    }

    #[test]
    fn directive_on_opening_line_reaches_a_literal_on_an_inner_line() {
        // ZR005 reports at the string literal, which sits on the middle
        // line of the call. A directive on either bracket line covers it.
        let src = "\
def test_x(client):
    response = client.fetch(  # zorilla: ignore[ZR005] -- local double
        \"https://api.example.com/v1\",
    )
    assert response.ok
";
        let s = from_tree(src);
        assert!(s.is_suppressed(3, "ZR005"));
        assert!(s.is_suppressed(2, "ZR005"));
        assert!(s.is_suppressed(4, "ZR005"));
        assert!(!s.is_suppressed(5, "ZR005"));
    }

    #[test]
    fn compound_statement_header_is_covered_but_its_body_is_not() {
        let src = "\
def test_x(paths):
    for path in (
        paths.a,
        paths.b,
    ):  # zorilla: ignore[ZR001] -- fixture setup
        path.touch()
        assert path.exists()
";
        let s = from_tree(src);
        assert!(s.is_suppressed(2, "ZR001"), "the `for` line anchors ZR001");
        assert!(s.is_suppressed(5, "ZR001"));
        assert!(!s.is_suppressed(6, "ZR001"), "the loop body is its own statement");
        assert!(!s.is_suppressed(7, "ZR001"));
    }

    #[test]
    fn bare_ignore_inside_a_statement_covers_every_code_on_every_line_of_it() {
        let src = "\
def test_x(client):
    if client.get(
        \"https://api.example.com/v1\"
    ).ok:  # zorilla: ignore -- probe
        assert True
";
        let s = from_tree(src);
        assert!(s.is_suppressed(2, "ZR001"));
        assert!(s.is_suppressed(3, "ZR005"));
        assert!(!s.is_suppressed(5, "ZR001"));
    }

    #[test]
    fn directives_on_two_lines_of_one_statement_merge() {
        let src = "\
def test_x():
    assert (  # zorilla: ignore[ZR004]
        \"https://api.example.com/v1\" in seen
    )  # zorilla: ignore[ZR005]
";
        let s = from_tree(src);
        assert!(s.is_suppressed(2, "ZR004"));
        assert!(s.is_suppressed(2, "ZR005"));
        assert!(s.is_suppressed(3, "ZR004"));
        assert!(s.is_suppressed(3, "ZR005"));
        assert!(!s.is_suppressed(3, "ZR001"));
    }

    #[test]
    fn brackets_inside_strings_do_not_open_a_statement_span() {
        // A `(` in a string literal must not glue the next statement onto
        // this one and let the directive leak across.
        let src = "\
def test_x():
    label = \"open (\"  # zorilla: ignore[ZR005]
    if label:
        assert label
";
        let s = from_tree(src);
        assert!(s.is_suppressed(2, "ZR005"));
        assert!(!s.is_suppressed(3, "ZR005"));
        assert!(!s.is_suppressed(3, "ZR001"));
    }

    #[test]
    fn directive_on_the_line_above_a_statement_still_does_not_reach_it() {
        // Statement scope widens within a statement only; "next line"
        // semantics remain deliberately unimplemented.
        let src = "\
def test_x():
    # zorilla: ignore[ZR001]
    if True:
        assert True
";
        let s = from_tree(src);
        assert!(!s.is_suppressed(3, "ZR001"));
    }

    #[test]
    fn directive_in_a_decorator_does_not_reach_the_def_line() {
        let src = "\
@patch(
    \"mod.a\"
)  # zorilla: ignore[ZR006]
def test_x(a):
    assert a
";
        let s = from_tree(src);
        assert!(s.is_suppressed(1, "ZR006"), "ZR006 anchors on the first `@patch` line");
        assert!(!s.is_suppressed(4, "ZR006"));
    }

    #[test]
    fn from_tree_with_no_line_directives_widens_nothing() {
        let src = "def test_x():\n    assert (\n        1\n    )\n";
        let s = from_tree(src);
        assert!(s.per_line.is_empty());
        assert!(!s.is_suppressed(2, "ZR004"));
        assert!(!s.suppresses_code("ZR004"));
    }

    #[test]
    fn an_unclosed_bracket_degrades_to_strict_per_line() {
        // `parse` returns a tree with `ERROR` nodes rather than failing, so
        // this reaches `bracket_spans`. Nothing after the unclosed `(` can
        // be widened — tree-sitter folds the rest of the file into the
        // error node — but the directive keeps its own line, and no span
        // is invented across the broken statement.
        let src = "\
def test_x():
    broken = f(
    ok = 1
    assert (
        \"https://api.example.com/v1\" in seen
    )  # zorilla: ignore[ZR005]
";
        let s = from_tree(src);
        assert!(s.is_suppressed(6, "ZR005"), "the directive's own line always counts");
        assert!(!s.is_suppressed(5, "ZR005"), "no widening past an unclosed bracket");
        assert!(!s.is_suppressed(2, "ZR005"), "the broken statement is not covered");
    }

    #[test]
    fn file_level_directives_are_unaffected_by_statement_scope() {
        let src = "# zorilla: ignore-file[ZR005]\ndef test_x():\n    assert (\n        1\n    )\n";
        let s = from_tree(src);
        assert!(s.suppresses_code("ZR005"));
        assert!(!s.is_suppressed(3, "ZR004"));
    }
}
