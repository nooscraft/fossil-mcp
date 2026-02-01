//! Constant propagation analysis.
//!
//! Performs forward iterative constant propagation over the CFG, tracking
//! which variables hold compile-time constant integer values. Also detects
//! dead branches whose conditions always evaluate to the same truth value.

use std::collections::HashMap;

use super::cfg::{CfgEdgeKind, CfgNodeId, ControlFlowGraph};
use super::dataflow::BlockDataFlow;

// ---------------------------------------------------------------------------
// Lattice value
// ---------------------------------------------------------------------------

/// Constant-propagation lattice value.
///
/// ```text
///       Top                  (not yet determined)
///      / | \
///   C(0) C(1) S("x") B(t)   (known constants)
///      \ | /
///      Bottom                (over-defined / conflict)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConstValue {
    /// Not yet determined (initial state).
    Top,
    /// Known constant integer value.
    Constant(i64),
    /// Known constant string value.
    StringConst(String),
    /// Known constant boolean value.
    BoolConst(bool),
    /// Over-defined — assigned different values on different paths.
    Bottom,
}

impl ConstValue {
    /// Lattice meet operation.
    ///
    /// - `Top  meet x = x`
    /// - `x    meet Top = x`
    /// - `Bottom meet _ = Bottom`
    /// - `_    meet Bottom = Bottom`
    /// - `Constant(a) meet Constant(b) = Constant(a)` if `a == b`, else `Bottom`
    /// - `StringConst(a) meet StringConst(b) = StringConst(a)` if `a == b`, else `Bottom`
    /// - `BoolConst(a) meet BoolConst(b) = BoolConst(a)` if `a == b`, else `Bottom`
    /// - Mixing different constant types yields `Bottom`.
    pub fn meet(&self, other: &ConstValue) -> ConstValue {
        match (self, other) {
            (ConstValue::Top, v) | (v, ConstValue::Top) => v.clone(),
            (ConstValue::Bottom, _) | (_, ConstValue::Bottom) => ConstValue::Bottom,
            (ConstValue::Constant(a), ConstValue::Constant(b)) => {
                if a == b {
                    ConstValue::Constant(*a)
                } else {
                    ConstValue::Bottom
                }
            }
            (ConstValue::StringConst(a), ConstValue::StringConst(b)) => {
                if a == b {
                    ConstValue::StringConst(a.clone())
                } else {
                    ConstValue::Bottom
                }
            }
            (ConstValue::BoolConst(a), ConstValue::BoolConst(b)) => {
                if a == b {
                    ConstValue::BoolConst(*a)
                } else {
                    ConstValue::Bottom
                }
            }
            // Mixing different constant types
            _ => ConstValue::Bottom,
        }
    }

    /// Returns `true` when the value is `Top`.
    pub fn is_top(&self) -> bool {
        matches!(self, ConstValue::Top)
    }

    /// Returns `true` when the value is `Bottom`.
    pub fn is_bottom(&self) -> bool {
        matches!(self, ConstValue::Bottom)
    }

    /// Returns the constant if this value is `Constant(_)`.
    pub fn as_constant(&self) -> Option<i64> {
        match self {
            ConstValue::Constant(v) => Some(*v),
            _ => None,
        }
    }

    /// Returns the string if this value is `StringConst(_)`.
    pub fn as_string(&self) -> Option<&str> {
        match self {
            ConstValue::StringConst(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// Returns the boolean if this value is `BoolConst(_)`.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ConstValue::BoolConst(b) => Some(*b),
            _ => None,
        }
    }

    /// Determine the truthiness of a constant value.
    ///
    /// - `Constant(0)` -> `false`, `Constant(n)` -> `true` (n != 0)
    /// - `BoolConst(b)` -> `b`
    /// - `StringConst("")` -> `false`, `StringConst(s)` -> `true` (s non-empty)
    /// - `Top` / `Bottom` -> `None` (unknown)
    pub fn is_truthy(&self) -> Option<bool> {
        match self {
            ConstValue::Constant(v) => Some(*v != 0),
            ConstValue::BoolConst(b) => Some(*b),
            ConstValue::StringConst(s) => Some(!s.is_empty()),
            ConstValue::Top | ConstValue::Bottom => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Constant environment (per-block state)
// ---------------------------------------------------------------------------

/// Maps variable names to their lattice values at a given program point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstEnv {
    pub bindings: HashMap<String, ConstValue>,
}

impl ConstEnv {
    /// Create an empty environment (all variables implicitly `Top`).
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Look up the value for `var`.  Returns `Top` for unknown variables.
    pub fn get(&self, var: &str) -> ConstValue {
        self.bindings.get(var).cloned().unwrap_or(ConstValue::Top)
    }

    /// Set the value for `var`.
    pub fn set(&mut self, var: impl Into<String>, val: ConstValue) {
        self.bindings.insert(var.into(), val);
    }

    /// Point-wise meet of two environments.
    ///
    /// Variables present in only one environment are treated as `Top` in the
    /// other, so the meet with `Top` yields the value from the environment
    /// that knows about it.
    pub fn meet(&self, other: &ConstEnv) -> ConstEnv {
        let mut result = ConstEnv::new();
        let mut all_vars: Vec<&String> = self.bindings.keys().collect();
        for k in other.bindings.keys() {
            if !self.bindings.contains_key(k) {
                all_vars.push(k);
            }
        }
        for var in all_vars {
            let a = self.get(var);
            let b = other.get(var);
            result.set(var.clone(), a.meet(&b));
        }
        result
    }
}

impl Default for ConstEnv {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Dead-branch descriptor
// ---------------------------------------------------------------------------

/// A branch whose condition always evaluates to the same truth value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeadBranch {
    /// The basic block that contains the branch condition.
    pub block_id: CfgNodeId,
    /// A textual representation of the condition expression.
    pub condition: String,
    /// The constant truth value of the condition (`true` or `false`).
    pub always_value: bool,
    /// Source line (0-indexed) of the branch, when available.
    pub start_line: usize,
}

// ---------------------------------------------------------------------------
// Analysis result
// ---------------------------------------------------------------------------

/// The complete result of constant propagation analysis.
#[derive(Debug, Clone)]
pub struct ConstPropResult {
    /// Branches that always take the same direction.
    pub dead_branches: Vec<DeadBranch>,
    /// Variables that are constant throughout the analysed scope.
    pub constant_vars: HashMap<String, i64>,
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run constant propagation over a CFG annotated with per-block data flow
/// facts.
///
/// The analysis performs a forward iterative fixed-point computation:
///
/// 1. For each assignment `x = <integer-literal>` → set `x` to `Constant(literal)`.
/// 2. For each assignment `x = <non-literal>` → set `x` to `Bottom`.
/// 3. At merge points the lattice meet is applied.
/// 4. Conditions that are always truthy or always falsy are reported as dead
///    branches.
pub fn analyze_constants(
    cfg: &ControlFlowGraph,
    block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
) -> ConstPropResult {
    // Initialize per-block IN/OUT environments.
    let block_ids: Vec<CfgNodeId> = {
        let mut ids: Vec<CfgNodeId> = cfg.blocks().map(|(&id, _)| id).collect();
        ids.sort();
        ids
    };

    let mut env_in: HashMap<CfgNodeId, ConstEnv> = HashMap::new();
    let mut env_out: HashMap<CfgNodeId, ConstEnv> = HashMap::new();

    for &bid in &block_ids {
        env_in.insert(bid, ConstEnv::new());
        env_out.insert(bid, ConstEnv::new());
    }

    // ----- fixed-point iteration -----
    let mut changed = true;
    while changed {
        changed = false;

        for &bid in &block_ids {
            // env_in[B] = meet of env_out[P] for all predecessors P
            let preds = cfg.predecessors(bid);
            let new_in = if preds.is_empty() {
                ConstEnv::new()
            } else {
                let mut merged: Option<ConstEnv> = None;
                for (pred_id, _) in &preds {
                    let pred_out = env_out.get(pred_id).cloned().unwrap_or_else(ConstEnv::new);
                    merged = Some(match merged {
                        None => pred_out,
                        Some(acc) => acc.meet(&pred_out),
                    });
                }
                merged.unwrap_or_default()
            };

            // Transfer function: process definitions in this block.
            let new_out = transfer(bid, &new_in, block_facts);

            if new_out != *env_out.get(&bid).unwrap_or(&ConstEnv::default()) {
                changed = true;
            }

            env_in.insert(bid, new_in);
            env_out.insert(bid, new_out);
        }
    }

    // ----- collect results -----
    let mut constant_vars: HashMap<String, i64> = HashMap::new();

    // A variable is "globally constant" when every env_out that defines it
    // agrees on the same constant value.
    let mut var_values: HashMap<String, ConstValue> = HashMap::new();
    for env in env_out.values() {
        for (var, val) in &env.bindings {
            let entry = var_values.entry(var.clone()).or_insert(ConstValue::Top);
            *entry = entry.meet(val);
        }
    }
    for (var, val) in &var_values {
        if let ConstValue::Constant(c) = val {
            constant_vars.insert(var.clone(), *c);
        }
    }

    // ----- dead-branch detection -----
    let dead_branches = detect_dead_branches(cfg, &env_in, block_facts);

    ConstPropResult {
        dead_branches,
        constant_vars,
    }
}

// ---------------------------------------------------------------------------
// Transfer function
// ---------------------------------------------------------------------------

/// Apply the constant-propagation transfer function for one block.
///
/// Starting from `env_in`, process each definition in `block_facts` for
/// `block_id` and return the resulting `env_out`.
fn transfer(
    block_id: CfgNodeId,
    env_in: &ConstEnv,
    block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
) -> ConstEnv {
    let mut env = env_in.clone();

    if let Some(facts) = block_facts.get(&block_id) {
        for def in &facts.defs {
            let var_name = &def.var.name;

            // Heuristic: if the variable name looks like a literal integer we
            // recorded from the RHS, treat it as a constant.  In practice the
            // DefPoint only captures the LHS variable name, so we fall back to
            // a simple naming convention check.  A more precise implementation
            // would carry the RHS expression in the DefPoint, but we work with
            // what the existing framework provides.
            //
            // For the current framework the RHS is not directly stored.  We
            // therefore mark every definition as `Bottom` (conservative) in the
            // base transfer function.  The `analyze_constants_with_values`
            // variant below allows callers to supply known constant
            // assignments.
            env.set(var_name.clone(), ConstValue::Bottom);
        }
    }

    env
}

// ---------------------------------------------------------------------------
// Extended entry point with explicit constant assignments
// ---------------------------------------------------------------------------

/// A known constant assignment: variable `var` is assigned the literal value
/// `value` in block `block_id` at definition index `def_index`.
#[derive(Debug, Clone)]
pub struct ConstantAssignment {
    pub var: String,
    pub value: i64,
    pub block_id: CfgNodeId,
    pub def_index: usize,
}

/// Run constant propagation with explicitly supplied constant assignments.
///
/// This is the recommended entry point when the caller can determine which
/// assignments are integer literals (e.g. by inspecting the tree-sitter AST).
pub fn analyze_constants_with_values(
    cfg: &ControlFlowGraph,
    block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
    constants: &[ConstantAssignment],
) -> ConstPropResult {
    // Build a lookup: (block_id, def_index) -> constant value
    let const_lookup: HashMap<(CfgNodeId, usize), i64> = constants
        .iter()
        .map(|ca| ((ca.block_id, ca.def_index), ca.value))
        .collect();

    let block_ids: Vec<CfgNodeId> = {
        let mut ids: Vec<CfgNodeId> = cfg.blocks().map(|(&id, _)| id).collect();
        ids.sort();
        ids
    };

    let mut env_in: HashMap<CfgNodeId, ConstEnv> = HashMap::new();
    let mut env_out: HashMap<CfgNodeId, ConstEnv> = HashMap::new();

    for &bid in &block_ids {
        env_in.insert(bid, ConstEnv::new());
        env_out.insert(bid, ConstEnv::new());
    }

    let mut changed = true;
    while changed {
        changed = false;

        for &bid in &block_ids {
            let preds = cfg.predecessors(bid);
            let new_in = if preds.is_empty() {
                ConstEnv::new()
            } else {
                let mut merged: Option<ConstEnv> = None;
                for (pred_id, _) in &preds {
                    let pred_out = env_out.get(pred_id).cloned().unwrap_or_else(ConstEnv::new);
                    merged = Some(match merged {
                        None => pred_out,
                        Some(acc) => acc.meet(&pred_out),
                    });
                }
                merged.unwrap_or_default()
            };

            let new_out = transfer_with_values(bid, &new_in, block_facts, &const_lookup);

            if new_out != *env_out.get(&bid).unwrap_or(&ConstEnv::default()) {
                changed = true;
            }

            env_in.insert(bid, new_in);
            env_out.insert(bid, new_out);
        }
    }

    // Collect constant vars
    let mut constant_vars: HashMap<String, i64> = HashMap::new();
    let mut var_values: HashMap<String, ConstValue> = HashMap::new();
    for env in env_out.values() {
        for (var, val) in &env.bindings {
            let entry = var_values.entry(var.clone()).or_insert(ConstValue::Top);
            *entry = entry.meet(val);
        }
    }
    for (var, val) in &var_values {
        if let ConstValue::Constant(c) = val {
            constant_vars.insert(var.clone(), *c);
        }
    }

    let dead_branches = detect_dead_branches(cfg, &env_in, block_facts);

    ConstPropResult {
        dead_branches,
        constant_vars,
    }
}

/// Transfer function that consults the constant-assignment lookup.
fn transfer_with_values(
    block_id: CfgNodeId,
    env_in: &ConstEnv,
    block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
    const_lookup: &HashMap<(CfgNodeId, usize), i64>,
) -> ConstEnv {
    let mut env = env_in.clone();

    if let Some(facts) = block_facts.get(&block_id) {
        for (idx, def) in facts.defs.iter().enumerate() {
            let var_name = &def.var.name;
            if let Some(&value) = const_lookup.get(&(block_id, idx)) {
                env.set(var_name.clone(), ConstValue::Constant(value));
            } else {
                env.set(var_name.clone(), ConstValue::Bottom);
            }
        }
    }

    env
}

// ---------------------------------------------------------------------------
// Expression evaluation public API
// ---------------------------------------------------------------------------

/// Evaluate an expression string against a constant environment.
///
/// This is the public API that wraps [`crate::graph::expr_evaluator::eval_const_expr`].
pub fn evaluate_expression(expr: &str, env: &ConstEnv) -> ConstValue {
    crate::graph::expr_evaluator::eval_const_expr(expr, &env.bindings)
}

// ---------------------------------------------------------------------------
// Extended constant assignment with expression text
// ---------------------------------------------------------------------------

/// An extended constant assignment that includes the RHS expression text
/// rather than a pre-computed integer value.
///
/// The expression will be evaluated using the constant environment at the
/// point of the assignment, allowing propagation of computed constants
/// like `x = 2 + 3`.
#[derive(Debug, Clone)]
pub struct ExprConstantAssignment {
    /// Variable being assigned.
    pub var: String,
    /// Source text of the RHS expression.
    pub expr: String,
    /// The basic block containing this assignment.
    pub block_id: CfgNodeId,
    /// Index within the block's definition list.
    pub def_index: usize,
}

/// Run constant propagation with expression evaluation support.
///
/// This combines the integer-literal constant assignments from
/// `int_constants` with expression-based assignments from
/// `expr_assignments`. Expression assignments are evaluated against
/// the current constant environment, potentially resolving computed
/// constants like `x = a + b` when `a` and `b` are themselves known
/// constants.
pub fn analyze_constants_with_expressions(
    cfg: &ControlFlowGraph,
    block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
    int_constants: &[ConstantAssignment],
    expr_assignments: &[ExprConstantAssignment],
) -> ConstPropResult {
    // Build lookup tables
    let int_lookup: HashMap<(CfgNodeId, usize), i64> = int_constants
        .iter()
        .map(|ca| ((ca.block_id, ca.def_index), ca.value))
        .collect();

    let expr_lookup: HashMap<(CfgNodeId, usize), &str> = expr_assignments
        .iter()
        .map(|ea| ((ea.block_id, ea.def_index), ea.expr.as_str()))
        .collect();

    let block_ids: Vec<CfgNodeId> = {
        let mut ids: Vec<CfgNodeId> = cfg.blocks().map(|(&id, _)| id).collect();
        ids.sort();
        ids
    };

    let mut env_in: HashMap<CfgNodeId, ConstEnv> = HashMap::new();
    let mut env_out: HashMap<CfgNodeId, ConstEnv> = HashMap::new();

    for &bid in &block_ids {
        env_in.insert(bid, ConstEnv::new());
        env_out.insert(bid, ConstEnv::new());
    }

    let mut changed = true;
    while changed {
        changed = false;

        for &bid in &block_ids {
            let preds = cfg.predecessors(bid);
            let new_in = if preds.is_empty() {
                ConstEnv::new()
            } else {
                let mut merged: Option<ConstEnv> = None;
                for (pred_id, _) in &preds {
                    let pred_out = env_out.get(pred_id).cloned().unwrap_or_else(ConstEnv::new);
                    merged = Some(match merged {
                        None => pred_out,
                        Some(acc) => acc.meet(&pred_out),
                    });
                }
                merged.unwrap_or_default()
            };

            let new_out =
                transfer_with_expressions(bid, &new_in, block_facts, &int_lookup, &expr_lookup);

            if new_out != *env_out.get(&bid).unwrap_or(&ConstEnv::default()) {
                changed = true;
            }

            env_in.insert(bid, new_in);
            env_out.insert(bid, new_out);
        }
    }

    // Collect constant vars (integer only for the result type)
    let mut constant_vars: HashMap<String, i64> = HashMap::new();
    let mut var_values: HashMap<String, ConstValue> = HashMap::new();
    for env in env_out.values() {
        for (var, val) in &env.bindings {
            let entry = var_values.entry(var.clone()).or_insert(ConstValue::Top);
            *entry = entry.meet(val);
        }
    }
    for (var, val) in &var_values {
        if let ConstValue::Constant(c) = val {
            constant_vars.insert(var.clone(), *c);
        }
    }

    let dead_branches = detect_dead_branches(cfg, &env_in, block_facts);

    ConstPropResult {
        dead_branches,
        constant_vars,
    }
}

/// Transfer function that consults both integer constants and expression
/// assignments.
fn transfer_with_expressions(
    block_id: CfgNodeId,
    env_in: &ConstEnv,
    block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
    int_lookup: &HashMap<(CfgNodeId, usize), i64>,
    expr_lookup: &HashMap<(CfgNodeId, usize), &str>,
) -> ConstEnv {
    let mut env = env_in.clone();

    if let Some(facts) = block_facts.get(&block_id) {
        for (idx, def) in facts.defs.iter().enumerate() {
            let var_name = &def.var.name;
            let key = (block_id, idx);

            if let Some(&value) = int_lookup.get(&key) {
                // Known integer literal
                env.set(var_name.clone(), ConstValue::Constant(value));
            } else if let Some(expr_text) = expr_lookup.get(&key) {
                // Expression — evaluate against current env
                let result =
                    crate::graph::expr_evaluator::eval_const_expr(expr_text, &env.bindings);
                if result.is_top() {
                    // Could not resolve — treat as Bottom (conservative)
                    env.set(var_name.clone(), ConstValue::Bottom);
                } else {
                    env.set(var_name.clone(), result);
                }
            } else {
                // Unknown assignment — conservative
                env.set(var_name.clone(), ConstValue::Bottom);
            }
        }
    }

    env
}

// ---------------------------------------------------------------------------
// Dead-branch detection
// ---------------------------------------------------------------------------

/// Recognised dead-branch patterns:
///
/// - Python: `if False:`, `if 0:`, `if True:`, `if 1:`
/// - Also: `if DEBUG and False:` (conservative — the `and False` dominates)
///
/// We inspect block labels and edge structure to identify conditional blocks,
/// then check whether the environment maps the condition variable to a known
/// constant that is always truthy or falsy.
fn detect_dead_branches(
    cfg: &ControlFlowGraph,
    env_in: &HashMap<CfgNodeId, ConstEnv>,
    block_facts: &HashMap<CfgNodeId, BlockDataFlow>,
) -> Vec<DeadBranch> {
    let mut dead = Vec::new();

    for (&bid, block) in cfg.blocks() {
        let succs = cfg.successors(bid);

        // Only consider blocks with exactly one true-edge and one false-edge
        // (i.e. conditional branches).
        let has_true = succs
            .iter()
            .any(|(_, k)| *k == CfgEdgeKind::ConditionalTrue);
        let has_false = succs
            .iter()
            .any(|(_, k)| *k == CfgEdgeKind::ConditionalFalse);

        if !has_true || !has_false {
            continue;
        }

        let env = env_in.get(&bid).cloned().unwrap_or_else(ConstEnv::new);

        // Attempt to resolve the condition from the block's uses.
        // A simple condition like `if x:` will have `x` as a use in this
        // block, and if x is a known constant we can decide the branch.
        if let Some(facts) = block_facts.get(&bid) {
            // Look at all uses in this block — the last use is most likely
            // the condition variable in a simple conditional.
            for use_pt in facts.uses.iter().rev() {
                let val = env.get(&use_pt.var.name);
                if let Some(always_value) = val.is_truthy() {
                    dead.push(DeadBranch {
                        block_id: bid,
                        condition: use_pt.var.name.clone(),
                        always_value,
                        start_line: block.statements.first().map_or(0, |s| s.start),
                    });
                    // One dead branch per block is enough.
                    break;
                }
            }
        }

        // Check for literal condition patterns in block label.
        // Some CFG builders embed condition text in block labels or
        // statement text. We try common patterns.
        // This is a heuristic; production code would inspect the AST.
        let label_lower = block.label.to_lowercase();
        if (label_lower.contains("false") || label_lower.contains("if_0"))
            && !dead.iter().any(|d| d.block_id == bid)
        {
            // Condition is always false
            dead.push(DeadBranch {
                block_id: bid,
                condition: block.label.clone(),
                always_value: false,
                start_line: block.statements.first().map_or(0, |s| s.start),
            });
        } else if (label_lower.contains("true") || label_lower.contains("if_1"))
            && !dead.iter().any(|d| d.block_id == bid)
        {
            dead.push(DeadBranch {
                block_id: bid,
                condition: block.label.clone(),
                always_value: true,
                start_line: block.statements.first().map_or(0, |s| s.start),
            });
        }
    }

    dead
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SourceSpan;
    use crate::graph::cfg::{CfgEdgeKind, ControlFlowGraph};
    use crate::graph::dataflow::{BlockDataFlow, DefPoint, UsePoint, VarRef};
    use std::collections::HashSet;

    // ---- helpers ----------------------------------------------------------

    fn make_var(name: &str) -> VarRef {
        VarRef::new(name)
    }

    fn make_def(name: &str, block: CfgNodeId, stmt: usize, start: usize, end: usize) -> DefPoint {
        DefPoint {
            var: make_var(name),
            block,
            stmt_index: stmt,
            start_byte: start,
            end_byte: end,
        }
    }

    fn make_use(name: &str, block: CfgNodeId, stmt: usize, start: usize, end: usize) -> UsePoint {
        UsePoint {
            var: make_var(name),
            block,
            stmt_index: stmt,
            start_byte: start,
            end_byte: end,
        }
    }

    fn empty_block_data() -> BlockDataFlow {
        BlockDataFlow {
            defs: vec![],
            uses: vec![],
            gen: HashSet::new(),
            kill: HashSet::new(),
        }
    }

    // ---- lattice tests ----------------------------------------------------

    #[test]
    fn test_meet_top_with_constant() {
        let top = ConstValue::Top;
        let c = ConstValue::Constant(42);
        assert_eq!(top.meet(&c), ConstValue::Constant(42));
        assert_eq!(c.meet(&top), ConstValue::Constant(42));
    }

    #[test]
    fn test_meet_same_constant() {
        let a = ConstValue::Constant(7);
        let b = ConstValue::Constant(7);
        assert_eq!(a.meet(&b), ConstValue::Constant(7));
    }

    #[test]
    fn test_meet_different_constants() {
        let a = ConstValue::Constant(1);
        let b = ConstValue::Constant(2);
        assert_eq!(a.meet(&b), ConstValue::Bottom);
    }

    #[test]
    fn test_meet_with_bottom() {
        let c = ConstValue::Constant(5);
        let bot = ConstValue::Bottom;
        assert_eq!(c.meet(&bot), ConstValue::Bottom);
        assert_eq!(bot.meet(&c), ConstValue::Bottom);
    }

    #[test]
    fn test_meet_top_with_top() {
        assert_eq!(ConstValue::Top.meet(&ConstValue::Top), ConstValue::Top);
    }

    #[test]
    fn test_meet_bottom_with_bottom() {
        assert_eq!(
            ConstValue::Bottom.meet(&ConstValue::Bottom),
            ConstValue::Bottom
        );
    }

    // ---- ConstEnv tests ---------------------------------------------------

    #[test]
    fn test_env_default_is_top() {
        let env = ConstEnv::new();
        assert_eq!(env.get("x"), ConstValue::Top);
    }

    #[test]
    fn test_env_set_and_get() {
        let mut env = ConstEnv::new();
        env.set("x", ConstValue::Constant(10));
        assert_eq!(env.get("x"), ConstValue::Constant(10));
    }

    #[test]
    fn test_env_meet_same_constants() {
        let mut a = ConstEnv::new();
        a.set("x", ConstValue::Constant(3));
        let mut b = ConstEnv::new();
        b.set("x", ConstValue::Constant(3));
        let merged = a.meet(&b);
        assert_eq!(merged.get("x"), ConstValue::Constant(3));
    }

    #[test]
    fn test_env_meet_different_constants() {
        let mut a = ConstEnv::new();
        a.set("x", ConstValue::Constant(3));
        let mut b = ConstEnv::new();
        b.set("x", ConstValue::Constant(5));
        let merged = a.meet(&b);
        assert_eq!(merged.get("x"), ConstValue::Bottom);
    }

    #[test]
    fn test_env_meet_one_side_missing() {
        let mut a = ConstEnv::new();
        a.set("x", ConstValue::Constant(7));
        let b = ConstEnv::new(); // x is Top
        let merged = a.meet(&b);
        // Top meet Constant(7) = Constant(7)
        assert_eq!(merged.get("x"), ConstValue::Constant(7));
    }

    // ---- analyze_constants (conservative) tests ---------------------------

    #[test]
    fn test_analyze_constants_linear() {
        // BB0 (entry): def x
        // BB1 (exit):  use x
        // Without explicit constant info, x should be Bottom (conservative).
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);
        cfg.add_statement(entry, SourceSpan::new(0, 5));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![make_def("x", entry, 0, 0, 5)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(exit, empty_block_data());

        let result = analyze_constants(&cfg, &block_facts);
        // Conservative: no constants detected.
        assert!(result.constant_vars.is_empty());
    }

    // ---- analyze_constants_with_values tests ------------------------------

    #[test]
    fn test_with_values_single_constant() {
        // entry: x = 42  (constant)
        // exit: (empty)
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);
        cfg.add_statement(entry, SourceSpan::new(0, 6));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![make_def("x", entry, 0, 0, 6)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(exit, empty_block_data());

        let constants = vec![ConstantAssignment {
            var: "x".into(),
            value: 42,
            block_id: entry,
            def_index: 0,
        }];

        let result = analyze_constants_with_values(&cfg, &block_facts, &constants);
        assert_eq!(result.constant_vars.get("x"), Some(&42));
    }

    #[test]
    fn test_with_values_multiple_constants() {
        // entry: x = 10, y = 20
        // exit: (empty)
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);
        cfg.add_statement(entry, SourceSpan::new(0, 12));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![
                    make_def("x", entry, 0, 0, 5),
                    make_def("y", entry, 1, 6, 12),
                ],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(exit, empty_block_data());

        let constants = vec![
            ConstantAssignment {
                var: "x".into(),
                value: 10,
                block_id: entry,
                def_index: 0,
            },
            ConstantAssignment {
                var: "y".into(),
                value: 20,
                block_id: entry,
                def_index: 1,
            },
        ];

        let result = analyze_constants_with_values(&cfg, &block_facts, &constants);
        assert_eq!(result.constant_vars.get("x"), Some(&10));
        assert_eq!(result.constant_vars.get("y"), Some(&20));
    }

    #[test]
    fn test_with_values_conflicting_at_merge() {
        // Two paths assign different values to x, then merge.
        //
        //   entry
        //   /   \
        //  T     F
        //  x=1   x=2
        //   \   /
        //   merge
        //    |
        //   exit
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let true_blk = cfg.create_block("if_true");
        let false_blk = cfg.create_block("if_false");
        let merge = cfg.create_block("merge");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, true_blk, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(entry, false_blk, CfgEdgeKind::ConditionalFalse);
        cfg.add_edge(true_blk, merge, CfgEdgeKind::FallThrough);
        cfg.add_edge(false_blk, merge, CfgEdgeKind::FallThrough);
        cfg.add_edge(merge, exit, CfgEdgeKind::FallThrough);

        cfg.add_statement(true_blk, SourceSpan::new(0, 5));
        cfg.add_statement(false_blk, SourceSpan::new(6, 11));

        let mut block_facts = HashMap::new();
        block_facts.insert(entry, empty_block_data());
        block_facts.insert(
            true_blk,
            BlockDataFlow {
                defs: vec![make_def("x", true_blk, 0, 0, 5)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            false_blk,
            BlockDataFlow {
                defs: vec![make_def("x", false_blk, 0, 6, 11)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(merge, empty_block_data());
        block_facts.insert(exit, empty_block_data());

        let constants = vec![
            ConstantAssignment {
                var: "x".into(),
                value: 1,
                block_id: true_blk,
                def_index: 0,
            },
            ConstantAssignment {
                var: "x".into(),
                value: 2,
                block_id: false_blk,
                def_index: 0,
            },
        ];

        let result = analyze_constants_with_values(&cfg, &block_facts, &constants);
        // x gets different constants on different paths -> Bottom -> not in constant_vars
        assert!(!result.constant_vars.contains_key("x"));
    }

    #[test]
    fn test_with_values_agreeing_at_merge() {
        // Two paths assign the SAME value to x.
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let true_blk = cfg.create_block("if_true");
        let false_blk = cfg.create_block("if_false");
        let merge = cfg.create_block("merge");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, true_blk, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(entry, false_blk, CfgEdgeKind::ConditionalFalse);
        cfg.add_edge(true_blk, merge, CfgEdgeKind::FallThrough);
        cfg.add_edge(false_blk, merge, CfgEdgeKind::FallThrough);
        cfg.add_edge(merge, exit, CfgEdgeKind::FallThrough);

        cfg.add_statement(true_blk, SourceSpan::new(0, 5));
        cfg.add_statement(false_blk, SourceSpan::new(6, 11));

        let mut block_facts = HashMap::new();
        block_facts.insert(entry, empty_block_data());
        block_facts.insert(
            true_blk,
            BlockDataFlow {
                defs: vec![make_def("x", true_blk, 0, 0, 5)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            false_blk,
            BlockDataFlow {
                defs: vec![make_def("x", false_blk, 0, 6, 11)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(merge, empty_block_data());
        block_facts.insert(exit, empty_block_data());

        let constants = vec![
            ConstantAssignment {
                var: "x".into(),
                value: 99,
                block_id: true_blk,
                def_index: 0,
            },
            ConstantAssignment {
                var: "x".into(),
                value: 99,
                block_id: false_blk,
                def_index: 0,
            },
        ];

        let result = analyze_constants_with_values(&cfg, &block_facts, &constants);
        // Same constant on both paths -> Constant(99)
        assert_eq!(result.constant_vars.get("x"), Some(&99));
    }

    #[test]
    fn test_with_values_mixed_constant_and_non_constant() {
        // x = 5 (constant), y = f() (non-constant)
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);
        cfg.add_statement(entry, SourceSpan::new(0, 20));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![
                    make_def("x", entry, 0, 0, 5),
                    make_def("y", entry, 1, 6, 20),
                ],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(exit, empty_block_data());

        // Only x is a known constant; y has no ConstantAssignment -> Bottom.
        let constants = vec![ConstantAssignment {
            var: "x".into(),
            value: 5,
            block_id: entry,
            def_index: 0,
        }];

        let result = analyze_constants_with_values(&cfg, &block_facts, &constants);
        assert_eq!(result.constant_vars.get("x"), Some(&5));
        assert!(!result.constant_vars.contains_key("y"));
    }

    // ---- dead-branch detection tests --------------------------------------

    #[test]
    fn test_dead_branch_constant_condition() {
        // entry: x = 0
        // cond_block: if x  (uses x; x == 0 -> always false)
        //   -> true_block
        //   -> false_block
        // exit
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let cond = cfg.create_block("cond");
        let true_blk = cfg.create_block("if_true");
        let false_blk = cfg.create_block("if_false");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, cond, CfgEdgeKind::FallThrough);
        cfg.add_edge(cond, true_blk, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(cond, false_blk, CfgEdgeKind::ConditionalFalse);
        cfg.add_edge(true_blk, exit, CfgEdgeKind::FallThrough);
        cfg.add_edge(false_blk, exit, CfgEdgeKind::FallThrough);

        cfg.add_statement(entry, SourceSpan::new(0, 5));
        cfg.add_statement(cond, SourceSpan::new(6, 10));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![make_def("x", entry, 0, 0, 5)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            cond,
            BlockDataFlow {
                defs: vec![],
                uses: vec![make_use("x", cond, 0, 6, 7)],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(true_blk, empty_block_data());
        block_facts.insert(false_blk, empty_block_data());
        block_facts.insert(exit, empty_block_data());

        let constants = vec![ConstantAssignment {
            var: "x".into(),
            value: 0,
            block_id: entry,
            def_index: 0,
        }];

        let result = analyze_constants_with_values(&cfg, &block_facts, &constants);
        // x == 0 -> condition is always false
        assert!(!result.dead_branches.is_empty());
        let db = &result.dead_branches[0];
        assert_eq!(db.block_id, cond);
        assert!(!db.always_value); // always false
    }

    #[test]
    fn test_dead_branch_always_true() {
        // entry: x = 1
        // cond_block: if x  (x == 1 -> always true)
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let cond = cfg.create_block("cond");
        let true_blk = cfg.create_block("if_true");
        let false_blk = cfg.create_block("if_false");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, cond, CfgEdgeKind::FallThrough);
        cfg.add_edge(cond, true_blk, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(cond, false_blk, CfgEdgeKind::ConditionalFalse);
        cfg.add_edge(true_blk, exit, CfgEdgeKind::FallThrough);
        cfg.add_edge(false_blk, exit, CfgEdgeKind::FallThrough);

        cfg.add_statement(entry, SourceSpan::new(0, 5));
        cfg.add_statement(cond, SourceSpan::new(6, 10));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![make_def("x", entry, 0, 0, 5)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            cond,
            BlockDataFlow {
                defs: vec![],
                uses: vec![make_use("x", cond, 0, 6, 7)],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(true_blk, empty_block_data());
        block_facts.insert(false_blk, empty_block_data());
        block_facts.insert(exit, empty_block_data());

        let constants = vec![ConstantAssignment {
            var: "x".into(),
            value: 1,
            block_id: entry,
            def_index: 0,
        }];

        let result = analyze_constants_with_values(&cfg, &block_facts, &constants);
        assert!(!result.dead_branches.is_empty());
        let db = result
            .dead_branches
            .iter()
            .find(|d| d.block_id == cond)
            .expect("expected dead branch at cond block");
        assert!(db.always_value); // always true
    }

    #[test]
    fn test_no_dead_branch_when_non_constant() {
        // entry: x = f()  (non-constant)
        // cond_block: if x
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let cond = cfg.create_block("cond");
        let true_blk = cfg.create_block("if_true");
        let false_blk = cfg.create_block("if_false");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, cond, CfgEdgeKind::FallThrough);
        cfg.add_edge(cond, true_blk, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(cond, false_blk, CfgEdgeKind::ConditionalFalse);
        cfg.add_edge(true_blk, exit, CfgEdgeKind::FallThrough);
        cfg.add_edge(false_blk, exit, CfgEdgeKind::FallThrough);

        cfg.add_statement(entry, SourceSpan::new(0, 10));
        cfg.add_statement(cond, SourceSpan::new(11, 15));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![make_def("x", entry, 0, 0, 10)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            cond,
            BlockDataFlow {
                defs: vec![],
                uses: vec![make_use("x", cond, 0, 11, 12)],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(true_blk, empty_block_data());
        block_facts.insert(false_blk, empty_block_data());
        block_facts.insert(exit, empty_block_data());

        // No ConstantAssignment for x -> Bottom -> no dead branch
        let result = analyze_constants_with_values(&cfg, &block_facts, &[]);
        let cond_dead: Vec<_> = result
            .dead_branches
            .iter()
            .filter(|d| d.block_id == cond)
            .collect();
        assert!(cond_dead.is_empty());
    }

    #[test]
    fn test_loop_constant_propagation() {
        // Constant assigned before a loop should propagate into the loop
        // header (at least on the first iteration path).
        //
        //   entry: x = 42
        //     |
        //   header (loop condition uses x)
        //   /    \
        //  body  after
        //  |       |
        //  +->header exit
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let header = cfg.create_block("loop_header");
        let body = cfg.create_block("loop_body");
        let after = cfg.create_block("after_loop");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, header, CfgEdgeKind::FallThrough);
        cfg.add_edge(header, body, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(header, after, CfgEdgeKind::ConditionalFalse);
        cfg.add_edge(body, header, CfgEdgeKind::LoopBack);
        cfg.add_edge(after, exit, CfgEdgeKind::FallThrough);

        cfg.add_statement(entry, SourceSpan::new(0, 6));
        cfg.add_statement(header, SourceSpan::new(7, 12));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![make_def("x", entry, 0, 0, 6)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            header,
            BlockDataFlow {
                defs: vec![],
                uses: vec![make_use("x", header, 0, 7, 8)],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(body, empty_block_data());
        block_facts.insert(after, empty_block_data());
        block_facts.insert(exit, empty_block_data());

        let constants = vec![ConstantAssignment {
            var: "x".into(),
            value: 42,
            block_id: entry,
            def_index: 0,
        }];

        let result = analyze_constants_with_values(&cfg, &block_facts, &constants);
        assert_eq!(result.constant_vars.get("x"), Some(&42));
    }

    #[test]
    fn test_const_value_accessors() {
        assert!(ConstValue::Top.is_top());
        assert!(!ConstValue::Top.is_bottom());
        assert!(ConstValue::Bottom.is_bottom());
        assert!(!ConstValue::Bottom.is_top());
        assert_eq!(ConstValue::Constant(3).as_constant(), Some(3));
        assert_eq!(ConstValue::Top.as_constant(), None);
        assert_eq!(ConstValue::Bottom.as_constant(), None);
    }

    #[test]
    fn test_const_prop_result_empty_cfg() {
        // Edge case: CFG with no blocks at all (no entry/exit).
        let cfg = ControlFlowGraph::new("empty");
        let block_facts = HashMap::new();
        let result = analyze_constants(&cfg, &block_facts);
        assert!(result.dead_branches.is_empty());
        assert!(result.constant_vars.is_empty());
    }

    #[test]
    fn test_const_prop_result_entry_exit_only() {
        // Minimal CFG: entry -> exit, no defs or uses.
        let mut cfg = ControlFlowGraph::new("minimal");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);

        let mut block_facts = HashMap::new();
        block_facts.insert(entry, empty_block_data());
        block_facts.insert(exit, empty_block_data());

        let result = analyze_constants(&cfg, &block_facts);
        assert!(result.dead_branches.is_empty());
        assert!(result.constant_vars.is_empty());
    }

    #[test]
    fn test_negative_constant() {
        // x = -1 (negative constant)
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![make_def("x", entry, 0, 0, 6)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(exit, empty_block_data());

        let constants = vec![ConstantAssignment {
            var: "x".into(),
            value: -1,
            block_id: entry,
            def_index: 0,
        }];

        let result = analyze_constants_with_values(&cfg, &block_facts, &constants);
        assert_eq!(result.constant_vars.get("x"), Some(&-1));
    }

    #[test]
    fn test_reassignment_kills_constant() {
        // entry: x = 5
        // body:  x = f()  (non-constant reassignment)
        // exit
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
                defs: vec![make_def("x", entry, 0, 0, 5)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            body,
            BlockDataFlow {
                defs: vec![make_def("x", body, 0, 6, 15)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(exit, empty_block_data());

        // Only the first assignment is constant; the second is not.
        let constants = vec![ConstantAssignment {
            var: "x".into(),
            value: 5,
            block_id: entry,
            def_index: 0,
        }];

        let result = analyze_constants_with_values(&cfg, &block_facts, &constants);
        // x is reassigned to Bottom in body -> overall not constant
        assert!(!result.constant_vars.contains_key("x"));
    }

    // ==== New tests for extended ConstValue variants =======================

    // ---- StringConst meet tests -------------------------------------------

    #[test]
    fn test_meet_string_same() {
        let a = ConstValue::StringConst("hello".into());
        let b = ConstValue::StringConst("hello".into());
        assert_eq!(a.meet(&b), ConstValue::StringConst("hello".into()));
    }

    #[test]
    fn test_meet_string_different() {
        let a = ConstValue::StringConst("hello".into());
        let b = ConstValue::StringConst("world".into());
        assert_eq!(a.meet(&b), ConstValue::Bottom);
    }

    #[test]
    fn test_meet_string_with_top() {
        let a = ConstValue::StringConst("hi".into());
        assert_eq!(
            ConstValue::Top.meet(&a),
            ConstValue::StringConst("hi".into())
        );
        assert_eq!(
            a.meet(&ConstValue::Top),
            ConstValue::StringConst("hi".into())
        );
    }

    #[test]
    fn test_meet_string_with_bottom() {
        let a = ConstValue::StringConst("hi".into());
        assert_eq!(a.meet(&ConstValue::Bottom), ConstValue::Bottom);
    }

    // ---- BoolConst meet tests ---------------------------------------------

    #[test]
    fn test_meet_bool_same() {
        assert_eq!(
            ConstValue::BoolConst(true).meet(&ConstValue::BoolConst(true)),
            ConstValue::BoolConst(true)
        );
        assert_eq!(
            ConstValue::BoolConst(false).meet(&ConstValue::BoolConst(false)),
            ConstValue::BoolConst(false)
        );
    }

    #[test]
    fn test_meet_bool_different() {
        assert_eq!(
            ConstValue::BoolConst(true).meet(&ConstValue::BoolConst(false)),
            ConstValue::Bottom
        );
    }

    #[test]
    fn test_meet_bool_with_top() {
        assert_eq!(
            ConstValue::Top.meet(&ConstValue::BoolConst(true)),
            ConstValue::BoolConst(true)
        );
    }

    // ---- Cross-type meet tests --------------------------------------------

    #[test]
    fn test_meet_int_with_string() {
        let a = ConstValue::Constant(42);
        let b = ConstValue::StringConst("42".into());
        assert_eq!(a.meet(&b), ConstValue::Bottom);
    }

    #[test]
    fn test_meet_int_with_bool() {
        let a = ConstValue::Constant(1);
        let b = ConstValue::BoolConst(true);
        assert_eq!(a.meet(&b), ConstValue::Bottom);
    }

    #[test]
    fn test_meet_string_with_bool() {
        let a = ConstValue::StringConst("true".into());
        let b = ConstValue::BoolConst(true);
        assert_eq!(a.meet(&b), ConstValue::Bottom);
    }

    // ---- is_truthy tests --------------------------------------------------

    #[test]
    fn test_is_truthy_integer() {
        assert_eq!(ConstValue::Constant(0).is_truthy(), Some(false));
        assert_eq!(ConstValue::Constant(1).is_truthy(), Some(true));
        assert_eq!(ConstValue::Constant(-1).is_truthy(), Some(true));
        assert_eq!(ConstValue::Constant(42).is_truthy(), Some(true));
    }

    #[test]
    fn test_is_truthy_bool() {
        assert_eq!(ConstValue::BoolConst(true).is_truthy(), Some(true));
        assert_eq!(ConstValue::BoolConst(false).is_truthy(), Some(false));
    }

    #[test]
    fn test_is_truthy_string() {
        assert_eq!(ConstValue::StringConst("".into()).is_truthy(), Some(false));
        assert_eq!(
            ConstValue::StringConst("hello".into()).is_truthy(),
            Some(true)
        );
    }

    #[test]
    fn test_is_truthy_top_bottom() {
        assert_eq!(ConstValue::Top.is_truthy(), None);
        assert_eq!(ConstValue::Bottom.is_truthy(), None);
    }

    // ---- as_string / as_bool accessor tests -------------------------------

    #[test]
    fn test_as_string() {
        assert_eq!(
            ConstValue::StringConst("abc".into()).as_string(),
            Some("abc")
        );
        assert_eq!(ConstValue::Constant(42).as_string(), None);
        assert_eq!(ConstValue::Top.as_string(), None);
    }

    #[test]
    fn test_as_bool() {
        assert_eq!(ConstValue::BoolConst(true).as_bool(), Some(true));
        assert_eq!(ConstValue::BoolConst(false).as_bool(), Some(false));
        assert_eq!(ConstValue::Constant(1).as_bool(), None);
        assert_eq!(ConstValue::Top.as_bool(), None);
    }

    // ---- evaluate_expression tests ----------------------------------------

    #[test]
    fn test_evaluate_expression_arithmetic() {
        let mut env = ConstEnv::new();
        env.set("a", ConstValue::Constant(10));
        env.set("b", ConstValue::Constant(3));

        assert_eq!(evaluate_expression("a + b", &env), ConstValue::Constant(13));
        assert_eq!(evaluate_expression("a * b", &env), ConstValue::Constant(30));
        assert_eq!(evaluate_expression("a - b", &env), ConstValue::Constant(7));
    }

    #[test]
    fn test_evaluate_expression_string() {
        let env = ConstEnv::new();
        assert_eq!(
            evaluate_expression("\"hello\" + \" world\"", &env),
            ConstValue::StringConst("hello world".into())
        );
    }

    #[test]
    fn test_evaluate_expression_comparison() {
        let mut env = ConstEnv::new();
        env.set("x", ConstValue::Constant(5));
        assert_eq!(
            evaluate_expression("x > 10", &env),
            ConstValue::BoolConst(false)
        );
        assert_eq!(
            evaluate_expression("x == 5", &env),
            ConstValue::BoolConst(true)
        );
    }

    // ---- analyze_constants_with_expressions tests -------------------------

    #[test]
    fn test_expr_propagation_simple() {
        // entry: a = 2, b = 3
        // body:  x = a + b  (expression)
        // exit
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let body = cfg.create_block("body");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, body, CfgEdgeKind::FallThrough);
        cfg.add_edge(body, exit, CfgEdgeKind::FallThrough);

        cfg.add_statement(entry, SourceSpan::new(0, 12));
        cfg.add_statement(body, SourceSpan::new(13, 22));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![
                    make_def("a", entry, 0, 0, 5),
                    make_def("b", entry, 1, 6, 12),
                ],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            body,
            BlockDataFlow {
                defs: vec![make_def("x", body, 0, 13, 22)],
                uses: vec![
                    make_use("a", body, 0, 17, 18),
                    make_use("b", body, 0, 21, 22),
                ],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(exit, empty_block_data());

        let int_constants = vec![
            ConstantAssignment {
                var: "a".into(),
                value: 2,
                block_id: entry,
                def_index: 0,
            },
            ConstantAssignment {
                var: "b".into(),
                value: 3,
                block_id: entry,
                def_index: 1,
            },
        ];

        let expr_assignments = vec![ExprConstantAssignment {
            var: "x".into(),
            expr: "a + b".into(),
            block_id: body,
            def_index: 0,
        }];

        let result = analyze_constants_with_expressions(
            &cfg,
            &block_facts,
            &int_constants,
            &expr_assignments,
        );
        // x = a + b = 2 + 3 = 5
        assert_eq!(result.constant_vars.get("a"), Some(&2));
        assert_eq!(result.constant_vars.get("b"), Some(&3));
        assert_eq!(result.constant_vars.get("x"), Some(&5));
    }

    #[test]
    fn test_expr_propagation_dead_branch() {
        // entry:   x = 2 + 3  (expression, resolves to 5)
        // compute: cond_result = x > 10  (expression, resolves to false)
        // cond:    if cond_result (uses cond_result; always false)
        //   -> true_block
        //   -> false_block
        // exit
        //
        // The condition variable is computed in a block *before* the
        // conditional so that it appears in env_in[cond] for dead-branch
        // detection.
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let compute = cfg.create_block("compute");
        let cond = cfg.create_block("cond");
        let true_blk = cfg.create_block("if_true");
        let false_blk = cfg.create_block("if_false");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, compute, CfgEdgeKind::FallThrough);
        cfg.add_edge(compute, cond, CfgEdgeKind::FallThrough);
        cfg.add_edge(cond, true_blk, CfgEdgeKind::ConditionalTrue);
        cfg.add_edge(cond, false_blk, CfgEdgeKind::ConditionalFalse);
        cfg.add_edge(true_blk, exit, CfgEdgeKind::FallThrough);
        cfg.add_edge(false_blk, exit, CfgEdgeKind::FallThrough);

        cfg.add_statement(entry, SourceSpan::new(0, 9));
        cfg.add_statement(compute, SourceSpan::new(10, 26));
        cfg.add_statement(cond, SourceSpan::new(27, 35));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![make_def("x", entry, 0, 0, 9)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            compute,
            BlockDataFlow {
                defs: vec![make_def("cond_result", compute, 0, 10, 26)],
                uses: vec![make_use("x", compute, 0, 10, 11)],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            cond,
            BlockDataFlow {
                defs: vec![],
                uses: vec![make_use("cond_result", cond, 0, 27, 35)],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(true_blk, empty_block_data());
        block_facts.insert(false_blk, empty_block_data());
        block_facts.insert(exit, empty_block_data());

        let expr_assignments = vec![
            ExprConstantAssignment {
                var: "x".into(),
                expr: "2 + 3".into(),
                block_id: entry,
                def_index: 0,
            },
            ExprConstantAssignment {
                var: "cond_result".into(),
                expr: "x > 10".into(),
                block_id: compute,
                def_index: 0,
            },
        ];

        let result = analyze_constants_with_expressions(&cfg, &block_facts, &[], &expr_assignments);

        // x should resolve to 5
        assert_eq!(result.constant_vars.get("x"), Some(&5));

        // cond_result = (5 > 10) = false -> dead branch at cond block
        let db = result.dead_branches.iter().find(|d| d.block_id == cond);
        assert!(db.is_some(), "expected a dead branch at the cond block");
        assert!(!db.unwrap().always_value); // always false
    }

    #[test]
    fn test_expr_propagation_chain() {
        // entry: a = 10
        // b1:    b = a * 2   -> 20
        // b2:    c = b + 5   -> 25
        // exit
        let mut cfg = ControlFlowGraph::new("test");
        let entry = cfg.create_entry();
        let b1 = cfg.create_block("b1");
        let b2 = cfg.create_block("b2");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, b1, CfgEdgeKind::FallThrough);
        cfg.add_edge(b1, b2, CfgEdgeKind::FallThrough);
        cfg.add_edge(b2, exit, CfgEdgeKind::FallThrough);

        cfg.add_statement(entry, SourceSpan::new(0, 6));
        cfg.add_statement(b1, SourceSpan::new(7, 16));
        cfg.add_statement(b2, SourceSpan::new(17, 26));

        let mut block_facts = HashMap::new();
        block_facts.insert(
            entry,
            BlockDataFlow {
                defs: vec![make_def("a", entry, 0, 0, 6)],
                uses: vec![],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            b1,
            BlockDataFlow {
                defs: vec![make_def("b", b1, 0, 7, 16)],
                uses: vec![make_use("a", b1, 0, 11, 12)],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(
            b2,
            BlockDataFlow {
                defs: vec![make_def("c", b2, 0, 17, 26)],
                uses: vec![make_use("b", b2, 0, 21, 22)],
                gen: HashSet::new(),
                kill: HashSet::new(),
            },
        );
        block_facts.insert(exit, empty_block_data());

        let int_constants = vec![ConstantAssignment {
            var: "a".into(),
            value: 10,
            block_id: entry,
            def_index: 0,
        }];

        let expr_assignments = vec![
            ExprConstantAssignment {
                var: "b".into(),
                expr: "a * 2".into(),
                block_id: b1,
                def_index: 0,
            },
            ExprConstantAssignment {
                var: "c".into(),
                expr: "b + 5".into(),
                block_id: b2,
                def_index: 0,
            },
        ];

        let result = analyze_constants_with_expressions(
            &cfg,
            &block_facts,
            &int_constants,
            &expr_assignments,
        );

        assert_eq!(result.constant_vars.get("a"), Some(&10));
        assert_eq!(result.constant_vars.get("b"), Some(&20));
        assert_eq!(result.constant_vars.get("c"), Some(&25));
    }
}
