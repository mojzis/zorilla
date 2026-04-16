//! Rule framework.
//!
//! A rule is a stateless check that inspects a parsed Python file via a
//! [`Context`] and pushes zero or more [`Finding`]s onto a caller-owned
//! `Vec`. The engine in [`crate::lint`] parses once per file and then
//! invokes every enabled rule with the shared [`Context`].

use std::path::Path;

use crate::report::Finding;
use crate::suppress::Suppressions;

pub mod registry;
pub mod zr001_conditional;

pub use crate::report::Severity;

/// Per-rule configuration carried via [`Context`].
///
/// Phase 2 stub: rules don't read any knobs yet. PLAN.md Step 4 onward
/// introduces `[tool.zorilla.rules.ZR***]` tables that materialize here.
#[derive(Debug, Default, Clone, Copy)]
pub struct RuleConfig;

/// Everything a rule needs to inspect one file.
///
/// Borrows are all `'a` — the engine constructs one `Context` per file
/// and threads it through every enabled rule, so no rule ever extends
/// the lifetime of the parsed [`tree_sitter::Tree`].
pub struct Context<'a> {
    pub file: &'a Path,
    pub source: &'a str,
    pub tree: &'a tree_sitter::Tree,
    pub config: &'a RuleConfig,
    pub suppressions: &'a Suppressions,
}

/// A single rule.
///
/// Implementations must be `Sync` so the engine can fan out over files
/// in parallel while holding the registry's `&'static dyn Rule`.
pub trait Rule: Sync {
    /// Short machine-readable identifier, e.g. `"ZR001"`.
    fn code(&self) -> &'static str;

    /// Human-readable kebab-case name, e.g. `"conditional-test-logic"`.
    fn name(&self) -> &'static str;

    /// Whether the rule is on when the user writes no per-rule config.
    fn default_enabled(&self) -> bool {
        true
    }

    /// Run the rule, pushing any findings onto `out`.
    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>);
}
