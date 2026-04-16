//! `zorilla-core` — library crate that powers the `zorilla` CLI.
//!
//! Phase 1 surface: configuration loading, file discovery, and a
//! [`Report`] type returned by [`lint`]. Rule execution and parsing
//! arrive in Phase 2.

pub mod config;
pub mod discovery;
pub mod report;

use std::path::Path;

pub use config::Config;
pub use discovery::{discover, DiscoveryError};
pub use report::{Finding, Report, Severity};

/// Error surfaced to callers of [`lint`].
#[derive(Debug, thiserror::Error)]
pub enum LintError {
    /// File discovery failed (e.g. glob compilation, walker IO).
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
}

/// Run the linter against `paths` using `config`.
///
/// In Phase 1 no rules are registered, so the returned [`Report`] always
/// has zero findings. The `files_discovered` count reflects the number of
/// Python files that matched `config.include` and survived
/// `config.exclude`.
pub fn lint<P: AsRef<Path>>(paths: &[P], config: &Config) -> Result<Report, LintError> {
    let files = discover(paths, config)?;
    Ok(Report { findings: Vec::new(), files_discovered: files.len() })
}
