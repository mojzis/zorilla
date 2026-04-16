//! Suppression state.
//!
//! # Stub
//!
//! Phase 2 only wires the *type* of [`Suppressions`] through the
//! [`crate::rules::Context`]. The actual comment parser — which reads
//! `# zorilla: ignore`, `# zorilla: ignore[ZR001,…]`, and
//! `# zorilla: ignore-file` — lands in PLAN.md Step 7 and is out of scope
//! here. Everything on [`Suppressions`] is a no-op placeholder.

/// Opaque collection of suppression annotations for one file.
///
/// Rules call [`Suppressions::is_suppressed`] before emitting findings.
/// In Phase 2 the answer is always `false`.
#[derive(Debug, Default, Clone, Copy)]
pub struct Suppressions;

impl Suppressions {
    /// Build an empty set of suppressions.
    #[must_use]
    pub fn empty() -> Self {
        Self
    }

    /// Would a finding on `_line` with rule code `_code` be suppressed?
    ///
    /// Always `false` until PLAN.md Step 7 lands. The `&self` signature
    /// is intentional — PLAN.md Step 7 will store parsed comment state on
    /// `Suppressions`, and keeping callers on the method form means no
    /// churn at the call site when that happens.
    #[must_use]
    #[allow(
        clippy::unused_self,
        clippy::trivially_copy_pass_by_ref,
        reason = "signature is fixed by PLAN.md Step 7; stub body here"
    )]
    pub fn is_suppressed(&self, _line: usize, _code: &str) -> bool {
        false
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
    }
}
