//! Suppression annotation parsing.
//!
//! Recognises four forms of inline directive (matching the biston dialect
//! described in `PLAN.md`):
//!
//! | Comment                              | Effect                                                |
//! | ------------------------------------ | ----------------------------------------------------- |
//! | `# zorilla: ignore-file`             | every finding in the file is suppressed               |
//! | `# zorilla: ignore-file[ZR005,ZR007]`| listed codes (case-insensitive) suppressed file-wide  |
//! | `# zorilla: ignore`                  | every finding **on the same line** is suppressed      |
//! | `# zorilla: ignore[ZR001,ZR003]`     | listed codes (case-insensitive) on the same line only |
//!
//! ## Strict same-line semantics
//!
//! A line-level suppression applies **only** to findings whose reported
//! line equals the comment's own line. A `# zorilla: ignore` on the
//! preceding line does *not* suppress the next statement — users sometimes
//! expect this from other linters' "next line" semantics, but zorilla
//! v0.1 deliberately does not implement it. Put the comment on the
//! offending line itself.
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

/// Suppression annotations parsed from one Python source file.
///
/// Built once per file by [`Suppressions::from_source`] and threaded into
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

    /// Parse every `# zorilla:` directive in `source`.
    ///
    /// Iterates `source.lines()` once, locates the first `#` per line, and
    /// classifies the comment. Unknown directives (e.g. typos) are
    /// silently ignored — they're treated like any other comment.
    #[must_use]
    pub fn from_source(source: &str) -> Self {
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
    /// by the file's suppression annotations. Strict same-line — see the
    /// module-level rustdoc.
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

/// Insert `incoming` at `line`, merging with any existing entry via
/// [`LineSuppression::merge`].
///
/// In practice a single [`Suppressions::from_source`] pass never produces
/// two entries for the same line: Python's `#` runs to end-of-line and
/// the parser locks onto the first `#` per source line, so the second
/// directive in `# zorilla: ignore[ZR001]  # zorilla: ignore` is consumed
/// inside the bracket scan. The merge branch therefore only fires when
/// callers stitch together two `Suppressions` from independent parses,
/// which the unit tests do explicitly to lock in the merge semantics.
/// See `first_hash_wins_when_two_directives_share_a_line` and
/// `merge_all_dominates_codes` in the test module.
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
}
