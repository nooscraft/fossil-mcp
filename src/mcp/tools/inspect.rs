//! MCP tool: `fossil_inspect`
//!
//! Unified function inspection tool with a `mode` parameter that dispatches
//! to call_graph, data_flow, or cfg analysis.

use std::collections::HashMap;

use serde_json::Value;

use super::{blast_radius, call_graph, cfg, data_flow};
use crate::mcp::context::AnalysisContext;

/// Dispatch to the appropriate inspection mode.
///
/// # Arguments
/// - `mode` (string, required): One of `"call_graph"`, `"data_flow"`, `"cfg"`,
///   or `"blast_radius"`.
/// - All other arguments are forwarded to the underlying tool.
pub fn execute(args: &HashMap<String, Value>, context: &AnalysisContext) -> Result<Value, String> {
    let mode = args
        .get("mode")
        .and_then(|v| v.as_str())
        .ok_or("Missing 'mode' argument (call_graph, data_flow, cfg, blast_radius)")?;

    match mode {
        "call_graph" => call_graph::execute(args, context),
        "data_flow" => data_flow::execute(args, context),
        "cfg" => cfg::execute(args, context),
        "blast_radius" => blast_radius::execute(args, context),
        _ => Err(format!(
            "Unknown mode '{}'. Use: call_graph, data_flow, cfg, blast_radius",
            mode
        )),
    }
}
