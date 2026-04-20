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
pub mod zr002_sleep;
pub mod zr003_no_assertion;
pub mod zr004_assertion_roulette;
pub mod zr005_mystery_guest;
pub mod zr006_patch_stack;
pub mod zr007_empty_test;

pub use crate::config::RuleConfig;
pub use crate::report::Severity;

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

    /// Long-form markdown documentation for this rule — the body shown
    /// by `zorilla explain <code>`. Each rule embeds its own
    /// `docs/rules/ZR00N.md` via `include_str!` so the binary stays
    /// self-contained. No default — every rule must opt in so a new
    /// rule cannot ship without documentation.
    fn doc(&self) -> &'static str;

    /// Run the rule, pushing any findings onto `out`.
    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>);
}

#[cfg(test)]
mod doc_test {
    use super::registry::all;

    #[test]
    fn every_rule_has_nonempty_doc_starting_with_its_own_zr_heading() {
        // Tightened from "starts with `# ZR`" to "starts with `# <code>`"
        // so a copy-paste failure (e.g. `zr003_no_assertion.rs` pointing
        // its `include_str!` at `docs/rules/ZR002.md`) is caught here
        // rather than surfacing as a confusing `explain` output at
        // runtime.
        for rule in all() {
            let doc = rule.doc();
            assert!(!doc.is_empty(), "{} doc is empty", rule.code());
            let expected_prefix = format!("# {}", rule.code());
            assert!(
                doc.starts_with(&expected_prefix),
                "{} doc must start with `{}`, got: {:?}",
                rule.code(),
                expected_prefix,
                &doc[..doc.len().min(40)],
            );
        }
    }
}
