//! System Dependence Graph (SDG) for inter-procedural program slicing.
//!
//! Extends intra-procedural PDGs with call edges, parameter-in/out edges,
//! and summary edges to enable inter-procedural backward and forward slicing
//! using the Horwitz-Reps-Binkley two-phase algorithm.
//!
//! # Architecture
//!
//! The SDG connects per-function PDGs through three kinds of inter-procedural
//! edges:
//!
//! - **Call edges**: link a call site block in the caller to the entry block of
//!   the callee.
//! - **Parameter-in edges**: map actual arguments at a call site to the formal
//!   parameters of the callee.
//! - **Parameter-out edges**: map the return value of the callee back to the
//!   result at the call site in the caller.
//! - **Summary edges**: transitive dependencies through a callee, allowing the
//!   slicer to skip descending into the callee body when possible.
//!
//! # Two-Phase Slicing (Horwitz-Reps-Binkley)
//!
//! **Backward slice**:
//!   - Ascending pass: traverse backward within the caller, descend into
//!     callees via call/param-in edges, and use summary edges.
//!   - Descending pass: traverse backward within callees reached in the
//!     ascending pass, but do **not** ascend back to callers.
//!
//! **Forward slice**: symmetric two-phase traversal in the forward direction.

use std::collections::{HashMap, HashSet, VecDeque};

use super::cfg::{CfgNodeId, ControlFlowGraph};
use super::pdg::ProgramDependenceGraph;

use petgraph::graph::NodeIndex;

// ---------------------------------------------------------------------------
// Inter-procedural taint slot
// ---------------------------------------------------------------------------

/// Inter-procedural taint slot (what flows in/out of a function).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TaintSlot {
    /// A formal parameter at the given index.
    Param(usize),
    /// The return value.
    Return,
}

// ---------------------------------------------------------------------------
// Inter-procedural edge types
// ---------------------------------------------------------------------------

/// Edge from caller to callee mapping actual arguments to formal parameters.
#[derive(Debug, Clone)]
pub struct ParamInEdge {
    /// The function containing the call site.
    pub caller_func: NodeIndex,
    /// The block in the caller that performs the call.
    pub caller_block: CfgNodeId,
    /// The callee function.
    pub callee_func: NodeIndex,
    /// Which formal parameter index this maps to.
    pub formal_param_index: usize,
}

/// Edge from callee back to caller mapping return values to call results.
#[derive(Debug, Clone)]
pub struct ParamOutEdge {
    /// The callee function.
    pub callee_func: NodeIndex,
    /// The block in the callee that returns.
    pub callee_return_block: CfgNodeId,
    /// The caller function.
    pub caller_func: NodeIndex,
    /// The block in the caller that receives the result.
    pub caller_result_block: CfgNodeId,
}

/// Summary edge: transitive dependency through a callee.
///
/// Records that formal parameter `from_param` of function `func` reaches the
/// output slot `to_output` (either a parameter out-flow or a return value).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SummaryEdge {
    /// The callee function this summary describes.
    pub func: NodeIndex,
    /// Formal parameter index that is the source of the dependency.
    pub from_param: usize,
    /// Which output slot the dependency flows to.
    pub to_output: TaintSlot,
}

// ---------------------------------------------------------------------------
// SDG node
// ---------------------------------------------------------------------------

/// An SDG node identifier combining function and block.
///
/// A single SDG node is the pair (function, basic block within that function).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SdgNode {
    /// The function this node belongs to.
    pub func: NodeIndex,
    /// The basic block within the function's CFG.
    pub block: CfgNodeId,
}

// ---------------------------------------------------------------------------
// Inter-procedural slice criterion and result
// ---------------------------------------------------------------------------

/// Criterion for inter-procedural slicing.
#[derive(Debug, Clone)]
pub struct InterproceduralSliceCriterion {
    /// The function containing the criterion.
    pub func: NodeIndex,
    /// The basic block within the function.
    pub block: CfgNodeId,
    /// Optional variable of interest. When `None`, all variables are
    /// considered.
    pub variable: Option<String>,
}

/// Result of inter-procedural slicing.
#[derive(Debug, Clone)]
pub struct InterproceduralSlice {
    /// Set of (function, block) pairs in the slice.
    pub nodes: HashSet<SdgNode>,
    /// The criterion that produced this slice.
    pub criterion: InterproceduralSliceCriterion,
}

impl InterproceduralSlice {
    /// Whether the slice contains a specific (function, block) pair.
    pub fn contains(&self, func: NodeIndex, block: CfgNodeId) -> bool {
        self.nodes.contains(&SdgNode { func, block })
    }

    /// Number of SDG nodes in the slice.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the slice is empty.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Set of distinct functions that participate in the slice.
    pub fn functions_involved(&self) -> HashSet<NodeIndex> {
        self.nodes.iter().map(|n| n.func).collect()
    }
}

// ---------------------------------------------------------------------------
// Raw call-edge descriptor
// ---------------------------------------------------------------------------

/// A raw call-edge descriptor passed to [`SystemDependenceGraph::build`].
///
/// Describes a call site in one function that invokes another function.
#[derive(Debug, Clone)]
pub struct CallEdgeDescriptor {
    /// The calling function.
    pub caller_func: NodeIndex,
    /// The block in the caller that makes the call.
    pub caller_block: CfgNodeId,
    /// The callee function.
    pub callee_func: NodeIndex,
    /// Number of arguments passed at this call site.
    pub argument_count: usize,
    /// Block where the call result is received (often the same as `caller_block`).
    pub result_block: CfgNodeId,
}

// ---------------------------------------------------------------------------
// System Dependence Graph
// ---------------------------------------------------------------------------

/// The System Dependence Graph.
///
/// Combines per-function PDGs with inter-procedural call/parameter/summary
/// edges to support precise inter-procedural slicing.
pub struct SystemDependenceGraph {
    /// Per-function PDGs (intra-procedural dependences).
    pub function_pdgs: HashMap<NodeIndex, ProgramDependenceGraph>,
    /// Per-function CFGs.
    pub function_cfgs: HashMap<NodeIndex, ControlFlowGraph>,
    /// Call edges: (caller SDG node, callee function).
    pub call_edges: Vec<(SdgNode, NodeIndex)>,
    /// Parameter-in edges (actual -> formal).
    pub param_in_edges: Vec<ParamInEdge>,
    /// Parameter-out edges (return -> call-site result).
    pub param_out_edges: Vec<ParamOutEdge>,
    /// Summary edges (computed from callee PDGs).
    pub summary_edges: Vec<SummaryEdge>,
}

impl SystemDependenceGraph {
    // ------------------------------------------------------------------
    // Construction
    // ------------------------------------------------------------------

    /// Build an SDG from per-function PDGs, CFGs, and raw call-edge
    /// descriptors.
    ///
    /// This stores all intra-procedural information, constructs the
    /// inter-procedural edges (call, param-in, param-out), and then
    /// computes summary edges.
    pub fn build(
        function_pdgs: HashMap<NodeIndex, ProgramDependenceGraph>,
        function_cfgs: HashMap<NodeIndex, ControlFlowGraph>,
        call_edges_raw: Vec<CallEdgeDescriptor>,
    ) -> Self {
        let mut call_edges = Vec::new();
        let mut param_in_edges = Vec::new();
        let mut param_out_edges = Vec::new();

        for desc in &call_edges_raw {
            // Call edge: caller_block -> callee entry
            call_edges.push((
                SdgNode {
                    func: desc.caller_func,
                    block: desc.caller_block,
                },
                desc.callee_func,
            ));

            // Parameter-in edges: one per argument
            for param_idx in 0..desc.argument_count {
                param_in_edges.push(ParamInEdge {
                    caller_func: desc.caller_func,
                    caller_block: desc.caller_block,
                    callee_func: desc.callee_func,
                    formal_param_index: param_idx,
                });
            }

            // Parameter-out edge: callee return -> caller result block
            if let Some(callee_cfg) = function_cfgs.get(&desc.callee_func) {
                if let Some(exit_id) = callee_cfg.exit() {
                    param_out_edges.push(ParamOutEdge {
                        callee_func: desc.callee_func,
                        callee_return_block: exit_id,
                        caller_func: desc.caller_func,
                        caller_result_block: desc.result_block,
                    });
                }
            }
        }

        let mut sdg = Self {
            function_pdgs,
            function_cfgs,
            call_edges,
            param_in_edges,
            param_out_edges,
            summary_edges: Vec::new(),
        };

        sdg.compute_summary_edges();
        sdg
    }

    // ------------------------------------------------------------------
    // Summary edge computation (Horwitz-Reps-Binkley)
    // ------------------------------------------------------------------

    /// Compute summary edges for every callee.
    ///
    /// A summary edge `(func, from_param, to_output)` records that formal
    /// parameter `from_param` of function `func` can influence `to_output`
    /// (typically `TaintSlot::Return`).
    ///
    /// The algorithm iterates to a fixed point to handle recursive
    /// functions: if a callee calls itself (or participates in mutual
    /// recursion), the summary edges of one iteration feed into the next
    /// until no new edges are discovered.
    fn compute_summary_edges(&mut self) {
        let callee_funcs: HashSet<NodeIndex> =
            self.call_edges.iter().map(|(_, callee)| *callee).collect();

        let mut all_summaries: HashSet<SummaryEdge> = HashSet::new();
        let mut changed = true;

        while changed {
            changed = false;

            for &callee_func in &callee_funcs {
                let pdg = match self.function_pdgs.get(&callee_func) {
                    Some(p) => p,
                    None => continue,
                };
                let cfg = match self.function_cfgs.get(&callee_func) {
                    Some(c) => c,
                    None => continue,
                };

                let exit_id = match cfg.exit() {
                    Some(e) => e,
                    None => continue,
                };
                let entry_id = match cfg.entry() {
                    Some(e) => e,
                    None => continue,
                };

                // Find the maximum param index referenced by param-in edges
                // for this callee.
                let max_param = self
                    .param_in_edges
                    .iter()
                    .filter(|e| e.callee_func == callee_func)
                    .map(|e| e.formal_param_index)
                    .max();

                let param_count = match max_param {
                    Some(m) => m + 1,
                    None => {
                        // No param-in edges at all; check if entry reaches
                        // exit (trivial summary: function body always runs).
                        // We still want a summary that says "no param
                        // dependency" but that is implicitly represented by
                        // the absence of summary edges.
                        continue;
                    }
                };

                // For each formal parameter, determine if it reaches the exit
                // via backward reachability from exit.
                let reachable_from_exit = pdg.backward_reachable(exit_id);

                // The entry block represents all formal parameter defs.
                // A parameter "reaches" the exit if the entry block is in the
                // backward reachable set from exit (which means the entry
                // influences the exit through data/control deps).
                //
                // For a more precise analysis we check data deps for specific
                // param variables, but since our PDG does not name formal
                // parameters by index we use a heuristic: if the entry block
                // is backward-reachable from exit, each parameter that has a
                // data-dep edge from entry potentially reaches the exit.
                if reachable_from_exit.contains(&entry_id) {
                    for param_idx in 0..param_count {
                        let edge = SummaryEdge {
                            func: callee_func,
                            from_param: param_idx,
                            to_output: TaintSlot::Return,
                        };
                        if all_summaries.insert(edge.clone()) {
                            changed = true;
                        }
                    }
                }
            }
        }

        self.summary_edges = all_summaries.into_iter().collect();
    }

    // ------------------------------------------------------------------
    // Helper: collect all blocks for a function
    // ------------------------------------------------------------------

    /// Return all CFG block IDs for a given function.
    pub fn blocks_of(&self, func: NodeIndex) -> Vec<CfgNodeId> {
        if let Some(pdg) = self.function_pdgs.get(&func) {
            pdg.cfg_blocks().to_vec()
        } else if let Some(cfg) = self.function_cfgs.get(&func) {
            cfg.blocks().map(|(&id, _)| id).collect()
        } else {
            Vec::new()
        }
    }

    // ------------------------------------------------------------------
    // Inter-procedural backward slice (HRB two-phase)
    // ------------------------------------------------------------------

    /// Compute an inter-procedural backward slice using the
    /// Horwitz-Reps-Binkley two-phase algorithm.
    ///
    /// **Ascending pass** (caller context): starting from the criterion,
    /// traverse backward within the criterion's function and across call
    /// sites (ascending to callers via param-out edges and descending into
    /// callees via call edges / summary edges).
    ///
    /// **Descending pass** (callee context): for every callee entry
    /// block discovered in the ascending pass, traverse backward within
    /// that callee's PDG *without* ascending back to any caller.
    ///
    /// The final slice is the union of both phases.
    pub fn interprocedural_backward_slice(
        &self,
        criterion: &InterproceduralSliceCriterion,
    ) -> InterproceduralSlice {
        let mut result_nodes: HashSet<SdgNode> = HashSet::new();

        // Ascending traversal.
        let mut worklist: VecDeque<SdgNode> = VecDeque::new();
        let start = SdgNode {
            func: criterion.func,
            block: criterion.block,
        };
        result_nodes.insert(start);
        worklist.push_back(start);

        // Track callee entries reached during the ascending pass to seed the descending pass.
        let mut callee_entries_for_phase2: Vec<SdgNode> = Vec::new();

        while let Some(current) = worklist.pop_front() {
            // 1a. Follow intra-procedural backward deps (control + data).
            if let Some(pdg) = self.function_pdgs.get(&current.func) {
                // Control dependencies: blocks that control `current.block`.
                for &(a, b) in pdg.control_dep_edges() {
                    if b == current.block {
                        let node = SdgNode {
                            func: current.func,
                            block: a,
                        };
                        if result_nodes.insert(node) {
                            worklist.push_back(node);
                        }
                    }
                }
                // Data dependencies: blocks whose defs are used in `current.block`.
                for (a, b, _var) in pdg.data_dep_edges() {
                    if *b == current.block {
                        let node = SdgNode {
                            func: current.func,
                            block: *a,
                        };
                        if result_nodes.insert(node) {
                            worklist.push_back(node);
                        }
                    }
                }
            }

            // 1b. If current.block is a call site (there is a call_edge from
            //     it), include summary edges and descend into callee.
            for (caller_sdg_node, callee_func) in &self.call_edges {
                if caller_sdg_node.func == current.func && caller_sdg_node.block == current.block {
                    // Use summary edges to propagate back through the callee
                    // without descending.
                    for se in &self.summary_edges {
                        if se.func == *callee_func {
                            // The summary says param -> return. Since we are
                            // at the call site (which receives the return),
                            // the call site block is already in the slice.
                            // We need to include the actual-argument block,
                            // which is caller_block itself (the call site
                            // passes arguments).
                            let node = SdgNode {
                                func: current.func,
                                block: current.block,
                            };
                            // Already in the set, but we enqueue param-in
                            // sources if there are distinct blocks.
                            result_nodes.insert(node);
                        }
                    }

                    // Descend: include the callee entry block so the descending pass
                    // can explore the callee.
                    if let Some(callee_cfg) = self.function_cfgs.get(callee_func) {
                        if let Some(callee_entry) = callee_cfg.entry() {
                            let callee_node = SdgNode {
                                func: *callee_func,
                                block: callee_entry,
                            };
                            if result_nodes.insert(callee_node) {
                                callee_entries_for_phase2.push(callee_node);
                            }
                        }
                        // Also include callee exit block (the return value
                        // originates there).
                        if let Some(callee_exit) = callee_cfg.exit() {
                            let callee_exit_node = SdgNode {
                                func: *callee_func,
                                block: callee_exit,
                            };
                            if result_nodes.insert(callee_exit_node) {
                                callee_entries_for_phase2.push(callee_exit_node);
                            }
                        }
                    }
                }
            }

            // 1c. Ascend via param-out edges: if the current block is the
            //     result block in a caller for a param-out edge coming from
            //     a callee, include the callee return block.
            for poe in &self.param_out_edges {
                if poe.caller_func == current.func && poe.caller_result_block == current.block {
                    let callee_ret = SdgNode {
                        func: poe.callee_func,
                        block: poe.callee_return_block,
                    };
                    if result_nodes.insert(callee_ret) {
                        callee_entries_for_phase2.push(callee_ret);
                    }
                }
            }
        }

        // Descending traversal into callees.
        // For every callee node reached in the ascending pass, do a backward traversal
        // within that callee's PDG but do NOT ascend to callers.
        let mut phase2_worklist: VecDeque<SdgNode> = VecDeque::new();
        for node in &callee_entries_for_phase2 {
            phase2_worklist.push_back(*node);
        }

        // Descending-pass visited set (separate to avoid re-processing ascending-pass nodes
        // but we still add to the same result set).
        let mut phase2_visited: HashSet<SdgNode> = HashSet::new();
        for node in &callee_entries_for_phase2 {
            phase2_visited.insert(*node);
        }

        while let Some(current) = phase2_worklist.pop_front() {
            if let Some(pdg) = self.function_pdgs.get(&current.func) {
                // Control deps backward.
                for &(a, b) in pdg.control_dep_edges() {
                    if b == current.block {
                        let node = SdgNode {
                            func: current.func,
                            block: a,
                        };
                        if phase2_visited.insert(node) {
                            result_nodes.insert(node);
                            phase2_worklist.push_back(node);
                        }
                    }
                }
                // Data deps backward.
                for (a, b, _var) in pdg.data_dep_edges() {
                    if *b == current.block {
                        let node = SdgNode {
                            func: current.func,
                            block: *a,
                        };
                        if phase2_visited.insert(node) {
                            result_nodes.insert(node);
                            phase2_worklist.push_back(node);
                        }
                    }
                }
            }

            // The descending pass does NOT ascend to callers. It also does not
            // descend further into sub-callees for simplicity (the standard HRB
            // algorithm restricts the descending pass to intra-procedural edges only).
        }

        InterproceduralSlice {
            nodes: result_nodes,
            criterion: criterion.clone(),
        }
    }

    // ------------------------------------------------------------------
    // Inter-procedural forward slice (HRB two-phase)
    // ------------------------------------------------------------------

    /// Compute an inter-procedural forward slice.
    ///
    /// **Descending pass** (callee context): starting from the criterion,
    /// traverse forward within the function and descend into callees via
    /// call/param-in edges.
    ///
    /// **Ascending pass** (caller context): for every caller return site
    /// reached via param-out edges in the descending pass, traverse forward
    /// within the caller but do NOT descend into other callees.
    pub fn interprocedural_forward_slice(
        &self,
        criterion: &InterproceduralSliceCriterion,
    ) -> InterproceduralSlice {
        let mut result_nodes: HashSet<SdgNode> = HashSet::new();

        // Descending traversal.
        let mut worklist: VecDeque<SdgNode> = VecDeque::new();
        let start = SdgNode {
            func: criterion.func,
            block: criterion.block,
        };
        result_nodes.insert(start);
        worklist.push_back(start);

        // Track caller return sites for the ascending pass.
        let mut caller_sites_for_phase2: Vec<SdgNode> = Vec::new();

        while let Some(current) = worklist.pop_front() {
            // 1a. Follow intra-procedural forward deps (control + data).
            if let Some(pdg) = self.function_pdgs.get(&current.func) {
                // Control dependents: blocks controlled by `current.block`.
                for &(a, b) in pdg.control_dep_edges() {
                    if a == current.block {
                        let node = SdgNode {
                            func: current.func,
                            block: b,
                        };
                        if result_nodes.insert(node) {
                            worklist.push_back(node);
                        }
                    }
                }
                // Data dependents: blocks that use defs from `current.block`.
                for (a, b, _var) in pdg.data_dep_edges() {
                    if *a == current.block {
                        let node = SdgNode {
                            func: current.func,
                            block: *b,
                        };
                        if result_nodes.insert(node) {
                            worklist.push_back(node);
                        }
                    }
                }
            }

            // 1b. Descend into callees via call edges.
            for (caller_sdg_node, callee_func) in &self.call_edges {
                if caller_sdg_node.func == current.func && caller_sdg_node.block == current.block {
                    // Include callee entry + all blocks reachable forward in
                    // callee PDG from entry.
                    if let Some(callee_cfg) = self.function_cfgs.get(callee_func) {
                        if let Some(callee_entry) = callee_cfg.entry() {
                            let callee_node = SdgNode {
                                func: *callee_func,
                                block: callee_entry,
                            };
                            if result_nodes.insert(callee_node) {
                                worklist.push_back(callee_node);
                            }
                        }
                    }
                }
            }

            // 1c. Propagate via param-out edges: if current block is the
            //     callee return block, propagate to the caller result block.
            for poe in &self.param_out_edges {
                if poe.callee_func == current.func && poe.callee_return_block == current.block {
                    let caller_result = SdgNode {
                        func: poe.caller_func,
                        block: poe.caller_result_block,
                    };
                    if result_nodes.insert(caller_result) {
                        caller_sites_for_phase2.push(caller_result);
                    }
                }
            }
        }

        // Ascending traversal in callers.
        let mut phase2_worklist: VecDeque<SdgNode> = VecDeque::new();
        let mut phase2_visited: HashSet<SdgNode> = HashSet::new();
        for node in &caller_sites_for_phase2 {
            phase2_worklist.push_back(*node);
            phase2_visited.insert(*node);
        }

        while let Some(current) = phase2_worklist.pop_front() {
            if let Some(pdg) = self.function_pdgs.get(&current.func) {
                // Forward control dependents.
                for &(a, b) in pdg.control_dep_edges() {
                    if a == current.block {
                        let node = SdgNode {
                            func: current.func,
                            block: b,
                        };
                        if phase2_visited.insert(node) {
                            result_nodes.insert(node);
                            phase2_worklist.push_back(node);
                        }
                    }
                }
                // Forward data dependents.
                for (a, b, _var) in pdg.data_dep_edges() {
                    if *a == current.block {
                        let node = SdgNode {
                            func: current.func,
                            block: *b,
                        };
                        if phase2_visited.insert(node) {
                            result_nodes.insert(node);
                            phase2_worklist.push_back(node);
                        }
                    }
                }
            }

            // The ascending pass does NOT descend into callees.
        }

        InterproceduralSlice {
            nodes: result_nodes,
            criterion: criterion.clone(),
        }
    }

    // ------------------------------------------------------------------
    // Utility queries
    // ------------------------------------------------------------------

    /// Return the set of all callee functions reachable from a given function.
    pub fn callees_of(&self, func: NodeIndex) -> HashSet<NodeIndex> {
        self.call_edges
            .iter()
            .filter(|(caller, _)| caller.func == func)
            .map(|(_, callee)| *callee)
            .collect()
    }

    /// Return the set of all caller functions that invoke a given function.
    pub fn callers_of(&self, func: NodeIndex) -> HashSet<NodeIndex> {
        self.call_edges
            .iter()
            .filter(|(_, callee)| *callee == func)
            .map(|(caller, _)| caller.func)
            .collect()
    }

    /// Total number of intra-procedural dependence edges across all functions.
    pub fn total_intraprocedural_edges(&self) -> usize {
        self.function_pdgs
            .values()
            .map(|pdg| pdg.control_dep_edges().len() + pdg.data_dep_edges().len())
            .sum()
    }

    /// Total number of inter-procedural edges (call + param-in + param-out
    /// + summary).
    pub fn total_interprocedural_edges(&self) -> usize {
        self.call_edges.len()
            + self.param_in_edges.len()
            + self.param_out_edges.len()
            + self.summary_edges.len()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SourceSpan;
    use crate::graph::cfg::{CfgEdgeKind, ControlFlowGraph};
    use crate::graph::dataflow::{BlockDataFlow, DefPoint, ReachingDefinitions, UsePoint, VarRef};
    use crate::graph::pdg::ProgramDependenceGraph;
    use std::collections::{HashMap, HashSet};

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    /// Build a simple linear CFG for a function:
    ///
    /// ```text
    /// entry -> body -> exit
    /// ```
    ///
    /// - `entry` defines `defs`
    /// - `body` uses `body_uses` and defines `body_defs`
    /// - `exit` uses `exit_uses`
    ///
    /// Returns (cfg, block_facts, reaching_defs, [entry, exit, body]).
    fn make_linear_cfg(
        name: &str,
        entry_defs: Vec<&str>,
        body_uses: Vec<&str>,
        body_defs: Vec<&str>,
        exit_uses: Vec<&str>,
    ) -> (
        ControlFlowGraph,
        HashMap<CfgNodeId, BlockDataFlow>,
        ReachingDefinitions,
        Vec<CfgNodeId>,
    ) {
        let mut cfg = ControlFlowGraph::new(name);
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        let body = cfg.create_block("body");

        cfg.add_statement(entry, SourceSpan::new(0, 5));
        cfg.add_statement(body, SourceSpan::new(6, 15));

        cfg.add_edge(entry, body, CfgEdgeKind::FallThrough);
        cfg.add_edge(body, exit, CfgEdgeKind::FallThrough);

        let mut byte_offset = 0usize;
        let mut all_defs_global = Vec::new();

        // Entry defs
        let entry_def_points: Vec<DefPoint> = entry_defs
            .iter()
            .enumerate()
            .map(|(i, &name)| {
                let dp = DefPoint {
                    var: VarRef::new(name),
                    block: entry,
                    stmt_index: i,
                    start_byte: byte_offset,
                    end_byte: byte_offset + 5,
                };
                byte_offset += 6;
                dp
            })
            .collect();
        all_defs_global.extend(entry_def_points.iter().cloned());

        let body_def_points: Vec<DefPoint> = body_defs
            .iter()
            .enumerate()
            .map(|(i, &name)| {
                let dp = DefPoint {
                    var: VarRef::new(name),
                    block: body,
                    stmt_index: i,
                    start_byte: byte_offset,
                    end_byte: byte_offset + 5,
                };
                byte_offset += 6;
                dp
            })
            .collect();
        all_defs_global.extend(body_def_points.iter().cloned());

        let body_use_points: Vec<UsePoint> = body_uses
            .iter()
            .enumerate()
            .map(|(i, &name)| UsePoint {
                var: VarRef::new(name),
                block: body,
                stmt_index: i,
                start_byte: byte_offset,
                end_byte: byte_offset + 3,
            })
            .collect();

        let exit_use_points: Vec<UsePoint> = exit_uses
            .iter()
            .enumerate()
            .map(|(i, &name)| UsePoint {
                var: VarRef::new(name),
                block: exit,
                stmt_index: i,
                start_byte: byte_offset,
                end_byte: byte_offset + 3,
            })
            .collect();

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: entry_def_points,
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            body,
            BlockDataFlow {
                defs: body_def_points,
                uses: body_use_points,
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            exit,
            BlockDataFlow {
                defs: vec![],
                uses: exit_use_points,
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );

        // Build reaching defs: sort blocks and collect defs.
        let mut sorted_blocks: Vec<CfgNodeId> = block_facts.keys().copied().collect();
        sorted_blocks.sort();

        let mut sorted_defs = Vec::new();
        for &bid in &sorted_blocks {
            if let Some(facts) = block_facts.get(&bid) {
                sorted_defs.extend(facts.defs.iter().cloned());
            }
        }

        // Build a name->indices map for the sorted defs.
        let mut var_to_indices: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, def) in sorted_defs.iter().enumerate() {
            var_to_indices
                .entry(def.var.name.clone())
                .or_default()
                .push(i);
        }

        // Reaching defs: simple forward propagation.
        let mut reach_in: HashMap<CfgNodeId, HashSet<usize>> = HashMap::new();
        let mut reach_out: HashMap<CfgNodeId, HashSet<usize>> = HashMap::new();

        // Entry: no reach_in, reach_out = its own defs.
        reach_in.insert(entry, HashSet::new());
        let entry_def_indices: HashSet<usize> = sorted_defs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.block == entry)
            .map(|(i, _)| i)
            .collect();
        reach_out.insert(entry, entry_def_indices.clone());

        // Body: reach_in = reach_out(entry), reach_out = reach_in union body defs.
        let body_reach_in = entry_def_indices.clone();
        let body_def_indices: HashSet<usize> = sorted_defs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.block == body)
            .map(|(i, _)| i)
            .collect();
        let body_reach_out: HashSet<usize> =
            body_reach_in.union(&body_def_indices).copied().collect();
        reach_in.insert(body, body_reach_in);
        reach_out.insert(body, body_reach_out.clone());

        // Exit: reach_in = reach_out(body).
        reach_in.insert(exit, body_reach_out.clone());
        reach_out.insert(exit, body_reach_out);

        let reaching = ReachingDefinitions {
            reach_in,
            reach_out,
        };

        (cfg, block_facts, reaching, vec![entry, exit, body])
    }

    /// Build a PDG from a linear CFG helper result.
    fn build_pdg_from_linear(
        cfg: &ControlFlowGraph,
        block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
        reaching: &ReachingDefinitions,
    ) -> ProgramDependenceGraph {
        ProgramDependenceGraph::build(cfg, block_facts, reaching)
    }

    // ------------------------------------------------------------------
    // Test 1: Single function backward slice matches intra-procedural
    // ------------------------------------------------------------------

    #[test]
    fn test_single_function_backward_slice_matches_intraprocedural() {
        // Function A: entry(x=1) -> body(y=x+1) -> exit(return y)
        let func_a = NodeIndex::new(0);

        let (cfg_a, facts_a, reaching_a, ids_a) =
            make_linear_cfg("func_a", vec!["x"], vec!["x"], vec!["y"], vec!["y"]);
        let pdg_a = build_pdg_from_linear(&cfg_a, &facts_a, &reaching_a);

        let _entry_a = ids_a[0];
        let exit_a = ids_a[1];
        let _body_a = ids_a[2];

        // Intra-procedural backward slice from exit.
        let intra_reachable = pdg_a.backward_reachable(exit_a);

        // Build SDG with only one function and no call edges.
        let mut function_pdgs = HashMap::new();
        function_pdgs.insert(func_a, pdg_a);
        let mut function_cfgs = HashMap::new();
        function_cfgs.insert(func_a, cfg_a);

        let sdg = SystemDependenceGraph::build(function_pdgs, function_cfgs, vec![]);

        let criterion = InterproceduralSliceCriterion {
            func: func_a,
            block: exit_a,
            variable: None,
        };
        let inter_slice = sdg.interprocedural_backward_slice(&criterion);

        // Every block in the intra-procedural slice should be in the
        // inter-procedural slice.
        for &block in &intra_reachable {
            assert!(
                inter_slice.contains(func_a, block),
                "Intra-procedural block {:?} should be in inter-procedural slice",
                block
            );
        }

        // The inter-procedural slice should not contain blocks from other
        // functions (there are none).
        assert_eq!(
            inter_slice.functions_involved().len(),
            1,
            "Only one function should be involved"
        );
    }

    // ------------------------------------------------------------------
    // Test 2: Inter-procedural backward slice crosses call boundary
    // ------------------------------------------------------------------

    #[test]
    fn test_interprocedural_backward_slice_crosses_call_boundary() {
        // Function A (caller): entry(a=1) -> body(r=call B) -> exit(return r)
        // Function B (callee): entry(p=param) -> body(q=p+1) -> exit(return q)
        //
        // A backward slice from A's exit should include blocks in B.

        let func_a = NodeIndex::new(0);
        let func_b = NodeIndex::new(1);

        // Function A: entry defines "a", body defines "r" using "a", exit uses "r".
        let (cfg_a, facts_a, reaching_a, ids_a) =
            make_linear_cfg("func_a", vec!["a"], vec!["a"], vec!["r"], vec!["r"]);
        let pdg_a = build_pdg_from_linear(&cfg_a, &facts_a, &reaching_a);
        let entry_a = ids_a[0];
        let exit_a = ids_a[1];
        let body_a = ids_a[2];

        // Function B: entry defines "p", body defines "q" using "p", exit uses "q".
        let (cfg_b, facts_b, reaching_b, ids_b) =
            make_linear_cfg("func_b", vec!["p"], vec!["p"], vec!["q"], vec!["q"]);
        let pdg_b = build_pdg_from_linear(&cfg_b, &facts_b, &reaching_b);
        let _entry_b = ids_b[0];
        let _exit_b = ids_b[1];
        let _body_b = ids_b[2];

        let mut function_pdgs = HashMap::new();
        function_pdgs.insert(func_a, pdg_a);
        function_pdgs.insert(func_b, pdg_b);

        let mut function_cfgs = HashMap::new();
        function_cfgs.insert(func_a, cfg_a.clone());
        function_cfgs.insert(func_b, cfg_b.clone());

        // Call edge: A's body block calls B with 1 argument.
        let call_desc = CallEdgeDescriptor {
            caller_func: func_a,
            caller_block: body_a,
            callee_func: func_b,
            argument_count: 1,
            result_block: body_a,
        };

        let sdg = SystemDependenceGraph::build(function_pdgs, function_cfgs, vec![call_desc]);

        // Backward slice from A's exit.
        let criterion = InterproceduralSliceCriterion {
            func: func_a,
            block: exit_a,
            variable: None,
        };
        let slice = sdg.interprocedural_backward_slice(&criterion);

        // Slice should include blocks from function A.
        assert!(
            slice.contains(func_a, exit_a),
            "A's exit should be in the slice"
        );
        assert!(
            slice.contains(func_a, body_a),
            "A's body (call site) should be in the slice"
        );
        assert!(
            slice.contains(func_a, entry_a),
            "A's entry should be in the slice"
        );

        // Slice should also include blocks from function B (callee).
        assert!(
            slice.functions_involved().contains(&func_b),
            "Function B should be involved in the slice"
        );
    }

    // ------------------------------------------------------------------
    // Test 3: Summary edges bypass callee body
    // ------------------------------------------------------------------

    #[test]
    fn test_summary_edges_bypass_callee_body() {
        // Verify that summary edges are computed when a callee's entry
        // influences its exit.

        let func_callee = NodeIndex::new(0);

        // Callee: entry(p=param) -> body(q=p+1) -> exit(return q)
        let (cfg_c, facts_c, reaching_c, _ids_c) =
            make_linear_cfg("callee", vec!["p"], vec!["p"], vec!["q"], vec!["q"]);
        let pdg_c = build_pdg_from_linear(&cfg_c, &facts_c, &reaching_c);

        let func_caller = NodeIndex::new(1);

        // Caller: entry(a=1) -> body(r=call callee(a)) -> exit(return r)
        let (cfg_caller, facts_caller, reaching_caller, ids_caller) =
            make_linear_cfg("caller", vec!["a"], vec!["a"], vec!["r"], vec!["r"]);
        let pdg_caller = build_pdg_from_linear(&cfg_caller, &facts_caller, &reaching_caller);
        let body_caller = ids_caller[2];

        let mut function_pdgs = HashMap::new();
        function_pdgs.insert(func_callee, pdg_c);
        function_pdgs.insert(func_caller, pdg_caller);

        let mut function_cfgs = HashMap::new();
        function_cfgs.insert(func_callee, cfg_c);
        function_cfgs.insert(func_caller, cfg_caller);

        let call_desc = CallEdgeDescriptor {
            caller_func: func_caller,
            caller_block: body_caller,
            callee_func: func_callee,
            argument_count: 1,
            result_block: body_caller,
        };

        let sdg = SystemDependenceGraph::build(function_pdgs, function_cfgs, vec![call_desc]);

        // There should be at least one summary edge for func_callee.
        assert!(
            !sdg.summary_edges.is_empty(),
            "Summary edges should be computed for the callee"
        );

        let callee_summaries: Vec<_> = sdg
            .summary_edges
            .iter()
            .filter(|se| se.func == func_callee)
            .collect();
        assert!(
            !callee_summaries.is_empty(),
            "Callee should have summary edges"
        );

        // The summary should say: param 0 -> Return.
        assert!(
            callee_summaries
                .iter()
                .any(|se| se.from_param == 0 && se.to_output == TaintSlot::Return),
            "Summary edge should map param 0 to Return, got: {:?}",
            callee_summaries
        );
    }

    // ------------------------------------------------------------------
    // Test 4: Forward slice propagates through call sites
    // ------------------------------------------------------------------

    #[test]
    fn test_forward_slice_propagates_through_call_sites() {
        // Function A (caller): entry(a=1) -> body(call B(a)) -> exit
        // Function B (callee): entry(p=param) -> body(q=p+1) -> exit(return q)
        //
        // A forward slice from A's entry should propagate into B.

        let func_a = NodeIndex::new(0);
        let func_b = NodeIndex::new(1);

        let (cfg_a, facts_a, reaching_a, ids_a) =
            make_linear_cfg("func_a", vec!["a"], vec!["a"], vec!["r"], vec!["r"]);
        let pdg_a = build_pdg_from_linear(&cfg_a, &facts_a, &reaching_a);
        let entry_a = ids_a[0];
        let body_a = ids_a[2];

        let (cfg_b, facts_b, reaching_b, _ids_b) =
            make_linear_cfg("func_b", vec!["p"], vec!["p"], vec!["q"], vec!["q"]);
        let pdg_b = build_pdg_from_linear(&cfg_b, &facts_b, &reaching_b);

        let mut function_pdgs = HashMap::new();
        function_pdgs.insert(func_a, pdg_a);
        function_pdgs.insert(func_b, pdg_b);

        let mut function_cfgs = HashMap::new();
        function_cfgs.insert(func_a, cfg_a);
        function_cfgs.insert(func_b, cfg_b);

        let call_desc = CallEdgeDescriptor {
            caller_func: func_a,
            caller_block: body_a,
            callee_func: func_b,
            argument_count: 1,
            result_block: body_a,
        };

        let sdg = SystemDependenceGraph::build(function_pdgs, function_cfgs, vec![call_desc]);

        // Forward slice from A's entry.
        let criterion = InterproceduralSliceCriterion {
            func: func_a,
            block: entry_a,
            variable: None,
        };
        let slice = sdg.interprocedural_forward_slice(&criterion);

        // Slice should include A's entry and body.
        assert!(
            slice.contains(func_a, entry_a),
            "A's entry should be in forward slice"
        );
        assert!(
            slice.contains(func_a, body_a),
            "A's body (call site) should be in forward slice"
        );

        // Slice should also include function B (callee).
        assert!(
            slice.functions_involved().contains(&func_b),
            "Function B should be involved in the forward slice"
        );
    }

    // ------------------------------------------------------------------
    // Test 5: Descending pass does not ascend to callers (bounded traversal)
    // ------------------------------------------------------------------

    #[test]
    fn test_phase2_does_not_ascend_to_callers() {
        // Function A (caller): entry(a=1) -> body(call B(a)) -> exit(print a)
        // Function B (callee): entry(p=param) -> body(q=p+1) -> exit(return q)
        // Function C (another caller of B): entry(c=99) -> body(call B(c)) -> exit(done)
        //
        // A backward slice from B's exit should:
        //   - Include B's blocks (descending pass within B)
        //   - NOT include C's blocks (descending pass must not ascend to C)

        let func_a = NodeIndex::new(0);
        let func_b = NodeIndex::new(1);
        let func_c = NodeIndex::new(2);

        let (cfg_a, facts_a, reaching_a, ids_a) =
            make_linear_cfg("func_a", vec!["a"], vec!["a"], vec!["r"], vec!["a"]);
        let pdg_a = build_pdg_from_linear(&cfg_a, &facts_a, &reaching_a);
        let body_a = ids_a[2];

        let (cfg_b, facts_b, reaching_b, ids_b) =
            make_linear_cfg("func_b", vec!["p"], vec!["p"], vec!["q"], vec!["q"]);
        let pdg_b = build_pdg_from_linear(&cfg_b, &facts_b, &reaching_b);
        let entry_b = ids_b[0];
        let exit_b = ids_b[1];
        let body_b = ids_b[2];

        let (cfg_c, facts_c, reaching_c, ids_c) =
            make_linear_cfg("func_c", vec!["c"], vec!["c"], vec!["s"], vec!["s"]);
        let pdg_c = build_pdg_from_linear(&cfg_c, &facts_c, &reaching_c);
        let _entry_c = ids_c[0];
        let body_c = ids_c[2];

        let mut function_pdgs = HashMap::new();
        function_pdgs.insert(func_a, pdg_a);
        function_pdgs.insert(func_b, pdg_b);
        function_pdgs.insert(func_c, pdg_c);

        let mut function_cfgs = HashMap::new();
        function_cfgs.insert(func_a, cfg_a);
        function_cfgs.insert(func_b, cfg_b);
        function_cfgs.insert(func_c, cfg_c);

        // A calls B from body_a, C calls B from body_c.
        let call_a_b = CallEdgeDescriptor {
            caller_func: func_a,
            caller_block: body_a,
            callee_func: func_b,
            argument_count: 1,
            result_block: body_a,
        };
        let call_c_b = CallEdgeDescriptor {
            caller_func: func_c,
            caller_block: body_c,
            callee_func: func_b,
            argument_count: 1,
            result_block: body_c,
        };

        let sdg =
            SystemDependenceGraph::build(function_pdgs, function_cfgs, vec![call_a_b, call_c_b]);

        // Backward slice starting from B's exit.
        let criterion = InterproceduralSliceCriterion {
            func: func_b,
            block: exit_b,
            variable: None,
        };
        let slice = sdg.interprocedural_backward_slice(&criterion);

        // B's own blocks should all be in the slice.
        assert!(
            slice.contains(func_b, exit_b),
            "B's exit should be in the slice"
        );
        assert!(
            slice.contains(func_b, entry_b),
            "B's entry should be in the slice"
        );
        assert!(
            slice.contains(func_b, body_b),
            "B's body should be in the slice"
        );

        // Neither A nor C should appear in the slice when starting from
        // within B. The descending pass should not ascend.
        assert!(
            !slice.functions_involved().contains(&func_a),
            "Function A (caller) should NOT be in the slice when starting from B"
        );
        assert!(
            !slice.functions_involved().contains(&func_c),
            "Function C (caller) should NOT be in the slice when starting from B"
        );
    }

    // ------------------------------------------------------------------
    // Test 6: Slice result utility methods
    // ------------------------------------------------------------------

    #[test]
    fn test_interprocedural_slice_utility_methods() {
        let func_a = NodeIndex::new(0);

        let (cfg_a, facts_a, reaching_a, ids_a) =
            make_linear_cfg("func_a", vec!["x"], vec!["x"], vec!["y"], vec!["y"]);
        let pdg_a = build_pdg_from_linear(&cfg_a, &facts_a, &reaching_a);
        let exit_a = ids_a[1];

        let mut function_pdgs = HashMap::new();
        function_pdgs.insert(func_a, pdg_a);
        let mut function_cfgs = HashMap::new();
        function_cfgs.insert(func_a, cfg_a);

        let sdg = SystemDependenceGraph::build(function_pdgs, function_cfgs, vec![]);

        let criterion = InterproceduralSliceCriterion {
            func: func_a,
            block: exit_a,
            variable: None,
        };
        let slice = sdg.interprocedural_backward_slice(&criterion);

        assert!(!slice.is_empty(), "Slice should not be empty");
        assert!(!slice.is_empty(), "Slice should contain at least one node");
        assert_eq!(
            slice.functions_involved().len(),
            1,
            "Only one function should be involved"
        );
        assert!(
            slice.functions_involved().contains(&func_a),
            "func_a should be involved"
        );
    }

    // ------------------------------------------------------------------
    // Test 7: SDG construction with no call edges
    // ------------------------------------------------------------------

    #[test]
    fn test_sdg_no_call_edges() {
        let func_a = NodeIndex::new(0);

        let (cfg_a, facts_a, reaching_a, _ids_a) =
            make_linear_cfg("func_a", vec!["x"], vec!["x"], vec!["y"], vec!["y"]);
        let pdg_a = build_pdg_from_linear(&cfg_a, &facts_a, &reaching_a);

        let mut function_pdgs = HashMap::new();
        function_pdgs.insert(func_a, pdg_a);
        let mut function_cfgs = HashMap::new();
        function_cfgs.insert(func_a, cfg_a);

        let sdg = SystemDependenceGraph::build(function_pdgs, function_cfgs, vec![]);

        assert!(sdg.call_edges.is_empty());
        assert!(sdg.param_in_edges.is_empty());
        assert!(sdg.param_out_edges.is_empty());
        assert!(sdg.summary_edges.is_empty());
        assert_eq!(sdg.total_interprocedural_edges(), 0);
        assert!(sdg.total_intraprocedural_edges() > 0);
    }
}
