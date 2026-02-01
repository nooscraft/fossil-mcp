//! MCP tool modules for the Fossil code analysis server.
//!
//! Each module exposes an `execute` function that implements a single MCP tool.

pub mod blast_radius;
pub mod call_graph;
pub mod cfg;
pub mod data_flow;
pub mod explain_finding;
pub mod inspect;
pub mod scaffolding;
pub mod trace;
