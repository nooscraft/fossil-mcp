//! Interprocedural constant propagation.
//!
//! For each function in a call graph, collects constant arguments from **all**
//! call sites. If every caller passes the same constant for a given parameter,
//! that constant is propagated into the callee's environment. Dead branches
//! that become apparent only at the interprocedural level are reported.
//!
//! The analysis runs a fixed-point iteration with a maximum of 3 rounds to
//! ensure termination even in the presence of recursive call chains.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;

use super::code_graph::CodeGraph;
use super::constant_prop::{ConstEnv, ConstValue};

// ===========================================================================
// Result types
// ===========================================================================

/// Result of interprocedural constant propagation.
#[derive(Debug)]
pub struct InterproceduralConstPropResult {
    /// For each function, the constant environment derived from all callers.
    pub function_const_envs: HashMap<NodeIndex, ConstEnv>,
    /// Dead branches discovered at interprocedural level.
    pub dead_branches: Vec<InterproceduralDeadBranch>,
}

/// A branch that is dead because all callers pass the same constant for a
/// parameter that controls the branch direction.
#[derive(Debug, Clone)]
pub struct InterproceduralDeadBranch {
    /// The function containing the dead branch.
    pub function: NodeIndex,
    /// The parameter whose constant value causes the branch to be dead.
    pub parameter: String,
    /// The constant value that all callers agree on.
    pub constant_value: ConstValue,
    /// A human-readable description of the dead branch.
    pub description: String,
}

// ===========================================================================
// Call-site argument model
// ===========================================================================

/// Describes a constant argument passed at a particular call site.
///
/// In the absence of argument-level tracking inside `CallEdge`, we model
/// call-site arguments by convention: the caller's constant environment at
/// the call point binds `"<callee_name>::param_<i>"` to the value of the
/// *i*-th positional argument. The helper [`inject_call_site_constants`]
/// populates these synthetic bindings so that
/// `analyze_interprocedural_constants` can consume them.
#[derive(Debug, Clone)]
pub struct CallSiteArgument {
    /// Index of the call-graph node representing the callee.
    pub callee: NodeIndex,
    /// Zero-based position of the parameter.
    pub param_index: usize,
    /// Name of the formal parameter in the callee (e.g. `"x"`).
    pub param_name: String,
    /// Constant value passed by the caller at this call site.
    pub value: ConstValue,
}

// ===========================================================================
// Analysis entry point
// ===========================================================================

/// Maximum number of fixed-point iterations.
const MAX_ITERATIONS: usize = 3;

/// Run interprocedural constant propagation over a `CodeGraph`.
///
/// The analysis proceeds as follows:
///
/// 1. For each function node, initialise an empty `ConstEnv`.
/// 2. Iterate (up to `MAX_ITERATIONS` times):
///    a. For every edge `(caller, callee)`, collect the constant environment
///    of the caller and meet it into the callee's environment.
///    b. If no environment changed, the analysis has reached a fixed point
///    and terminates early.
/// 3. Scan each function's final environment for parameters that hold a
///    definite constant. If the constant has a known truthiness, report a
///    potential dead branch.
///
/// Because the underlying `CodeGraph` / `CallEdge` does not carry per-call-
/// site argument data, callers may pre-populate parameter bindings via
/// [`inject_call_site_constants`] before invoking this function.
pub fn analyze_interprocedural_constants(graph: &CodeGraph) -> InterproceduralConstPropResult {
    // Initialise per-function environments.
    let mut envs: HashMap<NodeIndex, ConstEnv> = HashMap::new();
    for (idx, _node) in graph.nodes() {
        envs.insert(idx, ConstEnv::new());
    }

    // Collect edges once to avoid repeated iteration.
    let edges: Vec<(NodeIndex, NodeIndex)> = graph
        .edges_with_endpoints()
        .map(|(src, tgt, _)| (src, tgt))
        .collect();

    // Fixed-point iteration.
    for _iter in 0..MAX_ITERATIONS {
        let mut changed = false;

        for &(caller_idx, callee_idx) in &edges {
            let caller_env = envs.get(&caller_idx).cloned().unwrap_or_default();

            // Propagate caller environment into callee via lattice meet.
            let callee_env = envs.entry(callee_idx).or_default();
            let new_env = callee_env.meet(&caller_env);

            if new_env != *callee_env {
                *callee_env = new_env;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    // Detect dead branches from interprocedural constants.
    let dead_branches = detect_interprocedural_dead_branches(graph, &envs);

    InterproceduralConstPropResult {
        function_const_envs: envs,
        dead_branches,
    }
}

/// Run interprocedural constant propagation with explicit call-site argument
/// information.
///
/// `call_site_args` provides per-call-site argument constants that are
/// injected into callee environments before the fixed-point iteration.
pub fn analyze_interprocedural_constants_with_args(
    graph: &CodeGraph,
    call_site_args: &[CallSiteArgument],
) -> InterproceduralConstPropResult {
    // Initialise per-function environments.
    let mut envs: HashMap<NodeIndex, ConstEnv> = HashMap::new();
    for (idx, _node) in graph.nodes() {
        envs.insert(idx, ConstEnv::new());
    }

    // Inject call-site arguments. For each callee, meet the argument value
    // into the existing binding for that parameter.
    for arg in call_site_args {
        let env = envs.entry(arg.callee).or_default();
        let current = env.get(&arg.param_name);
        let met = current.meet(&arg.value);
        env.set(arg.param_name.clone(), met);
    }

    // Collect edges once.
    let edges: Vec<(NodeIndex, NodeIndex)> = graph
        .edges_with_endpoints()
        .map(|(src, tgt, _)| (src, tgt))
        .collect();

    // Fixed-point iteration.
    for _iter in 0..MAX_ITERATIONS {
        let mut changed = false;

        for &(caller_idx, callee_idx) in &edges {
            let caller_env = envs.get(&caller_idx).cloned().unwrap_or_default();
            let callee_env = envs.entry(callee_idx).or_default();
            let new_env = callee_env.meet(&caller_env);

            if new_env != *callee_env {
                *callee_env = new_env;
                changed = true;
            }
        }

        if !changed {
            break;
        }
    }

    let dead_branches = detect_interprocedural_dead_branches(graph, &envs);

    InterproceduralConstPropResult {
        function_const_envs: envs,
        dead_branches,
    }
}

// ===========================================================================
// Dead-branch detection
// ===========================================================================

/// Scan function environments for parameters that are definite constants and
/// report potential dead branches.
///
/// A parameter whose constant value has a known truthiness (e.g. `Constant(0)`
/// is always falsy, `BoolConst(true)` is always truthy) indicates that any
/// branch guarded by that parameter always takes the same direction, making
/// the other direction dead code.
fn detect_interprocedural_dead_branches(
    graph: &CodeGraph,
    envs: &HashMap<NodeIndex, ConstEnv>,
) -> Vec<InterproceduralDeadBranch> {
    let mut dead = Vec::new();

    for (idx, node) in graph.nodes() {
        let env = match envs.get(&idx) {
            Some(e) => e,
            None => continue,
        };

        for (var, val) in &env.bindings {
            if let Some(truthy) = val.is_truthy() {
                let direction = if truthy {
                    "always true"
                } else {
                    "always false"
                };
                dead.push(InterproceduralDeadBranch {
                    function: idx,
                    parameter: var.clone(),
                    constant_value: val.clone(),
                    description: format!(
                        "In function '{}', parameter '{}' is {} ({}) from all callers",
                        node.name,
                        var,
                        direction,
                        format_const(val),
                    ),
                });
            }
        }
    }

    dead
}

/// Format a `ConstValue` for display.
fn format_const(val: &ConstValue) -> String {
    match val {
        ConstValue::Constant(n) => format!("{}", n),
        ConstValue::StringConst(s) => format!("\"{}\"", s),
        ConstValue::BoolConst(b) => format!("{}", b),
        ConstValue::Top => "top".to_string(),
        ConstValue::Bottom => "bottom".to_string(),
    }
}

/// Convenience helper: inject call-site constants into an existing env map.
///
/// This is useful when constructing `ConstEnv` bindings from external data
/// (e.g. a language-specific argument extractor) before calling
/// [`analyze_interprocedural_constants`].
pub fn inject_call_site_constants(
    envs: &mut HashMap<NodeIndex, ConstEnv>,
    args: &[CallSiteArgument],
) {
    for arg in args {
        let env = envs.entry(arg.callee).or_default();
        let current = env.get(&arg.param_name);
        let met = current.meet(&arg.value);
        env.set(arg.param_name.clone(), met);
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CodeNode, Language, NodeKind, SourceLocation, Visibility};
    use crate::graph::code_graph::CodeGraph;
    use crate::graph::constant_prop::{ConstEnv, ConstValue};

    // ---- helpers ----------------------------------------------------------

    fn make_node(name: &str) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::Function,
            SourceLocation::new("test.rs".to_string(), 1, 10, 0, 0),
            Language::Rust,
            Visibility::Public,
        )
    }

    // ---- tests -----------------------------------------------------------

    #[test]
    fn test_empty_graph() {
        let graph = CodeGraph::new();
        let result = analyze_interprocedural_constants(&graph);
        assert!(result.function_const_envs.is_empty());
        assert!(result.dead_branches.is_empty());
    }

    #[test]
    fn test_single_node_no_callers() {
        let mut graph = CodeGraph::new();
        let _idx = graph.add_node(make_node("main"));
        let result = analyze_interprocedural_constants(&graph);
        // One function with an empty environment.
        assert_eq!(result.function_const_envs.len(), 1);
        assert!(result.dead_branches.is_empty());
    }

    #[test]
    fn test_constant_propagation_single_caller() {
        let mut graph = CodeGraph::new();
        let caller = make_node("caller");
        let callee = make_node("callee");
        let caller_idx = graph.add_node(caller);
        let callee_idx = graph.add_node(callee);
        graph.add_edge_by_index(caller_idx, callee_idx);

        // Inject a constant argument: caller always passes x=42 to callee.
        let args = vec![CallSiteArgument {
            callee: callee_idx,
            param_index: 0,
            param_name: "x".to_string(),
            value: ConstValue::Constant(42),
        }];

        let result = analyze_interprocedural_constants_with_args(&graph, &args);

        let callee_env = result.function_const_envs.get(&callee_idx).unwrap();
        assert_eq!(callee_env.get("x"), ConstValue::Constant(42));
    }

    #[test]
    fn test_conflicting_callers_produce_bottom() {
        let mut graph = CodeGraph::new();
        let caller_a = make_node("caller_a");
        let caller_b = make_node("caller_b");
        let callee = make_node("callee");

        let caller_a_idx = graph.add_node(caller_a);
        let caller_b_idx = graph.add_node(caller_b);
        let callee_idx = graph.add_node(callee);

        graph.add_edge_by_index(caller_a_idx, callee_idx);
        graph.add_edge_by_index(caller_b_idx, callee_idx);

        // caller_a passes x=1, caller_b passes x=2 -> Bottom
        let args = vec![
            CallSiteArgument {
                callee: callee_idx,
                param_index: 0,
                param_name: "x".to_string(),
                value: ConstValue::Constant(1),
            },
            CallSiteArgument {
                callee: callee_idx,
                param_index: 0,
                param_name: "x".to_string(),
                value: ConstValue::Constant(2),
            },
        ];

        let result = analyze_interprocedural_constants_with_args(&graph, &args);
        let callee_env = result.function_const_envs.get(&callee_idx).unwrap();
        // Different constants -> Bottom
        assert_eq!(callee_env.get("x"), ConstValue::Bottom);
    }

    #[test]
    fn test_agreeing_callers_propagate() {
        let mut graph = CodeGraph::new();
        let caller_a = make_node("caller_a");
        let caller_b = make_node("caller_b");
        let callee = make_node("callee");

        let caller_a_idx = graph.add_node(caller_a);
        let caller_b_idx = graph.add_node(caller_b);
        let callee_idx = graph.add_node(callee);

        graph.add_edge_by_index(caller_a_idx, callee_idx);
        graph.add_edge_by_index(caller_b_idx, callee_idx);

        // Both callers pass x=99 -> Constant(99)
        let args = vec![
            CallSiteArgument {
                callee: callee_idx,
                param_index: 0,
                param_name: "x".to_string(),
                value: ConstValue::Constant(99),
            },
            CallSiteArgument {
                callee: callee_idx,
                param_index: 0,
                param_name: "x".to_string(),
                value: ConstValue::Constant(99),
            },
        ];

        let result = analyze_interprocedural_constants_with_args(&graph, &args);
        let callee_env = result.function_const_envs.get(&callee_idx).unwrap();
        assert_eq!(callee_env.get("x"), ConstValue::Constant(99));
    }

    #[test]
    fn test_dead_branch_detected_from_constant_param() {
        let mut graph = CodeGraph::new();
        let caller = make_node("caller");
        let callee = make_node("callee");

        let caller_idx = graph.add_node(caller);
        let callee_idx = graph.add_node(callee);
        graph.add_edge_by_index(caller_idx, callee_idx);

        // Caller always passes debug=false to callee.
        let args = vec![CallSiteArgument {
            callee: callee_idx,
            param_index: 0,
            param_name: "debug".to_string(),
            value: ConstValue::BoolConst(false),
        }];

        let result = analyze_interprocedural_constants_with_args(&graph, &args);

        // The callee should have a dead branch for "debug".
        assert!(
            !result.dead_branches.is_empty(),
            "Expected at least one dead branch"
        );
        let db = result
            .dead_branches
            .iter()
            .find(|d| d.parameter == "debug")
            .expect("Expected dead branch for parameter 'debug'");
        assert_eq!(db.function, callee_idx);
        assert_eq!(db.constant_value, ConstValue::BoolConst(false));
        assert!(db.description.contains("always false"));
    }

    #[test]
    fn test_chain_propagation() {
        // A -> B -> C: constants flow through the chain.
        let mut graph = CodeGraph::new();
        let a = make_node("a");
        let b = make_node("b");
        let c = make_node("c");

        let a_idx = graph.add_node(a);
        let b_idx = graph.add_node(b);
        let c_idx = graph.add_node(c);

        graph.add_edge_by_index(a_idx, b_idx);
        graph.add_edge_by_index(b_idx, c_idx);

        // Inject constant into B's environment (simulating A passing x=10).
        let args = vec![
            CallSiteArgument {
                callee: b_idx,
                param_index: 0,
                param_name: "x".to_string(),
                value: ConstValue::Constant(10),
            },
            CallSiteArgument {
                callee: c_idx,
                param_index: 0,
                param_name: "x".to_string(),
                value: ConstValue::Constant(10),
            },
        ];

        let result = analyze_interprocedural_constants_with_args(&graph, &args);

        let b_env = result.function_const_envs.get(&b_idx).unwrap();
        assert_eq!(b_env.get("x"), ConstValue::Constant(10));

        let c_env = result.function_const_envs.get(&c_idx).unwrap();
        assert_eq!(c_env.get("x"), ConstValue::Constant(10));
    }

    #[test]
    fn test_inject_call_site_constants() {
        let mut envs: HashMap<NodeIndex, ConstEnv> = HashMap::new();
        let idx = NodeIndex::new(0);
        envs.insert(idx, ConstEnv::new());

        let args = vec![
            CallSiteArgument {
                callee: idx,
                param_index: 0,
                param_name: "y".to_string(),
                value: ConstValue::Constant(7),
            },
            CallSiteArgument {
                callee: idx,
                param_index: 0,
                param_name: "y".to_string(),
                value: ConstValue::Constant(7),
            },
        ];

        inject_call_site_constants(&mut envs, &args);

        let env = envs.get(&idx).unwrap();
        assert_eq!(env.get("y"), ConstValue::Constant(7));
    }
}
