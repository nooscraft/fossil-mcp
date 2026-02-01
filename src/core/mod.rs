//! # fossil_core
//!
//! Core types, traits, and errors for the Fossil code analysis toolkit.
//!
//! This crate provides the unified foundation used by all analysis crates:
//! - **Language** — 17 programming languages with extension detection
//! - **CodeNode / CallEdge** — call graph primitives
//! - **Severity / Confidence** — finding classification
//! - **Finding / Rule** — unified finding and rule types
//! - **Core traits** — `LanguageParser`, `ParseTree`, `TreeNode`, `Reporter`
//! - **text_utils** — `LineOffsetTable` for O(log n) byte→line conversion

pub mod error;
pub mod scoring;
pub mod text_utils;
pub mod traits;
pub mod types;

pub use error::{Error, Result};
pub use scoring::{
    adjust_dead_code_confidence, adjust_security_confidence, clone_confidence, compute_priority,
    PatternSignals, PriorityScore, StructuralSignals,
};
pub use text_utils::LineOffsetTable;
pub use traits::{
    CompiledPattern, LanguageParser, ParseTree, PatternMatch, PatternMatcher, ProgressReporter,
    Reporter, TreeNode,
};
pub use types::{
    CallEdge,
    ClassRelation,
    CodeNode,
    Confidence,
    EdgeConfidence,
    // Finding types
    Finding,
    FossilType,
    FrameworkPattern,
    // Language & enums
    Language,
    // Graph types
    NodeId,
    NodeKind,
    // Parsed structures
    ParsedFile,
    ParsedStructure,
    ParserConfig,
    PatternType,
    RemovalImpact,
    Rule,
    Severity,
    // Location types
    SourceLocation,
    SourceSpan,
    UnresolvedCall,
    Visibility,
};
