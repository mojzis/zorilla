//! Registry of compiled-in rules.
//!
//! The list is `&'static [&'static dyn Rule]` — a slice of references to
//! static rule instances. Callers iterate this slice and call
//! [`crate::rules::Rule::check`] for every enabled rule.

use super::zr000_hello::ZR000_HELLO;
use super::Rule;

// TEMPORARY: ZR000 is a Phase 2 pipeline proof and is removed in
// Phase 3 when ZR001 lands — see `zr000_hello.rs`.
static ALL_RULES: &[&dyn Rule] = &[&ZR000_HELLO];

/// Return every rule known to the engine, in ZR-code order.
#[must_use]
pub fn all() -> &'static [&'static dyn Rule] {
    ALL_RULES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_zr000_in_phase_two() {
        let codes: Vec<_> = all().iter().map(|r| r.code()).collect();
        assert_eq!(codes, vec!["ZR000"]);
    }
}
