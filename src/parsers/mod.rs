//! # fossil_parsers
//!
//! Multi-language tree-sitter parsers with zero-copy `ParseTree` for Fossil.
//!
//! Features:
//! - **Zero-copy `ParseTree`** — keeps `tree_sitter::Tree` alive, byte-range access into source
//! - **`define_parser!` macro** — generates boilerplate for all 17 language parsers
//! - **Real `query_matches()`** — `QueryCursor` execution on tree-sitter queries
//! - **Incremental parsing** — `parse(code, Some(&old_tree))`
//! - **`ParserRegistry`** — dynamic dispatch to the right parser by language/extension

mod extractor;
mod parse_tree;
mod parser_macro;
#[allow(clippy::module_inception)]
mod parsers;
mod registry;

pub use extractor::{
    extract_attributes, extract_calls, extract_class_hierarchy, extract_functions,
    extract_impl_blocks, extract_imports, extract_symbol_refs,
};
pub use parse_tree::ZeroCopyParseTree;
pub use registry::ParserRegistry;
