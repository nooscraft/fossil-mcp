//! Program slicing: backward and forward slicing over the PDG.
//!
//! A *program slice* is the subset of a program that can affect (forward)
//! or is affected by (backward) a given *criterion* — a `(block, variable)`
//! pair. Slicing is a powerful technique for dead-code detection: any
//! block that does not appear in the backward slice from any exit block
//! cannot influence program output and is therefore dead.

use std::collections::HashSet;

use super::cfg::{CfgNodeId, ControlFlowGraph};
use super::pdg::ProgramDependenceGraph;

// ---------------------------------------------------------------------------
// Slice criterion and result
// ---------------------------------------------------------------------------

/// A slicing criterion: a block and optionally a variable of interest.
///
/// When `variable` is `None`, all variables at the block are considered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceCriterion {
    pub block: CfgNodeId,
    pub variable: Option<String>,
}

/// The result of a slicing operation: the set of blocks in the slice
/// together with the criterion that produced it.
#[derive(Debug, Clone)]
pub struct ProgramSlice {
    pub blocks: HashSet<CfgNodeId>,
    pub criterion: SliceCriterion,
}

impl ProgramSlice {
    /// Whether a given block is part of this slice.
    pub fn contains(&self, block: CfgNodeId) -> bool {
        self.blocks.contains(&block)
    }

    /// Number of blocks in the slice.
    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    /// Whether the slice is empty.
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Slicing algorithms
// ---------------------------------------------------------------------------

/// Compute the **backward slice** from the given criterion.
///
/// The backward slice of `(block, variable)` is the set of all blocks
/// that can transitively influence the value of `variable` at `block`
/// (or, when no variable is specified, any computation at `block`).
///
/// Implementation: BFS backward on PDG edges (both control and data
/// dependence) starting from the criterion block.
pub fn backward_slice(pdg: &ProgramDependenceGraph, criterion: &SliceCriterion) -> ProgramSlice {
    let blocks = match &criterion.variable {
        Some(var) => pdg.backward_reachable_for_var(criterion.block, var),
        None => pdg.backward_reachable(criterion.block),
    };

    ProgramSlice {
        blocks,
        criterion: criterion.clone(),
    }
}

/// Compute the **forward slice** from the given criterion.
///
/// The forward slice of `(block, variable)` is the set of all blocks
/// that are transitively affected by the computation at `block`.
///
/// Implementation: BFS forward on PDG edges starting from the criterion
/// block.
pub fn forward_slice(pdg: &ProgramDependenceGraph, criterion: &SliceCriterion) -> ProgramSlice {
    let blocks = match &criterion.variable {
        Some(var) => pdg.forward_reachable_for_var(criterion.block, var),
        None => pdg.forward_reachable(criterion.block),
    };

    ProgramSlice {
        blocks,
        criterion: criterion.clone(),
    }
}

/// Detect dead blocks via slicing.
///
/// A block is considered dead if it does not appear in the backward slice
/// of **any** exit block (with no variable filter). Such blocks cannot
/// influence any program output.
///
/// Returns the list of dead `CfgNodeId`s (excludes the exit block itself
/// to avoid false positives on empty exit blocks).
pub fn find_dead_by_slicing(
    pdg: &ProgramDependenceGraph,
    cfg: &ControlFlowGraph,
) -> Vec<CfgNodeId> {
    // Collect all exit blocks.
    let exit_blocks: Vec<CfgNodeId> = cfg
        .blocks()
        .filter(|(_, bb)| bb.is_exit)
        .map(|(&id, _)| id)
        .collect();

    // Union of backward slices from every exit block.
    let mut live_blocks: HashSet<CfgNodeId> = HashSet::new();
    for &exit_id in &exit_blocks {
        let criterion = SliceCriterion {
            block: exit_id,
            variable: None,
        };
        let slice = backward_slice(pdg, &criterion);
        live_blocks.extend(slice.blocks);
    }

    // Also consider the entry block as always live (it is the program start).
    if let Some(entry) = cfg.entry() {
        live_blocks.insert(entry);
    }

    // Also keep exit blocks themselves live.
    for &exit_id in &exit_blocks {
        live_blocks.insert(exit_id);
    }

    // Every block NOT in the live set is dead.
    let all_blocks: Vec<CfgNodeId> = cfg.blocks().map(|(&id, _)| id).collect();
    let mut dead: Vec<CfgNodeId> = all_blocks
        .into_iter()
        .filter(|id| !live_blocks.contains(id))
        .collect();
    dead.sort();
    dead
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SourceSpan;
    use crate::graph::cfg::{CfgEdgeKind, ControlFlowGraph};
    use crate::graph::dataflow::{BlockDataFlow, DefPoint, ReachingDefinitions, UsePoint, VarRef};
    use crate::graph::pdg::ProgramDependenceGraph;
    use std::collections::{HashMap, HashSet};

    /// Helper: linear CFG with data flow.
    ///
    /// ```text
    /// entry(BB0): x = 1
    ///    |
    /// body(BB2): y = x + 1
    ///    |
    /// exit(BB1): return y
    /// ```
    fn make_linear_cfg() -> (
        ControlFlowGraph,
        HashMap<CfgNodeId, BlockDataFlow>,
        ReachingDefinitions,
        Vec<CfgNodeId>,
    ) {
        let mut cfg = ControlFlowGraph::new("linear");
        let entry = cfg.create_entry(); // BB0
        let exit = cfg.create_exit(); // BB1
        let body = cfg.create_block("body"); // BB2

        cfg.add_statement(entry, SourceSpan::new(0, 5));
        cfg.add_statement(body, SourceSpan::new(6, 15));

        cfg.add_edge(entry, body, CfgEdgeKind::FallThrough);
        cfg.add_edge(body, exit, CfgEdgeKind::FallThrough);

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

        // Build reaching defs.
        // Sorted block order: BB0(entry), BB1(exit), BB2(body).
        // Defs: idx 0 = x (BB0), idx 1 = y (BB2).
        let mut reach_in = HashMap::new();
        let mut reach_out = HashMap::new();

        reach_in.insert(entry, HashSet::new());
        reach_out.insert(entry, [0].into_iter().collect());

        reach_in.insert(body, [0].into_iter().collect());
        reach_out.insert(body, [0, 1].into_iter().collect());

        reach_in.insert(exit, [0, 1].into_iter().collect());
        reach_out.insert(exit, [0, 1].into_iter().collect());

        let reaching = ReachingDefinitions {
            reach_in,
            reach_out,
        };

        (cfg, block_facts, reaching, vec![entry, exit, body])
    }

    /// Helper: CFG with a dead block.
    ///
    /// ```text
    /// entry(BB0): x = 1
    ///    |
    /// exit(BB1): return x
    ///
    /// dead(BB2): z = 99   (no edges to/from main flow)
    /// ```
    fn make_cfg_with_dead_block() -> (
        ControlFlowGraph,
        HashMap<CfgNodeId, BlockDataFlow>,
        ReachingDefinitions,
        Vec<CfgNodeId>,
    ) {
        let mut cfg = ControlFlowGraph::new("with_dead");
        let entry = cfg.create_entry(); // BB0
        let exit = cfg.create_exit(); // BB1
        let dead = cfg.create_block("dead"); // BB2

        cfg.add_statement(entry, SourceSpan::new(0, 5));
        cfg.add_statement(dead, SourceSpan::new(20, 30));

        cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);
        // dead block has no edges connecting to exit.

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
            exit,
            BlockDataFlow {
                defs: vec![],
                uses: vec![UsePoint {
                    var: VarRef::new("x"),
                    block: exit,
                    stmt_index: 0,
                    start_byte: 6,
                    end_byte: 7,
                }],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            dead,
            BlockDataFlow {
                defs: vec![DefPoint {
                    var: VarRef::new("z"),
                    block: dead,
                    stmt_index: 0,
                    start_byte: 20,
                    end_byte: 30,
                }],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );

        // Reaching defs: sorted blocks BB0, BB1, BB2.
        // Defs: idx 0 = x (BB0), idx 1 = z (BB2).
        let mut reach_in = HashMap::new();
        let mut reach_out = HashMap::new();

        reach_in.insert(entry, HashSet::new());
        reach_out.insert(entry, [0].into_iter().collect());

        reach_in.insert(exit, [0].into_iter().collect());
        reach_out.insert(exit, [0].into_iter().collect());

        // dead block is isolated.
        reach_in.insert(dead, HashSet::new());
        reach_out.insert(dead, [1].into_iter().collect());

        let reaching = ReachingDefinitions {
            reach_in,
            reach_out,
        };

        (cfg, block_facts, reaching, vec![entry, exit, dead])
    }

    #[test]
    fn test_backward_slice_includes_correct_blocks() {
        let (cfg, block_facts, reaching, ids) = make_linear_cfg();
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let entry = ids[0];
        let exit = ids[1];
        let body = ids[2];

        // Backward slice from exit with variable "y".
        let criterion = SliceCriterion {
            block: exit,
            variable: Some("y".to_string()),
        };
        let slice = backward_slice(&pdg, &criterion);

        // body defines y -> should be in the slice.
        assert!(
            slice.contains(body),
            "backward slice for y at exit should contain body (defines y)"
        );
        // entry defines x which body uses -> transitively in the slice.
        assert!(
            slice.contains(entry),
            "backward slice for y at exit should contain entry (defines x used by body)"
        );
        // exit itself is always in the slice.
        assert!(
            slice.contains(exit),
            "slice should contain the criterion block"
        );
    }

    #[test]
    fn test_backward_slice_without_variable() {
        let (cfg, block_facts, reaching, ids) = make_linear_cfg();
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let entry = ids[0];
        let exit = ids[1];
        let body = ids[2];

        // Backward slice from exit with no variable filter.
        let criterion = SliceCriterion {
            block: exit,
            variable: None,
        };
        let slice = backward_slice(&pdg, &criterion);

        assert!(slice.contains(exit));
        assert!(slice.contains(body));
        assert!(slice.contains(entry));
    }

    #[test]
    fn test_forward_slice_includes_correct_blocks() {
        let (cfg, block_facts, reaching, ids) = make_linear_cfg();
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let entry = ids[0];
        let exit = ids[1];
        let body = ids[2];

        // Forward slice from entry with variable "x".
        let criterion = SliceCriterion {
            block: entry,
            variable: Some("x".to_string()),
        };
        let slice = forward_slice(&pdg, &criterion);

        // body uses x -> in the slice.
        assert!(
            slice.contains(body),
            "forward slice for x at entry should contain body (uses x)"
        );
        // exit uses y defined in body -> transitively in the slice.
        assert!(
            slice.contains(exit),
            "forward slice for x at entry should contain exit (uses y from body)"
        );
        assert!(
            slice.contains(entry),
            "slice should contain the criterion block"
        );
    }

    #[test]
    fn test_forward_slice_without_variable() {
        let (cfg, block_facts, reaching, ids) = make_linear_cfg();
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let entry = ids[0];
        let exit = ids[1];
        let body = ids[2];

        let criterion = SliceCriterion {
            block: entry,
            variable: None,
        };
        let slice = forward_slice(&pdg, &criterion);

        assert!(slice.contains(entry));
        assert!(slice.contains(body));
        assert!(slice.contains(exit));
    }

    #[test]
    fn test_dead_code_detection_via_slicing() {
        let (cfg, block_facts, reaching, ids) = make_cfg_with_dead_block();
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let dead = ids[2];

        let dead_blocks = find_dead_by_slicing(&pdg, &cfg);

        assert!(
            dead_blocks.contains(&dead),
            "dead block should be detected as dead, got: {:?}",
            dead_blocks
        );
    }

    #[test]
    fn test_no_false_positives_on_live_blocks() {
        let (cfg, block_facts, reaching, ids) = make_linear_cfg();
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let entry = ids[0];
        let exit = ids[1];
        let body = ids[2];

        let dead_blocks = find_dead_by_slicing(&pdg, &cfg);

        assert!(!dead_blocks.contains(&entry), "entry should not be dead");
        assert!(!dead_blocks.contains(&exit), "exit should not be dead");
        assert!(!dead_blocks.contains(&body), "body should not be dead");
    }

    #[test]
    fn test_slice_len_and_is_empty() {
        let (cfg, block_facts, reaching, ids) = make_linear_cfg();
        let pdg = ProgramDependenceGraph::build(&cfg, &block_facts, &reaching);

        let exit = ids[1];
        let criterion = SliceCriterion {
            block: exit,
            variable: None,
        };
        let slice = backward_slice(&pdg, &criterion);

        assert!(!slice.is_empty());
        assert!(!slice.is_empty());
    }
}
