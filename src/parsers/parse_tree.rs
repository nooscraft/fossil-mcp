//! Zero-copy ParseTree implementation.
//!
//! Keeps `tree_sitter::Tree` alive and provides byte-range access
//! into the original source string. No HashMap copy — ~3x memory reduction.

use crate::core::traits::{ParseTree, TreeNode};
use crate::core::{Language, SourceSpan};
use streaming_iterator::StreamingIterator;

/// Zero-copy parse tree wrapping a `tree_sitter::Tree` and source text.
pub struct ZeroCopyParseTree {
    tree: tree_sitter::Tree,
    source: String,
    language: Language,
}

impl ZeroCopyParseTree {
    pub fn new(tree: tree_sitter::Tree, source: String, language: Language) -> Self {
        Self {
            tree,
            source,
            language,
        }
    }

    /// Access the underlying tree-sitter tree.
    pub fn ts_tree(&self) -> &tree_sitter::Tree {
        &self.tree
    }

    /// Access the original source code.
    pub fn source_code(&self) -> &str {
        &self.source
    }

    /// Access the language.
    pub fn language(&self) -> Language {
        self.language
    }

    /// Execute a tree-sitter query and return matches.
    pub fn query_matches(&self, query_source: &str) -> Result<Vec<QueryMatch>, crate::core::Error> {
        let ts_lang = self.tree.language();
        let query = tree_sitter::Query::new(&ts_lang, query_source)
            .map_err(|e| crate::core::Error::parse(format!("Invalid query: {e}")))?;

        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.matches(&query, self.tree.root_node(), self.source.as_bytes());

        let mut results = Vec::new();
        while let Some(m) = matches.next() {
            let mut captures = Vec::new();
            for cap in m.captures {
                let node = cap.node;
                let text = &self.source[node.byte_range()];
                let name = query.capture_names()[cap.index as usize].to_string();
                captures.push(QueryCapture {
                    name,
                    text: text.to_string(),
                    start_byte: node.start_byte(),
                    end_byte: node.end_byte(),
                    start_line: node.start_position().row,
                    end_line: node.end_position().row,
                });
            }
            results.push(QueryMatch {
                pattern_index: m.pattern_index,
                captures,
            });
        }
        Ok(results)
    }

    /// Get text for a byte range.
    pub fn text_for_range(&self, start: usize, end: usize) -> &str {
        &self.source[start..end]
    }

    /// Get a SourceSpan for a tree-sitter node.
    pub fn node_span(&self, node: &tree_sitter::Node) -> SourceSpan {
        SourceSpan::new(node.start_byte(), node.end_byte())
    }
}

impl std::fmt::Debug for ZeroCopyParseTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZeroCopyParseTree")
            .field("language", &self.language)
            .field("source_len", &self.source.len())
            .finish()
    }
}

impl ParseTree for ZeroCopyParseTree {
    fn language(&self) -> Language {
        self.language
    }

    fn root(&self) -> Box<dyn TreeNode + '_> {
        Box::new(ZeroCopyTreeNode {
            node: self.tree.root_node(),
            source: &self.source,
        })
    }

    fn debug_string(&self) -> String {
        self.tree.root_node().to_sexp()
    }

    fn source(&self) -> &str {
        &self.source
    }
}

/// A tree node that borrows from the source string (zero-copy).
pub struct ZeroCopyTreeNode<'a> {
    node: tree_sitter::Node<'a>,
    source: &'a str,
}

impl<'a> TreeNode for ZeroCopyTreeNode<'a> {
    fn node_type(&self) -> &str {
        self.node.kind()
    }

    fn children(&self) -> Vec<Box<dyn TreeNode + '_>> {
        let mut cursor = self.node.walk();
        self.node
            .children(&mut cursor)
            .map(|child| {
                Box::new(ZeroCopyTreeNode {
                    node: child,
                    source: self.source,
                }) as Box<dyn TreeNode + '_>
            })
            .collect()
    }

    fn text(&self) -> &str {
        &self.source[self.node.byte_range()]
    }

    fn start_byte(&self) -> usize {
        self.node.start_byte()
    }

    fn end_byte(&self) -> usize {
        self.node.end_byte()
    }

    fn start_line(&self) -> usize {
        self.node.start_position().row
    }

    fn start_column(&self) -> usize {
        self.node.start_position().column
    }

    fn end_line(&self) -> usize {
        self.node.end_position().row
    }

    fn end_column(&self) -> usize {
        self.node.end_position().column
    }
}

/// A match from a tree-sitter query execution.
#[derive(Debug, Clone)]
pub struct QueryMatch {
    pub pattern_index: usize,
    pub captures: Vec<QueryCapture>,
}

/// A single capture within a query match.
#[derive(Debug, Clone)]
pub struct QueryCapture {
    pub name: String,
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
}
