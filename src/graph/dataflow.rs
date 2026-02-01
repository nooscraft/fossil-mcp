//! Data flow analysis framework: reaching definitions, liveness, def-use chains.
//!
//! Provides forward and backward fixed-point analyses over the CFG.

use std::collections::{HashMap, HashSet};

use super::cfg::{CfgNodeId, ControlFlowGraph};

/// A variable reference.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VarRef {
    pub name: String,
    pub scope: Option<String>,
}

impl VarRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            scope: None,
        }
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }
}

/// A definition point (where a variable is assigned).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DefPoint {
    pub var: VarRef,
    pub block: CfgNodeId,
    pub stmt_index: usize,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// A use point (where a variable is read).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct UsePoint {
    pub var: VarRef,
    pub block: CfgNodeId,
    pub stmt_index: usize,
    pub start_byte: usize,
    pub end_byte: usize,
}

/// Per-block data flow facts.
#[derive(Debug, Clone, Default)]
pub struct BlockDataFlow {
    pub defs: Vec<DefPoint>,
    pub uses: Vec<UsePoint>,
    /// GEN set: definitions generated in this block.
    pub gen: HashSet<usize>,
    /// KILL set: definitions killed in this block.
    pub kill: HashSet<usize>,
}

/// Result of reaching definitions analysis (forward).
#[derive(Debug, Clone)]
pub struct ReachingDefinitions {
    /// Definitions reaching the entry of each block.
    pub reach_in: HashMap<CfgNodeId, HashSet<usize>>,
    /// Definitions reaching the exit of each block.
    pub reach_out: HashMap<CfgNodeId, HashSet<usize>>,
}

/// Result of liveness analysis (backward).
#[derive(Debug, Clone)]
pub struct LivenessResult {
    /// Variables live at the entry of each block.
    pub live_in: HashMap<CfgNodeId, HashSet<VarRef>>,
    /// Variables live at the exit of each block.
    pub live_out: HashMap<CfgNodeId, HashSet<VarRef>>,
}

/// A def-use chain entry: a definition and all its uses.
#[derive(Debug, Clone)]
pub struct DefUseChain {
    pub def: DefPoint,
    pub uses: Vec<UsePoint>,
}

/// Complete data flow graph for a function.
#[derive(Debug)]
pub struct DataFlowGraph {
    /// The underlying CFG.
    cfg: ControlFlowGraph,
    /// All definitions across all blocks (indexed by position in this vec).
    all_defs: Vec<DefPoint>,
    /// Per-block data flow facts.
    block_facts: HashMap<CfgNodeId, BlockDataFlow>,
    /// Reaching definitions result (computed lazily).
    reaching_defs: Option<ReachingDefinitions>,
    /// Liveness result (computed lazily).
    liveness: Option<LivenessResult>,
    /// Def-use chains (computed lazily).
    def_use_chains: Option<Vec<DefUseChain>>,
}

impl DataFlowGraph {
    /// Create a new DataFlowGraph from a CFG and extracted facts.
    pub fn new(cfg: ControlFlowGraph, block_facts: HashMap<CfgNodeId, BlockDataFlow>) -> Self {
        let mut all_defs = Vec::new();
        for facts in block_facts.values() {
            all_defs.extend(facts.defs.iter().cloned());
        }

        Self {
            cfg,
            all_defs,
            block_facts,
            reaching_defs: None,
            liveness: None,
            def_use_chains: None,
        }
    }

    /// Get the underlying CFG.
    pub fn cfg(&self) -> &ControlFlowGraph {
        &self.cfg
    }

    /// Get all definitions.
    pub fn all_defs(&self) -> &[DefPoint] {
        &self.all_defs
    }

    /// Get block facts.
    pub fn block_facts(&self) -> &HashMap<CfgNodeId, BlockDataFlow> {
        &self.block_facts
    }

    /// Compute reaching definitions (forward fixed-point).
    pub fn compute_reaching_definitions(&mut self) -> &ReachingDefinitions {
        if let Some(ref rd) = self.reaching_defs {
            return rd;
        }

        let block_ids: Vec<CfgNodeId> = self.block_facts.keys().copied().collect();

        let mut reach_in: HashMap<CfgNodeId, HashSet<usize>> = HashMap::new();
        let mut reach_out: HashMap<CfgNodeId, HashSet<usize>> = HashMap::new();

        for &bid in &block_ids {
            reach_in.insert(bid, HashSet::new());
            reach_out.insert(bid, HashSet::new());
        }

        // Build gen/kill sets indexed by global def position
        let mut block_gen: HashMap<CfgNodeId, HashSet<usize>> = HashMap::new();
        let mut block_kill: HashMap<CfgNodeId, HashSet<usize>> = HashMap::new();

        // Index: variable name -> set of def indices
        let mut var_to_defs: HashMap<String, HashSet<usize>> = HashMap::new();
        for (i, def) in self.all_defs.iter().enumerate() {
            var_to_defs
                .entry(def.var.name.clone())
                .or_default()
                .insert(i);
        }

        for (&bid, facts) in &self.block_facts {
            let mut gen = HashSet::new();
            let mut kill = HashSet::new();

            for def in &facts.defs {
                // Find global index of this def
                if let Some(idx) = self.all_defs.iter().position(|d| d == def) {
                    gen.insert(idx);
                    // Kill all other defs of the same variable
                    if let Some(other_defs) = var_to_defs.get(&def.var.name) {
                        for &other_idx in other_defs {
                            if other_idx != idx {
                                kill.insert(other_idx);
                            }
                        }
                    }
                }
            }

            block_gen.insert(bid, gen);
            block_kill.insert(bid, kill);
        }

        // Fixed-point iteration
        let mut changed = true;
        while changed {
            changed = false;
            for &bid in &block_ids {
                // reach_in[B] = union of reach_out[P] for all predecessors P
                let mut new_in = HashSet::new();
                for (pred_id, _) in self.cfg.predecessors(bid) {
                    if let Some(pred_out) = reach_out.get(&pred_id) {
                        new_in = new_in.union(pred_out).copied().collect();
                    }
                }

                // reach_out[B] = gen[B] ∪ (reach_in[B] - kill[B])
                let gen = block_gen.get(&bid).cloned().unwrap_or_default();
                let kill = block_kill.get(&bid).cloned().unwrap_or_default();
                let mut new_out: HashSet<usize> = new_in.difference(&kill).copied().collect();
                new_out = new_out.union(&gen).copied().collect();

                if new_out != *reach_out.get(&bid).unwrap_or(&HashSet::new()) {
                    changed = true;
                }

                reach_in.insert(bid, new_in);
                reach_out.insert(bid, new_out);
            }
        }

        self.reaching_defs = Some(ReachingDefinitions {
            reach_in,
            reach_out,
        });
        self.reaching_defs.as_ref().unwrap()
    }

    /// Compute liveness analysis (backward fixed-point).
    pub fn compute_liveness(&mut self) -> &LivenessResult {
        if let Some(ref lv) = self.liveness {
            return lv;
        }

        let block_ids: Vec<CfgNodeId> = self.block_facts.keys().copied().collect();

        let mut live_in: HashMap<CfgNodeId, HashSet<VarRef>> = HashMap::new();
        let mut live_out: HashMap<CfgNodeId, HashSet<VarRef>> = HashMap::new();

        for &bid in &block_ids {
            live_in.insert(bid, HashSet::new());
            live_out.insert(bid, HashSet::new());
        }

        // Per-block use and def variable sets
        let mut block_use: HashMap<CfgNodeId, HashSet<VarRef>> = HashMap::new();
        let mut block_def: HashMap<CfgNodeId, HashSet<VarRef>> = HashMap::new();

        for (&bid, facts) in &self.block_facts {
            let uses: HashSet<VarRef> = facts.uses.iter().map(|u| u.var.clone()).collect();
            let defs: HashSet<VarRef> = facts.defs.iter().map(|d| d.var.clone()).collect();
            block_use.insert(bid, uses);
            block_def.insert(bid, defs);
        }

        // Backward fixed-point
        let mut changed = true;
        while changed {
            changed = false;
            for &bid in &block_ids {
                // live_out[B] = union of live_in[S] for all successors S
                let mut new_out: HashSet<VarRef> = HashSet::new();
                for (succ_id, _) in self.cfg.successors(bid) {
                    if let Some(succ_in) = live_in.get(&succ_id) {
                        new_out = new_out.union(succ_in).cloned().collect();
                    }
                }

                // live_in[B] = use[B] ∪ (live_out[B] - def[B])
                let uses = block_use.get(&bid).cloned().unwrap_or_default();
                let defs = block_def.get(&bid).cloned().unwrap_or_default();
                let mut new_in: HashSet<VarRef> = new_out.difference(&defs).cloned().collect();
                new_in = new_in.union(&uses).cloned().collect();

                if new_in != *live_in.get(&bid).unwrap_or(&HashSet::new()) {
                    changed = true;
                }

                live_in.insert(bid, new_in);
                live_out.insert(bid, new_out);
            }
        }

        self.liveness = Some(LivenessResult { live_in, live_out });
        self.liveness.as_ref().unwrap()
    }

    /// Build def-use chains from reaching definitions.
    pub fn build_def_use_chains(&mut self) -> &[DefUseChain] {
        if let Some(ref duc) = self.def_use_chains {
            return duc;
        }

        // Ensure reaching definitions are computed
        if self.reaching_defs.is_none() {
            self.compute_reaching_definitions();
        }

        let reaching = self.reaching_defs.as_ref().unwrap();
        let mut chains: HashMap<usize, Vec<UsePoint>> = HashMap::new();

        for (&bid, facts) in &self.block_facts {
            let reach_in = reaching.reach_in.get(&bid).cloned().unwrap_or_default();
            for use_point in &facts.uses {
                // Find which reaching definitions could be used here
                for &def_idx in &reach_in {
                    if def_idx < self.all_defs.len()
                        && self.all_defs[def_idx].var.name == use_point.var.name
                    {
                        chains.entry(def_idx).or_default().push(use_point.clone());
                    }
                }
            }
        }

        let result: Vec<DefUseChain> = self
            .all_defs
            .iter()
            .enumerate()
            .map(|(i, def)| DefUseChain {
                def: def.clone(),
                uses: chains.remove(&i).unwrap_or_default(),
            })
            .collect();

        self.def_use_chains = Some(result);
        self.def_use_chains.as_ref().unwrap()
    }

    /// Find dead stores: definitions that are never used.
    pub fn find_dead_stores(&mut self) -> Vec<&DefPoint> {
        let chains = self.build_def_use_chains();
        chains
            .iter()
            .filter(|chain| chain.uses.is_empty())
            .map(|chain| &chain.def)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SourceSpan;
    use crate::graph::cfg::{CfgEdgeKind, ControlFlowGraph};

    fn make_var(name: &str) -> VarRef {
        VarRef::new(name)
    }

    #[test]
    fn test_reaching_definitions_simple() {
        // BB0 (entry): x = 1
        // BB1 (body):  y = x + 1
        // BB2 (exit):  return y
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let body = cfg.create_block("body");
        let exit = cfg.create_exit();
        cfg.add_edge(entry, body, CfgEdgeKind::FallThrough);
        cfg.add_edge(body, exit, CfgEdgeKind::FallThrough);
        cfg.add_statement(entry, SourceSpan::new(0, 5));
        cfg.add_statement(body, SourceSpan::new(6, 15));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![DefPoint {
                    var: make_var("x"),
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
                    var: make_var("y"),
                    block: body,
                    stmt_index: 0,
                    start_byte: 6,
                    end_byte: 15,
                }],
                uses: vec![UsePoint {
                    var: make_var("x"),
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
                    var: make_var("y"),
                    block: exit,
                    stmt_index: 0,
                    start_byte: 16,
                    end_byte: 17,
                }],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );

        let mut dfg = DataFlowGraph::new(cfg, block_facts);
        let rd = dfg.compute_reaching_definitions();
        // x's definition should reach body
        assert!(!rd.reach_in.get(&body).unwrap().is_empty());
    }

    #[test]
    fn test_dead_store_detection() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);
        cfg.add_statement(entry, SourceSpan::new(0, 10));

        let mut block_facts = HashMap::new();
        // x = 1; y = 2; (x is never used -> dead store)
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![
                    DefPoint {
                        var: make_var("x"),
                        block: entry,
                        stmt_index: 0,
                        start_byte: 0,
                        end_byte: 5,
                    },
                    DefPoint {
                        var: make_var("y"),
                        block: entry,
                        stmt_index: 1,
                        start_byte: 6,
                        end_byte: 10,
                    },
                ],
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
                    var: make_var("y"),
                    block: exit,
                    stmt_index: 0,
                    start_byte: 11,
                    end_byte: 12,
                }],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );

        let mut dfg = DataFlowGraph::new(cfg, block_facts);
        let dead_stores = dfg.find_dead_stores();
        // x is a dead store
        assert!(dead_stores.iter().any(|d| d.var.name == "x"));
    }

    #[test]
    fn test_liveness_analysis() {
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![DefPoint {
                    var: make_var("x"),
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
                    var: make_var("x"),
                    block: exit,
                    stmt_index: 0,
                    start_byte: 6,
                    end_byte: 7,
                }],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );

        let mut dfg = DataFlowGraph::new(cfg, block_facts);
        let liveness = dfg.compute_liveness();
        // x should be live-out at entry (used in exit)
        let entry_live_out = liveness.live_out.get(&entry).unwrap();
        assert!(entry_live_out.iter().any(|v| v.name == "x"));
    }
}
