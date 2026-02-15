//! Rapid Type Analysis (RTA) for precise virtual call resolution.
//!
//! RTA refines Class Hierarchy Analysis (CHA) by tracking which types are
//! actually instantiated in the program. Only instantiated types can be the
//! runtime receiver of virtual calls, so RTA eliminates impossible call targets
//! that CHA would include.
//!
//! # Algorithm overview
//!
//! 1. Start with entry points as reachable.
//! 2. For each reachable method:
//!    a. Find constructor calls (`new T()`) -> add `T` to `instantiated_types`.
//!    b. Find virtual calls -> resolve using `instantiated_types` intersected
//!    with the CHA subtypes.
//!    c. Add resolved targets to the reachable worklist.
//! 3. Iterate until no new methods become reachable (fixed-point).

use std::collections::{HashMap, HashSet, VecDeque};

use super::class_hierarchy::ClassHierarchy;
use super::code_graph::CodeGraph;

use crate::core::NodeKind;
use petgraph::graph::NodeIndex;

/// Result of Rapid Type Analysis.
#[derive(Debug, Clone)]
pub struct RapidTypeAnalysis {
    /// Types instantiated somewhere in the reachable program.
    pub instantiated_types: HashSet<String>,
    /// Methods/functions that are reachable.
    pub reachable_methods: HashSet<NodeIndex>,
    /// Resolved virtual call targets (call_site -> set of possible target type names).
    pub resolved_calls: HashMap<NodeIndex, HashSet<String>>,
}

impl RapidTypeAnalysis {
    /// Run RTA starting from the given entry points.
    ///
    /// The algorithm iterates to a fixed point: it discovers new instantiated
    /// types and new reachable methods until neither set changes.
    pub fn analyze(
        graph: &CodeGraph,
        hierarchy: &ClassHierarchy,
        entry_points: &HashSet<NodeIndex>,
    ) -> Self {
        let mut instantiated_types: HashSet<String> = HashSet::new();
        let mut reachable_methods: HashSet<NodeIndex> = HashSet::new();
        let mut resolved_calls: HashMap<NodeIndex, HashSet<String>> = HashMap::new();

        // Build a lookup: class name -> set of NodeIndex for that class's methods.
        let class_name_set = &hierarchy.types;
        let class_method_indices = Self::build_class_method_index(graph, hierarchy);

        // Worklist of methods to process.
        let mut worklist: VecDeque<NodeIndex> = VecDeque::new();

        // Seed with entry points.
        for &ep in entry_points {
            if reachable_methods.insert(ep) {
                worklist.push_back(ep);
            }
        }

        while let Some(current) = worklist.pop_front() {
            // 1. Scan current method for type instantiations (constructors).
            let new_types = Self::find_instantiated_types_from(graph, current, class_name_set);
            for t in &new_types {
                instantiated_types.insert(t.clone());
            }

            // 2. Scan for virtual/method calls from this node.
            let virtual_calls = Self::find_virtual_calls(graph, current, hierarchy);
            for (call_site, receiver_type, method_name) in &virtual_calls {
                // Resolve using RTA: CHA targets filtered by instantiated types.
                let cha_targets = hierarchy.resolve_virtual_call(receiver_type, method_name);
                let rta_targets: HashSet<String> = cha_targets
                    .into_iter()
                    .filter(|t| instantiated_types.contains(t))
                    .collect();

                resolved_calls.insert(*call_site, rta_targets.clone());

                // Add resolved target methods to worklist.
                for target_type in &rta_targets {
                    if let Some(indices) = class_method_indices.get(target_type) {
                        for &idx in indices {
                            if let Some(node) = graph.get_node(idx) {
                                if node.name == *method_name && reachable_methods.insert(idx) {
                                    worklist.push_back(idx);
                                }
                            }
                        }
                    }
                }
            }

            // 3. Follow all direct call edges (non-virtual calls, static methods, etc.).
            let callees: Vec<NodeIndex> = graph.calls_from(current).collect();
            for callee in callees {
                if let Some(callee_node) = graph.get_node(callee) {
                    // If the callee is a constructor, mark its type as instantiated.
                    if callee_node.kind == NodeKind::Constructor {
                        let type_name = Self::extract_type_from_constructor(callee_node, graph);
                        if let Some(t) = type_name {
                            instantiated_types.insert(t);
                        }
                    }

                    // If the callee is a static method, it is always reachable regardless
                    // of type instantiation.
                    if reachable_methods.insert(callee) {
                        worklist.push_back(callee);
                    }
                }
            }
        }

        // Second pass: now that we have all instantiated types, re-resolve any
        // virtual calls that may have gained new targets due to types discovered
        // later in the iteration.
        // OPTIMIZATION: Track only newly-added methods instead of re-scanning all reachable_methods.
        // This reduces iteration from O(reachable_methods) × (virtual calls) to O(new_methods) × (virtual calls).
        let mut methods_to_reprocess: Vec<NodeIndex> = worklist.drain(..).collect();
        let mut iteration = 0;
        const MAX_ITERATIONS: usize = 100; // Safety limit to prevent infinite loops

        while !methods_to_reprocess.is_empty() && iteration < MAX_ITERATIONS {
            iteration += 1;
            let mut new_methods = Vec::new();

            for method_idx in methods_to_reprocess.drain(..) {
                let virtual_calls = Self::find_virtual_calls(graph, method_idx, hierarchy);
                for (call_site, receiver_type, method_name) in &virtual_calls {
                    let cha_targets = hierarchy.resolve_virtual_call(receiver_type, method_name);
                    let rta_targets: HashSet<String> = cha_targets
                        .into_iter()
                        .filter(|t| instantiated_types.contains(t))
                        .collect();

                    let entry = resolved_calls.entry(*call_site).or_default();
                    for target_type in &rta_targets {
                        if entry.insert(target_type.clone()) {
                            // Add newly resolved target methods for next iteration.
                            if let Some(indices) = class_method_indices.get(target_type) {
                                for &idx in indices {
                                    if let Some(node) = graph.get_node(idx) {
                                        if node.name == *method_name
                                            && reachable_methods.insert(idx)
                                        {
                                            new_methods.push(idx);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Drain any newly added worklist items.
                while let Some(new_method) = worklist.pop_front() {
                    let callees: Vec<NodeIndex> = graph.calls_from(new_method).collect();
                    for callee in callees {
                        if let Some(callee_node) = graph.get_node(callee) {
                            if callee_node.kind == NodeKind::Constructor {
                                let type_name =
                                    Self::extract_type_from_constructor(callee_node, graph);
                                if let Some(t) = type_name {
                                    if instantiated_types.insert(t) {
                                        // New type discovered, add current method for reprocessing
                                        new_methods.push(method_idx);
                                    }
                                }
                            }
                            if reachable_methods.insert(callee) {
                                worklist.push_back(callee);
                            }
                        }
                    }

                    let new_types =
                        Self::find_instantiated_types_from(graph, new_method, class_name_set);
                    for t in new_types {
                        if instantiated_types.insert(t) {
                            // New type discovered, add current method for reprocessing
                            new_methods.push(method_idx);
                        }
                    }
                }
            }

            // Prepare next iteration with newly discovered methods
            methods_to_reprocess = new_methods;
        }

        Self {
            instantiated_types,
            reachable_methods,
            resolved_calls,
        }
    }

    /// Resolve a virtual call to only instantiated types.
    ///
    /// Returns the CHA targets filtered to those types that have been
    /// instantiated somewhere in the reachable program.
    pub fn resolve_call(
        &self,
        hierarchy: &ClassHierarchy,
        receiver_type: &str,
        method: &str,
    ) -> Vec<String> {
        let cha_targets = hierarchy.resolve_virtual_call(receiver_type, method);
        let mut result: Vec<String> = cha_targets
            .into_iter()
            .filter(|t| self.instantiated_types.contains(t))
            .collect();
        result.sort();
        result
    }

    // =========================================================================
    // Helper: build an index from class name -> NodeIndices of its methods.
    // =========================================================================

    fn build_class_method_index(
        graph: &CodeGraph,
        hierarchy: &ClassHierarchy,
    ) -> HashMap<String, Vec<NodeIndex>> {
        let mut index: HashMap<String, Vec<NodeIndex>> = HashMap::new();

        for (idx, node) in graph.nodes() {
            match node.kind {
                NodeKind::Method
                | NodeKind::AsyncMethod
                | NodeKind::Constructor
                | NodeKind::StaticMethod => {
                    // Try to associate via full_name prefix (e.g. "Dog.speak" or "Dog::speak").
                    let sep_pos = node
                        .full_name
                        .rfind('.')
                        .or_else(|| node.full_name.rfind("::"));
                    if let Some(pos) = sep_pos {
                        let type_part = &node.full_name[..pos];
                        if hierarchy.types.contains(type_part) {
                            index.entry(type_part.to_string()).or_default().push(idx);
                            continue;
                        }
                    }
                    // Fallback: match by parent_id via graph scan.
                    if let Some(parent_id) = node.parent_id {
                        if let Some(parent_idx) = graph.get_index(parent_id) {
                            if let Some(parent_node) = graph.get_node(parent_idx) {
                                if hierarchy.types.contains(&parent_node.name) {
                                    index.entry(parent_node.name.clone()).or_default().push(idx);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        index
    }

    // =========================================================================
    // Helper: find types instantiated from a single reachable method.
    // =========================================================================

    /// Scan a reachable method node for constructor patterns and type
    /// instantiation evidence.
    fn find_instantiated_types_from(
        graph: &CodeGraph,
        method_idx: NodeIndex,
        class_names: &HashSet<String>,
    ) -> HashSet<String> {
        let mut types = HashSet::new();

        // Check if the method node itself is a constructor.
        if let Some(node) = graph.get_node(method_idx) {
            if node.kind == NodeKind::Constructor {
                // The constructor's parent class is the instantiated type.
                if let Some(parent_id) = node.parent_id {
                    if let Some(parent_idx) = graph.get_index(parent_id) {
                        if let Some(parent_node) = graph.get_node(parent_idx) {
                            types.insert(parent_node.name.clone());
                        }
                    }
                }
                // Also try full_name pattern (e.g. "Dog.__init__" or "Dog::new").
                let sep_pos = node
                    .full_name
                    .rfind('.')
                    .or_else(|| node.full_name.rfind("::"));
                if let Some(pos) = sep_pos {
                    let type_part = &node.full_name[..pos];
                    if class_names.contains(type_part) {
                        types.insert(type_part.to_string());
                    }
                }
            }
        }

        // Check outgoing call targets for constructors.
        let callees: Vec<NodeIndex> = graph.calls_from(method_idx).collect();
        for callee in callees {
            if let Some(callee_node) = graph.get_node(callee) {
                if callee_node.kind == NodeKind::Constructor {
                    if let Some(t) = Self::extract_type_from_constructor(callee_node, graph) {
                        types.insert(t);
                    }
                }
                // Also check if the callee name matches a class name (e.g. Dog()).
                if class_names.contains(&callee_node.name) {
                    types.insert(callee_node.name.clone());
                }
            }
        }

        types
    }

    // =========================================================================
    // Helper: extract type name from a constructor node.
    // =========================================================================

    fn extract_type_from_constructor(
        constructor: &crate::core::CodeNode,
        graph: &CodeGraph,
    ) -> Option<String> {
        // Try parent_id first.
        if let Some(parent_id) = constructor.parent_id {
            if let Some(parent_idx) = graph.get_index(parent_id) {
                if let Some(parent_node) = graph.get_node(parent_idx) {
                    return Some(parent_node.name.clone());
                }
            }
        }
        // Try full_name pattern (e.g. "Dog.__init__", "Dog.constructor", "Dog::new").
        let sep_pos = constructor
            .full_name
            .rfind('.')
            .or_else(|| constructor.full_name.rfind("::"));
        if let Some(pos) = sep_pos {
            let type_part = &constructor.full_name[..pos];
            if !type_part.is_empty() {
                return Some(type_part.to_string());
            }
        }
        None
    }

    // =========================================================================
    // Helper: find virtual call sites from a given method node.
    // =========================================================================

    /// Returns `(call_site_index, receiver_type, method_name)` tuples for
    /// method/virtual calls reachable from the given node.
    fn find_virtual_calls(
        graph: &CodeGraph,
        method_idx: NodeIndex,
        hierarchy: &ClassHierarchy,
    ) -> Vec<(NodeIndex, String, String)> {
        let mut calls = Vec::new();

        let callees: Vec<NodeIndex> = graph.calls_from(method_idx).collect();
        for callee in callees {
            if let Some(callee_node) = graph.get_node(callee) {
                match callee_node.kind {
                    NodeKind::Method | NodeKind::AsyncMethod | NodeKind::Function => {
                        // Extract receiver type from full_name (e.g. "Animal.speak" or "Animal::speak").
                        let sep_pos = callee_node
                            .full_name
                            .rfind('.')
                            .or_else(|| callee_node.full_name.rfind("::"));
                        if let Some(pos) = sep_pos {
                            let receiver_type = &callee_node.full_name[..pos];
                            if hierarchy.types.contains(receiver_type) {
                                calls.push((
                                    callee,
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

    fn make_static_method(name: &str, parent_id: crate::core::NodeId) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::StaticMethod,
            make_loc(),
            Language::Python,
            Visibility::Public,
        )
        .with_parent_id(parent_id)
    }

    /// Build a hierarchy + graph for the Animal/Dog/Cat example.
    ///
    /// Hierarchy: Dog extends Animal, Cat extends Animal.
    /// All three declare `speak`.
    fn build_animal_scenario() -> (CodeGraph, ClassHierarchy) {
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_inheritance("Dog", "Animal");
        hierarchy.add_inheritance("Cat", "Animal");
        hierarchy.add_method("Animal", "speak");
        hierarchy.add_method("Dog", "speak");
        hierarchy.add_method("Cat", "speak");

        let mut graph = CodeGraph::new();

        // Class nodes.
        let animal_class = make_class("Animal");
        let animal_class_id = animal_class.id;
        let dog_class = make_class_with_attrs("Dog", vec!["extends:Animal".to_string()]);
        let dog_class_id = dog_class.id;
        let cat_class = make_class_with_attrs("Cat", vec!["extends:Animal".to_string()]);
        let cat_class_id = cat_class.id;

        graph.add_node(animal_class);
        graph.add_node(dog_class);
        graph.add_node(cat_class);

        // Method nodes.
        let animal_speak =
            make_method("speak", animal_class_id).with_full_name("Animal.speak".to_string());
        let dog_speak = make_method("speak", dog_class_id).with_full_name("Dog.speak".to_string());
        let cat_speak = make_method("speak", cat_class_id).with_full_name("Cat.speak".to_string());

        let animal_speak_id = animal_speak.id;
        let _dog_speak_id = dog_speak.id;
        let _cat_speak_id = cat_speak.id;

        let _animal_speak_idx = graph.add_node(animal_speak);
        let _dog_speak_idx = graph.add_node(dog_speak);
        let _cat_speak_idx = graph.add_node(cat_speak);

        // Constructor nodes.
        let dog_ctor =
            make_constructor("__init__", dog_class_id).with_full_name("Dog.__init__".to_string());
        let dog_ctor_id = dog_ctor.id;
        let _dog_ctor_idx = graph.add_node(dog_ctor);

        let cat_ctor =
            make_constructor("__init__", cat_class_id).with_full_name("Cat.__init__".to_string());
        let cat_ctor_id = cat_ctor.id;
        let _cat_ctor_idx = graph.add_node(cat_ctor);

        // Entry point: a "main" function that calls Dog() and Animal.speak().
        let main_fn = make_function("main");
        let main_fn_id = main_fn.id;
        let main_idx = graph.add_node(main_fn);
        graph.add_entry_point(main_idx);

        // main -> Dog.__init__ (instantiates Dog).
        graph
            .add_edge(CallEdge::certain(main_fn_id, dog_ctor_id))
            .unwrap();
        // main -> Animal.speak (virtual call).
        graph
            .add_edge(CallEdge::certain(main_fn_id, animal_speak_id))
            .unwrap();

        // For second test variant: a helper that also creates Cat.
        let helper_fn = make_function("create_cat");
        let helper_fn_id = helper_fn.id;
        let _helper_idx = graph.add_node(helper_fn);

        // create_cat -> Cat.__init__.
        graph
            .add_edge(CallEdge::certain(helper_fn_id, cat_ctor_id))
            .unwrap();

        (graph, hierarchy)
    }

    // -------------------------------------------------------------------------
    // Test: Only Dog.speak() reachable when only Dog is instantiated.
    // -------------------------------------------------------------------------

    #[test]
    fn test_rta_only_dog_instantiated() {
        let (graph, hierarchy) = build_animal_scenario();

        let entry_points = graph.entry_points().clone();
        let rta = RapidTypeAnalysis::analyze(&graph, &hierarchy, &entry_points);

        // Dog should be instantiated, Cat should not.
        assert!(
            rta.instantiated_types.contains("Dog"),
            "Dog should be instantiated"
        );
        assert!(
            !rta.instantiated_types.contains("Cat"),
            "Cat should NOT be instantiated (create_cat is unreachable)"
        );

        // Resolve Animal.speak() with RTA: should only include Dog.
        let targets = rta.resolve_call(&hierarchy, "Animal", "speak");
        assert!(
            targets.contains(&"Dog".to_string()),
            "Dog.speak() should be a target"
        );
        assert!(
            !targets.contains(&"Cat".to_string()),
            "Cat.speak() should NOT be a target (Cat is not instantiated)"
        );
    }

    // -------------------------------------------------------------------------
    // Test: Both Dog and Cat reachable when both are instantiated.
    // -------------------------------------------------------------------------

    #[test]
    fn test_rta_both_instantiated() {
        let (mut graph, hierarchy) = build_animal_scenario();

        // Make create_cat reachable by adding main -> create_cat edge.
        let main_idx = graph.find_node_by_name("main").unwrap();
        let create_cat_idx = graph.find_node_by_name("create_cat").unwrap();
        let main_id = graph.get_node(main_idx).unwrap().id;
        let create_cat_id = graph.get_node(create_cat_idx).unwrap().id;
        graph
            .add_edge(CallEdge::certain(main_id, create_cat_id))
            .unwrap();

        let entry_points = graph.entry_points().clone();
        let rta = RapidTypeAnalysis::analyze(&graph, &hierarchy, &entry_points);

        assert!(rta.instantiated_types.contains("Dog"));
        assert!(rta.instantiated_types.contains("Cat"));

        let targets = rta.resolve_call(&hierarchy, "Animal", "speak");
        assert!(targets.contains(&"Dog".to_string()));
        assert!(targets.contains(&"Cat".to_string()));
    }

    // -------------------------------------------------------------------------
    // Test: Static methods are always reachable regardless of instantiation.
    // -------------------------------------------------------------------------

    #[test]
    fn test_rta_static_methods_always_reachable() {
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_type("Utils");
        hierarchy.add_method("Utils", "helper");

        let mut graph = CodeGraph::new();

        let utils_class = make_class("Utils");
        let utils_class_id = utils_class.id;
        graph.add_node(utils_class);

        let static_method =
            make_static_method("helper", utils_class_id).with_full_name("Utils.helper".to_string());
        let static_method_id = static_method.id;
        let static_method_idx = graph.add_node(static_method);

        let main_fn = make_function("main");
        let main_fn_id = main_fn.id;
        let main_idx = graph.add_node(main_fn);
        graph.add_entry_point(main_idx);

        // main -> Utils.helper (static call, no instantiation).
        graph
            .add_edge(CallEdge::certain(main_fn_id, static_method_id))
            .unwrap();

        let entry_points = graph.entry_points().clone();
        let rta = RapidTypeAnalysis::analyze(&graph, &hierarchy, &entry_points);

        // Utils is not instantiated, but the static method is reachable.
        assert!(
            rta.reachable_methods.contains(&static_method_idx),
            "Static methods should be reachable even without type instantiation"
        );
    }

    // -------------------------------------------------------------------------
    // Test: Entry point transitivity works.
    // -------------------------------------------------------------------------

    #[test]
    fn test_rta_entry_point_transitivity() {
        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_type("Foo");
        hierarchy.add_method("Foo", "run");

        let mut graph = CodeGraph::new();

        let foo_class = make_class("Foo");
        let foo_class_id = foo_class.id;
        graph.add_node(foo_class);

        let foo_ctor =
            make_constructor("__init__", foo_class_id).with_full_name("Foo.__init__".to_string());
        let foo_ctor_id = foo_ctor.id;
        graph.add_node(foo_ctor);

        let foo_run = make_method("run", foo_class_id).with_full_name("Foo.run".to_string());
        let foo_run_id = foo_run.id;
        let foo_run_idx = graph.add_node(foo_run);

        // Chain: main -> setup -> Foo() -> Foo.run().
        let setup_fn = make_function("setup");
        let setup_fn_id = setup_fn.id;
        let setup_idx = graph.add_node(setup_fn);

        let main_fn = make_function("main");
        let main_fn_id = main_fn.id;
        let main_idx = graph.add_node(main_fn);
        graph.add_entry_point(main_idx);

        graph
            .add_edge(CallEdge::certain(main_fn_id, setup_fn_id))
            .unwrap();
        graph
            .add_edge(CallEdge::certain(setup_fn_id, foo_ctor_id))
            .unwrap();
        graph
            .add_edge(CallEdge::certain(setup_fn_id, foo_run_id))
            .unwrap();

        let entry_points = graph.entry_points().clone();
        let rta = RapidTypeAnalysis::analyze(&graph, &hierarchy, &entry_points);

        assert!(rta.reachable_methods.contains(&main_idx));
        assert!(rta.reachable_methods.contains(&setup_idx));
        assert!(
            rta.reachable_methods.contains(&foo_run_idx),
            "Foo.run should be transitively reachable through setup"
        );
        assert!(
            rta.instantiated_types.contains("Foo"),
            "Foo should be instantiated via setup -> Foo()"
        );
    }

    // -------------------------------------------------------------------------
    // Test: resolve_call produces sorted, deduplicated output.
    // -------------------------------------------------------------------------

    #[test]
    fn test_rta_resolve_call_sorted() {
        let rta = RapidTypeAnalysis {
            instantiated_types: ["Ant", "Zebra", "Animal"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            reachable_methods: HashSet::new(),
            resolved_calls: HashMap::new(),
        };

        let mut hierarchy = ClassHierarchy::new();
        hierarchy.add_inheritance("Zebra", "Animal");
        hierarchy.add_inheritance("Ant", "Animal");
        hierarchy.add_method("Animal", "move");
        hierarchy.add_method("Zebra", "move");
        hierarchy.add_method("Ant", "move");

        let targets = rta.resolve_call(&hierarchy, "Animal", "move");
        assert_eq!(targets, vec!["Animal", "Ant", "Zebra"]);
    }

    // -------------------------------------------------------------------------
    // Test: empty graph produces empty results.
    // -------------------------------------------------------------------------

    #[test]
    fn test_rta_empty_graph() {
        let graph = CodeGraph::new();
        let hierarchy = ClassHierarchy::new();
        let entry_points = HashSet::new();

        let rta = RapidTypeAnalysis::analyze(&graph, &hierarchy, &entry_points);

        assert!(rta.instantiated_types.is_empty());
        assert!(rta.reachable_methods.is_empty());
        assert!(rta.resolved_calls.is_empty());
    }

    // -------------------------------------------------------------------------
    // Test: RTA is more precise than CHA.
    // -------------------------------------------------------------------------

    #[test]
    fn test_rta_more_precise_than_cha() {
        let (graph, hierarchy) = build_animal_scenario();

        // CHA would resolve Animal.speak() to {Animal, Cat, Dog}.
        let cha_targets = hierarchy.resolve_virtual_call("Animal", "speak");
        assert_eq!(cha_targets.len(), 3);

        // RTA should resolve to only {Dog} since only Dog is instantiated.
        let entry_points = graph.entry_points().clone();
        let rta = RapidTypeAnalysis::analyze(&graph, &hierarchy, &entry_points);
        let rta_targets = rta.resolve_call(&hierarchy, "Animal", "speak");

        assert!(
            rta_targets.len() < cha_targets.len(),
            "RTA should be more precise than CHA: RTA={:?} vs CHA={:?}",
            rta_targets,
            cha_targets
        );
    }
}
