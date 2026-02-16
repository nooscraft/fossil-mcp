//! Graph statistics using sketch-based data structures (UltraLogLog, etc.)
//!
//! Provides memory-efficient cardinality estimates and statistics for code graphs
//! without retaining all data in memory.

use crate::graph::CodeGraph;
use sketch_oxide::UltraLogLog;
use std::collections::HashMap;

/// Statistics about a code graph using sketch-based algorithms.
#[derive(Debug, Clone)]
pub struct CodeGraphStats {
    /// Approximate number of unique functions
    pub approx_functions: u32,
    /// Approximate number of unique callers
    pub approx_callers: u32,
    /// Approximate number of unique callees
    pub approx_callees: u32,
    /// Approximate number of distinct call edges
    pub approx_call_edges: u32,
    /// Number of source files analyzed
    pub file_count: usize,
    /// Estimated average functions per file
    pub avg_functions_per_file: f64,
    /// Estimated average calls per function
    pub avg_calls_per_function: f64,
    /// Estimated call graph density (edges / nodes²)
    pub graph_density: f64,
}

impl CodeGraphStats {
    /// Compute statistics for the given code graph using UltraLogLog sketches.
    ///
    /// This uses sketch-based algorithms (UltraLogLog) to estimate cardinalities
    /// efficiently without retaining all data in memory.
    pub fn compute(graph: &CodeGraph) -> Self {
        use xxhash_rust::xxh3::xxh3_64;

        // UltraLogLog with 12 bits (±2% error rate)
        let mut ull_functions =
            UltraLogLog::new(12).unwrap_or_else(|_| UltraLogLog::new(10).unwrap());
        let mut ull_callers =
            UltraLogLog::new(12).unwrap_or_else(|_| UltraLogLog::new(10).unwrap());
        let mut ull_callees =
            UltraLogLog::new(12).unwrap_or_else(|_| UltraLogLog::new(10).unwrap());
        let mut ull_edges = UltraLogLog::new(12).unwrap_or_else(|_| UltraLogLog::new(10).unwrap());

        let mut file_paths = HashMap::new();

        // Process all nodes
        for (node_idx, node) in graph.nodes() {
            // Track unique functions
            let func_id = format!("{}:{}", node.location.file, node.name);
            let func_id_hash = xxh3_64(func_id.as_bytes());
            ull_functions.add(&func_id_hash);

            // Track files
            file_paths
                .entry(node.location.file.clone())
                .or_insert_with(Vec::new)
                .push(node_idx);

            // Track callers: add once per node that has at least one outgoing call
            if graph.calls_from(node_idx).next().is_some() {
                let caller_hash = xxh3_64(format!("{:?}", node_idx).as_bytes());
                ull_callers.add(&caller_hash);
            }
        }

        // Process all edges to track callees and edge count
        for (src_idx, tgt_idx, _edge) in graph.edges_with_endpoints() {
            let callee_hash = xxh3_64(format!("{:?}", tgt_idx).as_bytes());
            ull_callees.add(&callee_hash);

            // Track edges by (caller, callee) pair
            let edge_id = format!("{:?}->{:?}", src_idx, tgt_idx);
            let edge_id_hash = xxh3_64(edge_id.as_bytes());
            ull_edges.add(&edge_id_hash);
        }

        let file_count = file_paths.len();
        let approx_functions = ull_functions.cardinality() as u32;
        let approx_callers = ull_callers.cardinality() as u32;
        let approx_callees = ull_callees.cardinality() as u32;
        let approx_call_edges = ull_edges.cardinality() as u32;

        let avg_functions_per_file = if file_count > 0 {
            approx_functions as f64 / file_count as f64
        } else {
            0.0
        };

        let avg_calls_per_function = if approx_functions > 0 {
            approx_call_edges as f64 / approx_functions as f64
        } else {
            0.0
        };

        let graph_density = if approx_functions > 1 {
            approx_call_edges as f64 / (approx_functions as f64 * approx_functions as f64)
        } else {
            0.0
        };

        CodeGraphStats {
            approx_functions,
            approx_callers,
            approx_callees,
            approx_call_edges,
            file_count,
            avg_functions_per_file,
            avg_calls_per_function,
            graph_density,
        }
    }

    /// Format statistics as a human-readable report.
    pub fn report(&self) -> String {
        format!(
            r#"
════════════════════════════════════════════════════════════════════
CODE GRAPH STATISTICS (UltraLogLog ±2% confidence)
════════════════════════════════════════════════════════════════════

Graph Composition:
  • Unique functions:          ~{}
  • Unique callers:            ~{}
  • Unique callees:            ~{}
  • Source files:              {}

Call Graph Metrics:
  • Distinct call edges:       ~{}
  • Avg functions per file:    {:.1}
  • Avg calls per function:    {:.1}
  • Graph density:             {:.6}

Analysis Note:
  All cardinality estimates (functions, callers, callees, edges) use
  UltraLogLog sketches with ±2% statistical error confidence.
  These estimates are computed in O(1) space independent of graph size.

════════════════════════════════════════════════════════════════════
"#,
            self.approx_functions,
            self.approx_callers,
            self.approx_callees,
            self.file_count,
            self.approx_call_edges,
            self.avg_functions_per_file,
            self.avg_calls_per_function,
            self.graph_density,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_graph_stats() {
        let graph = CodeGraph::new();
        let stats = CodeGraphStats::compute(&graph);

        assert_eq!(stats.approx_functions, 0);
        assert_eq!(stats.approx_callers, 0);
        assert_eq!(stats.approx_callees, 0);
        assert_eq!(stats.approx_call_edges, 0);
        assert_eq!(stats.file_count, 0);
    }

    #[test]
    fn test_stats_report_format() {
        let graph = CodeGraph::new();
        let stats = CodeGraphStats::compute(&graph);
        let report = stats.report();

        // Verify report contains expected sections
        assert!(report.contains("CODE GRAPH STATISTICS"));
        assert!(report.contains("UltraLogLog"));
        assert!(report.contains("Graph Composition"));
        assert!(report.contains("Call Graph Metrics"));
    }
}
