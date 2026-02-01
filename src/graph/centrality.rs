//! Graph centrality metrics: PageRank and betweenness centrality.
//!
//! Used to identify critical code nodes and adjust dead code confidence.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;
use petgraph::Direction;

use super::code_graph::CodeGraph;

/// Centrality scores for all nodes in the graph.
#[derive(Debug, Clone)]
pub struct CentralityScores {
    pub pagerank: HashMap<NodeIndex, f64>,
    pub betweenness: HashMap<NodeIndex, f64>,
}

/// Importance level derived from centrality scores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ImportanceLevel {
    Low,
    Normal,
    High,
    Critical,
}

impl std::fmt::Display for ImportanceLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportanceLevel::Low => write!(f, "low"),
            ImportanceLevel::Normal => write!(f, "normal"),
            ImportanceLevel::High => write!(f, "high"),
            ImportanceLevel::Critical => write!(f, "critical"),
        }
    }
}

/// Compute PageRank scores for all nodes in the graph.
///
/// Uses the standard iterative algorithm with configurable damping factor.
pub fn compute_pagerank(
    graph: &CodeGraph,
    damping: f64,
    iterations: usize,
) -> HashMap<NodeIndex, f64> {
    let inner = graph.inner();
    let n = inner.node_count();
    if n == 0 {
        return HashMap::new();
    }

    let n_f64 = n as f64;
    let initial = 1.0 / n_f64;
    let mut scores: HashMap<NodeIndex, f64> =
        inner.node_indices().map(|idx| (idx, initial)).collect();

    for _ in 0..iterations {
        let mut new_scores: HashMap<NodeIndex, f64> = HashMap::with_capacity(n);

        // Collect dangling node mass (nodes with no outgoing edges)
        let dangling_sum: f64 = inner
            .node_indices()
            .filter(|&idx| inner.neighbors_directed(idx, Direction::Outgoing).count() == 0)
            .map(|idx| scores[&idx])
            .sum();

        for idx in inner.node_indices() {
            let mut rank = (1.0 - damping) / n_f64;
            rank += damping * dangling_sum / n_f64;

            // Sum contributions from incoming neighbors
            for pred in inner.neighbors_directed(idx, Direction::Incoming) {
                let pred_out_degree =
                    inner.neighbors_directed(pred, Direction::Outgoing).count() as f64;
                if pred_out_degree > 0.0 {
                    rank += damping * scores[&pred] / pred_out_degree;
                }
            }

            new_scores.insert(idx, rank);
        }

        scores = new_scores;
    }

    scores
}

/// Compute betweenness centrality using Brandes' algorithm.
///
/// Betweenness centrality measures how often a node lies on shortest paths
/// between other node pairs. High betweenness = critical connector.
pub fn compute_betweenness(graph: &CodeGraph) -> HashMap<NodeIndex, f64> {
    let inner = graph.inner();
    let n = inner.node_count();
    if n == 0 {
        return HashMap::new();
    }

    let mut centrality: HashMap<NodeIndex, f64> =
        inner.node_indices().map(|idx| (idx, 0.0)).collect();

    for source in inner.node_indices() {
        // BFS from source
        let mut stack = Vec::new();
        let mut predecessors: HashMap<NodeIndex, Vec<NodeIndex>> = HashMap::new();
        let mut sigma: HashMap<NodeIndex, f64> =
            inner.node_indices().map(|idx| (idx, 0.0)).collect();
        let mut dist: HashMap<NodeIndex, i64> = inner.node_indices().map(|idx| (idx, -1)).collect();
        let mut delta: HashMap<NodeIndex, f64> =
            inner.node_indices().map(|idx| (idx, 0.0)).collect();

        *sigma.get_mut(&source).unwrap() = 1.0;
        *dist.get_mut(&source).unwrap() = 0;

        let mut queue = std::collections::VecDeque::new();
        queue.push_back(source);

        while let Some(v) = queue.pop_front() {
            stack.push(v);
            let d_v = dist[&v];

            for w in inner.neighbors_directed(v, Direction::Outgoing) {
                // First visit?
                if dist[&w] < 0 {
                    *dist.get_mut(&w).unwrap() = d_v + 1;
                    queue.push_back(w);
                }
                // Shortest path via v?
                if dist[&w] == d_v + 1 {
                    *sigma.get_mut(&w).unwrap() += sigma[&v];
                    predecessors.entry(w).or_default().push(v);
                }
            }
        }

        // Accumulation
        while let Some(w) = stack.pop() {
            if let Some(preds) = predecessors.get(&w) {
                for &v in preds {
                    let contribution = (sigma[&v] / sigma[&w]) * (1.0 + delta[&w]);
                    *delta.get_mut(&v).unwrap() += contribution;
                }
            }
            if w != source {
                *centrality.get_mut(&w).unwrap() += delta[&w];
            }
        }
    }

    // Normalize by (n-1)(n-2) for directed graphs
    let norm = if n > 2 {
        1.0 / ((n - 1) as f64 * (n - 2) as f64)
    } else {
        1.0
    };

    for val in centrality.values_mut() {
        *val *= norm;
    }

    centrality
}

/// Compute centrality scores (both PageRank and betweenness).
pub fn compute_centrality(graph: &CodeGraph) -> CentralityScores {
    let pagerank = compute_pagerank(graph, 0.85, 100);
    let betweenness = compute_betweenness(graph);
    CentralityScores {
        pagerank,
        betweenness,
    }
}

/// Classify node importance based on centrality percentiles.
pub fn classify_importance(
    pagerank: f64,
    betweenness: f64,
    all_pageranks: &[f64],
    all_betweenness: &[f64],
) -> ImportanceLevel {
    let pr_percentile = percentile_rank(pagerank, all_pageranks);
    let bt_percentile = percentile_rank(betweenness, all_betweenness);
    let combined = (pr_percentile + bt_percentile) / 2.0;

    if combined >= 0.95 {
        ImportanceLevel::Critical
    } else if combined >= 0.80 {
        ImportanceLevel::High
    } else if combined >= 0.20 {
        ImportanceLevel::Normal
    } else {
        ImportanceLevel::Low
    }
}

fn percentile_rank(value: f64, sorted_values: &[f64]) -> f64 {
    if sorted_values.is_empty() {
        return 0.5;
    }
    let count_below = sorted_values.iter().filter(|&&v| v < value).count();
    count_below as f64 / sorted_values.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CallEdge, CodeNode, Language, NodeKind, SourceLocation, Visibility};

    fn make_node(name: &str) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::Function,
            SourceLocation::new("test.rs".to_string(), 1, 10, 0, 0),
            Language::Rust,
            Visibility::Public,
        )
    }

    #[test]
    fn test_pagerank_basic() {
        let mut graph = CodeGraph::new();
        let a = make_node("a");
        let a_id = a.id;
        let b = make_node("b");
        let b_id = b.id;
        let c = make_node("c");
        let c_id = c.id;

        let a_idx = graph.add_node(a);
        let _b_idx = graph.add_node(b);
        let c_idx = graph.add_node(c);

        graph.add_edge(CallEdge::certain(a_id, b_id)).unwrap();
        graph.add_edge(CallEdge::certain(a_id, c_id)).unwrap();
        graph.add_edge(CallEdge::certain(b_id, c_id)).unwrap();

        let pr = compute_pagerank(&graph, 0.85, 100);
        assert_eq!(pr.len(), 3);
        // c should have highest PageRank (most incoming links)
        assert!(pr[&c_idx] > pr[&a_idx]);
    }

    #[test]
    fn test_betweenness_basic() {
        let mut graph = CodeGraph::new();
        let a = make_node("a");
        let a_id = a.id;
        let b = make_node("b");
        let b_id = b.id;
        let c = make_node("c");

        graph.add_node(a);
        let b_idx = graph.add_node(b);
        let _c_idx = graph.add_node(c);

        // a -> b -> c: b is on all paths from a to c
        graph.add_edge(CallEdge::certain(a_id, b_id)).unwrap();
        graph
            .add_edge(CallEdge::certain(
                b_id,
                graph.get_node(NodeIndex::new(2)).unwrap().id,
            ))
            .unwrap();

        let bt = compute_betweenness(&graph);
        assert_eq!(bt.len(), 3);
        // b should have highest betweenness
        assert!(bt[&b_idx] >= bt[&NodeIndex::new(0)]);
    }

    #[test]
    fn test_empty_graph() {
        let graph = CodeGraph::new();
        let pr = compute_pagerank(&graph, 0.85, 100);
        assert!(pr.is_empty());
        let bt = compute_betweenness(&graph);
        assert!(bt.is_empty());
    }

    #[test]
    fn test_importance_classification() {
        let all_pr = vec![0.01, 0.02, 0.05, 0.1, 0.5];
        let all_bt = vec![0.0, 0.01, 0.02, 0.05, 0.1];

        // pagerank=0.5 -> percentile 4/5=0.8, betweenness=0.1 -> percentile 4/5=0.8
        // combined = 0.8, which is >= 0.80 -> High
        assert_eq!(
            classify_importance(0.5, 0.1, &all_pr, &all_bt),
            ImportanceLevel::High
        );

        // Values above all elements -> percentile 5/5=1.0 each, combined=1.0 -> Critical
        assert_eq!(
            classify_importance(1.0, 1.0, &all_pr, &all_bt),
            ImportanceLevel::Critical
        );

        assert_eq!(
            classify_importance(0.01, 0.0, &all_pr, &all_bt),
            ImportanceLevel::Low
        );
    }
}
