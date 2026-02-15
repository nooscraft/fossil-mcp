//! Code graph construction and analysis for Fossil.
//!
//! Provides:
//! - `CodeGraph` — petgraph-based directed graph of code nodes and call edges
//! - `GraphBuilder` trait — per-language graph construction from parsed files
//! - `CFG` — intra-procedural control flow graph
//! - `Dominance` — dominator tree construction

pub mod builder;
pub mod centrality;
pub mod cfg;
pub mod class_hierarchy;
pub mod code_graph;
pub mod constant_prop;
pub mod csr_format;
pub mod dataflow;
pub mod expr_evaluator;
pub mod import_resolver;
pub mod interprocedural_const_prop;
pub mod pdg;
pub mod rta;
pub mod sdg;
pub mod slicing;
pub mod symbol_table;
pub mod var_extractor;
pub mod vta;

pub use builder::GraphBuilder;
pub use centrality::{
    classify_importance, compute_betweenness, compute_centrality, compute_pagerank,
    CentralityScores, ImportanceLevel,
};
pub use cfg::{BasicBlock, CfgEdgeKind, CfgNodeId, ControlFlowGraph};
pub use class_hierarchy::ClassHierarchy;
pub use code_graph::CodeGraph;
pub use csr_format::CsrGraph;
pub use constant_prop::{
    analyze_constants, analyze_constants_with_expressions, analyze_constants_with_values,
    evaluate_expression, ConstEnv, ConstPropResult, ConstValue, ConstantAssignment, DeadBranch,
    ExprConstantAssignment,
};
pub use dataflow::{
    BlockDataFlow, DataFlowGraph, DefPoint, DefUseChain, LivenessResult, ReachingDefinitions,
    UsePoint, VarRef,
};
pub use import_resolver::ImportResolver;
pub use interprocedural_const_prop::{
    analyze_interprocedural_constants, InterproceduralConstPropResult, InterproceduralDeadBranch,
};
pub use pdg::ProgramDependenceGraph;
pub use rta::RapidTypeAnalysis;
pub use sdg::{
    InterproceduralSlice, InterproceduralSliceCriterion, SdgNode, SystemDependenceGraph,
};
pub use slicing::{
    backward_slice, find_dead_by_slicing, forward_slice, ProgramSlice, SliceCriterion,
};
pub use symbol_table::SymbolTable;
pub use var_extractor::extract_defs_and_uses;
pub use vta::VariableTypeAnalysis;
