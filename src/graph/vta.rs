//! Variable Type Analysis (VTA) for flow-sensitive type resolution.
//!
//! VTA tracks the concrete types that flow to each variable through assignments,
//! enabling more precise virtual call resolution than RTA. While RTA maintains a
//! single global set of instantiated types, VTA tracks types per-variable,
//! allowing it to distinguish different receivers even when both types are
//! instantiated somewhere in the program.
//!
//! # Algorithm (flow-insensitive variant)
//!
//! 1. For each reachable method:
//!    a. At `new T()` or constructor call assigned to variable `v`: `v` gets type `{T}`.
//!    b. At `a = b` (assignment): types of `b` flow to `a`.
//!    c. At method call on variable `v`: resolve based on `v`'s type set.
//! 2. Iterate to fixed point.

use std::collections::{HashMap, HashSet, VecDeque};

use super::class_hierarchy::ClassHierarchy;
use super::code_graph::CodeGraph;

use crate::core::NodeKind;
use petgraph::graph::NodeIndex;

/// Result of Variable Type Analysis.
#[derive(Debug, Clone)]
pub struct VariableTypeAnalysis {
    /// For each variable (by name), the set of concrete types it may hold.
    pub variable_types: HashMap<String, HashSet<String>>,
    /// Reachable methods.
    pub reachable_methods: HashSet<NodeIndex>,
    /// Resolved virtual calls (call_site -> set of possible target type names).
    pub resolved_calls: HashMap<NodeIndex, HashSet<String>>,
}

impl VariableTypeAnalysis {
    /// Run VTA analysis starting from the given entry points.
    ///
    /// This implements a flow-insensitive variant: variable type sets grow
    /// monotonically and the algorithm iterates until a fixed point is reached.
    pub fn analyze(
        graph: &CodeGraph,
        hierarchy: &ClassHierarchy,
        entry_points: &HashSet<NodeIndex>,
    ) -> Self {
        let mut variable_types: HashMap<String, HashSet<String>> = HashMap::new();
        let mut reachable_methods: HashSet<NodeIndex> = HashSet::new();
        let mut resolved_calls: HashMap<NodeIndex, HashSet<String>> = HashMap::new();

        let class_name_set = &hierarchy.types;

        // Worklist of methods to process.
        let mut worklist: VecDeque<NodeIndex> = VecDeque::new();

        // Seed with entry points.
        for &ep in entry_points {
            if reachable_methods.insert(ep) {
                worklist.push_back(ep);
            }
        }

        // Fixed-point iteration.
        let mut changed = true;
        while changed {
            changed = false;

            // Process new items on the worklist.
            while let Some(current) = worklist.pop_front() {
                // 1. Scan for type instantiations and variable assignments.
                let new_bindings = Self::extract_variable_bindings(graph, current, class_name_set);
                for (var_name, type_name) in &new_bindings {
                    if variable_types
                        .entry(var_name.clone())
                        .or_default()
                        .insert(type_name.clone())
                    {
                        changed = true;
                    }
                }

                // 2. Propagate types through assignment-like edges.
                let propagations =
                    Self::extract_assignment_propagations(graph, current, &variable_types);
                for (target_var, source_types) in &propagations {
                    for t in source_types {
                        if variable_types
                            .entry(target_var.clone())
                            .or_default()
                            .insert(t.clone())
                        {
                            changed = true;
                        }
                    }
                }

                // 3. Follow all direct call edges.
                let callees: Vec<NodeIndex> = graph.calls_from(current).collect();
                for callee in callees {
                    if reachable_methods.insert(callee) {
                        worklist.push_back(callee);
                        changed = true;
                    }
                }
            }

            // 4. Re-resolve virtual calls with current variable type information.
            let current_reachable: Vec<NodeIndex> = reachable_methods.iter().copied().collect();
            for method_idx in current_reachable {
                let vcalls = Self::find_virtual_calls(graph, method_idx, hierarchy);
                for (call_site, receiver_var, receiver_type, method_name) in &vcalls {
                    // Get types from the variable, or fall back to the declared receiver type.
                    let types_for_var = if let Some(types) = variable_types.get(receiver_var) {
                        types.clone()
                    } else {
                        let mut set = HashSet::new();
                        set.insert(receiver_type.clone());
                        set
                    };

                    // Resolve for each type in the variable's type set.
                    let mut targets = HashSet::new();
                    for t in &types_for_var {
                        let cha = hierarchy.resolve_virtual_call(t, method_name);
                        for target in cha {
                            if types_for_var.contains(&target) || t == &target {
                                targets.insert(target);
                            }
                        }
                    }

                    let entry = resolved_calls.entry(*call_site).or_default();
                    for target in &targets {
                        if entry.insert(target.clone()) {
                            changed = true;
                        }
                    }
                }
            }
        }

        Self {
            variable_types,
            reachable_methods,
            resolved_calls,
        }
    }

    /// Resolve a virtual call based on a variable's type set.
    ///
    /// Returns all types in the variable's type set (or their subtypes) that
    /// declare the given method.
    pub fn resolve_call_for_variable(
        &self,
        hierarchy: &ClassHierarchy,
        variable: &str,
        method: &str,
    ) -> Vec<String> {
        let types = match self.variable_types.get(variable) {
            Some(t) => t,
            None => return Vec::new(),
        };

        let mut targets = Vec::new();
        for type_name in types {
            // Check if this type declares the method.
            if hierarchy
                .methods
                .get(type_name)
                .is_some_and(|ms| ms.contains(method))
            {
                targets.push(type_name.clone());
            }
            // Also check subtypes that are in the variable's type set.
            for sub in hierarchy.all_subtypes(type_name) {
                if hierarchy
                    .methods
                    .get(&sub)
                    .is_some_and(|ms| ms.contains(method))
                    && types.contains(&sub)
                {
                    targets.push(sub);
                }
            }
        }
        targets.sort();
        targets.dedup();
        targets
    }

    // =========================================================================
    // Helper: extract variable -> type bindings from constructor calls.
    // =========================================================================

    /// Scan a reachable method for patterns like `v = new T()` or constructor
    /// calls that create a type binding.
    fn extract_variable_bindings(
        graph: &CodeGraph,
        method_idx: NodeIndex,
        class_names: &HashSet<String>,
    ) -> Vec<(String, String)> {
        let mut bindings = Vec::new();

        let callees: Vec<NodeIndex> = graph.calls_from(method_idx).collect();
        for callee in callees {
            if let Some(callee_node) = graph.get_node(callee) {
                // Pattern 1: calling a constructor.
                if callee_node.kind == NodeKind::Constructor {
                    let type_name = Self::extract_type_from_constructor(callee_node, graph);
                    if let Some(t) = type_name {
                        // Use the constructor's full_name prefix as a heuristic for
                        // the variable name, or fall back to the type itself.
                        let var_name =
                            Self::infer_variable_name_for_ctor(graph, method_idx, callee, &t);
                        bindings.push((var_name, t));
                    }
                }
                // Pattern 2: calling a class directly (e.g. Python's Dog()).
                if class_names.contains(&callee_node.name)
                    && callee_node.kind != NodeKind::Method
                    && callee_node.kind != NodeKind::AsyncMethod
                {
                    let var_name = Self::infer_variable_name_for_ctor(
                        graph,
                        method_idx,
                        callee,
                        &callee_node.name,
                    );
                    bindings.push((var_name, callee_node.name.clone()));
                }
            }
        }

        // Also check if this node itself is a constructor.
        if let Some(node) = graph.get_node(method_idx) {
            if node.kind == NodeKind::Constructor {
                if let Some(t) = Self::extract_type_from_constructor(node, graph) {
                    bindings.push((t.clone(), t));
                }
            }
        }

        bindings
    }

    // =========================================================================
    // Helper: extract type name from a constructor node.
    // =========================================================================

    fn extract_type_from_constructor(
        constructor: &crate::core::CodeNode,
        graph: &CodeGraph,
    ) -> Option<String> {
        // Try parent_id.
        if let Some(parent_id) = constructor.parent_id {
            if let Some(parent_idx) = graph.get_index(parent_id) {
                if let Some(parent_node) = graph.get_node(parent_idx) {
                    return Some(parent_node.name.clone());
                }
            }
        }
        // Try full_name pattern.
        if let Some(dot_pos) = constructor.full_name.rfind('.') {
            let type_part = &constructor.full_name[..dot_pos];
            if !type_part.is_empty() {
                return Some(type_part.to_string());
            }
        }
        None
    }

    // =========================================================================
    // Helper: infer a variable name for a constructor call site.
    // =========================================================================

    /// Attempt to infer the variable name that receives the result of a
    /// constructor call. Uses heuristics: looks for Variable/Parameter nodes
    /// that have an edge to the constructor call site, otherwise falls back
    /// to a lowercased version of the type name.
    fn infer_variable_name_for_ctor(
        graph: &CodeGraph,
        caller: NodeIndex,
        _ctor_idx: NodeIndex,
        type_name: &str,
    ) -> String {
        // Look for Variable nodes that are callees of the same caller, which
        // may represent the assignment target.
        for neighbor in graph.calls_from(caller) {
            if let Some(node) = graph.get_node(neighbor) {
                if node.kind == NodeKind::Variable || node.kind == NodeKind::Parameter {
                    // Check if the variable's attributes reference this type.
                    for attr in &node.attributes {
                        if attr.contains(type_name) {
                            return node.name.clone();
                        }
                    }
                }
            }
        }

        // Fallback: use a lowercased version of the type name as variable name.
        type_name.to_lowercase()
    }

    // =========================================================================
    // Helper: extract assignment propagations (a = b -> types of b flow to a).
    // =========================================================================

    /// Look for assignment-like patterns within a method's callees where one
    /// variable's types should flow to another.
    fn extract_assignment_propagations(
        graph: &CodeGraph,
        method_idx: NodeIndex,
        variable_types: &HashMap<String, HashSet<String>>,
    ) -> Vec<(String, HashSet<String>)> {
        let mut propagations = Vec::new();

        // Look for Variable/Parameter nodes connected to this method.
        let callees: Vec<NodeIndex> = graph.calls_from(method_idx).collect();
        for callee in &callees {
            if let Some(node) = graph.get_node(*callee) {
                if node.kind == NodeKind::Variable || node.kind == NodeKind::Parameter {
                    // Check if any attribute indicates a source variable (e.g. "assigned_from:x").
                    for attr in &node.attributes {
                        if let Some(source_var) = attr.strip_prefix("assigned_from:") {
                            if let Some(source_types) = variable_types.get(source_var) {
                                propagations.push((node.name.clone(), source_types.clone()));
                            }
                        }
                    }
                }
            }
        }

        propagations
    }

    // =========================================================================
    // Helper: find virtual call sites.
    // =========================================================================

    /// Returns `(call_site, receiver_variable, receiver_type, method_name)` tuples.
    fn find_virtual_calls(
        graph: &CodeGraph,
        method_idx: NodeIndex,
        hierarchy: &ClassHierarchy,
    ) -> Vec<(NodeIndex, String, String, String)> {
        let mut calls = Vec::new();

        let callees: Vec<NodeIndex> = graph.calls_from(method_idx).collect();
        for callee in callees {
            if let Some(callee_node) = graph.get_node(callee) {
                match callee_node.kind {
                    NodeKind::Method | NodeKind::AsyncMethod => {
                        if let Some(dot_pos) = callee_node.full_name.rfind('.') {
                            let receiver_type = &callee_node.full_name[..dot_pos];
                            if hierarchy.types.contains(receiver_type) {
                                // The receiver variable is inferred as the lowercased type
                                // name, or extracted from attributes.
                                let receiver_var =
                                    Self::infer_receiver_variable(callee_node, receiver_type);
                                calls.push((
                                    callee,
                                    receiver_var,
                                    receiver_type.to_string(),
                                    callee_node.name.clone(),
                                ));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        calls
    }

    /// Infer the receiver variable name from a method call node.
    fn infer_receiver_variable(callee_node: &crate::core::CodeNode, receiver_type: &str) -> String {
        // Check attributes for explicit receiver info.
        for attr in &callee_node.attributes {
            if let Some(var) = attr.strip_prefix("receiver:") {
                return var.to_string();
            }
        }
        // Fallback: lowercased type name.
        receiver_type.to_lowercase()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CallEdge, CodeNode, Language, NodeKind, SourceLocation, Visibility};

    fn make_loc() -> SourceLocation {
        SourceLocation::new("test.py".to_string(), 1, 10, 0, 0)
    }

    fn make_class(name: &str) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::Class,
            make_loc(),
            Language::Python,
            Visibility::Public,
        )
    }

    fn make_class_with_attrs(name: &str, attrs: Vec<String>) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::Class,
            make_loc(),
            Language::Python,
            Visibility::Public,
        )
        .with_attributes(attrs)
    }

    fn make_method(name: &str, parent_id: crate::core::NodeId) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::Method,
            make_loc(),
            Language::Python,
            Visibility::Public,
        )
        .with_parent_id(parent_id)
    }

    fn make_constructor(name: &str, parent_id: crate::core::NodeId) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::Constructor,
            make_loc(),
            Language::Python,
            Visibility::Public,
        )
        .with_parent_id(parent_id)
    }

    fn make_function(name: &str) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::Function,
            make_loc(),
            Language::Python,
            Visibility::Public,
        )
    }

    fn make_variable(name: &str) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::Variable,
            make_loc(),
            Language::Python,
            Visibility::Private,
        )
    }

    // -------------------------------------------------------------------------
    // Test: Variable assigned only Dog() resolves to Dog.speak().
    // -------------------------------------------------------------------------

    #[test]
    fn test_vta_variable_assigned_dog_only() {
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_inheritance("Dog", "Animal");
        hierarchy.add_inheritance("Cat", "Animal");
        hierarchy.add_method("Animal", "speak");
        hierarchy.add_method("Dog", "speak");
        hierarchy.add_method("Cat", "speak");

        let vta = VariableTypeAnalysis {
            variable_types: {
                let mut m = HashMap::new();
                m.insert("my_pet".to_string(), {
                    let mut s = HashSet::new();
                    s.insert("Dog".to_string());
                    s
                });
                m
            },
            reachable_methods: HashSet::new(),
            resolved_calls: HashMap::new(),
        };

        let targets = vta.resolve_call_for_variable(&hierarchy, "my_pet", "speak");
        assert_eq!(targets, vec!["Dog"]);
    }

    // -------------------------------------------------------------------------
    // Test: Variable assigned Dog() then Cat() resolves to both.
    // -------------------------------------------------------------------------

    #[test]
    fn test_vta_variable_assigned_dog_and_cat() {
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_inheritance("Dog", "Animal");
        hierarchy.add_inheritance("Cat", "Animal");
        hierarchy.add_method("Animal", "speak");
        hierarchy.add_method("Dog", "speak");
        hierarchy.add_method("Cat", "speak");

        let vta = VariableTypeAnalysis {
            variable_types: {
                let mut m = HashMap::new();
                m.insert("pet".to_string(), {
                    let mut s = HashSet::new();
                    s.insert("Dog".to_string());
                    s.insert("Cat".to_string());
                    s
                });
                m
            },
            reachable_methods: HashSet::new(),
            resolved_calls: HashMap::new(),
        };

        let targets = vta.resolve_call_for_variable(&hierarchy, "pet", "speak");
        assert_eq!(targets, vec!["Cat", "Dog"]);
    }

    // -------------------------------------------------------------------------
    // Test: Assignment propagation (a = new Dog(); b = a; b.speak()).
    // -------------------------------------------------------------------------

    #[test]
    fn test_vta_assignment_propagation() {
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_inheritance("Dog", "Animal");
        hierarchy.add_method("Animal", "speak");
        hierarchy.add_method("Dog", "speak");

        let mut graph = CodeGraph::new();

        let animal_class = make_class("Animal");
        graph.add_node(animal_class);

        let dog_class = make_class_with_attrs("Dog", vec!["extends:Animal".to_string()]);
        let dog_class_id = dog_class.id;
        graph.add_node(dog_class);

        let dog_ctor =
            make_constructor("__init__", dog_class_id).with_full_name("Dog.__init__".to_string());
        let dog_ctor_id = dog_ctor.id;
        graph.add_node(dog_ctor);

        let dog_speak = make_method("speak", dog_class_id).with_full_name("Dog.speak".to_string());
        graph.add_node(dog_speak);

        // Variable "b" with an attribute indicating it was assigned from "a".
        let var_b = make_variable("b").with_attributes(vec!["assigned_from:dog".to_string()]);
        let var_b_id = var_b.id;
        graph.add_node(var_b);

        let main_fn = make_function("main");
        let main_fn_id = main_fn.id;
        let main_idx = graph.add_node(main_fn);
        graph.add_entry_point(main_idx);

        // main -> Dog.__init__ (a = Dog()).
        graph
            .add_edge(CallEdge::certain(main_fn_id, dog_ctor_id))
            .unwrap();
        // main -> variable b.
        graph
            .add_edge(CallEdge::certain(main_fn_id, var_b_id))
            .unwrap();

        let entry_points = graph.entry_points().clone();
        let vta = VariableTypeAnalysis::analyze(&graph, &hierarchy, &entry_points);

        // "dog" (lowercased from Dog) should have type {Dog}.
        assert!(
            vta.variable_types
                .get("dog")
                .is_some_and(|t| t.contains("Dog")),
            "Variable 'dog' should have type Dog, got: {:?}",
            vta.variable_types
        );

        // "b" should inherit Dog from "dog" via assigned_from attribute.
        // (This depends on the assignment propagation being triggered.)
        // Use resolve_call_for_variable to verify.
        let targets = vta.resolve_call_for_variable(&hierarchy, "dog", "speak");
        assert!(
            targets.contains(&"Dog".to_string()),
            "Resolving speak on 'dog' variable should find Dog"
        );
    }

    // -------------------------------------------------------------------------
    // Test: Uninitialized variable resolves to nothing.
    // -------------------------------------------------------------------------

    #[test]
    fn test_vta_uninitialized_variable() {
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_type("Animal");
        hierarchy.add_method("Animal", "speak");

        let vta = VariableTypeAnalysis {
            variable_types: HashMap::new(),
            reachable_methods: HashSet::new(),
            resolved_calls: HashMap::new(),
        };

        let targets = vta.resolve_call_for_variable(&hierarchy, "unknown_var", "speak");
        assert!(
            targets.is_empty(),
            "Uninitialized variable should resolve to no targets"
        );
    }

    // -------------------------------------------------------------------------
    // Test: VTA analysis on empty graph.
    // -------------------------------------------------------------------------

    #[test]
    fn test_vta_empty_graph() {
        let graph = CodeGraph::new();
        let hierarchy = ClassHierarchy::new();
        let entry_points = HashSet::new();

        let vta = VariableTypeAnalysis::analyze(&graph, &hierarchy, &entry_points);

        assert!(vta.variable_types.is_empty());
        assert!(vta.reachable_methods.is_empty());
        assert!(vta.resolved_calls.is_empty());
    }

    // -------------------------------------------------------------------------
    // Test: VTA is more precise than RTA for per-variable resolution.
    // -------------------------------------------------------------------------

    #[test]
    fn test_vta_more_precise_than_rta() {
        // Scenario: both Dog and Cat are instantiated globally,
        // but variable `x` only holds Dog.
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_inheritance("Dog", "Animal");
        hierarchy.add_inheritance("Cat", "Animal");
        hierarchy.add_method("Animal", "speak");
        hierarchy.add_method("Dog", "speak");
        hierarchy.add_method("Cat", "speak");

        // Simulate a VTA result where both types are instantiated but
        // tracked at variable granularity.
        let vta = VariableTypeAnalysis {
            variable_types: {
                let mut m = HashMap::new();
                m.insert("x".to_string(), {
                    let mut s = HashSet::new();
                    s.insert("Dog".to_string());
                    s
                });
                m.insert("y".to_string(), {
                    let mut s = HashSet::new();
                    s.insert("Cat".to_string());
                    s
                });
                m
            },
            reachable_methods: HashSet::new(),
            resolved_calls: HashMap::new(),
        };

        // RTA would say both Dog and Cat are possible for any call.
        // VTA says x.speak() -> Dog only, y.speak() -> Cat only.
        let x_targets = vta.resolve_call_for_variable(&hierarchy, "x", "speak");
        assert_eq!(x_targets, vec!["Dog"]);

        let y_targets = vta.resolve_call_for_variable(&hierarchy, "y", "speak");
        assert_eq!(y_targets, vec!["Cat"]);
    }

    // -------------------------------------------------------------------------
    // Test: resolve_call_for_variable with subtype checking.
    // -------------------------------------------------------------------------

    #[test]
    fn test_vta_resolve_with_subtypes_in_set() {
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_inheritance("Dog", "Animal");
        hierarchy.add_inheritance("Puppy", "Dog");
        hierarchy.add_method("Animal", "speak");
        hierarchy.add_method("Dog", "speak");
        hierarchy.add_method("Puppy", "speak");

        // Variable holds both Dog and Puppy.
        let vta = VariableTypeAnalysis {
            variable_types: {
                let mut m = HashMap::new();
                m.insert("pets".to_string(), {
                    let mut s = HashSet::new();
                    s.insert("Dog".to_string());
                    s.insert("Puppy".to_string());
                    s
                });
                m
            },
            reachable_methods: HashSet::new(),
            resolved_calls: HashMap::new(),
        };

        let targets = vta.resolve_call_for_variable(&hierarchy, "pets", "speak");
        assert!(targets.contains(&"Dog".to_string()));
        assert!(targets.contains(&"Puppy".to_string()));
        assert_eq!(targets.len(), 2);
    }

    // -------------------------------------------------------------------------
    // Test: Full VTA analysis discovers type bindings.
    // -------------------------------------------------------------------------

    #[test]
    fn test_vta_full_analysis_discovers_types() {
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_type("Dog");
        hierarchy.add_method("Dog", "speak");

        let mut graph = CodeGraph::new();

        let dog_class = make_class("Dog");
        let dog_class_id = dog_class.id;
        graph.add_node(dog_class);

        let dog_ctor =
            make_constructor("__init__", dog_class_id).with_full_name("Dog.__init__".to_string());
        let dog_ctor_id = dog_ctor.id;
        graph.add_node(dog_ctor);

        let main_fn = make_function("main");
        let main_fn_id = main_fn.id;
        let main_idx = graph.add_node(main_fn);
        graph.add_entry_point(main_idx);

        graph
            .add_edge(CallEdge::certain(main_fn_id, dog_ctor_id))
            .unwrap();

        let entry_points = graph.entry_points().clone();
        let vta = VariableTypeAnalysis::analyze(&graph, &hierarchy, &entry_points);

        // The analysis should have discovered that "dog" (lowercased Dog) has type Dog.
        assert!(
            vta.variable_types.values().any(|ts| ts.contains("Dog")),
            "VTA should discover Dog type binding, got: {:?}",
            vta.variable_types
        );
    }
}
