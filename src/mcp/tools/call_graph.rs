//! MCP tool: `fossil_get_call_graph`
//!
//! Returns callers, callees, and reachability info for a named function.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::{json, Value};

use crate::mcp::context::AnalysisContext;

/// Get call graph information for a function.
///
/// # Arguments
/// - `function_name` (string, required): Name of the function to query.
/// - `depth` (number, optional): Maximum BFS depth for transitive callees (default 2).
pub fn execute(args: &HashMap<String, Value>, context: &AnalysisContext) -> Result<Value, String> {
    let function_name = args
        .get("function_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing required argument 'function_name'")?;

    let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(2) as usize;

    let graph = &context.graph;

    // Find the function node(s) by name.
    let node_indices = graph.find_nodes_by_name(function_name);
    if node_indices.is_empty() {
        return Err(format!(
            "Function '{}' not found in the code graph",
            function_name
        ));
    }

    // Use the first matching node.
    let node_idx = node_indices[0];
    let node = graph
        .get_node(node_idx)
        .ok_or("Internal error: node index valid but node not found")?;

    // Direct callers (same language only).
    let callers: Vec<Value> = graph
        .callers_of_compatible(node_idx)
        .into_iter()
        .filter_map(|idx| graph.get_node(idx))
        .map(|n| {
            json!({
                "name": n.name,
                "full_name": n.full_name,
                "kind": n.kind.to_string(),
                "file": n.location.file,
                "line": n.location.line_start,
            })
        })
        .collect();

    // Direct callees (same language only).
    let callees: Vec<Value> = graph
        .calls_from_compatible(node_idx)
        .into_iter()
        .filter_map(|idx| graph.get_node(idx))
        .map(|n| {
            json!({
                "name": n.name,
                "full_name": n.full_name,
                "kind": n.kind.to_string(),
                "file": n.location.file,
                "line": n.location.line_start,
            })
        })
        .collect();

    // Transitive callees via BFS up to `depth` (same language only).
    let mut transitive_callees: Vec<Value> = Vec::new();
    {
        let mut visited = HashSet::new();
        visited.insert(node_idx);
        let mut queue = VecDeque::new();

        // Seed with direct callees at depth 1.
        for callee_idx in graph.calls_from_compatible(node_idx) {
            if visited.insert(callee_idx) {
                queue.push_back((callee_idx, 1_usize));
            }
        }

        while let Some((current, current_depth)) = queue.pop_front() {
            if let Some(n) = graph.get_node(current) {
                transitive_callees.push(json!({
                    "name": n.name,
                    "full_name": n.full_name,
                    "kind": n.kind.to_string(),
                    "file": n.location.file,
                    "line": n.location.line_start,
                    "depth": current_depth,
                }));
            }

            if current_depth < depth {
                for next_idx in graph.calls_from_compatible(current) {
                    if visited.insert(next_idx) {
                        queue.push_back((next_idx, current_depth + 1));
                    }
                }
            }
        }
    }

    // Reachability: is this function reachable from any entry point?
    let all_entry_points = graph.entry_points();
    let reachable_set = graph.compute_reachable(all_entry_points);
    let is_reachable = reachable_set.contains(&node_idx);

    let result = json!({
        "function": function_name,
        "full_name": node.full_name,
        "kind": node.kind.to_string(),
        "file": node.location.file,
        "line": node.location.line_start,
        "callers": callers,
        "callees": callees,
        "transitive_callees": transitive_callees,
        "is_reachable": is_reachable,
    });

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
        }]
    }))
}
