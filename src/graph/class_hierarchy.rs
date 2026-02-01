//! Class hierarchy analysis for resolving virtual/dynamic method calls.
//!
//! Provides `ClassHierarchy` which tracks type inheritance relationships
//! and declared methods to determine all possible targets of a virtual call.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::core::{CodeNode, NodeKind};

/// Class hierarchy tracking types, inheritance, and method declarations.
///
/// Used to resolve virtual/dynamic dispatch by determining which concrete
/// types could respond to a method call on a given receiver type.
#[derive(Debug, Clone)]
pub struct ClassHierarchy {
    /// All known types in the hierarchy.
    pub types: HashSet<String>,
    /// Map from a type to its direct subtypes (children).
    pub subtypes: HashMap<String, HashSet<String>>,
    /// Map from a type to its direct supertypes (parents).
    pub supertypes: HashMap<String, HashSet<String>>,
    /// Map from a type to its declared method names.
    pub methods: HashMap<String, HashSet<String>>,
}

impl ClassHierarchy {
    /// Create a new empty class hierarchy.
    pub fn new() -> Self {
        Self {
            types: HashSet::new(),
            subtypes: HashMap::new(),
            supertypes: HashMap::new(),
            methods: HashMap::new(),
        }
    }

    /// Register a type in the hierarchy.
    pub fn add_type(&mut self, name: &str) {
        self.types.insert(name.to_string());
    }

    /// Register an inheritance relationship: `child` extends/implements `parent`.
    ///
    /// Both types are automatically added to the hierarchy if not already present.
    pub fn add_inheritance(&mut self, child: &str, parent: &str) {
        self.add_type(child);
        self.add_type(parent);

        self.subtypes
            .entry(parent.to_string())
            .or_default()
            .insert(child.to_string());

        self.supertypes
            .entry(child.to_string())
            .or_default()
            .insert(parent.to_string());
    }

    /// Register that a type declares a method with the given name.
    ///
    /// The type is automatically added to the hierarchy if not already present.
    pub fn add_method(&mut self, type_name: &str, method_name: &str) {
        self.add_type(type_name);
        self.methods
            .entry(type_name.to_string())
            .or_default()
            .insert(method_name.to_string());
    }

    /// Compute the transitive closure of all subtypes of the given type.
    ///
    /// Returns all types that directly or transitively extend/implement
    /// `type_name`. Does **not** include `type_name` itself.
    pub fn all_subtypes(&self, type_name: &str) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(type_name.to_string());

        while let Some(current) = queue.pop_front() {
            if let Some(children) = self.subtypes.get(&current) {
                for child in children {
                    if result.insert(child.clone()) {
                        queue.push_back(child.clone());
                    }
                }
            }
        }

        result
    }

    /// Compute the transitive closure of all supertypes of the given type.
    ///
    /// Returns all types that `type_name` directly or transitively
    /// extends/implements. Does **not** include `type_name` itself.
    pub fn all_supertypes(&self, type_name: &str) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut queue = VecDeque::new();
        queue.push_back(type_name.to_string());

        while let Some(current) = queue.pop_front() {
            if let Some(parents) = self.supertypes.get(&current) {
                for parent in parents {
                    if result.insert(parent.clone()) {
                        queue.push_back(parent.clone());
                    }
                }
            }
        }

        result
    }

    /// Resolve a virtual method call on a given receiver type.
    ///
    /// Returns all type names that could respond to the call — i.e.,
    /// `receiver_type` itself (if it declares the method) plus any
    /// transitive subtype that declares (overrides) the method.
    ///
    /// This implements Class Hierarchy Analysis (CHA): every subtype
    /// that declares the method is a potential call target.
    pub fn resolve_virtual_call(&self, receiver_type: &str, method: &str) -> Vec<String> {
        let mut targets = Vec::new();

        // Check receiver type itself
        if self.type_has_method(receiver_type, method) {
            targets.push(receiver_type.to_string());
        }

        // Check all transitive subtypes
        let subs = self.all_subtypes(receiver_type);
        for sub in &subs {
            if self.type_has_method(sub, method) {
                targets.push(sub.clone());
            }
        }

        targets.sort();
        targets
    }

    /// Build a `ClassHierarchy` from a slice of `CodeNode`s.
    ///
    /// Extraction rules:
    /// - Nodes with `kind == NodeKind::Class`, `NodeKind::Struct`,
    ///   `NodeKind::Interface`, or `NodeKind::Trait` are registered as types.
    /// - Inheritance is extracted from node `attributes` entries that start
    ///   with `"extends:"` or `"implements:"` (e.g., `"extends:Animal"`).
    /// - Methods are associated with their parent type via `parent_id`:
    ///   any node with `kind == Method | AsyncMethod | Constructor | StaticMethod`
    ///   whose `parent_id` matches a known class node is registered as a method.
    pub fn build_from_nodes(nodes: &[CodeNode]) -> Self {
        let mut hierarchy = Self::new();

        // First pass: collect all type nodes and index by NodeId.
        let mut id_to_type_name: HashMap<crate::core::NodeId, String> = HashMap::new();

        for node in nodes {
            match node.kind {
                NodeKind::Class | NodeKind::Struct | NodeKind::Interface | NodeKind::Trait => {
                    hierarchy.add_type(&node.name);
                    id_to_type_name.insert(node.id, node.name.clone());

                    // Extract inheritance from attributes
                    for attr in &node.attributes {
                        if let Some(parent) = attr.strip_prefix("extends:") {
                            hierarchy.add_inheritance(&node.name, parent);
                        } else if let Some(iface) = attr.strip_prefix("implements:") {
                            hierarchy.add_inheritance(&node.name, iface);
                        }
                    }
                }
                _ => {}
            }
        }

        // Second pass: associate methods with their parent types.
        for node in nodes {
            match node.kind {
                NodeKind::Method
                | NodeKind::AsyncMethod
                | NodeKind::Constructor
                | NodeKind::StaticMethod
                | NodeKind::Function => {
                    if let Some(parent_id) = node.parent_id {
                        if let Some(type_name) = id_to_type_name.get(&parent_id) {
                            hierarchy.add_method(type_name, &node.name);
                        }
                    }
                    // Also try to extract type name from full_name (e.g. "MyClass.method" or "MyStruct::method")
                    if node.parent_id.is_none() {
                        let sep_pos = node
                            .full_name
                            .rfind('.')
                            .or_else(|| node.full_name.rfind("::"));
                        if let Some(pos) = sep_pos {
                            let type_part = &node.full_name[..pos];
                            if hierarchy.types.contains(type_part) {
                                hierarchy.add_method(type_part, &node.name);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        hierarchy
    }

    /// Check whether a type declares a given method.
    fn type_has_method(&self, type_name: &str, method: &str) -> bool {
        self.methods
            .get(type_name)
            .is_some_and(|ms| ms.contains(method))
    }
}

impl Default for ClassHierarchy {
    fn default() -> Self {
        Self::new()
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Language, NodeId, SourceLocation, Visibility};

    fn make_loc() -> SourceLocation {
        SourceLocation::new("test.rs".to_string(), 1, 10, 0, 0)
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

    fn make_method(name: &str, parent_id: NodeId) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::Method,
            make_loc(),
            Language::Python,
            Visibility::Public,
        )
        .with_parent_id(parent_id)
    }

    // -------------------------------------------------------------------------
    // Basic type registration
    // -------------------------------------------------------------------------

    #[test]
    fn test_add_type() {
        let mut h = ClassHierarchy::new();
        h.add_type("Animal");
        h.add_type("Dog");
        assert!(h.types.contains("Animal"));
        assert!(h.types.contains("Dog"));
        assert_eq!(h.types.len(), 2);
    }

    #[test]
    fn test_add_type_idempotent() {
        let mut h = ClassHierarchy::new();
        h.add_type("Animal");
        h.add_type("Animal");
        assert_eq!(h.types.len(), 1);
    }

    // -------------------------------------------------------------------------
    // Inheritance relationships
    // -------------------------------------------------------------------------

    #[test]
    fn test_add_inheritance() {
        let mut h = ClassHierarchy::new();
        h.add_inheritance("Dog", "Animal");

        assert!(h.types.contains("Dog"));
        assert!(h.types.contains("Animal"));

        assert!(h.subtypes["Animal"].contains("Dog"));
        assert!(h.supertypes["Dog"].contains("Animal"));
    }

    #[test]
    fn test_multiple_inheritance() {
        let mut h = ClassHierarchy::new();
        h.add_inheritance("C", "A");
        h.add_inheritance("C", "B");

        assert_eq!(h.supertypes["C"].len(), 2);
        assert!(h.supertypes["C"].contains("A"));
        assert!(h.supertypes["C"].contains("B"));
    }

    #[test]
    fn test_diamond_inheritance() {
        let mut h = ClassHierarchy::new();
        // Diamond: D extends B, C; B extends A; C extends A
        h.add_inheritance("B", "A");
        h.add_inheritance("C", "A");
        h.add_inheritance("D", "B");
        h.add_inheritance("D", "C");

        let d_supertypes = h.all_supertypes("D");
        assert!(d_supertypes.contains("B"));
        assert!(d_supertypes.contains("C"));
        assert!(d_supertypes.contains("A"));
        assert_eq!(d_supertypes.len(), 3);

        let a_subtypes = h.all_subtypes("A");
        assert!(a_subtypes.contains("B"));
        assert!(a_subtypes.contains("C"));
        assert!(a_subtypes.contains("D"));
        assert_eq!(a_subtypes.len(), 3);
    }

    // -------------------------------------------------------------------------
    // Transitive subtype/supertype computation
    // -------------------------------------------------------------------------

    #[test]
    fn test_all_subtypes_simple() {
        let mut h = ClassHierarchy::new();
        h.add_inheritance("Dog", "Animal");
        h.add_inheritance("Cat", "Animal");

        let subs = h.all_subtypes("Animal");
        assert_eq!(subs.len(), 2);
        assert!(subs.contains("Dog"));
        assert!(subs.contains("Cat"));
    }

    #[test]
    fn test_all_subtypes_deep_chain() {
        let mut h = ClassHierarchy::new();
        h.add_inheritance("B", "A");
        h.add_inheritance("C", "B");
        h.add_inheritance("D", "C");

        let subs = h.all_subtypes("A");
        assert_eq!(subs.len(), 3);
        assert!(subs.contains("B"));
        assert!(subs.contains("C"));
        assert!(subs.contains("D"));
    }

    #[test]
    fn test_all_subtypes_empty() {
        let mut h = ClassHierarchy::new();
        h.add_type("Leaf");
        let subs = h.all_subtypes("Leaf");
        assert!(subs.is_empty());
    }

    #[test]
    fn test_all_subtypes_unknown_type() {
        let h = ClassHierarchy::new();
        let subs = h.all_subtypes("NonExistent");
        assert!(subs.is_empty());
    }

    #[test]
    fn test_all_supertypes_simple() {
        let mut h = ClassHierarchy::new();
        h.add_inheritance("Dog", "Animal");
        h.add_inheritance("Animal", "LivingThing");

        let supers = h.all_supertypes("Dog");
        assert_eq!(supers.len(), 2);
        assert!(supers.contains("Animal"));
        assert!(supers.contains("LivingThing"));
    }

    #[test]
    fn test_all_supertypes_no_parent() {
        let mut h = ClassHierarchy::new();
        h.add_type("Root");
        let supers = h.all_supertypes("Root");
        assert!(supers.is_empty());
    }

    // -------------------------------------------------------------------------
    // Virtual call resolution
    // -------------------------------------------------------------------------

    #[test]
    fn test_resolve_virtual_call_basic() {
        let mut h = ClassHierarchy::new();
        h.add_inheritance("Dog", "Animal");
        h.add_inheritance("Cat", "Animal");
        h.add_method("Animal", "speak");
        h.add_method("Dog", "speak");
        h.add_method("Cat", "speak");

        let targets = h.resolve_virtual_call("Animal", "speak");
        assert_eq!(targets.len(), 3);
        assert!(targets.contains(&"Animal".to_string()));
        assert!(targets.contains(&"Dog".to_string()));
        assert!(targets.contains(&"Cat".to_string()));
    }

    #[test]
    fn test_resolve_virtual_call_partial_override() {
        let mut h = ClassHierarchy::new();
        h.add_inheritance("Dog", "Animal");
        h.add_inheritance("Cat", "Animal");
        h.add_method("Animal", "speak");
        h.add_method("Dog", "speak");
        // Cat does NOT override speak

        let targets = h.resolve_virtual_call("Animal", "speak");
        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&"Animal".to_string()));
        assert!(targets.contains(&"Dog".to_string()));
        assert!(!targets.contains(&"Cat".to_string()));
    }

    #[test]
    fn test_resolve_virtual_call_no_override() {
        let mut h = ClassHierarchy::new();
        h.add_inheritance("Dog", "Animal");
        h.add_method("Animal", "speak");

        let targets = h.resolve_virtual_call("Animal", "speak");
        assert_eq!(targets.len(), 1);
        assert!(targets.contains(&"Animal".to_string()));
    }

    #[test]
    fn test_resolve_virtual_call_method_not_found() {
        let mut h = ClassHierarchy::new();
        h.add_type("Animal");

        let targets = h.resolve_virtual_call("Animal", "nonexistent");
        assert!(targets.is_empty());
    }

    #[test]
    fn test_resolve_virtual_call_deep_hierarchy() {
        let mut h = ClassHierarchy::new();
        h.add_inheritance("B", "A");
        h.add_inheritance("C", "B");
        h.add_inheritance("D", "C");
        h.add_method("A", "run");
        h.add_method("C", "run");
        h.add_method("D", "run");

        let targets = h.resolve_virtual_call("A", "run");
        assert_eq!(targets.len(), 3);
        assert!(targets.contains(&"A".to_string()));
        assert!(targets.contains(&"C".to_string()));
        assert!(targets.contains(&"D".to_string()));
        // B does NOT override run, so should not be in results
        assert!(!targets.contains(&"B".to_string()));
    }

    #[test]
    fn test_resolve_virtual_call_unknown_type() {
        let h = ClassHierarchy::new();
        let targets = h.resolve_virtual_call("Unknown", "method");
        assert!(targets.is_empty());
    }

    #[test]
    fn test_resolve_virtual_call_sorted() {
        let mut h = ClassHierarchy::new();
        h.add_inheritance("Zebra", "Animal");
        h.add_inheritance("Ant", "Animal");
        h.add_method("Animal", "move");
        h.add_method("Zebra", "move");
        h.add_method("Ant", "move");

        let targets = h.resolve_virtual_call("Animal", "move");
        // Results should be sorted alphabetically
        assert_eq!(targets, vec!["Animal", "Ant", "Zebra"]);
    }

    // -------------------------------------------------------------------------
    // build_from_nodes
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_from_nodes_basic() {
        let animal = make_class("Animal");
        let animal_id = animal.id;
        let dog = make_class_with_attrs("Dog", vec!["extends:Animal".to_string()]);
        let dog_id = dog.id;
        let speak_animal = make_method("speak", animal_id);
        let speak_dog = make_method("speak", dog_id);
        let fetch = make_method("fetch", dog_id);

        let nodes = vec![animal, dog, speak_animal, speak_dog, fetch];
        let h = ClassHierarchy::build_from_nodes(&nodes);

        assert!(h.types.contains("Animal"));
        assert!(h.types.contains("Dog"));
        assert!(h.subtypes["Animal"].contains("Dog"));
        assert!(h.methods["Animal"].contains("speak"));
        assert!(h.methods["Dog"].contains("speak"));
        assert!(h.methods["Dog"].contains("fetch"));
    }

    #[test]
    fn test_build_from_nodes_implements() {
        let iface = CodeNode::new(
            "Serializable".to_string(),
            NodeKind::Interface,
            make_loc(),
            Language::Java,
            Visibility::Public,
        );
        let class = make_class_with_attrs("User", vec!["implements:Serializable".to_string()]);

        let nodes = vec![iface, class];
        let h = ClassHierarchy::build_from_nodes(&nodes);

        assert!(h.subtypes["Serializable"].contains("User"));
        assert!(h.supertypes["User"].contains("Serializable"));
    }

    #[test]
    fn test_build_from_nodes_virtual_call() {
        let animal = make_class("Animal");
        let animal_id = animal.id;
        let dog = make_class_with_attrs("Dog", vec!["extends:Animal".to_string()]);
        let dog_id = dog.id;
        let cat = make_class_with_attrs("Cat", vec!["extends:Animal".to_string()]);
        let cat_id = cat.id;

        let speak_animal = make_method("speak", animal_id);
        let speak_dog = make_method("speak", dog_id);
        let speak_cat = make_method("speak", cat_id);

        let nodes = vec![animal, dog, cat, speak_animal, speak_dog, speak_cat];
        let h = ClassHierarchy::build_from_nodes(&nodes);

        let targets = h.resolve_virtual_call("Animal", "speak");
        assert_eq!(targets.len(), 3);
        assert!(targets.contains(&"Animal".to_string()));
        assert!(targets.contains(&"Dog".to_string()));
        assert!(targets.contains(&"Cat".to_string()));
    }

    #[test]
    fn test_build_from_nodes_empty() {
        let h = ClassHierarchy::build_from_nodes(&[]);
        assert!(h.types.is_empty());
        assert!(h.subtypes.is_empty());
        assert!(h.supertypes.is_empty());
        assert!(h.methods.is_empty());
    }

    #[test]
    fn test_build_from_nodes_no_classes() {
        let func = CodeNode::new(
            "my_func".to_string(),
            NodeKind::Function,
            make_loc(),
            Language::Python,
            Visibility::Public,
        );
        let h = ClassHierarchy::build_from_nodes(&[func]);
        assert!(h.types.is_empty());
    }

    #[test]
    fn test_build_from_nodes_trait() {
        let trait_node = CodeNode::new(
            "Display".to_string(),
            NodeKind::Trait,
            make_loc(),
            Language::Rust,
            Visibility::Public,
        );
        let h = ClassHierarchy::build_from_nodes(&[trait_node]);
        assert!(h.types.contains("Display"));
    }

    #[test]
    fn test_build_from_nodes_struct() {
        let struct_node = CodeNode::new(
            "Point".to_string(),
            NodeKind::Struct,
            make_loc(),
            Language::Rust,
            Visibility::Public,
        );
        let h = ClassHierarchy::build_from_nodes(&[struct_node]);
        assert!(h.types.contains("Point"));
    }

    #[test]
    fn test_default_trait() {
        let h = ClassHierarchy::default();
        assert!(h.types.is_empty());
    }
}
