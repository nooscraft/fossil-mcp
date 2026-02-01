//! Core traits for the Fossil toolkit.

use super::error::Result;
use super::types::{Finding, Language, ParsedFile};
use std::path::Path;

/// Abstract syntax tree representation (language-agnostic).
pub trait ParseTree: Send + Sync {
    /// Language of the parsed source.
    fn language(&self) -> Language;

    /// Root node of the tree.
    fn root(&self) -> Box<dyn TreeNode + '_>;

    /// Debug string representation of the tree.
    fn debug_string(&self) -> String;

    /// The original source code.
    fn source(&self) -> &str;
}

/// A node in a parsed syntax tree.
pub trait TreeNode: Send + Sync {
    /// Node type string (e.g., "function_definition").
    fn node_type(&self) -> &str;

    /// Child nodes.
    fn children(&self) -> Vec<Box<dyn TreeNode + '_>>;

    /// Source text of this node.
    fn text(&self) -> &str;

    fn start_byte(&self) -> usize;
    fn end_byte(&self) -> usize;
    fn start_line(&self) -> usize;
    fn start_column(&self) -> usize;
    fn end_line(&self) -> usize;
    fn end_column(&self) -> usize;
}

/// Trait for parsing source code into a `ParseTree`.
pub trait LanguageParser: Send + Sync {
    /// Language this parser handles.
    fn language(&self) -> Language;

    /// Supported file extensions (without dots).
    fn extensions(&self) -> &[&str];

    /// Parse source code.
    fn parse(&self, source: &str) -> Result<Box<dyn ParseTree>>;

    /// Parse with file path context.
    fn parse_file(&self, file_path: &str, source: &str) -> Result<ParsedFile>;
}

/// Compiled pattern for matching against code.
pub trait CompiledPattern: Send + Sync {
    fn find_matches(
        &self,
        tree: &dyn ParseTree,
        file_path: &Path,
        code: &str,
    ) -> Result<Vec<PatternMatch>>;

    fn rule_id(&self) -> &str;
    fn language(&self) -> Language;
}

/// A pattern matcher that compiles rules into executable patterns.
pub trait PatternMatcher: Send + Sync {
    fn compile_pattern(&self, pattern: &str) -> Result<Box<dyn CompiledPattern>>;
    fn language(&self) -> Language;
}

/// A location where a pattern matched in source code.
#[derive(Debug, Clone)]
pub struct PatternMatch {
    pub rule_id: String,
    pub start_line: usize,
    pub start_column: usize,
    pub end_line: usize,
    pub end_column: usize,
    pub matched_text: String,
    pub bindings: Vec<(String, String)>,
}

/// Report findings in a specific format.
pub trait Reporter: Send + Sync {
    fn report(&self, findings: &[Finding]) -> Result<String>;
    fn format_name(&self) -> &str;
}

/// Report progress during analysis.
pub trait ProgressReporter: Send + Sync {
    fn update(&self, current: u64, total: u64, message: &str);
    fn finish(&self, summary: &str);
    fn warn(&self, message: &str);
}
