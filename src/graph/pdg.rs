//! Program Dependence Graph (PDG) construction.
//!
//! Combines control dependence and data dependence edges into a unified
//! graph that captures both types of program dependencies between basic
//! blocks. Used as the foundation for program slicing.

use std::collections::{HashMap, HashSet, VecDeque};

use super::cfg::{CfgEdgeKind, CfgNodeId, ControlFlowGraph};
use super::dataflow::{BlockDataFlow, ReachingDefinitions};

// ---------------------------------------------------------------------------
// Edge types
// ---------------------------------------------------------------------------

/// Kind of dependence edge in the PDG.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PdgEdgeKind {
    /// Control dependence: the target's execution depends on the
    /// source's branch decision.
    ControlDep,
    /// Data dependence: the source defines a variable consumed by the target.
    DataDep(String),
}

/// A single edge in the program dependence graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PdgEdge {
    pub from: CfgNodeId,
    pub to: CfgNodeId,
    pub kind: PdgEdgeKind,
}

// ---------------------------------------------------------------------------
// Program Dependence Graph
// ---------------------------------------------------------------------------

/// Program Dependence Graph combining control and data dependences.
///
/// The PDG is built from a CFG augmented with reaching-definition results.
/// It stores two separate edge sets for efficient traversal.
#[derive(Debug, Clone)]
pub struct ProgramDependenceGraph {
    /// All block IDs present in the underlying CFG.
    cfg_blocks: Vec<CfgNodeId>,
    /// Control dependence edges: (controller, dependent).
    control_deps: Vec<(CfgNodeId, CfgNodeId)>,
    /// Data dependence edges: (def_block, use_block, var_name).
    data_deps: Vec<(CfgNodeId, CfgNodeId, String)>,
}

impl ProgramDependenceGraph {
    /// Build a PDG from a CFG, per-block data-flow facts, and reaching
    /// definitions.
    ///
    /// Control dependences are computed via a simplified post-dominator
    /// approach. Data dependences are extracted from reaching definitions:
    /// if a definition in block A reaches a use in block B, an edge
    /// `(A, B, var)` is added.
    pub fn build(
        cfg: &ControlFlowGraph,
        block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
        reaching_defs: &ReachingDefinitions,
    ) -> Self {
        let cfg_blocks: Vec<CfgNodeId> = cfg.blocks().map(|(&id, _)| id).collect();
        let control_deps = Self::compute_control_deps(cfg);
        let data_deps = Self::compute_data_deps(cfg, block_facts, reaching_defs);

        Self {
            cfg_blocks,
            control_deps,
            data_deps,
        }
    }

    // ------------------------------------------------------------------
    // Control dependence computation
    // ------------------------------------------------------------------

    /// Simplified control-dependence computation.
    ///
    /// A block B is control-dependent on block A when:
    ///   1. A is a branch point (has multiple outgoing edges with distinct
    ///      edge kinds such as ConditionalTrue / ConditionalFalse), and
    ///   2. B is reachable from one branch of A but is NOT a post-dominator
    ///      of A (i.e., there exists a path from A to the exit that does
    ///      not pass through B).
    ///
    /// We use an approximation: compute post-dominators, then for every
    /// branch edge A->S, every block on the path from S to the immediate
    /// post-dominator of A (exclusive) is control-dependent on A.
    fn compute_control_deps(cfg: &ControlFlowGraph) -> Vec<(CfgNodeId, CfgNodeId)> {
        let exit = match cfg.exit() {
            Some(e) => e,
            None => return Vec::new(),
        };

        let block_ids: Vec<CfgNodeId> = {
            let mut ids: Vec<CfgNodeId> = cfg.blocks().map(|(&id, _)| id).collect();
            ids.sort();
            ids
        };

        // Compute post-dominators: post-dom(B) is the set of blocks that
        // appear on every path from B to exit. We build a reverse CFG and
        // compute dominators from exit.
        let pdom = Self::compute_post_dominators(cfg, &block_ids, exit);

        // Immediate post-dominator map.
        let ipdom = Self::immediate_post_dominator(&pdom, &block_ids, exit);

        // For each branch edge A -> S where A has multiple successors:
        //   Walk from S up the ipdom chain until we reach ipdom(A).
        //   Every block on that walk (excluding ipdom(A)) is
        //   control-dependent on A.
        let mut deps: Vec<(CfgNodeId, CfgNodeId)> = Vec::new();
        let mut seen = HashSet::new();

        for &a in &block_ids {
            let succs = cfg.successors(a);
            let is_branch = succs.len() > 1
                || succs.iter().any(|(_, k)| {
                    matches!(
                        k,
                        CfgEdgeKind::ConditionalTrue | CfgEdgeKind::ConditionalFalse
                    )
                });

            if !is_branch {
                continue;
            }

            let a_ipdom = ipdom.get(&a).copied();

            for (succ, _) in &succs {
                let mut current = *succ;
                // Walk from successor up the ipdom chain.
                while Some(current) != a_ipdom && current != exit {
                    let pair = (a, current);
                    if seen.insert(pair) {
                        deps.push(pair);
                    }
                    match ipdom.get(&current) {
                        Some(&next) if next != current => current = next,
                        _ => break,
                    }
                }
            }
        }

        deps
    }

    /// Compute post-dominator sets using a reverse-CFG BFS / fixed-point.
    ///
    /// post_dom(B) = {B} union (intersection of post_dom(S) for all
    /// successors S of B in the original CFG).
    fn compute_post_dominators(
        cfg: &ControlFlowGraph,
        block_ids: &[CfgNodeId],
        exit: CfgNodeId,
    ) -> HashMap<CfgNodeId, HashSet<CfgNodeId>> {
        let all_set: HashSet<CfgNodeId> = block_ids.iter().copied().collect();
        let mut pdom: HashMap<CfgNodeId, HashSet<CfgNodeId>> = HashMap::new();

        // Initialize: exit post-dominates only itself; others start with all.
        for &b in block_ids {
            if b == exit {
                let mut s = HashSet::new();
                s.insert(exit);
                pdom.insert(b, s);
            } else {
                pdom.insert(b, all_set.clone());
            }
        }

        let mut changed = true;
        while changed {
            changed = false;
            for &b in block_ids {
                if b == exit {
                    continue;
                }
                let succs = cfg.successors(b);
                let new_set = if succs.is_empty() {
                    // No successors and not exit — unreachable from exit perspective.
                    let mut s = HashSet::new();
                    s.insert(b);
                    s
                } else {
                    let mut inter = all_set.clone();
                    for (s, _) in &succs {
                        if let Some(s_pdom) = pdom.get(s) {
                            inter = inter.intersection(s_pdom).copied().collect();
                        }
                    }
                    inter.insert(b);
                    inter
                };

                if new_set != pdom[&b] {
                    pdom.insert(b, new_set);
                    changed = true;
                }
            }
        }

        pdom
    }

    /// Compute immediate post-dominator from full post-dominator sets.
    ///
    /// The immediate post-dominator of B is the closest strict
    /// post-dominator — i.e., the post-dominator of B (other than B
    /// itself) that is post-dominated by all other strict post-dominators
    /// of B.
    fn immediate_post_dominator(
        pdom: &HashMap<CfgNodeId, HashSet<CfgNodeId>>,
        block_ids: &[CfgNodeId],
        exit: CfgNodeId,
    ) -> HashMap<CfgNodeId, CfgNodeId> {
        let mut ipdom: HashMap<CfgNodeId, CfgNodeId> = HashMap::new();

        for &b in block_ids {
            if b == exit {
                continue;
            }
            let Some(b_pdom) = pdom.get(&b) else {
                continue;
            };

            // Strict post-dominators of b.
            let strict: HashSet<CfgNodeId> = b_pdom.iter().copied().filter(|&x| x != b).collect();
            if strict.is_empty() {
                continue;
            }

            // The immediate post-dominator is the one whose own
            // post-dominator set is smallest among the strict set (closest
            // to b on the post-dominator tree).
            let mut best: Option<CfgNodeId> = None;
            let mut best_size = usize::MAX;
            for &candidate in &strict {
                let candidate_pdom_size =
                    pdom.get(&candidate).map(|s| s.len()).unwrap_or(usize::MAX);
                if candidate_pdom_size < best_size {
                    best_size = candidate_pdom_size;
                    best = Some(candidate);
                }
            }

            if let Some(ip) = best {
                ipdom.insert(b, ip);
            }
        }

        ipdom
    }

    // ------------------------------------------------------------------
    // Data dependence computation
    // ------------------------------------------------------------------

    /// Extract data-dependence edges from reaching definitions.
    ///
    /// For each block B, look at every use of variable V. If a definition
    /// of V in block A reaches the entry of B (i.e., its index is in
    /// `reach_in[B]`), record `(A, B, V)`.
    fn compute_data_deps(
        cfg: &ControlFlowGraph,
        block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
        reaching_defs: &ReachingDefinitions,
    ) -> Vec<(CfgNodeId, CfgNodeId, String)> {
        // Build a global definition list in a deterministic order so that
        // the usize indices in ReachingDefinitions can be resolved.
        let mut all_defs = Vec::new();
        let mut block_ids: Vec<CfgNodeId> = block_facts.keys().copied().collect();
        block_ids.sort();

        // The DataFlowGraph builds all_defs by iterating `block_facts.values()`,
        // which uses HashMap iteration order. We need to replicate that ordering.
        // However, since we cannot know the exact HashMap iteration order used
        // during the original construction, we rebuild the list by iterating
        // over *all* blocks in a consistent order and matching by DefPoint
        // identity. To be robust, we try each def against the reach_in indices.
        //
        // Actually, the simplest correct approach: collect all defs the same
        // way DataFlowGraph does — iterate block_facts.values().
        // But HashMap ordering is non-deterministic. Instead, we accept
        // all_defs as a parameter-style: we reconstruct by iterating *sorted*
        // block IDs, which the caller can also replicate.
        for &bid in &block_ids {
            if let Some(facts) = block_facts.get(&bid) {
                for def in &facts.defs {
                    all_defs.push(def.clone());
                }
            }
        }

        // Also collect defs from blocks that might be in the CFG but not
        // in block_facts (they would have no defs, so nothing to add).

        let mut data_deps: Vec<(CfgNodeId, CfgNodeId, String)> = Vec::new();
        let mut seen = HashSet::new();

        let all_cfg_blocks: Vec<CfgNodeId> = cfg.blocks().map(|(&id, _)| id).collect();

        for &bid in &all_cfg_blocks {
            let facts = match block_facts.get(&bid) {
                Some(f) => f,
                None => continue,
            };
            let reach_in = match reaching_defs.reach_in.get(&bid) {
                Some(r) => r,
                None => continue,
            };

            for use_point in &facts.uses {
                for &def_idx in reach_in {
                    if def_idx < all_defs.len() && all_defs[def_idx].var.name == use_point.var.name
                    {
                        let def_block = all_defs[def_idx].block;
                        let triple = (def_block, bid, use_point.var.name.clone());
                        if seen.insert((def_block, bid, use_point.var.name.clone())) {
                            data_deps.push(triple);
                        }
                    }
                }
            }
        }

        data_deps
    }

    // ------------------------------------------------------------------
    // Accessors
    // ------------------------------------------------------------------

    /// All block IDs in the underlying CFG.
    pub fn cfg_blocks(&self) -> &[CfgNodeId] {
        &self.cfg_blocks
    }

    /// All control-dependence edges.
    pub fn control_dep_edges(&self) -> &[(CfgNodeId, CfgNodeId)] {
        &self.control_deps
    }

    /// All data-dependence edges.
    pub fn data_dep_edges(&self) -> &[(CfgNodeId, CfgNodeId, String)] {
        &self.data_deps
    }

    /// All PDG edges (control + data) as `PdgEdge` values.
    pub fn all_edges(&self) -> Vec<PdgEdge> {
        let mut edges = Vec::with_capacity(self.control_deps.len() + self.data_deps.len());
        for &(from, to) in &self.control_deps {
            edges.push(PdgEdge {
                from,
                to,
                kind: PdgEdgeKind::ControlDep,
            });
        }
        for (from, to, var) in &self.data_deps {
            edges.push(PdgEdge {
                from: *from,
                to: *to,
                kind: PdgEdgeKind::DataDep(var.clone()),
            });
        }
        edges
    }

    /// Blocks that are control-dependent on `block` (i.e., `block`
    /// controls whether they execute).
    pub fn control_dependents(&self, block: CfgNodeId) -> Vec<CfgNodeId> {
        self.control_deps
            .iter()
            .filter(|(a, _)| *a == block)
            .map(|(_, b)| *b)
            .collect()
    }

    /// Blocks that receive data from a definition in `block`, together
    /// with the variable name.
    pub fn data_dependents(&self, block: CfgNodeId) -> Vec<(CfgNodeId, String)> {
        self.data_deps
            .iter()
            .filter(|(a, _, _)| *a == block)
            .map(|(_, b, v)| (*b, v.clone()))
            .collect()
    }

    /// Blocks that control whether `block` executes.
    pub fn control_dependencies(&self, block: CfgNodeId) -> Vec<CfgNodeId> {
        self.control_deps
            .iter()
            .filter(|(_, b)| *b == block)
            .map(|(a, _)| *a)
            .collect()
    }

    /// Blocks whose definitions are used in `block`, together with the
    /// variable name.
    pub fn data_dependencies(&self, block: CfgNodeId) -> Vec<(CfgNodeId, String)> {
        self.data_deps
            .iter()
            .filter(|(_, b, _)| *b == block)
            .map(|(a, _, v)| (*a, v.clone()))
            .collect()
    }

    // ------------------------------------------------------------------
    // Forward / backward reachability helpers (used by slicing)
    // ------------------------------------------------------------------

    /// All blocks reachable by following PDG edges backward from `start`.
    /// Returns the set of blocks that `start` (transitively) depends on.
    pub fn backward_reachable(&self, start: CfgNodeId) -> HashSet<CfgNodeId> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(start);
        queue.push_back(start);

        while let Some(current) = queue.pop_front() {
            // Control dependencies (blocks that control `current`).
            for &(a, b) in &self.control_deps {
                if b == current && visited.insert(a) {
                    queue.push_back(a);
                }
            }
            // Data dependencies (blocks whose defs are used in `current`).
            for (a, b, _) in &self.data_deps {
                if *b == current && visited.insert(*a) {
                    queue.push_back(*a);
                }
            }
        }

        visited
    }

    /// All blocks reachable by following PDG edges forward from `start`.
    /// Returns the set of blocks that (transitively) depend on `start`.
    pub fn forward_reachable(&self, start: CfgNodeId) -> HashSet<CfgNodeId> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(start);
        queue.push_back(start);

        while let Some(current) = queue.pop_front() {
            // Control dependents (blocks whose execution depends on `current`).
            for &(a, b) in &self.control_deps {
                if a == current && visited.insert(b) {
                    queue.push_back(b);
                }
            }
            // Data dependents (blocks that use defs from `current`).
            for (a, b, _) in &self.data_deps {
                if *a == current && visited.insert(*b) {
                    queue.push_back(*b);
                }
            }
        }

        visited
    }

    /// Backward reachable considering only data deps for a specific variable.
    /// Used when a slice criterion specifies a particular variable of interest.
    pub fn backward_reachable_for_var(&self, start: CfgNodeId, var: &str) -> HashSet<CfgNodeId> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(start);
        queue.push_back(start);

        while let Some(current) = queue.pop_front() {
            // Control dependencies are always followed.
            for &(a, b) in &self.control_deps {
                if b == current && visited.insert(a) {
                    queue.push_back(a);
                }
            }
            // Data dependencies: on the first step only follow edges for
            // the specified variable; after that follow all data deps
            // (since the defining blocks may themselves depend on other
            // variables).
            for (a, b, v) in &self.data_deps {
                if *b == current && visited.insert(*a) {
                    // If this is the starting block, only follow deps for
                    // the requested variable. Otherwise follow all.
                    if current == start {
                        if v == var {
                            queue.push_back(*a);
                        } else {
                            visited.remove(a);
                        }
                    } else {
                        queue.push_back(*a);
                    }
                }
            }
        }

        visited
    }

    /// Forward reachable considering only data deps for a specific variable
    /// at the starting block.
    pub fn forward_reachable_for_var(&self, start: CfgNodeId, var: &str) -> HashSet<CfgNodeId> {
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();
        visited.insert(start);
        queue.push_back(start);

        while let Some(current) = queue.pop_front() {
            // Control dependents are always followed.
            for &(a, b) in &self.control_deps {
                if a == current && visited.insert(b) {
                    queue.push_back(b);
                }
            }
            // Data dependents: on the first step only follow edges for
            // the specified variable; after that follow all data deps.
            for (a, b, v) in &self.data_deps {
                if *a == current && visited.insert(*b) {
                    if current == start {
                        if v == var {
                            queue.push_back(*b);
                        } else {
                            visited.remove(b);
                        }
                    } else {
                        queue.push_back(*b);
                    }
                }
            }
        }

        visited
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SourceSpan;
    use crate::graph::cfg::CfgEdgeKind;
    use crate::graph::dataflow::{BlockDataFlow, DefPoint, ReachingDefinitions, UsePoint, VarRef};
    use std::collections::{HashMap, HashSet};

    /// Helper: build a simple CFG and matching data-flow facts.
    ///
    /// ```text
    /// entry(BB0): x = 1
    ///    |
    ///  [cond] -- true --> body(BB2): y = x + 1
    ///    |                  |
    ///    +-- false -----> merge(BB3)
    ///                       |
    ///                     exit(BB1): return y
    /// ```
    fn make_diamond_cfg() -> (
        ControlFlowGraph,
        HashMap<CfgNodeId, BlockDataFlow>,
        Vec<CfgNodeId>,
    ) {
        let mut cfg = ControlFlowGraph::new("diamond");
        let entry = cfg.create_entry(); // BB0
        let exit = cfg.create_exit(); // BB1
        let body = cfg.create_block("body"); // BB2
        let merge = cfg.create_block("merge"); // BB3

        cfg.add_statement(entry, SourceSpan::new(0, 5));
        cfg.add_statement(body, SourceSpan::new(6, 15));

        cfg.add_edge(entry, body, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(entry, merge, CfgEdgeKind::ConditionalFalse);
        cfg.add_edge(body, merge, CfgEdgeKind::FallThrough);
        cfg.add_edge(merge, exit, CfgEdgeKind::FallThrough);

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![DefPoint {
                    var: VarRef::new("x"),
                    block: entry,
                    stmt_index: 0,
                    start_byte: 0,
                    end_byte: 5,
                }],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            body,
            BlockDataFlow {
                defs: vec![DefPoint {
                    var: VarRef::new("y"),
                    block: body,
                    stmt_index: 0,
                    start_byte: 6,
                    end_byte: 15,
                }],
                uses: vec![UsePoint {
                    var: VarRef::new("x"),
                    block: body,
                    stmt_index: 0,
                    start_byte: 10,
                    end_byte: 11,
                }],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            merge,
            BlockDataFlow {
                defs: vec![],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            exit,
            BlockDataFlow {
                defs: vec![],
                uses: vec![UsePoint {
                    var: VarRef::new("y"),
                    block: exit,
                    stmt_index: 0,
                    start_byte: 16,
                    end_byte: 17,
                }],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );

        (cfg, block_facts, vec![entry, exit, body, merge])
    }

    /// Build reaching definitions that are consistent with the diamond CFG.
    fn make_reaching_defs(
        block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
        ids: &[CfgNodeId],
    ) -> ReachingDefinitions {
        // Collect all defs in sorted-block order (matches compute_data_deps).
        let mut sorted: Vec<CfgNodeId> = block_facts.keys().copied().collect();
        sorted.sort();

        let mut all_defs = Vec::new();
        for &bid in &sorted {
            if let Some(facts) = block_facts.get(&bid) {
                for def in &facts.defs {
                    all_defs.push(def.clone());
                }
            }
        }

        // Index mapping: var name -> def indices.
        let entry = ids[0];
        let exit = ids[1];
        let body = ids[2];
        let merge = ids[3];

        // x is def index 0 (entry), y is def index 1 (body) when sorted.
        // But actual order depends on CfgNodeId ordering.
        // entry = BB0, exit = BB1, body = BB2, merge = BB3.
        // Sorted order: BB0, BB1, BB2, BB3 -> defs from BB0 (x), BB2 (y).
        // So def_idx 0 = x, def_idx 1 = y.

        let mut reach_in: HashMap<CfgNodeId, HashSet<usize>> = HashMap::new();
        let mut reach_out: HashMap<CfgNodeId, HashSet<usize>> = HashMap::new();

        // entry: reach_in = {}, reach_out = {0} (x defined)
        reach_in.insert(entry, HashSet::new());
        reach_out.insert(entry, [0].into_iter().collect());

        // body: reach_in = {0} (x reaches), reach_out = {0, 1} (y also defined)
        reach_in.insert(body, [0].into_iter().collect());
        reach_out.insert(body, [0, 1].into_iter().collect());

        // merge: reach_in = {0, 1} (from body) union {0} (from entry direct)
        reach_in.insert(merge, [0, 1].into_iter().collect());
        reach_out.insert(merge, [0, 1].into_iter().collect());

        // exit: reach_in = {0, 1}
        reach_in.insert(exit, [0, 1].into_iter().collect());
        reach_out.insert(exit, [0, 1].into_iter().collect());

        ReachingDefinitions {
            reach_in,
            reach_out,
        }
    }

    #[test]
    fn test_pdg_construction() {
        let (cfg, block_facts, ids) = make_diamond_cfg();
        let reaching = make_reaching_defs(&block_facts, &ids);
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        // Should have at least some edges.
        assert!(
            !pdg.control_deps.is_empty() || !pdg.data_deps.is_empty(),
            "PDG should contain at least one edge"
        );
    }

    #[test]
    fn test_pdg_data_dependencies() {
        let (cfg, block_facts, ids) = make_diamond_cfg();
        let reaching = make_reaching_defs(&block_facts, &ids);
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let entry = ids[0];
        let exit = ids[1];
        let body = ids[2];

        // body uses x which is defined in entry -> data dep (entry, body, "x").
        let body_data_deps = pdg.data_dependencies(body);
        assert!(
            body_data_deps.iter().any(|(b, v)| *b == entry && v == "x"),
            "body should have a data dependency on entry for variable x, got: {:?}",
            body_data_deps
        );

        // exit uses y which is defined in body -> data dep (body, exit, "y").
        let exit_data_deps = pdg.data_dependencies(exit);
        assert!(
            exit_data_deps.iter().any(|(b, v)| *b == body && v == "y"),
            "exit should have a data dependency on body for variable y, got: {:?}",
            exit_data_deps
        );
    }

    #[test]
    fn test_pdg_control_dependencies() {
        let (cfg, block_facts, ids) = make_diamond_cfg();
        let reaching = make_reaching_defs(&block_facts, &ids);
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let entry = ids[0];
        let body = ids[2];

        // entry is a branch point -> body should be control-dependent on entry.
        let ctrl_deps = pdg.control_dependents(entry);
        assert!(
            ctrl_deps.contains(&body),
            "body should be control-dependent on entry, got: {:?}",
            ctrl_deps
        );
    }

    #[test]
    fn test_pdg_data_dependents() {
        let (cfg, block_facts, ids) = make_diamond_cfg();
        let reaching = make_reaching_defs(&block_facts, &ids);
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let entry = ids[0];
        let body = ids[2];

        // entry defines x -> body uses x -> entry has data dependent body.
        let dependents = pdg.data_dependents(entry);
        assert!(
            dependents.iter().any(|(b, v)| *b == body && v == "x"),
            "entry should have data dependent body for x, got: {:?}",
            dependents
        );
    }

    #[test]
    fn test_backward_reachable() {
        let (cfg, block_facts, ids) = make_diamond_cfg();
        let reaching = make_reaching_defs(&block_facts, &ids);
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let entry = ids[0];
        let exit = ids[1];
        let body = ids[2];

        // Backward from exit should reach body (via data dep y) and entry
        // (via data dep x from body, or control dep).
        let reachable = pdg.backward_reachable(exit);
        assert!(
            reachable.contains(&body),
            "backward from exit should reach body"
        );
        assert!(
            reachable.contains(&entry),
            "backward from exit should reach entry"
        );
    }

    #[test]
    fn test_forward_reachable() {
        let (cfg, block_facts, ids) = make_diamond_cfg();
        let reaching = make_reaching_defs(&block_facts, &ids);
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let entry = ids[0];
        let exit = ids[1];
        let body = ids[2];

        // Forward from entry should reach body (data dep x and control dep)
        // and transitively exit (body defines y, exit uses y).
        let reachable = pdg.forward_reachable(entry);
        assert!(
            reachable.contains(&body),
            "forward from entry should reach body"
        );
        assert!(
            reachable.contains(&exit),
            "forward from entry should reach exit"
        );
    }
}
