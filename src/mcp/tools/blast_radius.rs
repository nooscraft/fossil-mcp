//! MCP tool mode: `blast_radius`
//!
//! Given a function, returns all downstream functions that would be affected
//! if it changes — i.e. all transitive callees (what it calls) and all
//! transitive callers (what calls it).

use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::graph::NodeIndex;
use serde_json::{json, Value};

use crate::mcp::context::AnalysisContext;

/// Compute the blast radius for a function.
///
/// # Arguments
/// - `function_name` (string, required): Name of the function to analyze.
/// - `depth` (number, optional): Maximum BFS depth (default 10).
/// - `direction` (string, optional): `"downstream"` (callees), `"upstream"` (callers),
///   or `"both"` (default).
pub fn execute(args: &HashMap<String, Value>, context: &AnalysisContext) -> Result<Value, String> {
    let function_name = args
        .get("function_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing required argument 'function_name'")?;

    let max_depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    let direction = args
        .get("direction")
        .and_then(|v| v.as_str())
        .unwrap_or("both");

    let graph = &context.graph;

    let node_idx = graph
        .find_node_by_name(function_name)
        .ok_or_else(|| format!("Function '{}' not found in the code graph", function_name))?;

    let node = graph
        .get_node(node_idx)
        .ok_or("Internal error: node not found")?;

    let include_downstream = direction == "downstream" || direction == "both";
    let include_upstream = direction == "upstream" || direction == "both";

    // Downstream: everything this function calls transitively (same language only).
    let downstream = if include_downstream {
        bfs_collect(node_idx, max_depth, |idx| graph.calls_from_compatible(idx))
    } else {
        Vec::new()
    };

    // Upstream: everything that calls this function transitively (same language only).
    let upstream = if include_upstream {
        bfs_collect(node_idx, max_depth, |idx| graph.callers_of_compatible(idx))
    } else {
        Vec::new()
    };

    let format_node = |idx: NodeIndex, depth: usize| -> Value {
        if let Some(n) = graph.get_node(idx) {
            json!({
                "name": n.name,
                "full_name": n.full_name,
                "file": n.location.file,
                "line": n.location.line_start,
                "depth": depth,
            })
        } else {
            json!({ "index": idx.index(), "depth": depth })
        }
    };

    let downstream_json: Vec<Value> = downstream
        .iter()
        .map(|&(idx, depth)| format_node(idx, depth))
        .collect();

    let upstream_json: Vec<Value> = upstream
        .iter()
        .map(|&(idx, depth)| format_node(idx, depth))
        .collect();

    // Unique affected files.
    let mut affected_files: HashSet<&str> = HashSet::new();
    for &(idx, _) in downstream.iter().chain(upstream.iter()) {
        if let Some(n) = graph.get_node(idx) {
            affected_files.insert(&n.location.file);
        }
    }

    let total = downstream.len() + upstream.len();

    let result = json!({
        "function": function_name,
        "full_name": node.full_name,
        "file": node.location.file,
        "line": node.location.line_start,
        "total_affected": total,
        "affected_files": affected_files.len(),
        "downstream": {
            "count": downstream.len(),
            "functions": downstream_json,
        },
        "upstream": {
            "count": upstream.len(),
            "functions": upstream_json,
        },
    });

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
        }]
    }))
}

/// BFS from a starting node, collecting (NodeIndex, depth) pairs.
fn bfs_collect<F>(start: NodeIndex, max_depth: usize, neighbors: F) -> Vec<(NodeIndex, usize)>
where
    F: Fn(NodeIndex) -> Vec<NodeIndex>,
{
    let mut visited = HashSet::new();
    visited.insert(start);
    let mut queue = VecDeque::new();
    let mut result = Vec::new();

    for next in neighbors(start) {
        if visited.insert(next) {
            queue.push_back((next, 1_usize));
        }
    }

    while let Some((current, depth)) = queue.pop_front() {
        result.push((current, depth));
        if depth < max_depth {
            for next in neighbors(current) {
                if visited.insert(next) {
                    queue.push_back((next, depth + 1));
                }
            }
        }
    }

    result
}
