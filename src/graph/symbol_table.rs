//! Cross-module symbol table for resolving imports and exports.
//!
//! Provides `SymbolTable` which tracks exported and imported symbols across
//! files, resolves imports to exports by simple name matching, and identifies
//! unused exports, unused imports, and unresolved imports.

use std::collections::HashMap;

use crate::core::{CodeNode, NodeId, NodeKind};

// =============================================================================
// Symbol types
// =============================================================================

/// An exported symbol discovered from an `ExportDeclaration` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedSymbol {
    /// Short name of the exported symbol (e.g., `"MyComponent"`).
    pub name: String,
    /// Fully qualified name (e.g., `"src/components.MyComponent"`).
    pub full_name: String,
    /// File where the export is declared.
    pub file: String,
    /// Node ID of the export declaration.
    pub node_id: NodeId,
    /// Kind of the node (always `ExportDeclaration` for entries built by `build_from_nodes`).
    pub kind: NodeKind,
}

/// An imported symbol discovered from an `ImportDeclaration` node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedSymbol {
    /// Name of the imported symbol (e.g., `"MyComponent"`).
    pub name: String,
    /// Module or path the symbol is imported from, extracted from attributes.
    pub imported_from: String,
    /// Optional local alias (e.g., `import { Foo as Bar }` => alias = `"Bar"`).
    pub alias: Option<String>,
    /// File where the import is declared.
    pub file: String,
    /// Line number of the import declaration.
    pub line: usize,
}

/// A resolved import linking an `ImportedSymbol` to an optional export `NodeId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImport {
    /// The import that was resolved.
    pub import: ImportedSymbol,
    /// The `NodeId` of the matching export, or `None` if unresolved.
    pub resolved_to: Option<NodeId>,
}

// =============================================================================
// SymbolTable
// =============================================================================

/// Cross-module symbol table tracking exports, imports, and their resolution.
///
/// Built from a collection of `CodeNode`s, the symbol table provides
/// simple name-based matching of imports to exports. No path resolution
/// or module system semantics are applied -- matching is purely by symbol name.
#[derive(Debug, Clone)]
pub struct SymbolTable {
    /// Mapping from symbol name to all export locations with that name.
    pub exports: HashMap<String, Vec<ExportedSymbol>>,
    /// All import declarations across all files.
    pub imports: Vec<ImportedSymbol>,
    /// Resolved imports (each import paired with its resolution result).
    pub resolved: Vec<ResolvedImport>,
}

impl SymbolTable {
    /// Create an empty symbol table.
    pub fn new() -> Self {
        Self {
            exports: HashMap::new(),
            imports: Vec::new(),
            resolved: Vec::new(),
        }
    }

    /// Build a `SymbolTable` from a slice of `(NodeId, &CodeNode)` pairs.
    ///
    /// Extraction rules:
    /// - Nodes with `kind == NodeKind::ExportDeclaration` are added to exports,
    ///   keyed by their `name`.
    /// - Nodes with `kind == NodeKind::ImportDeclaration` are added to imports.
    ///   The `imported_from` field is extracted from the first attribute starting
    ///   with `"from:"`, and the `alias` from the first attribute starting with
    ///   `"alias:"`.
    /// - After collecting all exports and imports, each import is resolved by
    ///   looking up its `name` in the exports map.
    pub fn build_from_nodes(nodes: &[(NodeId, &CodeNode)]) -> Self {
        let mut table = Self::new();

        // First pass: collect exports and imports.
        for (node_id, node) in nodes {
            match node.kind {
                NodeKind::ExportDeclaration => {
                    let symbol = ExportedSymbol {
                        name: node.name.clone(),
                        full_name: node.full_name.clone(),
                        file: node.location.file.clone(),
                        node_id: *node_id,
                        kind: node.kind,
                    };
                    table
                        .exports
                        .entry(node.name.clone())
                        .or_default()
                        .push(symbol);
                }
                NodeKind::ImportDeclaration => {
                    let imported_from = node
                        .attributes
                        .iter()
                        .find_map(|attr| attr.strip_prefix("from:").map(String::from))
                        .unwrap_or_default();

                    let alias = node
                        .attributes
                        .iter()
                        .find_map(|attr| attr.strip_prefix("alias:").map(String::from));

                    let import = ImportedSymbol {
                        name: node.name.clone(),
                        imported_from,
                        alias,
                        file: node.location.file.clone(),
                        line: node.location.line_start,
                    };
                    table.imports.push(import);
                }
                _ => {}
            }
        }

        // Second pass: resolve imports by matching name to exports.
        for import in &table.imports {
            let resolved_to = table.resolve_import_to_node_id(&import.name);
            table.resolved.push(ResolvedImport {
                import: import.clone(),
                resolved_to,
            });
        }

        table
    }

    /// Find exports that are not referenced by any import.
    ///
    /// An export is considered "unused" if no import has the same `name`.
    pub fn find_unused_exports(&self) -> Vec<&ExportedSymbol> {
        let imported_names: std::collections::HashSet<&str> =
            self.imports.iter().map(|i| i.name.as_str()).collect();

        self.exports
            .values()
            .flatten()
            .filter(|export| !imported_names.contains(export.name.as_str()))
            .collect()
    }

    /// Find imports that do not resolve to any known export.
    ///
    /// These are likely external/third-party imports.
    pub fn find_unused_imports(&self) -> Vec<&ImportedSymbol> {
        self.imports
            .iter()
            .filter(|import| !self.exports.contains_key(&import.name))
            .collect()
    }

    /// Find imports that could not be resolved to any export.
    ///
    /// Returns all imports whose resolution result is `None`.
    pub fn find_unresolved_imports(&self) -> Vec<&ImportedSymbol> {
        self.resolved
            .iter()
            .filter(|r| r.resolved_to.is_none())
            .map(|r| &r.import)
            .collect()
    }

    /// Resolve an import name to the first matching export.
    pub fn resolve_import(&self, import_name: &str) -> Option<&ExportedSymbol> {
        self.exports
            .get(import_name)
            .and_then(|exports| exports.first())
    }

    /// Internal: resolve an import name to a `NodeId`.
    fn resolve_import_to_node_id(&self, import_name: &str) -> Option<NodeId> {
        self.exports
            .get(import_name)
            .and_then(|exports| exports.first())
            .map(|e| e.node_id)
    }
}

impl Default for SymbolTable {
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
    use crate::core::{Language, SourceLocation, Visibility};

    fn make_loc(file: &str, line: usize) -> SourceLocation {
        SourceLocation::new(file.to_string(), line, line, 0, 0)
    }

    fn make_export(name: &str, file: &str) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::ExportDeclaration,
            make_loc(file, 1),
            Language::JavaScript,
            Visibility::Public,
        )
    }

    fn make_import(name: &str, file: &str, from: &str, line: usize) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::ImportDeclaration,
            make_loc(file, line),
            Language::JavaScript,
            Visibility::Unknown,
        )
        .with_attributes(vec![format!("from:{from}")])
    }

    fn make_import_with_alias(
        name: &str,
        file: &str,
        from: &str,
        alias: &str,
        line: usize,
    ) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::ImportDeclaration,
            make_loc(file, line),
            Language::JavaScript,
            Visibility::Unknown,
        )
        .with_attributes(vec![format!("from:{from}"), format!("alias:{alias}")])
    }

    fn make_function(name: &str, file: &str) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            NodeKind::Function,
            make_loc(file, 5),
            Language::JavaScript,
            Visibility::Public,
        )
    }

    fn to_node_pairs(nodes: &[CodeNode]) -> Vec<(NodeId, &CodeNode)> {
        nodes.iter().map(|n| (n.id, n)).collect()
    }

    // -------------------------------------------------------------------------
    // build_from_nodes basics
    // -------------------------------------------------------------------------

    #[test]
    fn test_build_from_nodes_collects_exports() {
        let nodes = vec![make_export("Foo", "a.js"), make_export("Bar", "b.js")];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        assert_eq!(table.exports.len(), 2);
        assert!(table.exports.contains_key("Foo"));
        assert!(table.exports.contains_key("Bar"));
        assert_eq!(table.exports["Foo"].len(), 1);
        assert_eq!(table.exports["Foo"][0].file, "a.js");
    }

    #[test]
    fn test_build_from_nodes_collects_imports() {
        let nodes = vec![
            make_import("Foo", "main.js", "./a", 1),
            make_import("Bar", "main.js", "./b", 2),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        assert_eq!(table.imports.len(), 2);
        assert_eq!(table.imports[0].name, "Foo");
        assert_eq!(table.imports[0].imported_from, "./a");
        assert_eq!(table.imports[0].line, 1);
        assert_eq!(table.imports[1].name, "Bar");
        assert_eq!(table.imports[1].imported_from, "./b");
        assert_eq!(table.imports[1].line, 2);
    }

    #[test]
    fn test_build_from_nodes_ignores_other_kinds() {
        let nodes = vec![
            make_function("helper", "utils.js"),
            make_export("Foo", "a.js"),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        assert_eq!(table.exports.len(), 1);
        assert!(table.imports.is_empty());
    }

    #[test]
    fn test_build_from_nodes_with_exports_and_imports() {
        let nodes = vec![
            make_export("Foo", "a.js"),
            make_export("Bar", "b.js"),
            make_import("Foo", "main.js", "./a", 1),
            make_import("Baz", "main.js", "external", 3),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        assert_eq!(table.exports.len(), 2);
        assert_eq!(table.imports.len(), 2);
        assert_eq!(table.resolved.len(), 2);

        // "Foo" should resolve
        let foo_resolved = table
            .resolved
            .iter()
            .find(|r| r.import.name == "Foo")
            .unwrap();
        assert!(foo_resolved.resolved_to.is_some());
        assert_eq!(foo_resolved.resolved_to.unwrap(), nodes[0].id);

        // "Baz" should not resolve (external, no matching export)
        let baz_resolved = table
            .resolved
            .iter()
            .find(|r| r.import.name == "Baz")
            .unwrap();
        assert!(baz_resolved.resolved_to.is_none());
    }

    #[test]
    fn test_build_from_nodes_empty() {
        let pairs: Vec<(NodeId, &CodeNode)> = vec![];
        let table = SymbolTable::build_from_nodes(&pairs);

        assert!(table.exports.is_empty());
        assert!(table.imports.is_empty());
        assert!(table.resolved.is_empty());
    }

    // -------------------------------------------------------------------------
    // Duplicate export names
    // -------------------------------------------------------------------------

    #[test]
    fn test_duplicate_export_names_across_files() {
        let nodes = vec![make_export("Foo", "a.js"), make_export("Foo", "b.js")];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        assert_eq!(table.exports.len(), 1);
        assert_eq!(table.exports["Foo"].len(), 2);
        assert_eq!(table.exports["Foo"][0].file, "a.js");
        assert_eq!(table.exports["Foo"][1].file, "b.js");
    }

    // -------------------------------------------------------------------------
    // find_unused_exports
    // -------------------------------------------------------------------------

    #[test]
    fn test_find_unused_exports_returns_unreferenced() {
        let nodes = vec![
            make_export("Foo", "a.js"),
            make_export("Bar", "b.js"),
            make_export("Baz", "c.js"),
            make_import("Foo", "main.js", "./a", 1),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let unused = table.find_unused_exports();
        let unused_names: Vec<&str> = unused.iter().map(|e| e.name.as_str()).collect();

        assert_eq!(unused.len(), 2);
        assert!(unused_names.contains(&"Bar"));
        assert!(unused_names.contains(&"Baz"));
        assert!(!unused_names.contains(&"Foo"));
    }

    #[test]
    fn test_find_unused_exports_all_used() {
        let nodes = vec![
            make_export("Foo", "a.js"),
            make_import("Foo", "main.js", "./a", 1),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let unused = table.find_unused_exports();
        assert!(unused.is_empty());
    }

    #[test]
    fn test_find_unused_exports_no_imports() {
        let nodes = vec![make_export("Foo", "a.js"), make_export("Bar", "b.js")];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let unused = table.find_unused_exports();
        assert_eq!(unused.len(), 2);
    }

    // -------------------------------------------------------------------------
    // find_unused_imports
    // -------------------------------------------------------------------------

    #[test]
    fn test_find_unused_imports_returns_unmatched() {
        let nodes = vec![
            make_export("Foo", "a.js"),
            make_import("Foo", "main.js", "./a", 1),
            make_import("React", "main.js", "react", 2),
            make_import("lodash", "main.js", "lodash", 3),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let unused = table.find_unused_imports();
        let unused_names: Vec<&str> = unused.iter().map(|i| i.name.as_str()).collect();

        assert_eq!(unused.len(), 2);
        assert!(unused_names.contains(&"React"));
        assert!(unused_names.contains(&"lodash"));
    }

    #[test]
    fn test_find_unused_imports_all_resolved() {
        let nodes = vec![
            make_export("Foo", "a.js"),
            make_import("Foo", "main.js", "./a", 1),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let unused = table.find_unused_imports();
        assert!(unused.is_empty());
    }

    #[test]
    fn test_find_unused_imports_no_exports() {
        let nodes = vec![
            make_import("Foo", "main.js", "./a", 1),
            make_import("Bar", "main.js", "./b", 2),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let unused = table.find_unused_imports();
        assert_eq!(unused.len(), 2);
    }

    // -------------------------------------------------------------------------
    // find_unresolved_imports
    // -------------------------------------------------------------------------

    #[test]
    fn test_find_unresolved_imports() {
        let nodes = vec![
            make_export("Foo", "a.js"),
            make_import("Foo", "main.js", "./a", 1),
            make_import("External", "main.js", "some-lib", 2),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let unresolved = table.find_unresolved_imports();
        assert_eq!(unresolved.len(), 1);
        assert_eq!(unresolved[0].name, "External");
    }

    #[test]
    fn test_find_unresolved_imports_all_resolved() {
        let nodes = vec![
            make_export("Foo", "a.js"),
            make_import("Foo", "main.js", "./a", 1),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let unresolved = table.find_unresolved_imports();
        assert!(unresolved.is_empty());
    }

    #[test]
    fn test_find_unresolved_imports_none_resolved() {
        let nodes = vec![
            make_import("A", "main.js", "./a", 1),
            make_import("B", "main.js", "./b", 2),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let unresolved = table.find_unresolved_imports();
        assert_eq!(unresolved.len(), 2);
    }

    // -------------------------------------------------------------------------
    // resolve_import
    // -------------------------------------------------------------------------

    #[test]
    fn test_resolve_import_found() {
        let nodes = vec![
            make_export("Foo", "a.js"),
            make_import("Foo", "main.js", "./a", 1),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let resolved = table.resolve_import("Foo");
        assert!(resolved.is_some());
        let sym = resolved.unwrap();
        assert_eq!(sym.name, "Foo");
        assert_eq!(sym.file, "a.js");
        assert_eq!(sym.node_id, nodes[0].id);
    }

    #[test]
    fn test_resolve_import_not_found() {
        let nodes = vec![make_export("Foo", "a.js")];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let resolved = table.resolve_import("NonExistent");
        assert!(resolved.is_none());
    }

    #[test]
    fn test_resolve_import_returns_first_when_multiple() {
        let nodes = vec![make_export("Foo", "a.js"), make_export("Foo", "b.js")];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let resolved = table.resolve_import("Foo");
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().file, "a.js");
    }

    // -------------------------------------------------------------------------
    // Import alias handling
    // -------------------------------------------------------------------------

    #[test]
    fn test_import_with_alias() {
        let nodes = vec![
            make_export("Foo", "a.js"),
            make_import_with_alias("Foo", "main.js", "./a", "MyFoo", 1),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        assert_eq!(table.imports.len(), 1);
        assert_eq!(table.imports[0].alias, Some("MyFoo".to_string()));
        assert_eq!(table.imports[0].name, "Foo");
    }

    #[test]
    fn test_import_without_alias() {
        let nodes = vec![make_import("Foo", "main.js", "./a", 1)];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        assert_eq!(table.imports[0].alias, None);
    }

    // -------------------------------------------------------------------------
    // Default trait
    // -------------------------------------------------------------------------

    #[test]
    fn test_default_trait() {
        let table = SymbolTable::default();
        assert!(table.exports.is_empty());
        assert!(table.imports.is_empty());
        assert!(table.resolved.is_empty());
    }

    // -------------------------------------------------------------------------
    // Edge cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_import_without_from_attribute() {
        let node = CodeNode::new(
            "Mystery".to_string(),
            NodeKind::ImportDeclaration,
            make_loc("x.js", 10),
            Language::JavaScript,
            Visibility::Unknown,
        );
        // No "from:" attribute at all
        let nodes = vec![node];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        assert_eq!(table.imports.len(), 1);
        assert_eq!(table.imports[0].imported_from, "");
        assert_eq!(table.imports[0].alias, None);
    }

    #[test]
    fn test_self_import_resolves() {
        // A file exports and imports the same name (unusual but valid for re-exports).
        let nodes = vec![
            make_export("Foo", "a.js"),
            make_import("Foo", "a.js", "./a", 5),
        ];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        let unused_exports = table.find_unused_exports();
        assert!(unused_exports.is_empty(), "Foo is referenced by the import");

        let unused_imports = table.find_unused_imports();
        assert!(unused_imports.is_empty(), "Foo matches an export");

        let unresolved = table.find_unresolved_imports();
        assert!(unresolved.is_empty(), "Foo resolves to its own export");
    }

    #[test]
    fn test_export_full_name_preserved() {
        let mut node = make_export("Foo", "a.js");
        node.full_name = "src/components.Foo".to_string();
        let nodes = vec![node];
        let pairs = to_node_pairs(&nodes);
        let table = SymbolTable::build_from_nodes(&pairs);

        assert_eq!(table.exports["Foo"][0].full_name, "src/components.Foo");
    }
}
