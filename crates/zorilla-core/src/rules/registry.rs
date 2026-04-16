//! Registry of compiled-in rules.
//!
//! The list is `&'static [&'static dyn Rule]` — a slice of references to
//! static rule instances. Callers iterate this slice and call
//! [`crate::rules::Rule::check`] for every enabled rule.

use super::zr001_conditional::ZR001_CONDITIONAL;
use super::zr002_sleep::ZR002_SLEEP;
use super::zr007_empty_test::ZR007_EMPTY_TEST;
use super::Rule;

static ALL_RULES: &[&dyn Rule] = &[&ZR001_CONDITIONAL, &ZR002_SLEEP, &ZR007_EMPTY_TEST];

/// Return every rule known to the engine, in ZR-code order.
#[must_use]
pub fn all() -> &'static [&'static dyn Rule] {
    ALL_RULES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_contains_all_rules_in_code_order() {
        let codes: Vec<_> = all().iter().map(|r| r.code()).collect();
        assert_eq!(codes, vec!["ZR001", "ZR002", "ZR007"]);
    }
}
