#![forbid(unsafe_code)]
//! Fossil — multi-language static analysis toolkit.
//!
//! Detects dead code, code clones, and AI scaffolding artifacts.

pub mod analysis;
pub mod ast;
pub mod cli;
pub mod clones;
pub mod config;
pub mod core;
pub mod dead_code;
pub mod graph;
pub mod lsp;
pub mod mcp;
pub mod output;
pub mod parsers;
pub mod rules;
pub mod update;
