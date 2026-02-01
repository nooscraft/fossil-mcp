//! MCP tool: `fossil_trace`
//!
//! Finds call paths between two functions in the code graph.
//! Uses BFS to discover the shortest path from function A to function B
//! (forward direction: A calls ... calls B) and optionally the reverse.

use std::collections::{HashMap, VecDeque};

use petgraph::graph::NodeIndex;
use serde_json::{json, Value};

use crate::mcp::context::AnalysisContext;

/// Find call paths between two functions.
///
/// # Arguments
/// - `from_function` (string, required): Source function name.
/// - `to_function` (string, required): Target function name.
/// - `max_depth` (number, optional): Maximum path length (default 10).
/// - `max_paths` (number, optional): Maximum paths to return (default 3).
pub fn execute(args: &HashMap<String, Value>, context: &AnalysisContext) -> Result<Value, String> {
    let from_name = args
        .get("from_function")
        .and_then(|v| v.as_str())
        .ok_or("Missing required argument 'from_function'")?;

    let to_name = args
        .get("to_function")
        .and_then(|v| v.as_str())
        .ok_or("Missing required argument 'to_function'")?;

    let max_depth = args.get("max_depth").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

    let max_paths = args.get("max_paths").and_then(|v| v.as_u64()).unwrap_or(3) as usize;

    let graph = &context.graph;

    let from_idx = graph
        .find_node_by_name(from_name)
        .ok_or_else(|| format!("Function '{}' not found in the code graph", from_name))?;

    let to_idx = graph
        .find_node_by_name(to_name)
        .ok_or_else(|| format!("Function '{}' not found in the code graph", to_name))?;

    let from_node = graph.get_node(from_idx).unwrap();
    let to_node = graph.get_node(to_idx).unwrap();

    // Forward: from → ... → to (following call edges, same language only).
    let forward_paths = find_paths(from_idx, to_idx, max_depth, max_paths, |idx| {
        graph.calls_from_compatible(idx)
    });

    // Reverse: to → ... → from (following call edges, same language only).
    let reverse_paths = find_paths(to_idx, from_idx, max_depth, max_paths, |idx| {
        graph.calls_from_compatible(idx)
    });

    let format_path = |path: &[NodeIndex]| -> Value {
        let steps: Vec<Value> = path
            .iter()
            .map(|&idx| {
                if let Some(n) = graph.get_node(idx) {
                    json!({
                        "name": n.name,
                        "full_name": n.full_name,
                        "file": n.location.file,
                        "line": n.location.line_start,
                    })
                } else {
                    json!({ "index": idx.index() })
                }
            })
            .collect();

        // Human-readable chain: A → B → C → D
        let chain: String = path
            .iter()
            .filter_map(|&idx| graph.get_node(idx).map(|n| n.name.as_str()))
            .collect::<Vec<_>>()
            .join(" → ");

        json!({
            "hops": path.len() - 1,
            "chain": chain,
            "steps": steps,
        })
    };

    let forward_json: Vec<Value> = forward_paths.iter().map(|p| format_path(p)).collect();
    let reverse_json: Vec<Value> = reverse_paths.iter().map(|p| format_path(p)).collect();

    let connected = !forward_paths.is_empty() || !reverse_paths.is_empty();

    let result = json!({
        "from": {
            "name": from_node.name,
            "full_name": from_node.full_name,
            "file": from_node.location.file,
            "line": from_node.location.line_start,
        },
        "to": {
            "name": to_node.name,
            "full_name": to_node.full_name,
            "file": to_node.location.file,
            "line": to_node.location.line_start,
        },
        "connected": connected,
        "forward": {
            "found": !forward_paths.is_empty(),
            "count": forward_paths.len(),
            "paths": forward_json,
        },
        "reverse": {
            "found": !reverse_paths.is_empty(),
            "count": reverse_paths.len(),
            "paths": reverse_json,
        },
    });

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
        }]
    }))
}

/// BFS-based path finding. Returns up to `max_paths` shortest paths
/// from `start` to `target`, each at most `max_depth` hops.
fn find_paths<F>(
    start: NodeIndex,
    target: NodeIndex,
    max_depth: usize,
    max_paths: usize,
    neighbors: F,
) -> Vec<Vec<NodeIndex>>
where
    F: Fn(NodeIndex) -> Vec<NodeIndex>,
{
    if start == target {
        return vec![vec![start]];
    }

    let mut results = Vec::new();
    // BFS with path tracking: (current_node, path_so_far)
    let mut queue: VecDeque<(NodeIndex, Vec<NodeIndex>)> = VecDeque::new();
    queue.push_back((start, vec![start]));

    // Track visited to avoid cycles within a single path, but allow
    // different paths to share nodes. Use a global visited set with
    // depth tracking to prune — once we find the shortest path length,
    // only continue exploring at that same depth.
    let mut found_depth: Option<usize> = None;
    let mut visited_at_depth: HashMap<NodeIndex, usize> = HashMap::new();
    visited_at_depth.insert(start, 0);

    while let Some((current, path)) = queue.pop_front() {
        let depth = path.len() - 1;

        // If we already found paths and this path is longer, stop.
        if let Some(fd) = found_depth {
            if depth >= fd {
                continue;
            }
        }

        if depth >= max_depth {
            continue;
        }

        for next in neighbors(current) {
            if path.contains(&next) {
                // Skip cycles within this path.
                continue;
            }

            let next_depth = depth + 1;

            // Only visit if we haven't seen this node at a shorter depth.
            if let Some(&prev_depth) = visited_at_depth.get(&next) {
                if prev_depth < next_depth {
                    continue;
                }
            }
            visited_at_depth.insert(next, next_depth);

            let mut new_path = path.clone();
            new_path.push(next);

            if next == target {
                found_depth = Some(next_depth);
                results.push(new_path);
                if results.len() >= max_paths {
                    return results;
                }
            } else {
                queue.push_back((next, new_path));
            }
        }
    }

    results
}
