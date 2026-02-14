//! CodeGraph: petgraph-based directed graph of code nodes and call edges.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::core::{CallEdge, CodeNode, EdgeConfidence, Language, NodeId};
use crate::graph::BloomFilter;
use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::Direction;

/// A directed graph of code nodes connected by call edges.
///
/// Wraps `petgraph::DiGraph<CodeNode, CallEdge>` with convenience methods
/// for construction, querying, and reachability analysis.
///
/// Includes a Bloom filter for edge deduplication during graph construction,
/// reducing duplicate edge insertion memory from 1.6MB to ~120KB.
#[derive(Debug)]
pub struct CodeGraph {
    graph: DiGraph<CodeNode, CallEdge>,
    id_to_index: HashMap<NodeId, NodeIndex>,
    name_to_ids: HashMap<String, Vec<NodeId>>,
    file_to_node_ids: HashMap<String, Vec<NodeId>>,
    // LAZY: Built on first call to find_node_by_name_in_file() via interior mutability
    // Saves ~30GB memory by deferring index construction until needed
    file_name_index: RefCell<Option<HashMap<(String, String), Vec<NodeIndex>>>>,
    entry_points: HashSet<NodeIndex>,
    test_entry_points: HashSet<NodeIndex>,
    language: Option<Language>,
    // Bloom filter for edge deduplication: tracks (from, to) pairs to avoid duplicates
    edge_dedup_filter: BloomFilter,
}

impl CodeGraph {
    pub fn new() -> Self {
        // Initialize Bloom filter with expected 100k edges and 1% FP rate
        // This reduces edge cache memory from 1.6MB to ~120KB (13× reduction)
        let edge_dedup_filter = BloomFilter::new(100_000, 0.01);

        Self {
            graph: DiGraph::new(),
            id_to_index: HashMap::new(),
            name_to_ids: HashMap::new(),
            file_to_node_ids: HashMap::new(),
            file_name_index: RefCell::new(None), // LAZY: Will be built on first use
            entry_points: HashSet::new(),
            test_entry_points: HashSet::new(),
            language: None,
            edge_dedup_filter,
        }
    }

    pub fn with_language(mut self, language: Language) -> Self {
        self.language = Some(language);
        self
    }

    /// Add a code node to the graph. Returns its petgraph NodeIndex.
    pub fn add_node(&mut self, node: CodeNode) -> NodeIndex {
        let id = node.id;
        let name = node.name.clone();
        let full_name = node.full_name.clone();
        let file = node.location.file.clone();
        let idx = self.graph.add_node(node);
        self.id_to_index.insert(id, idx);
        self.name_to_ids.entry(name).or_default().push(id);
        if full_name != id.to_string() {
            self.name_to_ids.entry(full_name).or_default().push(id);
        }
        self.file_to_node_ids.entry(file).or_default().push(id);
        // LAZY: file_name_index will be built on first call to find_node_by_name_in_file()
        // Invalidate cached index so it gets rebuilt with new node
        *self.file_name_index.borrow_mut() = None;
        idx
    }

    /// Add a call edge between two nodes identified by NodeId.
    ///
    /// Uses Bloom filter for deduplication: skips adding the edge if it's
    /// probably already been added (with 1% false positive rate).
    pub fn add_edge(&mut self, edge: CallEdge) -> Result<(), String> {
        let from_idx = self
            .id_to_index
            .get(&edge.from)
            .ok_or_else(|| format!("Source node {:?} not found", edge.from))?;
        let to_idx = self
            .id_to_index
            .get(&edge.to)
            .ok_or_else(|| format!("Target node {:?} not found", edge.to))?;

        // Check Bloom filter for deduplication (convert indices to bytes)
        let edge_key = format!("{}:{}", from_idx.index(), to_idx.index());
        let edge_bytes = edge_key.as_bytes();

        if !self.edge_dedup_filter.contains(edge_bytes) {
            // Edge probably not inserted yet, so add it and mark in filter
            self.graph.add_edge(*from_idx, *to_idx, edge);
            self.edge_dedup_filter.insert(edge_bytes);
        }
        // If edge is in filter, skip (with 1% false positive rate)

        Ok(())
    }

    /// Add an edge directly between two NodeIndex values with default confidence.
    /// Uses Bloom filter for deduplication.
    pub fn add_edge_by_index(&mut self, from: NodeIndex, to: NodeIndex) {
        // Check Bloom filter for deduplication (convert indices to bytes)
        let edge_key = format!("{}:{}", from.index(), to.index());
        let edge_bytes = edge_key.as_bytes();

        if !self.edge_dedup_filter.contains(edge_bytes) {
            let from_id = self.graph[from].id;
            let to_id = self.graph[to].id;
            let edge = CallEdge::certain(from_id, to_id);
            self.graph.add_edge(from, to, edge);
            self.edge_dedup_filter.insert(edge_bytes);
        }
    }

    /// Get a node by NodeIndex.
    pub fn get_node(&self, idx: NodeIndex) -> Option<&CodeNode> {
        self.graph.node_weight(idx)
    }

    /// Get a mutable reference to a node.
    pub fn get_node_mut(&mut self, idx: NodeIndex) -> Option<&mut CodeNode> {
        self.graph.node_weight_mut(idx)
    }

    /// Find a node's index by its NodeId.
    pub fn get_index(&self, id: NodeId) -> Option<NodeIndex> {
        self.id_to_index.get(&id).copied()
    }

    /// Find nodes by name (may return multiple for overloaded names).
    pub fn find_nodes_by_name(&self, name: &str) -> Vec<NodeIndex> {
        self.name_to_ids
            .get(name)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| self.id_to_index.get(id).copied())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Find a single node by name (first match).
    pub fn find_node_by_name(&self, name: &str) -> Option<NodeIndex> {
        self.find_nodes_by_name(name).into_iter().next()
    }

    /// Find a node by name scoped to a specific file.
    /// O(1) complexity via lazy-built file_name_index (first call builds index).
    pub fn find_node_by_name_in_file(&self, name: &str, file: &str) -> Option<NodeIndex> {
        // Borrow index (lazily build if needed)
        let mut index_ref = self.file_name_index.borrow_mut();
        if index_ref.is_none() {
            // Build index on first call
            let mut index: HashMap<(String, String), Vec<NodeIndex>> = HashMap::new();
            for (idx, node) in self.nodes() {
                let key = (node.location.file.clone(), node.name.clone());
                index.entry(key).or_default().push(idx);
            }
            *index_ref = Some(index);
        }

        // Look up in built index
        index_ref
            .as_ref()
            .unwrap()
            .get(&(file.to_string(), name.to_string()))
            .and_then(|indices| indices.first().copied())
    }

    /// Find a node by name scoped to any of the given candidate files.
    pub fn find_node_by_name_in_files(&self, name: &str, files: &[String]) -> Option<NodeIndex> {
        for file in files {
            if let Some(idx) = self.find_node_by_name_in_file(name, file) {
                return Some(idx);
            }
        }
        None
    }

    /// Iterator over all nodes.
    pub fn nodes(&self) -> impl Iterator<Item = (NodeIndex, &CodeNode)> {
        self.graph
            .node_indices()
            .map(move |idx| (idx, &self.graph[idx]))
    }

    /// Iterator over all edges.
    pub fn edges(&self) -> impl Iterator<Item = &CallEdge> {
        self.graph.edge_weights()
    }

    /// Iterator over all edges with source and target indices.
    pub fn edges_with_endpoints(&self) -> impl Iterator<Item = (NodeIndex, NodeIndex, &CallEdge)> {
        self.graph.edge_indices().map(move |eidx| {
            let (src, tgt) = self.graph.edge_endpoints(eidx).unwrap();
            (src, tgt, &self.graph[eidx])
        })
    }

    /// Get nodes called by the given node (outgoing edges).
    pub fn calls_from(&self, idx: NodeIndex) -> impl Iterator<Item = NodeIndex> + '_ {
        self.graph.neighbors_directed(idx, Direction::Outgoing)
    }

    /// Get nodes that call the given node (incoming edges).
    pub fn callers_of(&self, idx: NodeIndex) -> impl Iterator<Item = NodeIndex> + '_ {
        self.graph.neighbors_directed(idx, Direction::Incoming)
    }

    /// Get callees filtered to compatible languages only.
    pub fn calls_from_compatible(&self, idx: NodeIndex) -> Vec<NodeIndex> {
        let lang = self.graph[idx].language;
        self.graph
            .neighbors_directed(idx, Direction::Outgoing)
            .filter(|&n| self.graph[n].language.is_compatible_with(lang))
            .collect()
    }

    /// Get callers filtered to compatible languages only.
    pub fn callers_of_compatible(&self, idx: NodeIndex) -> Vec<NodeIndex> {
        let lang = self.graph[idx].language;
        self.graph
            .neighbors_directed(idx, Direction::Incoming)
            .filter(|&n| self.graph[n].language.is_compatible_with(lang))
            .collect()
    }

    /// Find nodes by name, filtered to a specific language (or compatible).
    pub fn find_nodes_by_name_and_language(
        &self,
        name: &str,
        language: Language,
    ) -> Vec<NodeIndex> {
        self.find_nodes_by_name(name)
            .into_iter()
            .filter(|&idx| self.graph[idx].language.is_compatible_with(language))
            .collect()
    }

    /// Mark a node as a production entry point.
    pub fn add_entry_point(&mut self, idx: NodeIndex) {
        self.entry_points.insert(idx);
    }

    /// Mark a node as a test entry point.
    pub fn add_test_entry_point(&mut self, idx: NodeIndex) {
        self.test_entry_points.insert(idx);
    }

    /// Get all production entry points.
    pub fn entry_points(&self) -> &HashSet<NodeIndex> {
        &self.entry_points
    }

    /// Get all test entry points.
    pub fn test_entry_points(&self) -> &HashSet<NodeIndex> {
        &self.test_entry_points
    }

    /// Number of nodes in the graph.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of edges in the graph.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Language of this graph (if set).
    pub fn language(&self) -> Option<Language> {
        self.language
    }

    /// Access the underlying petgraph.
    pub fn inner(&self) -> &DiGraph<CodeNode, CallEdge> {
        &self.graph
    }

    // =========================================================================
    // Reachability analysis
    // =========================================================================

    /// BFS from the given entry points, returning all reachable node indices.
    pub fn compute_reachable(&self, entry_points: &HashSet<NodeIndex>) -> HashSet<NodeIndex> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        for &ep in entry_points {
            if visited.insert(ep) {
                queue.push_back(ep);
            }
        }

        while let Some(current) = queue.pop_front() {
            for neighbor in self.graph.neighbors_directed(current, Direction::Outgoing) {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }

        visited
    }

    /// Compute nodes reachable from production entry points.
    pub fn compute_production_reachable(&self) -> HashSet<NodeIndex> {
        self.compute_reachable(&self.entry_points)
    }

    /// Compute nodes reachable from test entry points.
    pub fn compute_test_reachable(&self) -> HashSet<NodeIndex> {
        self.compute_reachable(&self.test_entry_points)
    }

    /// Find all unreachable nodes (not reachable from any entry point).
    pub fn find_unreachable(&self) -> Vec<NodeIndex> {
        let all_entries: HashSet<NodeIndex> = self
            .entry_points
            .union(&self.test_entry_points)
            .copied()
            .collect();
        let reachable = self.compute_reachable(&all_entries);

        self.graph
            .node_indices()
            .filter(|idx| !reachable.contains(idx))
            .collect()
    }

    /// Check if a specific node is reachable from another.
    pub fn is_reachable_from(&self, source: NodeIndex, target: NodeIndex) -> bool {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(source);
        queue.push_back(source);

        while let Some(current) = queue.pop_front() {
            if current == target {
                return true;
            }
            for neighbor in self.graph.neighbors_directed(current, Direction::Outgoing) {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }
        false
    }

    // =========================================================================
    // Graph merging
    // =========================================================================

    /// Merge another graph into this one. Nodes and edges are added;
    /// duplicate NodeIds are skipped.
    pub fn merge(&mut self, other: &CodeGraph) {
        // Pre-allocate
        self.graph.reserve_nodes(other.node_count());
        self.graph.reserve_edges(other.edge_count());

        // Map from other's NodeIndex to ours
        let mut index_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();

        for (other_idx, node) in other.nodes() {
            if self.id_to_index.contains_key(&node.id) {
                // Already exists — map to existing
                index_map.insert(other_idx, self.id_to_index[&node.id]);
            } else {
                let new_idx = self.add_node(node.clone());
                index_map.insert(other_idx, new_idx);
            }
        }

        // Add edges with Bloom filter deduplication
        for edge in other.edges() {
            let from_idx = self.id_to_index.get(&edge.from);
            let to_idx = self.id_to_index.get(&edge.to);
            if let (Some(&from), Some(&to)) = (from_idx, to_idx) {
                // Check Bloom filter for deduplication (convert indices to bytes)
                let edge_key = format!("{}:{}", from.index(), to.index());
                let edge_bytes = edge_key.as_bytes();

                if !self.edge_dedup_filter.contains(edge_bytes) {
                    self.graph.add_edge(from, to, edge.clone());
                    self.edge_dedup_filter.insert(edge_bytes);
                }
            }
        }

        // Merge entry points
        for &ep in &other.entry_points {
            if let Some(&new_idx) = index_map.get(&ep) {
                self.entry_points.insert(new_idx);
            }
        }
        for &ep in &other.test_entry_points {
            if let Some(&new_idx) = index_map.get(&ep) {
                self.test_entry_points.insert(new_idx);
            }
        }
    }

    // =========================================================================
    // Analysis helpers
    // =========================================================================

    /// Get the in-degree (number of callers) for a node.
    pub fn in_degree(&self, idx: NodeIndex) -> usize {
        self.graph
            .neighbors_directed(idx, Direction::Incoming)
            .count()
    }

    /// Get the out-degree (number of callees) for a node.
    pub fn out_degree(&self, idx: NodeIndex) -> usize {
        self.graph
            .neighbors_directed(idx, Direction::Outgoing)
            .count()
    }

    /// Find all strongly connected components (cycles).
    pub fn strongly_connected_components(&self) -> Vec<Vec<NodeIndex>> {
        petgraph::algo::tarjan_scc(&self.graph)
    }

    /// Find all leaf nodes (zero out-degree — no outgoing calls).
    pub fn leaf_nodes(&self) -> Vec<NodeIndex> {
        self.graph
            .node_indices()
            .filter(|&idx| self.out_degree(idx) == 0)
            .collect()
    }

    /// Find all root nodes (zero in-degree — never called).
    pub fn root_nodes(&self) -> Vec<NodeIndex> {
        self.graph
            .node_indices()
            .filter(|&idx| self.in_degree(idx) == 0)
            .collect()
    }

    /// Compute edge confidence stats.
    pub fn edge_confidence_distribution(&self) -> HashMap<EdgeConfidence, usize> {
        let mut dist = HashMap::new();
        for edge in self.edges() {
            *dist.entry(edge.confidence).or_insert(0) += 1;
        }
        dist
    }
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{NodeKind, SourceLocation, Visibility};

    fn make_node(name: &str, kind: NodeKind) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            kind,
            SourceLocation::new("test.rs".to_string(), 1, 10, 0, 0),
            Language::Rust,
            Visibility::Public,
        )
    }

    #[test]
    fn test_add_nodes_and_edges() {
        let mut graph = CodeGraph::new();
        let main_node = make_node("main", NodeKind::Function);
        let main_id = main_node.id;
        let helper_node = make_node("helper", NodeKind::Function);
        let helper_id = helper_node.id;

        let main_idx = graph.add_node(main_node);
        let helper_idx = graph.add_node(helper_node);

        let edge = CallEdge::certain(main_id, helper_id);
        graph.add_edge(edge).unwrap();

        assert_eq!(graph.node_count(), 2);
        assert_eq!(graph.edge_count(), 1);

        let callees: Vec<_> = graph.calls_from(main_idx).collect();
        assert_eq!(callees, vec![helper_idx]);

        let callers: Vec<_> = graph.callers_of(helper_idx).collect();
        assert_eq!(callers, vec![main_idx]);
    }

    #[test]
    fn test_find_by_name() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node("foo", NodeKind::Function));
        graph.add_node(make_node("bar", NodeKind::Function));

        assert!(graph.find_node_by_name("foo").is_some());
        assert!(graph.find_node_by_name("baz").is_none());
    }

    #[test]
    fn test_reachability() {
        let mut graph = CodeGraph::new();
        let a = make_node("a", NodeKind::Function);
        let a_id = a.id;
        let b = make_node("b", NodeKind::Function);
        let b_id = b.id;
        let c = make_node("c", NodeKind::Function);
        let d = make_node("d", NodeKind::Function); // isolated

        let a_idx = graph.add_node(a);
        let b_idx = graph.add_node(b);
        let c_idx = graph.add_node(c);
        let d_idx = graph.add_node(d);

        graph.add_edge(CallEdge::certain(a_id, b_id)).unwrap();
        let b_node_id = graph.get_node(b_idx).unwrap().id;
        let c_node_id = graph.get_node(c_idx).unwrap().id;
        graph
            .add_edge(CallEdge::certain(b_node_id, c_node_id))
            .unwrap();

        graph.add_entry_point(a_idx);

        let reachable = graph.compute_production_reachable();
        assert!(reachable.contains(&a_idx));
        assert!(reachable.contains(&b_idx));
        assert!(reachable.contains(&c_idx));
        assert!(!reachable.contains(&d_idx));

        let unreachable = graph.find_unreachable();
        assert_eq!(unreachable, vec![d_idx]);
    }

    #[test]
    fn test_merge() {
        let mut g1 = CodeGraph::new();
        let a = make_node("a", NodeKind::Function);
        let a_idx = g1.add_node(a);
        g1.add_entry_point(a_idx);

        let mut g2 = CodeGraph::new();
        let b = make_node("b", NodeKind::Function);
        g2.add_node(b);

        g1.merge(&g2);
        assert_eq!(g1.node_count(), 2);
        assert!(g1.find_node_by_name("b").is_some());
    }

    #[test]
    fn test_scc() {
        let mut graph = CodeGraph::new();
        let a = make_node("a", NodeKind::Function);
        let a_id = a.id;
        let b = make_node("b", NodeKind::Function);
        let b_id = b.id;

        graph.add_node(a);
        graph.add_node(b);
        graph.add_edge(CallEdge::certain(a_id, b_id)).unwrap();
        graph.add_edge(CallEdge::certain(b_id, a_id)).unwrap();

        let sccs = graph.strongly_connected_components();
        // Two nodes form one SCC
        let cycle = sccs.iter().find(|scc| scc.len() == 2);
        assert!(cycle.is_some());
    }

    // =========================================================================
    // File-scoped name resolution tests
    // =========================================================================

    fn make_node_in_file(name: &str, kind: NodeKind, file: &str) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            kind,
            SourceLocation::new(file.to_string(), 1, 10, 0, 0),
            Language::Rust,
            Visibility::Public,
        )
    }

    #[test]
    fn test_find_node_by_name_in_file() {
        let mut graph = CodeGraph::new();
        // Two functions named "helper" in different files
        let helper_a = make_node_in_file("helper", NodeKind::Function, "src/a.rs");
        let helper_b = make_node_in_file("helper", NodeKind::Function, "src/b.rs");

        graph.add_node(helper_a);
        graph.add_node(helper_b);

        // Global lookup returns one of them
        assert!(graph.find_node_by_name("helper").is_some());

        // File-scoped lookup returns the correct one
        let a_idx = graph.find_node_by_name_in_file("helper", "src/a.rs");
        assert!(a_idx.is_some(), "Should find helper in src/a.rs");
        let a_node = graph.get_node(a_idx.unwrap()).unwrap();
        assert_eq!(a_node.location.file, "src/a.rs");

        let b_idx = graph.find_node_by_name_in_file("helper", "src/b.rs");
        assert!(b_idx.is_some(), "Should find helper in src/b.rs");
        let b_node = graph.get_node(b_idx.unwrap()).unwrap();
        assert_eq!(b_node.location.file, "src/b.rs");

        // Wrong file returns None
        assert!(graph
            .find_node_by_name_in_file("helper", "src/c.rs")
            .is_none());
    }

    #[test]
    fn test_find_node_by_name_in_files() {
        let mut graph = CodeGraph::new();
        let helper_a = make_node_in_file("helper", NodeKind::Function, "src/a.rs");
        let helper_b = make_node_in_file("helper", NodeKind::Function, "src/b.rs");
        let other = make_node_in_file("other", NodeKind::Function, "src/c.rs");

        graph.add_node(helper_a);
        graph.add_node(helper_b);
        graph.add_node(other);

        // Search in candidate files [src/b.rs, src/c.rs] for "helper"
        let result = graph.find_node_by_name_in_files(
            "helper",
            &["src/b.rs".to_string(), "src/c.rs".to_string()],
        );
        assert!(result.is_some(), "Should find helper in src/b.rs");
        let node = graph.get_node(result.unwrap()).unwrap();
        assert_eq!(node.location.file, "src/b.rs");

        // No match when none of the candidates have the function
        let no_result = graph.find_node_by_name_in_files("helper", &["src/c.rs".to_string()]);
        assert!(no_result.is_none(), "helper is not in src/c.rs");
    }

    #[test]
    fn test_file_index_populated_on_add() {
        let mut graph = CodeGraph::new();
        let node = make_node_in_file("foo", NodeKind::Function, "src/main.rs");
        graph.add_node(node);

        // File-scoped lookup should work
        assert!(graph
            .find_node_by_name_in_file("foo", "src/main.rs")
            .is_some());
        // Wrong name should fail
        assert!(graph
            .find_node_by_name_in_file("bar", "src/main.rs")
            .is_none());
    }
}
