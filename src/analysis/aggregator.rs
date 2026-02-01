//! Aggregator: merges per-file parsed data into a unified project graph.
//!
//! Two-phase aggregation:
//! 1. Add all nodes from all files
//! 2. Resolve cross-file edges by matching unresolved calls to known symbols

use std::collections::HashMap;

use crate::core::{CallEdge, EdgeConfidence, NodeId, ParsedFile};
use crate::graph::CodeGraph;

/// Aggregates multiple parsed files into a single project-level CodeGraph.
pub struct Aggregator;

impl Aggregator {
    /// Merge multiple parsed files into a single graph.
    ///
    /// 1. Add all nodes from all files.
    /// 2. Add intra-file edges.
    /// 3. Resolve cross-file edges using unresolved calls.
    pub fn aggregate(parsed_files: &[ParsedFile]) -> CodeGraph {
        let mut graph = CodeGraph::new();

        // Global name → NodeId index for cross-file resolution
        let mut global_name_index: HashMap<String, NodeId> = HashMap::new();

        // Add all nodes
        for pf in parsed_files {
            for node in &pf.nodes {
                graph.add_node(node.clone());
                global_name_index.insert(node.name.clone(), node.id);
                if node.full_name != node.name {
                    global_name_index.insert(node.full_name.clone(), node.id);
                }
            }
        }

        // Add intra-file edges
        for pf in parsed_files {
            for edge in &pf.edges {
                let _ = graph.add_edge(edge.clone());
            }

            // Mark entry points
            for &ep_id in &pf.entry_points {
                if let Some(idx) = graph.get_index(ep_id) {
                    graph.add_entry_point(idx);
                }
            }
        }

        // Resolve cross-file calls
        for pf in parsed_files {
            for unresolved in &pf.unresolved_calls {
                let callee_name = unresolved
                    .imported_as
                    .as_deref()
                    .unwrap_or(&unresolved.callee_name);

                if let Some(&callee_id) = global_name_index.get(callee_name) {
                    // Verify caller is in graph too
                    if graph.get_index(unresolved.caller_id).is_some()
                        && graph.get_index(callee_id).is_some()
                    {
                        let edge = CallEdge::new(
                            unresolved.caller_id,
                            callee_id,
                            EdgeConfidence::HighLikely,
                        );
                        let _ = graph.add_edge(edge);
                    }
                }
            }
        }

        graph
    }

    /// Get statistics about the aggregation.
    pub fn stats(graph: &CodeGraph) -> AggregationStats {
        let total_nodes = graph.node_count();
        let total_edges = graph.edge_count();
        let entry_points = graph.entry_points().len();
        let test_entry_points = graph.test_entry_points().len();

        let reachable = graph.compute_production_reachable();
        let unreachable_count = total_nodes - reachable.len();

        AggregationStats {
            total_nodes,
            total_edges,
            entry_points,
            test_entry_points,
            reachable_nodes: reachable.len(),
            unreachable_nodes: unreachable_count,
        }
    }
}

/// Statistics from graph aggregation.
#[derive(Debug, Clone)]
pub struct AggregationStats {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub entry_points: usize,
    pub test_entry_points: usize,
    pub reachable_nodes: usize,
    pub unreachable_nodes: usize,
}

impl std::fmt::Display for AggregationStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Nodes: {} ({} reachable, {} unreachable), Edges: {}, Entry: {}, Test: {}",
            self.total_nodes,
            self.reachable_nodes,
            self.unreachable_nodes,
            self.total_edges,
            self.entry_points,
            self.test_entry_points,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CodeNode, Language, NodeKind, SourceLocation, UnresolvedCall, Visibility};

    fn make_parsed_file(name: &str, nodes: Vec<CodeNode>, edges: Vec<CallEdge>) -> ParsedFile {
        let entry_points = nodes.iter().map(|n| n.id).collect();
        ParsedFile {
            path: format!("{name}.py"),
            language: Language::Python,
            source: String::new(),
            nodes,
            edges,
            entry_points,
            unresolved_calls: Vec::new(),
            class_relations: Vec::new(),
            parse_duration_ms: 0,
        }
    }

    #[test]
    fn test_aggregate_two_files() {
        let loc = SourceLocation::new("a.py".to_string(), 1, 5, 0, 0);
        let node_a = CodeNode::new(
            "a".to_string(),
            NodeKind::Function,
            loc.clone(),
            Language::Python,
            Visibility::Public,
        );
        let loc_b = SourceLocation::new("b.py".to_string(), 1, 5, 0, 0);
        let node_b = CodeNode::new(
            "b".to_string(),
            NodeKind::Function,
            loc_b,
            Language::Python,
            Visibility::Public,
        );

        let file_a = make_parsed_file("a", vec![node_a], vec![]);
        let file_b = make_parsed_file("b", vec![node_b], vec![]);

        let graph = Aggregator::aggregate(&[file_a, file_b]);
        assert_eq!(graph.node_count(), 2);
    }

    #[test]
    fn test_aggregate_with_cross_file_calls() {
        let loc_a = SourceLocation::new("a.py".to_string(), 1, 5, 0, 0);
        let caller = CodeNode::new(
            "caller".to_string(),
            NodeKind::Function,
            loc_a,
            Language::Python,
            Visibility::Public,
        );
        let caller_id = caller.id;

        let loc_b = SourceLocation::new("b.py".to_string(), 1, 5, 0, 0);
        let callee = CodeNode::new(
            "callee".to_string(),
            NodeKind::Function,
            loc_b,
            Language::Python,
            Visibility::Public,
        );

        let mut file_a = make_parsed_file("a", vec![caller], vec![]);
        file_a
            .unresolved_calls
            .push(UnresolvedCall::new(caller_id, "callee".to_string(), 2));

        let file_b = make_parsed_file("b", vec![callee], vec![]);

        let graph = Aggregator::aggregate(&[file_a, file_b]);
        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);
    }

    #[test]
    fn test_stats() {
        let loc = SourceLocation::new("test.py".to_string(), 1, 5, 0, 0);
        let node = CodeNode::new(
            "main".to_string(),
            NodeKind::Function,
            loc,
            Language::Python,
            Visibility::Public,
        );
        let file = make_parsed_file("test", vec![node], vec![]);

        let graph = Aggregator::aggregate(&[file]);
        let stats = Aggregator::stats(&graph);

        assert_eq!(stats.total_nodes, 1);
        assert_eq!(stats.entry_points, 1);
    }
}
