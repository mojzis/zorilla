// TEMPORARY: remove in Phase 3 when ZR001 lands.
//
// ZR000 exists only to prove that the Phase 2 pipeline — discovery,
// parse, rule dispatch, text output, exit codes — is wired end to end.
// It fires a "hello world" finding on every test function it sees. The
// rule, the file, and its registration in `registry.rs` are deleted
// together with the Phase 3 ZR001 work.

//! `ZR000 hello-world` — temporary pipeline proof.

use crate::ast::iter_test_functions;
use crate::report::{Finding, Severity};
use crate::rules::{Context, Rule};

/// Canonical message used by ZR000. Re-exported so tests can assert
/// against a single source of truth.
pub const MESSAGE: &str = "hello world (test function detected)";

/// The ZR000 rule instance registered in [`crate::rules::registry`].
pub static ZR000_HELLO: HelloRule = HelloRule;

/// Unit struct implementing [`Rule`].
pub struct HelloRule;

impl Rule for HelloRule {
    fn code(&self) -> &'static str {
        "ZR000"
    }

    fn name(&self) -> &'static str {
        "hello-world"
    }

    fn check(&self, ctx: &Context<'_>, out: &mut Vec<Finding>) {
        for node in iter_test_functions(ctx.tree, ctx.source) {
            let start = node.start_position();
            out.push(Finding {
                code: self.code(),
                message: MESSAGE.to_string(),
                file: ctx.file.to_path_buf(),
                // tree-sitter points are 0-indexed; findings are 1-indexed.
                line: start.row + 1,
                column: start.column + 1,
                severity: Severity::Warning,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use crate::rules::RuleConfig;
    use crate::suppress::Suppressions;
    use std::path::Path;

    #[test]
    fn fires_once_per_test_function() {
        let src = "\
def test_a():
    pass

class TestX:
    def test_b(self):
        pass

def _helper():
    pass
";
        let tree = parse(src).unwrap();
        let suppressions = Suppressions::empty();
        let config = RuleConfig;
        let ctx = Context {
            file: Path::new("example.py"),
            source: src,
            tree: &tree,
            config: &config,
            suppressions: &suppressions,
        };
        let mut out = Vec::new();
        ZR000_HELLO.check(&ctx, &mut out);

        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|f| f.code == "ZR000"));
        assert!(out.iter().all(|f| f.message == MESSAGE));
        // Lines are 1-indexed.
        assert_eq!(out[0].line, 1);
        assert_eq!(out[0].column, 1);
    }

    #[test]
    fn does_not_fire_when_no_test_functions() {
        let src = "def _helper():\n    pass\n";
        let tree = parse(src).unwrap();
        let suppressions = Suppressions::empty();
        let config = RuleConfig;
        let ctx = Context {
            file: Path::new("x.py"),
            source: src,
            tree: &tree,
            config: &config,
            suppressions: &suppressions,
        };
        let mut out = Vec::new();
        ZR000_HELLO.check(&ctx, &mut out);
        assert!(out.is_empty());
    }
}
