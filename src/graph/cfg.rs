//! Intra-procedural Control Flow Graph (CFG) construction.
//!
//! Builds a statement-level CFG from tree-sitter AST nodes.
//! Basic blocks contain sequences of straight-line statements;
//! edges connect blocks via branches, loops, and fall-through.

use std::collections::HashMap;
use std::fmt;

use crate::core::SourceSpan;

/// Unique identifier for a CFG node (basic block).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CfgNodeId(u32);

impl CfgNodeId {
    pub fn new(id: u32) -> Self {
        Self(id)
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Display for CfgNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BB{}", self.0)
    }
}

/// Kind of CFG edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CfgEdgeKind {
    /// Unconditional fall-through to next block.
    FallThrough,
    /// True branch of a conditional.
    ConditionalTrue,
    /// False branch of a conditional.
    ConditionalFalse,
    /// Loop back-edge.
    LoopBack,
    /// Break out of loop.
    LoopBreak,
    /// Continue to loop header.
    LoopContinue,
    /// Exception/error edge.
    Exception,
    /// Return from function.
    Return,
}

/// A basic block in the CFG.
#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: CfgNodeId,
    /// Byte spans of statements in this block.
    pub statements: Vec<SourceSpan>,
    /// Human-readable label (e.g., "entry", "if_true", "loop_header").
    pub label: String,
    /// Whether this is the entry block.
    pub is_entry: bool,
    /// Whether this is an exit block.
    pub is_exit: bool,
}

impl BasicBlock {
    pub fn new(id: CfgNodeId, label: impl Into<String>) -> Self {
        Self {
            id,
            statements: Vec::new(),
            label: label.into(),
            is_entry: false,
            is_exit: false,
        }
    }

    pub fn entry(id: CfgNodeId) -> Self {
        Self {
            id,
            statements: Vec::new(),
            label: "entry".to_string(),
            is_entry: true,
            is_exit: false,
        }
    }

    pub fn exit(id: CfgNodeId) -> Self {
        Self {
            id,
            statements: Vec::new(),
            label: "exit".to_string(),
            is_entry: false,
            is_exit: true,
        }
    }

    pub fn add_statement(&mut self, span: SourceSpan) {
        self.statements.push(span);
    }

    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }
}

/// An edge in the CFG.
#[derive(Debug, Clone)]
pub struct CfgEdge {
    pub from: CfgNodeId,
    pub to: CfgNodeId,
    pub kind: CfgEdgeKind,
}

/// Intra-procedural control flow graph.
///
/// Each function/method gets its own CFG. Basic blocks contain
/// sequences of statements; edges represent control flow.
#[derive(Debug, Clone)]
pub struct ControlFlowGraph {
    blocks: HashMap<CfgNodeId, BasicBlock>,
    edges: Vec<CfgEdge>,
    entry: Option<CfgNodeId>,
    exit: Option<CfgNodeId>,
    next_id: u32,
    /// The function/method name this CFG belongs to.
    pub function_name: String,
}

impl ControlFlowGraph {
    pub fn new(function_name: impl Into<String>) -> Self {
        Self {
            blocks: HashMap::new(),
            edges: Vec::new(),
            entry: None,
            exit: None,
            next_id: 0,
            function_name: function_name.into(),
        }
    }

    /// Create a new basic block and return its ID.
    pub fn create_block(&mut self, label: impl Into<String>) -> CfgNodeId {
        let id = CfgNodeId::new(self.next_id);
        self.next_id += 1;
        let block = BasicBlock::new(id, label);
        self.blocks.insert(id, block);
        id
    }

    /// Create the entry block.
    pub fn create_entry(&mut self) -> CfgNodeId {
        let id = CfgNodeId::new(self.next_id);
        self.next_id += 1;
        let block = BasicBlock::entry(id);
        self.blocks.insert(id, block);
        self.entry = Some(id);
        id
    }

    /// Create the exit block.
    pub fn create_exit(&mut self) -> CfgNodeId {
        let id = CfgNodeId::new(self.next_id);
        self.next_id += 1;
        let block = BasicBlock::exit(id);
        self.blocks.insert(id, block);
        self.exit = Some(id);
        id
    }

    /// Add an edge between two blocks.
    pub fn add_edge(&mut self, from: CfgNodeId, to: CfgNodeId, kind: CfgEdgeKind) {
        self.edges.push(CfgEdge { from, to, kind });
    }

    /// Get a block by ID.
    pub fn get_block(&self, id: CfgNodeId) -> Option<&BasicBlock> {
        self.blocks.get(&id)
    }

    /// Get a mutable block by ID.
    pub fn get_block_mut(&mut self, id: CfgNodeId) -> Option<&mut BasicBlock> {
        self.blocks.get_mut(&id)
    }

    /// Add a statement to a block.
    pub fn add_statement(&mut self, block: CfgNodeId, span: SourceSpan) {
        if let Some(b) = self.blocks.get_mut(&block) {
            b.add_statement(span);
        }
    }

    /// Entry block ID.
    pub fn entry(&self) -> Option<CfgNodeId> {
        self.entry
    }

    /// Exit block ID.
    pub fn exit(&self) -> Option<CfgNodeId> {
        self.exit
    }

    /// All blocks.
    pub fn blocks(&self) -> impl Iterator<Item = (&CfgNodeId, &BasicBlock)> {
        self.blocks.iter()
    }

    /// All edges.
    pub fn edges(&self) -> &[CfgEdge] {
        &self.edges
    }

    /// Number of basic blocks.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Number of edges.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Get successors of a block.
    pub fn successors(&self, block: CfgNodeId) -> Vec<(CfgNodeId, CfgEdgeKind)> {
        self.edges
            .iter()
            .filter(|e| e.from == block)
            .map(|e| (e.to, e.kind))
            .collect()
    }

    /// Get predecessors of a block.
    pub fn predecessors(&self, block: CfgNodeId) -> Vec<(CfgNodeId, CfgEdgeKind)> {
        self.edges
            .iter()
            .filter(|e| e.to == block)
            .map(|e| (e.from, e.kind))
            .collect()
    }

    /// Compute dominators using the iterative algorithm.
    /// Returns a map from each block to its immediate dominator.
    pub fn compute_dominators(&self) -> HashMap<CfgNodeId, CfgNodeId> {
        let entry = match self.entry {
            Some(e) => e,
            None => return HashMap::new(),
        };

        // Collect all block IDs in a deterministic order
        let mut block_ids: Vec<CfgNodeId> = self.blocks.keys().copied().collect();
        block_ids.sort();

        let idx_of: HashMap<CfgNodeId, usize> = block_ids
            .iter()
            .enumerate()
            .map(|(i, &id)| (id, i))
            .collect();

        let n = block_ids.len();
        let entry_idx = idx_of[&entry];

        // idom[i] = immediate dominator index (usize::MAX = undefined)
        let mut idom = vec![usize::MAX; n];
        idom[entry_idx] = entry_idx;

        // Build predecessor lists using our indices
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
        for edge in &self.edges {
            if let (Some(&from), Some(&to)) = (idx_of.get(&edge.from), idx_of.get(&edge.to)) {
                preds[to].push(from);
            }
        }

        // Reverse post-order (excluding entry)
        let rpo = self.reverse_post_order(entry, &block_ids, &idx_of);

        let intersect = |mut a: usize, mut b: usize, idom: &[usize]| -> usize {
            while a != b {
                while a > b {
                    a = idom[a];
                }
                while b > a {
                    b = idom[b];
                }
            }
            a
        };

        let mut changed = true;
        while changed {
            changed = false;
            for &b in &rpo {
                if b == entry_idx {
                    continue;
                }
                let mut new_idom = usize::MAX;
                for &p in &preds[b] {
                    if idom[p] != usize::MAX {
                        if new_idom == usize::MAX {
                            new_idom = p;
                        } else {
                            new_idom = intersect(new_idom, p, &idom);
                        }
                    }
                }
                if new_idom != idom[b] {
                    idom[b] = new_idom;
                    changed = true;
                }
            }
        }

        // Convert back to CfgNodeId map
        let mut result = HashMap::new();
        for (i, &dom) in idom.iter().enumerate() {
            if dom != usize::MAX && i != dom {
                result.insert(block_ids[i], block_ids[dom]);
            }
        }
        result
    }

    fn reverse_post_order(
        &self,
        entry: CfgNodeId,
        block_ids: &[CfgNodeId],
        idx_of: &HashMap<CfgNodeId, usize>,
    ) -> Vec<usize> {
        let n = block_ids.len();
        let entry_idx = idx_of[&entry];

        // Build successor lists
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); n];
        for edge in &self.edges {
            if let (Some(&from), Some(&to)) = (idx_of.get(&edge.from), idx_of.get(&edge.to)) {
                succs[from].push(to);
            }
        }

        let mut visited = vec![false; n];
        let mut order = Vec::with_capacity(n);

        fn dfs(node: usize, succs: &[Vec<usize>], visited: &mut [bool], order: &mut Vec<usize>) {
            visited[node] = true;
            for &s in &succs[node] {
                if !visited[s] {
                    dfs(s, succs, visited, order);
                }
            }
            order.push(node);
        }

        dfs(entry_idx, &succs, &mut visited, &mut order);
        order.reverse();
        order
    }
}

/// Build a simple CFG from a sequence of tree-sitter nodes within a function body.
///
/// This is a language-agnostic builder that recognizes common control flow
/// node types across languages.
pub struct CfgBuilder {
    cfg: ControlFlowGraph,
}

impl CfgBuilder {
    pub fn new(function_name: impl Into<String>) -> Self {
        Self {
            cfg: ControlFlowGraph::new(function_name),
        }
    }

    /// Build a CFG from a tree-sitter function body node.
    pub fn build_from_body(
        mut self,
        body_node: tree_sitter::Node<'_>,
        source: &str,
    ) -> ControlFlowGraph {
        let entry = self.cfg.create_entry();
        let exit = self.cfg.create_exit();

        let last_block = self.process_block(body_node, source, entry, exit);

        // Connect last block to exit if not already connected
        if let Some(last) = last_block {
            if !self.has_edge_from(last) {
                self.cfg.add_edge(last, exit, CfgEdgeKind::FallThrough);
            }
        } else {
            self.cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);
        }

        self.cfg
    }

    fn process_block(
        &mut self,
        node: tree_sitter::Node<'_>,
        source: &str,
        current_block: CfgNodeId,
        exit_block: CfgNodeId,
    ) -> Option<CfgNodeId> {
        let mut active_block = current_block;
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            match child.kind() {
                "if_statement" | "if_expression" => {
                    active_block = self.process_if(child, source, active_block, exit_block);
                }
                "while_statement" | "while_expression" | "loop_expression" => {
                    active_block = self.process_loop(child, source, active_block, exit_block);
                }
                "for_statement" | "for_expression" | "for_in_statement" => {
                    active_block = self.process_loop(child, source, active_block, exit_block);
                }
                "return_statement" | "return_expression" => {
                    let span = SourceSpan::new(child.start_byte(), child.end_byte());
                    self.cfg.add_statement(active_block, span);
                    self.cfg
                        .add_edge(active_block, exit_block, CfgEdgeKind::Return);
                    // Statements after return are dead — create a new unreachable block
                    active_block = self.cfg.create_block("after_return");
                }
                "try_statement" | "try_expression" => {
                    active_block = self.process_try(child, source, active_block, exit_block);
                }
                _ => {
                    // Regular statement — add to current block
                    if child.is_named() {
                        let span = SourceSpan::new(child.start_byte(), child.end_byte());
                        self.cfg.add_statement(active_block, span);
                    }
                }
            }
        }

        Some(active_block)
    }

    fn process_if(
        &mut self,
        node: tree_sitter::Node<'_>,
        source: &str,
        pred_block: CfgNodeId,
        exit_block: CfgNodeId,
    ) -> CfgNodeId {
        // Add condition to predecessor
        if let Some(cond) = node.child_by_field_name("condition") {
            let span = SourceSpan::new(cond.start_byte(), cond.end_byte());
            self.cfg.add_statement(pred_block, span);
        }

        let true_block = self.cfg.create_block("if_true");
        let merge_block = self.cfg.create_block("if_merge");

        self.cfg
            .add_edge(pred_block, true_block, CfgEdgeKind::ConditionalTrue);

        // Process true branch
        if let Some(consequence) = node.child_by_field_name("consequence") {
            if let Some(last) = self.process_block(consequence, source, true_block, exit_block) {
                if !self.has_edge_from(last) {
                    self.cfg
                        .add_edge(last, merge_block, CfgEdgeKind::FallThrough);
                }
            }
        } else {
            self.cfg
                .add_edge(true_block, merge_block, CfgEdgeKind::FallThrough);
        }

        // Process else branch
        if let Some(alternative) = node.child_by_field_name("alternative") {
            let false_block = self.cfg.create_block("if_false");
            self.cfg
                .add_edge(pred_block, false_block, CfgEdgeKind::ConditionalFalse);

            if let Some(last) = self.process_block(alternative, source, false_block, exit_block) {
                if !self.has_edge_from(last) {
                    self.cfg
                        .add_edge(last, merge_block, CfgEdgeKind::FallThrough);
                }
            }
        } else {
            self.cfg
                .add_edge(pred_block, merge_block, CfgEdgeKind::ConditionalFalse);
        }

        merge_block
    }

    fn process_loop(
        &mut self,
        node: tree_sitter::Node<'_>,
        source: &str,
        pred_block: CfgNodeId,
        exit_block: CfgNodeId,
    ) -> CfgNodeId {
        let header = self.cfg.create_block("loop_header");
        let body_block = self.cfg.create_block("loop_body");
        let after_loop = self.cfg.create_block("after_loop");

        self.cfg
            .add_edge(pred_block, header, CfgEdgeKind::FallThrough);

        // Add condition (if exists) to header
        if let Some(cond) = node.child_by_field_name("condition") {
            let span = SourceSpan::new(cond.start_byte(), cond.end_byte());
            self.cfg.add_statement(header, span);
        }

        self.cfg
            .add_edge(header, body_block, CfgEdgeKind::ConditionalTrue);
        self.cfg
            .add_edge(header, after_loop, CfgEdgeKind::ConditionalFalse);

        // Process loop body
        if let Some(body) = node.child_by_field_name("body") {
            if let Some(last) = self.process_block(body, source, body_block, exit_block) {
                if !self.has_edge_from(last) {
                    self.cfg.add_edge(last, header, CfgEdgeKind::LoopBack);
                }
            }
        } else {
            self.cfg.add_edge(body_block, header, CfgEdgeKind::LoopBack);
        }

        after_loop
    }

    fn process_try(
        &mut self,
        node: tree_sitter::Node<'_>,
        source: &str,
        pred_block: CfgNodeId,
        exit_block: CfgNodeId,
    ) -> CfgNodeId {
        let try_block = self.cfg.create_block("try");
        let merge_block = self.cfg.create_block("try_merge");

        self.cfg
            .add_edge(pred_block, try_block, CfgEdgeKind::FallThrough);

        // Process try body
        if let Some(body) = node.child_by_field_name("body") {
            if let Some(last) = self.process_block(body, source, try_block, exit_block) {
                if !self.has_edge_from(last) {
                    self.cfg
                        .add_edge(last, merge_block, CfgEdgeKind::FallThrough);
                }
            }
        } else {
            self.cfg
                .add_edge(try_block, merge_block, CfgEdgeKind::FallThrough);
        }

        // Process exception handlers
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if matches!(child.kind(), "except_clause" | "catch_clause" | "rescue") {
                let handler_block = self.cfg.create_block("catch");
                self.cfg
                    .add_edge(try_block, handler_block, CfgEdgeKind::Exception);

                if let Some(last) = self.process_block(child, source, handler_block, exit_block) {
                    if !self.has_edge_from(last) {
                        self.cfg
                            .add_edge(last, merge_block, CfgEdgeKind::FallThrough);
                    }
                }
            }
        }

        merge_block
    }

    fn has_edge_from(&self, block: CfgNodeId) -> bool {
        self.cfg.edges.iter().any(|e| e.from == block)
    }

    /// Consume the builder and return the CFG.
    pub fn finish(self) -> ControlFlowGraph {
        self.cfg
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cfg_basic_structure() {
        let mut cfg = ControlFlowGraph::new("test_fn");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        let body = cfg.create_block("body");

        cfg.add_edge(entry, body, CfgEdgeKind::FallThrough);
        cfg.add_statement(body, SourceSpan::new(0, 10));
        cfg.add_edge(body, exit, CfgEdgeKind::FallThrough);

        assert_eq!(cfg.block_count(), 3);
        assert_eq!(cfg.edge_count(), 2);
        assert_eq!(cfg.entry(), Some(entry));
        assert_eq!(cfg.exit(), Some(exit));
    }

    #[test]
    fn test_cfg_if_else() {
        let mut cfg = ControlFlowGraph::new("test_if");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        let true_block = cfg.create_block("true");
        let false_block = cfg.create_block("false");
        let merge = cfg.create_block("merge");

        cfg.add_edge(entry, true_block, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(entry, false_block, CfgEdgeKind::ConditionalFalse);
        cfg.add_edge(true_block, merge, CfgEdgeKind::FallThrough);
        cfg.add_edge(false_block, merge, CfgEdgeKind::FallThrough);
        cfg.add_edge(merge, exit, CfgEdgeKind::FallThrough);

        let succs = cfg.successors(entry);
        assert_eq!(succs.len(), 2);

        let preds = cfg.predecessors(merge);
        assert_eq!(preds.len(), 2);
    }

    #[test]
    fn test_cfg_loop() {
        let mut cfg = ControlFlowGraph::new("test_loop");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        let header = cfg.create_block("loop_header");
        let body = cfg.create_block("loop_body");
        let after = cfg.create_block("after_loop");

        cfg.add_edge(entry, header, CfgEdgeKind::FallThrough);
        cfg.add_edge(header, body, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(header, after, CfgEdgeKind::ConditionalFalse);
        cfg.add_edge(body, header, CfgEdgeKind::LoopBack);
        cfg.add_edge(after, exit, CfgEdgeKind::FallThrough);

        // Header has back-edge predecessor
        let header_preds = cfg.predecessors(header);
        assert_eq!(header_preds.len(), 2); // entry + back-edge
    }

    #[test]
    fn test_dominators() {
        let mut cfg = ControlFlowGraph::new("dom_test");
        let entry = cfg.create_entry();
        let _exit = cfg.create_exit();
        let a = cfg.create_block("A");
        let b = cfg.create_block("B");
        let c = cfg.create_block("C");

        cfg.add_edge(entry, a, CfgEdgeKind::FallThrough);
        cfg.add_edge(a, b, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(a, c, CfgEdgeKind::ConditionalFalse);

        let doms = cfg.compute_dominators();
        // Both B and C should be dominated by A
        assert_eq!(doms.get(&b), Some(&a));
        assert_eq!(doms.get(&c), Some(&a));
        // A is dominated by entry
        assert_eq!(doms.get(&a), Some(&entry));
    }
}
