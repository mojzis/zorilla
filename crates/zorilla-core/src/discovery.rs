//! File discovery — expand user-supplied paths into the set of Python
//! files to lint, respecting `.gitignore` / `.ignore` via the `ignore`
//! crate and filtering by the configured include / exclude globs.

use std::path::{Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

use crate::config::Config;

/// Error returned from [`discover`].
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    /// A glob pattern in `include` or `exclude` failed to compile.
    #[error("invalid glob {pattern:?}: {source}")]
    Glob {
        pattern: String,
        #[source]
        source: globset::Error,
    },
    /// The file walker surfaced an IO error.
    #[error("walking {path}: {source}")]
    Walk {
        path: PathBuf,
        #[source]
        source: ignore::Error,
    },
}

/// Discover Python files to lint.
///
/// For each input path:
/// - a file that matches `include` and not `exclude` is kept as-is;
/// - a directory is walked recursively (honoring ignore files) and the
///   same include/exclude rules are applied to each entry.
///
/// Results are de-duplicated while preserving first-seen order.
pub fn discover<P: AsRef<Path>>(
    paths: &[P],
    config: &Config,
) -> Result<Vec<PathBuf>, DiscoveryError> {
    let include = build_globset(&config.include)?;
    let exclude = build_globset(&config.exclude)?;

    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for path in paths {
        let path = path.as_ref();
        if !path.exists() {
            continue;
        }

        if path.is_file() {
            // Explicit file arguments bypass the `include` globs — if the
            // user named a file on the command line they almost certainly
            // want it linted, even when its name doesn't match
            // `test_*.py`. `exclude` still applies so users can skip
            // generated files by path convention.
            if !exclude.is_match(path) && seen.insert(path.to_path_buf()) {
                out.push(path.to_path_buf());
            }
            continue;
        }

        let walker = WalkBuilder::new(path).build();
        for entry in walker {
            let entry = entry
                .map_err(|source| DiscoveryError::Walk { path: path.to_path_buf(), source })?;
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                continue;
            }
            let candidate = entry.into_path();
            let rel = candidate.strip_prefix(path).unwrap_or(&candidate);
            if matches(rel, &include, &exclude) && seen.insert(candidate.clone()) {
                out.push(candidate);
            }
        }
    }

    Ok(out)
}

fn build_globset(patterns: &[String]) -> Result<GlobSet, DiscoveryError> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        let glob = Glob::new(pat)
            .map_err(|source| DiscoveryError::Glob { pattern: pat.clone(), source })?;
        builder.add(glob);
    }
    builder.build().map_err(|source| DiscoveryError::Glob { pattern: patterns.join(","), source })
}

fn matches(path: &Path, include: &GlobSet, exclude: &GlobSet) -> bool {
    if exclude.is_match(path) {
        return false;
    }
    include.is_match(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, "").unwrap();
    }

    #[test]
    fn discovers_test_files_in_directory() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join("tests/test_one.py"));
        touch(&tmp.path().join("tests/test_two.py"));
        touch(&tmp.path().join("src/app.py"));

        let cfg = Config::default();
        let files = discover(&[tmp.path()], &cfg).unwrap();
        let names: Vec<_> =
            files.iter().filter_map(|p| p.file_name().and_then(|s| s.to_str())).collect();
        assert!(names.contains(&"test_one.py"));
        assert!(names.contains(&"test_two.py"));
        assert!(!names.contains(&"app.py"));
    }

    #[test]
    fn exclude_filters_fixtures_directory() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join("tests/test_real.py"));
        touch(&tmp.path().join("tests/fixtures/test_fixture.py"));

        let cfg = Config::default();
        let files = discover(&[tmp.path()], &cfg).unwrap();
        let names: Vec<_> =
            files.iter().filter_map(|p| p.file_name().and_then(|s| s.to_str())).collect();
        assert!(names.contains(&"test_real.py"));
        assert!(!names.contains(&"test_fixture.py"));
    }

    #[test]
    fn file_argument_is_kept_when_pattern_matches() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("test_direct.py");
        touch(&file);

        let cfg = Config::default();
        let files = discover(&[&file], &cfg).unwrap();
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn explicit_file_argument_bypasses_include_globs() {
        // When the user names a file on the command line, we include it
        // even if its name doesn't match the default `test_*.py` globs.
        // Matches the behavior of ruff / rg / other path-explicit tools.
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("not_a_test.py");
        touch(&file);

        let cfg = Config::default();
        let files = discover(&[&file], &cfg).unwrap();
        assert_eq!(files, vec![file]);
    }

    #[test]
    fn explicit_file_argument_still_honors_exclude() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("fixtures").join("test_excluded.py");
        touch(&file);

        let cfg = Config::default();
        let files = discover(&[&file], &cfg).unwrap();
        assert!(files.is_empty(), "exclude must still apply to explicit file paths");
    }

    #[test]
    fn non_existent_path_is_skipped_silently() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("no_such_dir");
        let cfg = Config::default();
        let files = discover(&[&missing], &cfg).unwrap();
        assert!(files.is_empty());
    }
}
