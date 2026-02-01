//! Graph builder: constructs a CodeGraph from parsed files.
//!
//! The `GraphBuilder` builds a call graph from the extracted code nodes and edges
//! in `ParsedFile`s. It resolves intra-file calls by name matching and tracks
//! unresolved cross-file calls for later aggregation.

use std::collections::{HashMap, HashSet};

use crate::core::{
    CallEdge, CodeNode, EdgeConfidence, Language, NodeId, NodeKind, ParsedFile, SourceLocation,
    Visibility,
};
use crate::parsers::{extract_calls, extract_functions, ParserRegistry, ZeroCopyParseTree};
use petgraph::graph::NodeIndex;

use super::code_graph::CodeGraph;
use super::import_resolver::ImportResolver;

/// Builds a `CodeGraph` from parsed source files.
pub struct GraphBuilder {
    registry: ParserRegistry,
}

impl GraphBuilder {
    pub fn new() -> Result<Self, crate::core::Error> {
        Ok(Self {
            registry: ParserRegistry::with_defaults()?,
        })
    }

    /// Build a graph from a single source file.
    pub fn build_file_graph(
        &self,
        source: &str,
        file_path: &str,
        language: Language,
    ) -> Result<CodeGraph, crate::core::Error> {
        let ext = std::path::Path::new(file_path)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let parser = self
            .registry
            .get_parser_for_extension(ext)
            .or_else(|| self.registry.get_parser(language))
            .ok_or_else(|| {
                crate::core::Error::parse(format!("No parser for {}", language.name()))
            })?;

        let parsed = parser.parse_file(file_path, source)?;
        self.build_from_parsed_file(&parsed)
    }

    /// Build a graph from a `ParsedFile` (already parsed by a language parser).
    pub fn build_from_parsed_file(
        &self,
        parsed: &ParsedFile,
    ) -> Result<CodeGraph, crate::core::Error> {
        let mut graph = CodeGraph::new().with_language(parsed.language);

        // Index: name -> NodeId for intra-file call resolution
        let mut name_to_id: HashMap<String, NodeId> = HashMap::new();

        // Add all nodes
        for node in &parsed.nodes {
            graph.add_node(node.clone());
            name_to_id.insert(node.name.clone(), node.id);
            if node.full_name != node.name {
                name_to_id.insert(node.full_name.clone(), node.id);
            }
        }

        // Add resolved edges
        for edge in &parsed.edges {
            let _ = graph.add_edge(edge.clone());
        }

        // Mark entry points
        for &ep_id in &parsed.entry_points {
            if let Some(idx) = graph.get_index(ep_id) {
                graph.add_entry_point(idx);
            }
        }

        // Detect additional entry points by heuristics
        // Collect first, then mutate (avoids borrow checker issue)
        let entry_indices: Vec<NodeIndex> = graph
            .nodes()
            .filter(|(_, n)| is_entry_point_heuristic(n))
            .map(|(idx, _)| idx)
            .collect();
        let test_indices: Vec<NodeIndex> = graph
            .nodes()
            .filter(|(_, n)| is_test_entry_point(n))
            .map(|(idx, _)| idx)
            .collect();

        for idx in entry_indices {
            graph.add_entry_point(idx);
        }
        for idx in test_indices {
            graph.add_test_entry_point(idx);
        }

        Ok(graph)
    }

    /// Build a merged graph from multiple parsed files.
    pub fn build_project_graph(
        &self,
        parsed_files: &[ParsedFile],
    ) -> Result<CodeGraph, crate::core::Error> {
        let mut project_graph = CodeGraph::new();

        // Build per-file graphs and merge
        for pf in parsed_files {
            let file_graph = self.build_from_parsed_file(pf)?;
            project_graph.merge(&file_graph);
        }

        // Barrel file suffixes for re-export chain resolution
        let barrel_suffixes = [
            "index.ts",
            "index.js",
            "index.tsx",
            "index.jsx",
            "__init__.py",
        ];

        // Build import resolver for file-scoped name resolution
        let resolver = ImportResolver::new(parsed_files);

        // Resolve cross-file calls
        let mut cross_file_edges = Vec::new();
        for pf in parsed_files {
            for unresolved in &pf.unresolved_calls {
                // Try the imported_as name first, then fall back to callee_name
                let callee_name = unresolved
                    .imported_as
                    .as_deref()
                    .unwrap_or(&unresolved.callee_name);

                // Try file-scoped resolution first (higher confidence)
                let mut resolved = false;
                if let Some(source_module) = &unresolved.source_module {
                    let candidates = resolver.resolve(source_module, &pf.path, pf.language);
                    if let Some(callee_idx) =
                        project_graph.find_node_by_name_in_files(callee_name, &candidates)
                    {
                        if let Some(to_id) = project_graph.get_node(callee_idx).map(|n| n.id) {
                            cross_file_edges.push(CallEdge::new(
                                unresolved.caller_id,
                                to_id,
                                EdgeConfidence::HighLikely,
                            ));
                            resolved = true;
                        }
                    }
                    // Also try original callee_name in scoped files
                    if !resolved && unresolved.imported_as.is_some() {
                        if let Some(callee_idx) = project_graph
                            .find_node_by_name_in_files(&unresolved.callee_name, &candidates)
                        {
                            if let Some(to_id) = project_graph.get_node(callee_idx).map(|n| n.id) {
                                cross_file_edges.push(CallEdge::new(
                                    unresolved.caller_id,
                                    to_id,
                                    EdgeConfidence::HighLikely,
                                ));
                                resolved = true;
                            }
                        }
                    }

                    // If file-scoped lookup failed and candidate is a barrel file,
                    // follow re-export chain: extract barrel's re-exports to find real source.
                    if !resolved {
                        let barrel_candidates =
                            resolver.resolve(source_module, &pf.path, pf.language);
                        for candidate_file in &barrel_candidates {
                            if !barrel_suffixes.iter().any(|s| candidate_file.ends_with(s)) {
                                continue;
                            }
                            if let Some(barrel_pf) = parsed_files.iter().find(|p| {
                                p.path == *candidate_file
                                    || p.path.ends_with(candidate_file)
                                    || candidate_file.ends_with(&p.path)
                            }) {
                                for reexport in extract_barrel_reexports(&barrel_pf.source) {
                                    let reexport_candidates = resolver.resolve(
                                        &reexport.source_path,
                                        &barrel_pf.path,
                                        barrel_pf.language,
                                    );

                                    if reexport.exported_name == callee_name {
                                        // Exact named re-export: export { X } from './path'
                                        if let Some(idx) = project_graph.find_node_by_name_in_files(
                                            &reexport.original_name,
                                            &reexport_candidates,
                                        ) {
                                            if let Some(to_id) =
                                                project_graph.get_node(idx).map(|n| n.id)
                                            {
                                                cross_file_edges.push(CallEdge::new(
                                                    unresolved.caller_id,
                                                    to_id,
                                                    EdgeConfidence::HighLikely,
                                                ));
                                                resolved = true;
                                            }
                                        }
                                    } else if reexport.exported_name == "*" {
                                        // Wildcard re-export: export * from './path'
                                        // Look for the callee in the re-exported source files
                                        if let Some(idx) = project_graph.find_node_by_name_in_files(
                                            callee_name,
                                            &reexport_candidates,
                                        ) {
                                            if let Some(to_id) =
                                                project_graph.get_node(idx).map(|n| n.id)
                                            {
                                                cross_file_edges.push(CallEdge::new(
                                                    unresolved.caller_id,
                                                    to_id,
                                                    EdgeConfidence::HighLikely,
                                                ));
                                                resolved = true;
                                            }
                                        }
                                        // Also try original callee_name if it was renamed via import
                                        if !resolved && unresolved.imported_as.is_some() {
                                            if let Some(idx) = project_graph
                                                .find_node_by_name_in_files(
                                                    &unresolved.callee_name,
                                                    &reexport_candidates,
                                                )
                                            {
                                                if let Some(to_id) =
                                                    project_graph.get_node(idx).map(|n| n.id)
                                                {
                                                    cross_file_edges.push(CallEdge::new(
                                                        unresolved.caller_id,
                                                        to_id,
                                                        EdgeConfidence::HighLikely,
                                                    ));
                                                    resolved = true;
                                                }
                                            }
                                        }
                                    }
                                    if resolved {
                                        break;
                                    }
                                }
                            }
                            if resolved {
                                break;
                            }
                        }
                    }
                }

                // Fall back to proximity-based global lookup (language-scoped)
                if !resolved {
                    let caller_file = &pf.path;
                    let caller_lang = pf.language;
                    resolved = Self::resolve_global_with_proximity(
                        callee_name,
                        caller_file,
                        caller_lang,
                        &unresolved.caller_id,
                        &project_graph,
                        &mut cross_file_edges,
                    );
                    if !resolved && unresolved.imported_as.is_some() {
                        Self::resolve_global_with_proximity(
                            &unresolved.callee_name,
                            caller_file,
                            caller_lang,
                            &unresolved.caller_id,
                            &project_graph,
                            &mut cross_file_edges,
                        );
                    }
                }
            }
        }

        for edge in cross_file_edges {
            let _ = project_graph.add_edge(edge);
        }

        // Build dispatch edges from class hierarchy
        // Collect method names per class from the graph
        let mut class_methods: HashMap<String, HashSet<String>> = HashMap::new();
        for (_, node) in project_graph.nodes() {
            if let Some(class_name) = extract_class_from_full_name(&node.full_name) {
                class_methods
                    .entry(class_name)
                    .or_default()
                    .insert(node.name.clone());
            }
        }

        // For each class that extends/implements a parent, create dispatch edges
        let mut dispatch_edges = Vec::new();
        for pf in parsed_files {
            for rel in &pf.class_relations {
                for parent in &rel.parents {
                    // Find methods on child that also exist on parent
                    let child_methods = class_methods.get(&rel.class_name);
                    let parent_methods = class_methods.get(parent);
                    if let (Some(child_ms), Some(parent_ms)) = (child_methods, parent_methods) {
                        for method_name in child_ms.intersection(parent_ms) {
                            // Find the parent's method node and create edge to child's method
                            let parent_full = format!("{}.{}", parent, method_name);
                            let child_full = format!("{}.{}", rel.class_name, method_name);
                            if let (Some(parent_idx), Some(child_idx)) = (
                                project_graph.find_node_by_name(&parent_full),
                                project_graph.find_node_by_name(&child_full),
                            ) {
                                let parent_id = project_graph.get_node(parent_idx).unwrap().id;
                                let child_id = project_graph.get_node(child_idx).unwrap().id;
                                dispatch_edges.push(CallEdge::new(
                                    parent_id,
                                    child_id,
                                    EdgeConfidence::Possible,
                                ));
                            }
                        }
                    }
                }
            }
        }

        for edge in dispatch_edges {
            let _ = project_graph.add_edge(edge);
        }

        Ok(project_graph)
    }

    /// Proximity-based global fallback for cross-file call resolution.
    ///
    /// Language-scoped: only considers candidates in compatible languages.
    /// Three-level strategy to disambiguate when multiple functions share a name:
    ///   Level 1 — Same-file: unambiguous, `Certain` confidence.
    ///   Level 2 — Directory proximity: prefer same-dir, then parent-dir. `HighLikely`.
    ///   Level 3 — All matches: create edges to every candidate with `Possible` confidence.
    fn resolve_global_with_proximity(
        callee_name: &str,
        caller_file: &str,
        caller_lang: Language,
        caller_id: &NodeId,
        graph: &CodeGraph,
        edges: &mut Vec<CallEdge>,
    ) -> bool {
        use std::path::Path;

        // Level 1: Same-file preference
        if let Some(idx) = graph.find_node_by_name_in_file(callee_name, caller_file) {
            if let Some(to_id) = graph.get_node(idx).map(|n| n.id) {
                edges.push(CallEdge::new(*caller_id, to_id, EdgeConfidence::Certain));
                return true;
            }
        }

        // Get all candidates — filtered to compatible languages only
        let candidates = graph.find_nodes_by_name_and_language(callee_name, caller_lang);
        if candidates.is_empty() {
            return false;
        }
        if candidates.len() == 1 {
            if let Some(to_id) = graph.get_node(candidates[0]).map(|n| n.id) {
                edges.push(CallEdge::new(*caller_id, to_id, EdgeConfidence::HighLikely));
                return true;
            }
        }

        // Level 2: Directory proximity
        let caller_dir = Path::new(caller_file).parent().map(|p| p.to_path_buf());
        let caller_parent_dir = caller_dir
            .as_ref()
            .and_then(|d| d.parent())
            .map(|p| p.to_path_buf());

        let mut same_dir = Vec::new();
        let mut parent_dir = Vec::new();

        for &idx in &candidates {
            if let Some(node) = graph.get_node(idx) {
                let node_dir = Path::new(&node.location.file)
                    .parent()
                    .map(|p| p.to_path_buf());
                if node_dir == caller_dir {
                    same_dir.push(idx);
                } else if caller_parent_dir.is_some() && node_dir == caller_parent_dir {
                    parent_dir.push(idx);
                }
            }
        }

        if same_dir.len() == 1 {
            if let Some(to_id) = graph.get_node(same_dir[0]).map(|n| n.id) {
                edges.push(CallEdge::new(*caller_id, to_id, EdgeConfidence::HighLikely));
                return true;
            }
        }
        if same_dir.is_empty() && parent_dir.len() == 1 {
            if let Some(to_id) = graph.get_node(parent_dir[0]).map(|n| n.id) {
                edges.push(CallEdge::new(*caller_id, to_id, EdgeConfidence::HighLikely));
                return true;
            }
        }

        // Level 3: All matches — create edges to every candidate with Possible confidence
        for &idx in &candidates {
            if let Some(to_id) = graph.get_node(idx).map(|n| n.id) {
                edges.push(CallEdge::new(*caller_id, to_id, EdgeConfidence::Possible));
            }
        }
        true
    }

    /// Access the parser registry.
    pub fn registry(&self) -> &ParserRegistry {
        &self.registry
    }
}

/// Build a CodeGraph from a ZeroCopyParseTree using the extractors.
pub fn build_graph_from_tree(
    tree: &ZeroCopyParseTree,
    file_path: &str,
    language: Language,
) -> CodeGraph {
    let mut graph = CodeGraph::new().with_language(language);
    let functions = extract_functions(tree);
    let calls = extract_calls(tree);

    // Create nodes for each function
    let mut name_to_id: HashMap<String, NodeId> = HashMap::new();

    for (name, start_line, end_line, is_public) in &functions {
        let visibility = if *is_public {
            Visibility::Public
        } else {
            Visibility::Private
        };
        let kind = if name.chars().next().is_some_and(|c| c.is_uppercase()) {
            NodeKind::Class
        } else {
            NodeKind::Function
        };

        let node = CodeNode::new(
            name.clone(),
            kind,
            SourceLocation::new(file_path.to_string(), *start_line, *end_line, 0, 0),
            language,
            visibility,
        )
        .with_lines_of_code(end_line.saturating_sub(*start_line) + 1);

        let node_id = node.id;
        let idx = graph.add_node(node);
        name_to_id.insert(name.clone(), node_id);

        if is_main_like(name) {
            graph.add_entry_point(idx);
        }
        if is_test_like(name) {
            graph.add_test_entry_point(idx);
        }
    }

    // Create edges for calls
    for (call_line, callee_name) in &calls {
        let caller_id =
            find_containing_function(&functions, *call_line).and_then(|name| name_to_id.get(name));
        let callee_id = name_to_id.get(callee_name.as_str());

        if let (Some(&from_id), Some(&to_id)) = (caller_id, callee_id) {
            let edge = CallEdge::certain(from_id, to_id);
            let _ = graph.add_edge(edge);
        }
    }

    graph
}

fn find_containing_function(
    functions: &[(String, usize, usize, bool)],
    line: usize,
) -> Option<&str> {
    let mut best: Option<&(String, usize, usize, bool)> = None;
    for func in functions {
        if line >= func.1
            && line <= func.2
            && (best.is_none() || (func.2 - func.1) < (best.unwrap().2 - best.unwrap().1))
        {
            best = Some(func);
        }
    }
    best.map(|f| f.0.as_str())
}

fn is_entry_point_heuristic(node: &CodeNode) -> bool {
    is_main_like(&node.name)
        || node.attributes.iter().any(|a| {
            a.contains("route")
                || a.contains("handler")
                || a.contains("endpoint")
                || a.contains("api")
                || a.contains("main")
        })
}

fn is_test_entry_point(node: &CodeNode) -> bool {
    node.is_test || is_test_like(&node.name)
}

fn is_main_like(name: &str) -> bool {
    matches!(name, "main" | "__main__" | "Main" | "app" | "run" | "start")
}

/// Extract the class name from a full_name like "ClassName.method" or "ClassName::method".
fn extract_class_from_full_name(full_name: &str) -> Option<String> {
    // Try dot separator (Python, JS, Java, C#)
    if let Some(dot_pos) = full_name.rfind('.') {
        let class_part = &full_name[..dot_pos];
        if !class_part.is_empty() && !class_part.contains('/') {
            return Some(class_part.to_string());
        }
    }
    // Try :: separator (Rust, C++)
    if let Some(sep_pos) = full_name.rfind("::") {
        let class_part = &full_name[..sep_pos];
        if !class_part.is_empty() {
            return Some(class_part.to_string());
        }
    }
    None
}

fn is_test_like(name: &str) -> bool {
    name.starts_with("test_")
        || name.starts_with("Test")
        || name.ends_with("_test")
        || name.starts_with("it_")
        || name.starts_with("should_")
}

/// A re-export extracted from a barrel file's source text.
struct BarrelReexport {
    /// The name as exported (may be an alias).
    exported_name: String,
    /// The original name in the source module.
    original_name: String,
    /// The relative source module path (e.g. `./db`).
    source_path: String,
}

/// Extract re-export statements from a barrel file's source text.
///
/// Handles both JS/TS and Python barrel patterns:
/// - JS/TS: `export { Name } from './path'`, `export * from './path'`
/// - Python: `from .module import name`, `from .module import name as alias`, `from .module import *`
fn extract_barrel_reexports(source: &str) -> Vec<BarrelReexport> {
    let mut results = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();

        // JS/TS: export ... from '...'
        if trimmed.starts_with("export") {
            if let Some(reexports) = extract_js_reexports(trimmed) {
                results.extend(reexports);
            }
            continue;
        }

        // Python: from .module import name [as alias]
        if trimmed.starts_with("from ") && trimmed.contains(" import ") {
            if let Some(reexports) = extract_python_reexports(trimmed) {
                results.extend(reexports);
            }
        }
    }
    results
}

/// Parse JS/TS re-export line: `export { X } from './path'` or `export * from './path'`
fn extract_js_reexports(trimmed: &str) -> Option<Vec<BarrelReexport>> {
    // Extract the `from '...'` or `from "..."` path
    let from_path = {
        let pos = trimmed.find("from ")?;
        let after = trimmed[pos + 5..].trim().trim_end_matches(';').trim();
        if (after.starts_with('\'') && after.ends_with('\''))
            || (after.starts_with('"') && after.ends_with('"'))
        {
            Some(after[1..after.len() - 1].to_string())
        } else {
            None
        }
    }?;

    let mut results = Vec::new();

    // `export * from './path'`
    if trimmed.contains("export *") || trimmed.contains("export  *") {
        results.push(BarrelReexport {
            exported_name: "*".to_string(),
            original_name: "*".to_string(),
            source_path: from_path,
        });
        return Some(results);
    }

    // `export { ... } from './path'`
    let brace_start = trimmed.find('{')?;
    let brace_end = trimmed.find('}')?;
    let names_str = &trimmed[brace_start + 1..brace_end];
    for name_part in names_str.split(',') {
        let name_part = name_part.trim();
        if name_part.is_empty() {
            continue;
        }
        if let Some(as_pos) = name_part.find(" as ") {
            let original = name_part[..as_pos].trim().to_string();
            let alias = name_part[as_pos + 4..].trim().to_string();
            results.push(BarrelReexport {
                exported_name: alias,
                original_name: original,
                source_path: from_path.clone(),
            });
        } else {
            results.push(BarrelReexport {
                exported_name: name_part.to_string(),
                original_name: name_part.to_string(),
                source_path: from_path.clone(),
            });
        }
    }
    Some(results)
}

/// Parse Python re-export line: `from .module import name [as alias]`
fn extract_python_reexports(trimmed: &str) -> Option<Vec<BarrelReexport>> {
    // Split on " import " to get module path and imported names
    let import_pos = trimmed.find(" import ")?;
    let module_part = trimmed[5..import_pos].trim(); // after "from ", before " import "
    let names_part = trimmed[import_pos + 8..].trim(); // after " import "

    // Module must be a relative import (starts with .)
    if !module_part.starts_with('.') {
        return None;
    }

    let mut results = Vec::new();

    // `from .module import *`
    if names_part == "*" {
        results.push(BarrelReexport {
            exported_name: "*".to_string(),
            original_name: "*".to_string(),
            source_path: module_part.to_string(),
        });
        return Some(results);
    }

    // `from .module import A, B as C, D`
    // Handle parenthesized imports: `from .module import (A, B)`
    let names_str = names_part
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim();
    for name_part in names_str.split(',') {
        let name_part = name_part.trim();
        if name_part.is_empty() {
            continue;
        }
        if let Some(as_pos) = name_part.find(" as ") {
            let original = name_part[..as_pos].trim().to_string();
            let alias = name_part[as_pos + 4..].trim().to_string();
            results.push(BarrelReexport {
                exported_name: alias,
                original_name: original,
                source_path: module_part.to_string(),
            });
        } else {
            results.push(BarrelReexport {
                exported_name: name_part.to_string(),
                original_name: name_part.to_string(),
                source_path: module_part.to_string(),
            });
        }
    }
    Some(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_graph_from_python() {
        let builder = GraphBuilder::new().unwrap();
        let graph = builder
            .build_file_graph(
                r#"
def main():
    helper()

def helper():
    pass

def unused():
    pass
"#,
                "test.py",
                Language::Python,
            )
            .unwrap();

        assert!(graph.node_count() >= 3);
        assert!(graph.find_node_by_name("main").is_some());
    }

    #[test]
    fn test_build_graph_edges() {
        let builder = GraphBuilder::new().unwrap();
        let graph = builder
            .build_file_graph(
                r#"
def foo():
    bar()

def bar():
    pass
"#,
                "test.py",
                Language::Python,
            )
            .unwrap();

        assert!(graph.node_count() >= 2);
    }

    #[test]
    fn test_entry_point_heuristics() {
        assert!(is_main_like("main"));
        assert!(is_main_like("__main__"));
        assert!(!is_main_like("helper"));

        assert!(is_test_like("test_foo"));
        assert!(is_test_like("TestFoo"));
        assert!(!is_test_like("foo"));
    }

    #[test]
    fn test_extract_class_from_full_name() {
        assert_eq!(
            extract_class_from_full_name("MyClass.method"),
            Some("MyClass".to_string())
        );
        assert_eq!(
            extract_class_from_full_name("MyStruct::method"),
            Some("MyStruct".to_string())
        );
        assert_eq!(extract_class_from_full_name("bare_function"), None);
        // File paths shouldn't be treated as class names
        assert_eq!(extract_class_from_full_name("src/auth.rs"), None);
    }

    #[test]
    fn test_source_module_propagated_in_parsed_file() {
        // Verify that source_module is set when parsing JS/TS files with imports
        let builder = GraphBuilder::new().unwrap();
        let parsed = builder
            .registry()
            .get_parser(Language::TypeScript)
            .unwrap()
            .parse_file(
                "src/app.ts",
                r#"
import { helper } from './utils';

function main() {
    helper();
}
"#,
            )
            .unwrap();

        let has_source_module = parsed.unresolved_calls.iter().any(|u| {
            (u.callee_name == "helper" || u.imported_as.as_deref() == Some("helper"))
                && u.source_module.as_deref() == Some("./utils")
        });
        assert!(
            has_source_module,
            "UnresolvedCall for 'helper' should have source_module='./utils', got: {:?}",
            parsed
                .unresolved_calls
                .iter()
                .map(|u| (&u.callee_name, &u.imported_as, &u.source_module))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_import_scoped_resolution_picks_correct_file() {
        // Two files both define "helper", but only one is the import target.
        // File A (src/utils.ts) defines helper()
        // File B (src/other.ts) also defines helper()
        // File C (src/app.ts) imports helper from './utils' and calls it
        // The builder should resolve to the helper in utils.ts, not other.ts.
        let builder = GraphBuilder::new().unwrap();

        let parsed_a = builder
            .registry()
            .get_parser(Language::TypeScript)
            .unwrap()
            .parse_file(
                "src/utils.ts",
                r#"
export function helper() {
    return "from utils";
}
"#,
            )
            .unwrap();

        let parsed_b = builder
            .registry()
            .get_parser(Language::TypeScript)
            .unwrap()
            .parse_file(
                "src/other.ts",
                r#"
export function helper() {
    return "from other";
}
"#,
            )
            .unwrap();

        let parsed_c = builder
            .registry()
            .get_parser(Language::TypeScript)
            .unwrap()
            .parse_file(
                "src/app.ts",
                r#"
import { helper } from './utils';

function main() {
    helper();
}
"#,
            )
            .unwrap();

        let graph = builder
            .build_project_graph(&[parsed_a, parsed_b, parsed_c])
            .unwrap();

        // main should have an edge to helper
        let main_idx = graph.find_node_by_name("main");
        assert!(main_idx.is_some(), "Should find main node");

        // Check that main calls helper in utils.ts (file-scoped)
        let main_callees: Vec<_> = graph.calls_from(main_idx.unwrap()).collect();
        assert!(
            !main_callees.is_empty(),
            "main should have outgoing edges to helper"
        );

        // The resolved callee should be in src/utils.ts
        let callee_node = graph.get_node(main_callees[0]).unwrap();
        assert_eq!(
            callee_node.location.file, "src/utils.ts",
            "Should resolve to helper in src/utils.ts, not src/other.ts. Got: {}",
            callee_node.location.file
        );
    }

    #[test]
    fn test_class_relations_populated() {
        let builder = GraphBuilder::new().unwrap();
        let parsed = builder
            .registry()
            .get_parser(Language::Python)
            .unwrap()
            .parse_file(
                "test.py",
                r#"
class Animal:
    def speak(self):
        pass

class Dog(Animal):
    def speak(self):
        return "woof"
"#,
            )
            .unwrap();

        assert!(
            !parsed.class_relations.is_empty(),
            "Should have class relations, got: {:?}",
            parsed.class_relations
        );
        let dog_rel = parsed
            .class_relations
            .iter()
            .find(|r| r.class_name == "Dog");
        assert!(dog_rel.is_some(), "Should find Dog in class_relations");
        assert!(
            dog_rel.unwrap().parents.contains(&"Animal".to_string()),
            "Dog should extend Animal"
        );
    }

    #[test]
    fn test_extends_attributes_on_child_methods() {
        let builder = GraphBuilder::new().unwrap();
        let parsed = builder
            .registry()
            .get_parser(Language::Python)
            .unwrap()
            .parse_file(
                "test.py",
                r#"
class Animal:
    def speak(self):
        pass

class Dog(Animal):
    def speak(self):
        return "woof"
"#,
            )
            .unwrap();

        // The "speak" method in Dog should have "extends:Animal" attribute
        let dog_speak = parsed
            .nodes
            .iter()
            .find(|n| n.name == "speak" && n.attributes.iter().any(|a| a == "extends:Animal"));
        assert!(
            dog_speak.is_some(),
            "Dog.speak should have extends:Animal attribute. Nodes: {:?}",
            parsed
                .nodes
                .iter()
                .map(|n| (&n.name, &n.attributes, n.location.line_start))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_extract_barrel_reexports_js() {
        let source = r#"export { connect } from './db';
export { helper as h } from './utils';
export * from './all';
"#;
        let reexports = extract_barrel_reexports(source);
        assert_eq!(reexports.len(), 3);
        assert_eq!(reexports[0].exported_name, "connect");
        assert_eq!(reexports[0].original_name, "connect");
        assert_eq!(reexports[0].source_path, "./db");
        assert_eq!(reexports[1].exported_name, "h");
        assert_eq!(reexports[1].original_name, "helper");
        assert_eq!(reexports[1].source_path, "./utils");
        assert_eq!(reexports[2].exported_name, "*");
        assert_eq!(reexports[2].source_path, "./all");
    }

    #[test]
    fn test_extract_barrel_reexports_python() {
        let source = r#"from .db import connect
from .utils import helper as h
from .models import *
"#;
        let reexports = extract_barrel_reexports(source);
        assert_eq!(
            reexports.len(),
            3,
            "Should extract 3 Python re-exports, got: {:?}",
            reexports
                .iter()
                .map(|r| (&r.exported_name, &r.source_path))
                .collect::<Vec<_>>()
        );
        assert_eq!(reexports[0].exported_name, "connect");
        assert_eq!(reexports[0].original_name, "connect");
        assert_eq!(reexports[0].source_path, ".db");
        assert_eq!(reexports[1].exported_name, "h");
        assert_eq!(reexports[1].original_name, "helper");
        assert_eq!(reexports[1].source_path, ".utils");
        assert_eq!(reexports[2].exported_name, "*");
        assert_eq!(reexports[2].source_path, ".models");
    }

    #[test]
    fn test_extract_barrel_reexports_python_multi_import() {
        let source = "from .db import connect, disconnect, query\n";
        let reexports = extract_barrel_reexports(source);
        assert_eq!(reexports.len(), 3);
        assert_eq!(reexports[0].exported_name, "connect");
        assert_eq!(reexports[1].exported_name, "disconnect");
        assert_eq!(reexports[2].exported_name, "query");
    }

    #[test]
    fn test_extract_barrel_reexports_ignores_absolute_python_imports() {
        // Absolute Python imports (no leading dot) are not barrel re-exports
        let source = "from os import path\nfrom sys import argv\n";
        let reexports = extract_barrel_reexports(source);
        assert_eq!(
            reexports.len(),
            0,
            "Absolute Python imports should not be treated as barrel re-exports"
        );
    }
}
