//! Manifest of changed lines used by `zorilla check --changed-lines`.
//!
//! The manifest is one file per line. Ranges are optional, inclusive,
//! 1-indexed, and comma-separated:
//!
//! ```text
//! tests/test_orders.py:5-12,40-55
//! tests/test_returns.py:1-3
//! tests/test_new_file.py
//! ```
//!
//! Rules:
//! - No ranges → whole-file (no range filter for that path).
//! - A path may appear multiple times; ranges union. A whole-file
//!   appearance promotes any prior range entry to whole-file (more
//!   permissive wins).
//! - Blank lines and `#`-prefixed comments are skipped.
//! - Malformed lines are errors with the source line number — manifests
//!   are machine-generated, so a malformed line means an upstream bug.
//!
//! Filtering semantics: a finding is kept iff its file is not in the
//! manifest, or its file is in the manifest with no ranges (whole-file
//! entry), or its `line` is inside one of the merged ranges.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Component, Path, PathBuf};

/// Errors surfaced while parsing a `--changed-lines` manifest.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// IO error reading the manifest source.
    #[error("reading manifest: {0}")]
    Io(#[from] std::io::Error),
    /// A line in the manifest is malformed. Line numbers are 1-indexed
    /// and refer to the source manifest, not the file being linted.
    #[error("manifest line {line}: {message}")]
    Malformed { line: usize, message: String },
}

/// Sorted, non-overlapping, merged set of inclusive 1-indexed ranges.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RangeSet(Vec<(u32, u32)>);

impl RangeSet {
    fn contains(&self, line: u32) -> bool {
        // Linear scan is plenty for typical hunk counts per file (<20).
        self.0.iter().any(|&(s, e)| s <= line && line <= e)
    }
}

/// Parsed `--changed-lines` manifest.
#[derive(Debug, Clone, Default)]
pub struct ChangedLines {
    /// `None` ⇒ tracked, whole-file (no range filter).
    /// `Some(rs)` ⇒ keep findings whose line is in one of the ranges.
    files: HashMap<PathBuf, Option<RangeSet>>,
}

impl ChangedLines {
    /// Parse a manifest from any `BufRead` source.
    pub fn from_reader<R: BufRead>(r: R) -> Result<Self, ManifestError> {
        // Per-path raw ranges before merge. `None` means "whole-file
        // seen for this path" — once set, ranges for that path are
        // ignored (whole-file is more permissive and wins).
        let mut raw: HashMap<PathBuf, Option<Vec<(u32, u32)>>> = HashMap::new();

        for (idx, line) in r.lines().enumerate() {
            let line_no = idx + 1; // 1-indexed for error messages.
            let line = line?;
            let trimmed = line.strip_suffix('\r').unwrap_or(&line).trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let (path, ranges) = parse_line(trimmed, line_no)?;
            let entry = raw.entry(path).or_insert(Some(Vec::new()));
            match (entry, ranges) {
                // Whole-file already wins; subsequent ranges are absorbed.
                (None, _) => {}
                // Whole-file appears — collapse to None regardless of prior.
                (slot @ Some(_), None) => {
                    *slot = None;
                }
                (Some(acc), Some(rs)) => {
                    acc.extend(rs);
                }
            }
        }

        let files =
            raw.into_iter().map(|(path, ranges)| (path, ranges.map(merge_ranges))).collect();
        Ok(Self { files })
    }

    /// Parse a manifest from a path. `"-"` means stdin.
    pub fn from_path(p: &Path) -> Result<Self, ManifestError> {
        if p.as_os_str() == "-" {
            let stdin = std::io::stdin();
            let lock = stdin.lock();
            Self::from_reader(lock)
        } else {
            let f = std::fs::File::open(p)?;
            Self::from_reader(std::io::BufReader::new(f))
        }
    }

    /// True if the manifest contains zero file entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Iterate over the manifest's file paths.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.keys().map(PathBuf::as_path)
    }

    /// Should a finding at `file:line` survive the filter?
    ///
    /// Returns `true` when:
    /// - `file` is not in the manifest (not a changed file, untouched), or
    /// - `file` is in the manifest with no ranges (whole-file entry), or
    /// - `file` is in the manifest with ranges and `line` is in one of them.
    ///
    /// Returns `false` only when `file` is in the manifest with ranges
    /// AND `line` is outside every range.
    ///
    /// Path matching normalizes `./` prefixes on both sides so the
    /// discovery walker's `./tests/foo.py` matches a manifest entry of
    /// `tests/foo.py`.
    #[must_use]
    pub fn keeps(&self, file: &Path, line: usize) -> bool {
        let normalized = normalize_path(file);
        let entry = self.files.get(&normalized).or_else(|| self.files.get(file));
        match entry {
            None | Some(None) => true,
            Some(Some(rs)) => {
                let Ok(line_u32) = u32::try_from(line) else {
                    // Lines beyond u32::MAX are not real. Treat as
                    // "outside every range" — drop the finding. The
                    // alternative (always-keep) would be more surprising.
                    return false;
                };
                rs.contains(line_u32)
            }
        }
    }
}

/// Parsed contents of one non-blank, non-comment manifest line: a path
/// and either an explicit range list or `None` (whole-file).
type ParsedLine = (PathBuf, Option<Vec<(u32, u32)>>);

/// Strip leading `./` and any internal `.` components from a path so two
/// equivalent relative paths compare equal. We deliberately do NOT
/// canonicalize (no filesystem access), and we do NOT collapse `..` —
/// either would change semantics relative to the manifest's text.
fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Parse a single non-blank, non-comment manifest line.
///
/// Returns `(path, Some(ranges))` for `path:1-2,5-9` and `(path, None)`
/// for a bare `path` (whole-file entry).
fn parse_line(line: &str, line_no: usize) -> Result<ParsedLine, ManifestError> {
    // Find the LAST `:` so any range spec at the tail is the range
    // spec. Manifests are machine-generated with POSIX paths (per
    // context.md — relative to CWD, same as `--files-from`), so a path
    // containing `:` is not a practical concern.
    let Some(colon_idx) = line.rfind(':') else {
        // Bare path, whole-file entry.
        let path = PathBuf::from(line);
        if path.as_os_str().is_empty() {
            return Err(ManifestError::Malformed { line: line_no, message: "empty path".into() });
        }
        return Ok((path, None));
    };

    let (path_str, rest) = line.split_at(colon_idx);
    // `rest` still begins with the `:`.
    let range_str = &rest[1..];

    // A path with no range spec after the colon is malformed. We treat
    // an empty range as an explicit error per context.md — `foo.py:` is
    // not the same as `foo.py`.
    if range_str.is_empty() {
        return Err(ManifestError::Malformed {
            line: line_no,
            message: "empty range after ':'".into(),
        });
    }

    if path_str.is_empty() {
        return Err(ManifestError::Malformed { line: line_no, message: "empty path".into() });
    }

    let mut ranges = Vec::new();
    for piece in range_str.split(',') {
        let piece = piece.trim();
        if piece.is_empty() {
            return Err(ManifestError::Malformed {
                line: line_no,
                message: "empty range segment".into(),
            });
        }
        let (start_str, end_str) =
            piece.split_once('-').ok_or_else(|| ManifestError::Malformed {
                line: line_no,
                message: format!("range {piece:?} missing '-' separator"),
            })?;
        let start: u32 = start_str.parse().map_err(|_| ManifestError::Malformed {
            line: line_no,
            message: format!("range start {start_str:?} is not an integer"),
        })?;
        let end: u32 = end_str.parse().map_err(|_| ManifestError::Malformed {
            line: line_no,
            message: format!("range end {end_str:?} is not an integer"),
        })?;
        if start == 0 || end == 0 {
            return Err(ManifestError::Malformed {
                line: line_no,
                message: format!("range {piece:?} contains 0 (line numbers are 1-indexed)"),
            });
        }
        if start > end {
            return Err(ManifestError::Malformed {
                line: line_no,
                message: format!("range {piece:?} has start > end"),
            });
        }
        ranges.push((start, end));
    }

    Ok((PathBuf::from(path_str), Some(ranges)))
}

/// Sort, merge overlapping or adjacent ranges. Adjacent (`end + 1 == start`)
/// is treated as overlapping — `1-3,4-5` collapses to `1-5`.
fn merge_ranges(mut ranges: Vec<(u32, u32)>) -> RangeSet {
    if ranges.is_empty() {
        return RangeSet(Vec::new());
    }
    ranges.sort_unstable_by_key(|&(s, _)| s);
    let mut out: Vec<(u32, u32)> = Vec::with_capacity(ranges.len());
    for (s, e) in ranges {
        if let Some(last) = out.last_mut() {
            // Adjacent or overlapping — extend `last.1` to max(last.1, e).
            // `last.1 + 1 >= s` is the adjacency check; `last.1` may
            // already be `u32::MAX` but real line counts never reach
            // that, so saturating add is safe.
            if last.1.saturating_add(1) >= s {
                last.1 = last.1.max(e);
                continue;
            }
        }
        out.push((s, e));
    }
    RangeSet(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn parse(s: &str) -> Result<ChangedLines, ManifestError> {
        ChangedLines::from_reader(Cursor::new(s))
    }

    #[test]
    fn parses_single_range_entry() {
        let cl = parse("tests/test_a.py:5-12\n").unwrap();
        assert!(!cl.is_empty());
        assert!(cl.keeps(Path::new("tests/test_a.py"), 5));
        assert!(cl.keeps(Path::new("tests/test_a.py"), 12));
        assert!(!cl.keeps(Path::new("tests/test_a.py"), 4));
        assert!(!cl.keeps(Path::new("tests/test_a.py"), 13));
    }

    #[test]
    fn parses_multi_range_entry() {
        let cl = parse("tests/test_a.py:5-7,20-22\n").unwrap();
        assert!(cl.keeps(Path::new("tests/test_a.py"), 6));
        assert!(cl.keeps(Path::new("tests/test_a.py"), 21));
        assert!(!cl.keeps(Path::new("tests/test_a.py"), 10));
    }

    #[test]
    fn whole_file_entry_keeps_all_lines() {
        let cl = parse("tests/test_a.py\n").unwrap();
        assert!(cl.keeps(Path::new("tests/test_a.py"), 1));
        assert!(cl.keeps(Path::new("tests/test_a.py"), 999_999));
    }

    #[test]
    fn unions_ranges_across_repeated_entries() {
        let cl = parse("tests/test_a.py:1-3\ntests/test_a.py:5-7\n").unwrap();
        assert!(cl.keeps(Path::new("tests/test_a.py"), 1));
        assert!(cl.keeps(Path::new("tests/test_a.py"), 6));
        assert!(!cl.keeps(Path::new("tests/test_a.py"), 4));
    }

    #[test]
    fn whole_file_appearance_promotes_prior_range_entry() {
        let cl = parse("tests/test_a.py:1-3\ntests/test_a.py\n").unwrap();
        // Whole-file wins — every line is kept.
        assert!(cl.keeps(Path::new("tests/test_a.py"), 99));
    }

    #[test]
    fn whole_file_first_then_range_still_whole_file() {
        // Order shouldn't matter: bare path first, then range entry —
        // the bare path's "no filter" semantics still dominate.
        let cl = parse("tests/test_a.py\ntests/test_a.py:1-3\n").unwrap();
        assert!(cl.keeps(Path::new("tests/test_a.py"), 50));
    }

    #[test]
    fn merges_overlapping_and_adjacent_ranges() {
        let cl = parse("tests/test_a.py:1-5,3-7,8-10\n").unwrap();
        // Merged into single 1-10 range.
        assert!(cl.keeps(Path::new("tests/test_a.py"), 1));
        assert!(cl.keeps(Path::new("tests/test_a.py"), 7));
        assert!(cl.keeps(Path::new("tests/test_a.py"), 10));
        assert!(!cl.keeps(Path::new("tests/test_a.py"), 11));
    }

    #[test]
    fn skips_blank_lines_and_comments() {
        let cl = parse("\n# header\ntests/test_a.py:1-3\n\n   \n# trailer\n").unwrap();
        assert!(cl.keeps(Path::new("tests/test_a.py"), 2));
        assert_eq!(cl.paths().count(), 1);
    }

    #[test]
    fn empty_manifest_has_no_entries() {
        let cl = parse("").unwrap();
        assert!(cl.is_empty());
        let cl = parse("# only comments\n\n").unwrap();
        assert!(cl.is_empty());
    }

    /// Shared helper for the malformed-line tests: assert the parser
    /// returned `Malformed { line: expected_line }` and that the message
    /// matched `predicate`. Centralizing keeps each test compact and
    /// avoids clippy's `match_wildcard_for_single_variants` warning on
    /// every site.
    fn assert_malformed(
        err: ManifestError,
        expected_line: usize,
        predicate: impl Fn(&str) -> bool,
    ) {
        let ManifestError::Malformed { line, message } = err else {
            panic!("expected ManifestError::Malformed, got {err:?}");
        };
        assert_eq!(line, expected_line, "manifest line mismatch (message: {message})");
        assert!(predicate(&message), "predicate failed on message: {message}");
    }

    #[test]
    fn malformed_non_integer_errors_with_line_number() {
        let err = parse("tests/test_a.py:abc-5\n").unwrap_err();
        assert_malformed(err, 1, |m| m.contains("not an integer"));
    }

    #[test]
    fn malformed_reversed_range_errors() {
        let err = parse("\ntests/test_a.py:10-5\n").unwrap_err();
        // 1-indexed: the offending line is line 2.
        assert_malformed(err, 2, |m| m.contains("start > end"));
    }

    #[test]
    fn malformed_zero_line_errors() {
        let err = parse("tests/test_a.py:0-3\n").unwrap_err();
        assert_malformed(err, 1, |m| m.contains('0'));
    }

    #[test]
    fn malformed_empty_range_after_colon_errors() {
        let err = parse("tests/test_a.py:\n").unwrap_err();
        assert_malformed(err, 1, |m| m.contains("empty range"));
    }

    #[test]
    fn malformed_missing_hyphen_errors() {
        let err = parse("tests/test_a.py:5\n").unwrap_err();
        assert_malformed(err, 1, |m| m.contains("missing '-'"));
    }

    #[test]
    fn malformed_empty_range_segment_errors() {
        let err = parse("tests/test_a.py:1-3,,5-7\n").unwrap_err();
        assert_malformed(err, 1, |m| m.contains("empty range segment"));
    }

    #[test]
    fn untracked_file_is_kept_entirely() {
        let cl = parse("tests/test_a.py:1-3\n").unwrap();
        // File not in manifest — kept regardless of line.
        assert!(cl.keeps(Path::new("tests/other.py"), 100));
    }

    #[test]
    fn keeps_normalizes_dot_slash_prefix() {
        // Path canonicalization risk from context.md §"Sharp edges":
        // manifest typically has `tests/foo.py`, but the discovery
        // walker can emit `./tests/foo.py`. Both must match the same
        // manifest entry.
        let cl = parse("tests/test_a.py:1-3\n").unwrap();
        assert!(cl.keeps(Path::new("./tests/test_a.py"), 2));
        assert!(!cl.keeps(Path::new("./tests/test_a.py"), 99));
    }

    #[test]
    fn keeps_normalizes_dot_slash_in_manifest() {
        // The reverse direction: manifest entry has `./` prefix while
        // finding's path is bare. Should still match.
        let cl = parse("./tests/test_a.py:1-3\n").unwrap();
        assert!(cl.keeps(Path::new("tests/test_a.py"), 2));
    }

    #[test]
    fn paths_iter_returns_all_entries() {
        let cl = parse("a.py:1-1\nb.py\nc.py:5-7,10-12\n").unwrap();
        let mut names: Vec<_> = cl.paths().map(|p| p.to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, vec!["a.py", "b.py", "c.py"]);
    }

    #[test]
    fn single_line_range_is_exact_point() {
        let cl = parse("a.py:7-7\n").unwrap();
        assert!(cl.keeps(Path::new("a.py"), 7));
        assert!(!cl.keeps(Path::new("a.py"), 6));
        assert!(!cl.keeps(Path::new("a.py"), 8));
    }

    #[test]
    fn lint_filter_keeps_untracked_drops_out_of_range() {
        // Integration-shape test for the keeps() contract used by
        // lint_with_filter: tracked file with ranges drops out-of-range
        // hits, untracked file keeps everything.
        let cl = parse("tests/test_a.py:5-10\n").unwrap();
        assert!(cl.keeps(Path::new("tests/test_a.py"), 7));
        assert!(!cl.keeps(Path::new("tests/test_a.py"), 1));
        assert!(cl.keeps(Path::new("tests/other.py"), 1));
    }
}
