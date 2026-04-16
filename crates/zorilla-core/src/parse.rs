//! Thin wrapper around `tree-sitter-python`.
//!
//! Exposes [`parse`], which turns a source string into a
//! [`tree_sitter::Tree`]. Parser construction is cheap, so we create a
//! fresh parser per call — this keeps the API trivially `Send`/`Sync`
//! friendly for rayon-driven callers.

use thiserror::Error;

/// Error returned when parsing a Python source string fails.
#[derive(Debug, Error)]
pub enum ParseError {
    /// Setting the tree-sitter-python grammar on the parser failed.
    #[error("configuring tree-sitter language: {0}")]
    Language(String),
    /// Tree-sitter returned `None` from `parse`, which is rare and usually
    /// indicates an aborted parse or an impossibly long input.
    #[error("tree-sitter returned no tree for input")]
    NoTree,
}

/// Parse `source` as Python and return the resulting syntax tree.
///
/// Tree-sitter grammars never panic on malformed input; the returned
/// [`tree_sitter::Tree`] will simply contain `ERROR` nodes. Callers that
/// care about parse errors should inspect
/// [`tree_sitter::Tree::root_node`] for `has_error`.
pub fn parse(source: &str) -> Result<tree_sitter::Tree, ParseError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .map_err(|e| ParseError::Language(e.to_string()))?;
    parser.parse(source, None).ok_or(ParseError::NoTree)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_trivial_function() {
        let tree = parse("def test_foo():\n    pass\n").unwrap();
        let root = tree.root_node();
        assert_eq!(root.kind(), "module");
        assert!(!root.has_error());
    }

    #[test]
    fn empty_source_is_ok() {
        let tree = parse("").unwrap();
        assert_eq!(tree.root_node().kind(), "module");
    }

    #[test]
    fn malformed_source_still_returns_tree_with_error_node() {
        let tree = parse("def :").unwrap();
        assert!(tree.root_node().has_error());
    }
}
