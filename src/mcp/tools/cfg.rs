//! MCP tool: `fossil_get_cfg`
//!
//! Returns CFG blocks and edges for a named function.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::mcp::context::AnalysisContext;

/// Get the control flow graph for a function.
///
/// # Arguments
/// - `function_name` (string, required): Name of the function to look up.
pub fn execute(args: &HashMap<String, Value>, context: &AnalysisContext) -> Result<Value, String> {
    let function_name = args
        .get("function_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing required argument 'function_name'")?;

    // Look up the CFG by function name. Try both the plain name and fully
    // qualified name.
    let cfg = context
        .cfgs
        .get(function_name)
        .or_else(|| {
            // Try to find by matching any CFG whose function_name ends with the
            // requested name (handles cases where the key is a full_name).
            context.cfgs.values().find(|c| {
                c.function_name == function_name
                    || c.function_name.ends_with(&format!("::{}", function_name))
                    || c.function_name.ends_with(&format!(".{}", function_name))
            })
        })
        .ok_or_else(|| {
            let available: Vec<&String> = context.cfgs.keys().take(20).collect();
            format!(
                "No CFG found for function '{}'. Available functions (first 20): {:?}",
                function_name, available
            )
        })?;

    // Serialize blocks.
    let blocks: Vec<Value> = cfg
        .blocks()
        .map(|(id, block)| {
            json!({
                "id": id.as_u32(),
                "label": block.label,
                "is_entry": block.is_entry,
                "is_exit": block.is_exit,
                "statements_count": block.statements.len(),
            })
        })
        .collect();

    // Serialize edges.
    let edges: Vec<Value> = cfg
        .edges()
        .iter()
        .map(|edge| {
            json!({
                "from": edge.from.as_u32(),
                "to": edge.to.as_u32(),
                "kind": format!("{:?}", edge.kind),
            })
        })
        .collect();

    let result = json!({
        "function": function_name,
        "block_count": blocks.len(),
        "edge_count": edges.len(),
        "entry": cfg.entry().map(|id| id.as_u32()),
        "exit": cfg.exit().map(|id| id.as_u32()),
        "blocks": blocks,
        "edges": edges,
    });

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
        }]
    }))
}
