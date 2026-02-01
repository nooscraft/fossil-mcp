//! `define_parser!` macro — generates parser struct + `LanguageParser` impl for each language.
//!
//! Replaces ~3000 lines of copy-paste adapter code with a single macro invocation per language.

/// Generate a parser struct implementing `crate::core::LanguageParser`.
///
/// Usage:
/// ```ignore
/// define_parser!(PythonParser, Language::Python, tree_sitter_python::LANGUAGE, &["py"]);
/// ```
macro_rules! define_parser {
    ($name:ident, $lang:expr, $ts_lang:expr, $exts:expr) => {
        pub struct $name {
            parser: std::sync::Mutex<tree_sitter::Parser>,
        }

        impl $name {
            pub fn new() -> Result<Self, crate::core::Error> {
                let mut parser = tree_sitter::Parser::new();
                let lang = $ts_lang;
                parser.set_language(&lang.into()).map_err(|e| {
                    crate::core::Error::parse(format!(
                        "Failed to set {} language: {e}",
                        stringify!($name)
                    ))
                })?;
                Ok(Self {
                    parser: std::sync::Mutex::new(parser),
                })
            }

            /// Parse source code, returning a zero-copy parse tree.
            pub fn parse_to_tree(
                &self,
                source: &str,
            ) -> Result<crate::parsers::ZeroCopyParseTree, crate::core::Error> {
                let mut parser = self
                    .parser
                    .lock()
                    .map_err(|e| crate::core::Error::parse(format!("Parser lock poisoned: {e}")))?;
                let tree = parser
                    .parse(source, None)
                    .ok_or_else(|| crate::core::Error::parse("tree-sitter parse returned None"))?;
                Ok(crate::parsers::ZeroCopyParseTree::new(
                    tree,
                    source.to_string(),
                    $lang,
                ))
            }

            /// Incremental parse: pass the old tree for faster re-parsing.
            #[allow(dead_code)]
            pub fn parse_incremental(
                &self,
                source: &str,
                old_tree: &tree_sitter::Tree,
            ) -> Result<crate::parsers::ZeroCopyParseTree, crate::core::Error> {
                let mut parser = self
                    .parser
                    .lock()
                    .map_err(|e| crate::core::Error::parse(format!("Parser lock poisoned: {e}")))?;
                let tree = parser
                    .parse(source, Some(old_tree))
                    .ok_or_else(|| crate::core::Error::parse("Incremental parse returned None"))?;
                Ok(crate::parsers::ZeroCopyParseTree::new(
                    tree,
                    source.to_string(),
                    $lang,
                ))
            }
        }

        impl crate::core::LanguageParser for $name {
            fn language(&self) -> crate::core::Language {
                $lang
            }

            fn extensions(&self) -> &[&str] {
                $exts
            }

            fn parse(&self, source: &str) -> crate::core::Result<Box<dyn crate::core::ParseTree>> {
                let tree = self.parse_to_tree(source)?;
                Ok(Box::new(tree))
            }

            fn parse_file(
                &self,
                file_path: &str,
                source: &str,
            ) -> crate::core::Result<crate::core::ParsedFile> {
                use std::time::Instant;
                let start = Instant::now();
                let tree = self.parse_to_tree(source)?;

                let functions = crate::parsers::extract_functions(&tree);
                let calls = crate::parsers::extract_calls(&tree);
                let symbol_refs = crate::parsers::extract_symbol_refs(&tree);
                let attributes = crate::parsers::extract_attributes(&tree);
                let imports = crate::parsers::extract_imports(&tree);
                let class_hierarchy = crate::parsers::extract_class_hierarchy(&tree);

                // Build attribute map: (name, start_line) -> Vec<String>
                let attr_map: std::collections::HashMap<(&str, usize), &Vec<String>> =
                    attributes.iter().map(|(n, l, a)| ((n.as_str(), *l), a)).collect();

                // Build import lookup: imported_name/alias -> (original_name, source_path)
                let mut import_map: std::collections::HashMap<String, (String, String)> =
                    std::collections::HashMap::new();
                for (name, source_path, alias, _) in &imports {
                    let local_name = alias.as_deref().unwrap_or(name);
                    import_map.insert(local_name.to_string(), (name.clone(), source_path.clone()));
                }

                let mut parsed =
                    crate::core::ParsedFile::new(file_path.to_string(), $lang, source.to_string());

                // Build nodes from extracted functions
                for (name, start_line, end_line, is_public) in &functions {
                    let vis = if *is_public {
                        crate::core::Visibility::Public
                    } else {
                        crate::core::Visibility::Private
                    };
                    let loc = crate::core::SourceLocation::new(
                        file_path.to_string(),
                        *start_line,
                        *end_line,
                        0,
                        0,
                    );
                    let mut node = crate::core::CodeNode::new(
                        name.clone(),
                        crate::core::NodeKind::Function,
                        loc,
                        $lang,
                        vis,
                    )
                    .with_lines_of_code(end_line.saturating_sub(*start_line) + 1);

                    // Apply extracted attributes/decorators
                    if let Some(attrs) = attr_map.get(&(name.as_str(), *start_line)) {
                        node = node.with_attributes((*attrs).clone());
                        if attrs.iter().any(|a| a == "test" || a == "cfg_test") {
                            node = node.with_test();
                        }
                    }

                    // Only mark main-like functions as entry points, not all functions.
                    // The dead code detector has its own heuristics for additional entries.
                    if matches!(
                        name.as_str(),
                        "main" | "__main__" | "Main" | "app" | "run" | "start" | "init"
                            | "handler" | "lambda_handler"
                    ) {
                        parsed.entry_points.push(node.id);
                    }
                    parsed.nodes.push(node);
                }

                // Create a synthetic <module> node for top-level code in dynamic
                // languages (Python, JS/TS, Ruby, PHP). This ensures top-level
                // calls and references are attributed to a containing function node
                // instead of being silently dropped.
                let total_lines = source.lines().count();
                let is_dynamic_lang = matches!(
                    $lang,
                    crate::core::Language::Python
                        | crate::core::Language::JavaScript
                        | crate::core::Language::TypeScript
                        | crate::core::Language::Ruby
                        | crate::core::Language::PHP
                );
                let module_id = if is_dynamic_lang {
                    let module_name = format!("<module:{}>", file_path);
                    let loc = crate::core::SourceLocation::new(
                        file_path.to_string(),
                        1,
                        total_lines.max(1),
                        0,
                        0,
                    );
                    let module_node = crate::core::CodeNode::new(
                        module_name,
                        crate::core::NodeKind::Function,
                        loc,
                        $lang,
                        crate::core::Visibility::Public,
                    )
                    .with_lines_of_code(total_lines);
                    let mid = module_node.id;
                    parsed.entry_points.push(mid);
                    parsed.nodes.push(module_node);
                    Some(mid)
                } else {
                    None
                };

                // Store class hierarchy relations and add extends:/implements: attributes
                // to methods belonging to child classes
                let mut class_parent_map: std::collections::HashMap<String, Vec<String>> =
                    std::collections::HashMap::new();
                for (class_name, parents, line) in &class_hierarchy {
                    parsed.class_relations.push(crate::core::ClassRelation {
                        class_name: class_name.clone(),
                        parents: parents.clone(),
                        line: *line,
                    });
                    class_parent_map.insert(class_name.clone(), parents.clone());
                }

                // Add extends:/implements: attributes to methods within classes
                // that have inheritance. A method is "in" a class if it's defined
                // between the class start/end lines.
                // First, build a map: class_name -> (start_line, end_line)
                let class_ranges: std::collections::HashMap<&str, (usize, usize)> = functions
                    .iter()
                    .filter(|(name, _, _, _)| class_parent_map.contains_key(name.as_str()))
                    .map(|(name, start, end, _)| (name.as_str(), (*start, *end)))
                    .collect();

                for node in &mut parsed.nodes {
                    // Skip class/module nodes themselves
                    if node.name.starts_with("<module:") {
                        continue;
                    }
                    // Check if this node is inside a class with inheritance
                    for (class_name, (cls_start, cls_end)) in &class_ranges {
                        if node.location.line_start >= *cls_start
                            && node.location.line_end <= *cls_end
                        {
                            if let Some(parents) = class_parent_map.get(*class_name) {
                                for parent in parents {
                                    node.attributes.push(format!("extends:{}", parent));
                                }
                            }
                            break;
                        }
                    }
                }

                // Set full_name for methods inside classes/impl blocks.
                //
                // For non-Rust languages: set full_name = "ClassName.method" using
                // class ranges (classes identified by uppercase first character).
                //
                // For Rust: set full_name = "Type::method" using impl block info,
                // and add "implements:Trait" attribute for trait impls.
                if $lang == crate::core::Language::Rust {
                    let impl_blocks = crate::parsers::extract_impl_blocks(&tree);
                    for (trait_name, type_name, impl_start, impl_end) in &impl_blocks {
                        for node in &mut parsed.nodes {
                            if node.location.line_start >= *impl_start
                                && node.location.line_end <= *impl_end
                                && node.name != *type_name
                                && !node.name.starts_with("<module:")
                            {
                                node.full_name = format!("{}::{}", type_name, node.name);
                                if let Some(ref trait_name) = trait_name {
                                    node.attributes.push(format!("implements:{}", trait_name));
                                }
                            }
                        }
                    }
                } else {
                    // Non-Rust: build class ranges for ALL classes (not just those with parents)
                    let all_class_ranges: Vec<(&str, usize, usize)> = functions.iter()
                        .filter(|(name, _, _, _)| {
                            // Heuristic: classes start with uppercase (Python/TS/Java/C#/Ruby/PHP)
                            name.chars().next().is_some_and(|c| c.is_uppercase())
                        })
                        .map(|(name, start, end, _)| (name.as_str(), *start, *end))
                        .collect();

                    for node in &mut parsed.nodes {
                        if node.name.starts_with("<module:") { continue; }
                        for &(class_name, cls_start, cls_end) in &all_class_ranges {
                            if node.name != class_name
                                && node.location.line_start >= cls_start
                                && node.location.line_end <= cls_end
                            {
                                node.full_name = format!("{}.{}", class_name, node.name);
                                break;
                            }
                        }
                    }
                }

                // Build edges from calls
                let name_to_id: std::collections::HashMap<&str, crate::core::NodeId> = parsed
                    .nodes
                    .iter()
                    .map(|n| (n.name.as_str(), n.id))
                    .collect();

                for (caller_line, callee_name) in &calls {
                    // Find the caller node (function containing this line)
                    let caller = parsed.nodes.iter().find(|n| {
                        n.location.line_start <= *caller_line && n.location.line_end >= *caller_line
                            && !n.name.starts_with("<module:") // don't match the synthetic module as inner fn
                    });
                    // Fall back to synthetic module node for top-level calls
                    let caller_id = caller.map(|n| n.id).or(module_id);
                    if let Some(cid) = caller_id {
                        if let Some(&callee_id) = name_to_id.get(callee_name.as_str()) {
                            parsed
                                .edges
                                .push(crate::core::CallEdge::certain(cid, callee_id));
                        } else if callee_name.contains('.') {
                            // First try full_name match (e.g., "AuthService.login" matches
                            // node with full_name "AuthService.login")
                            let full_name_match = parsed.nodes.iter().find(|n| n.full_name == *callee_name);
                            if let Some(matched) = full_name_match {
                                parsed.edges.push(crate::core::CallEdge::new(
                                    cid,
                                    matched.id,
                                    crate::core::EdgeConfidence::HighLikely,
                                ));
                            } else {
                            // Fallback: try bare method name for obj.method() patterns.
                            // When code calls `bus.emit()`, the callee name is "bus.emit"
                            // which doesn't match any node. Strip the variable qualifier
                            // and look for methods named "emit".
                            let bare_method = callee_name.rsplit('.').next().unwrap_or(callee_name);
                            let obj_part = callee_name.split('.').next().unwrap_or("");
                            let candidates: Vec<crate::core::NodeId> = parsed.nodes.iter()
                                .filter(|n| n.name == bare_method && matches!(n.kind,
                                    crate::core::NodeKind::Method
                                    | crate::core::NodeKind::AsyncMethod
                                    | crate::core::NodeKind::Constructor
                                    | crate::core::NodeKind::Function
                                    | crate::core::NodeKind::AsyncFunction))
                                .map(|n| n.id)
                                .collect();
                            if candidates.len() == 1 {
                                // Unambiguous: single method/function with that name
                                parsed.edges.push(crate::core::CallEdge::new(
                                    cid,
                                    candidates[0],
                                    crate::core::EdgeConfidence::HighLikely,
                                ));
                            } else {
                                // Create unresolved call with source_module if the
                                // object part is an imported name (e.g., svc.findUser()
                                // where svc comes from import { svc } from './service')
                                let mut unresolved = crate::core::UnresolvedCall::new(
                                    cid,
                                    bare_method.to_string(),
                                    *caller_line,
                                );
                                if let Some((_original, source_path)) = import_map.get(obj_part) {
                                    unresolved.source_module = Some(source_path.clone());
                                }
                                parsed.unresolved_calls.push(unresolved);
                            }
                            } // close full_name else
                        } else {
                            // Check if callee is an imported name
                            let mut unresolved = crate::core::UnresolvedCall::new(
                                cid,
                                callee_name.clone(),
                                *caller_line,
                            );
                            if let Some((original_name, source_path)) = import_map.get(callee_name.as_str()) {
                                // Use the original imported name for cross-file resolution
                                if original_name != callee_name {
                                    unresolved.imported_as = Some(original_name.clone());
                                }
                                unresolved.source_module = Some(source_path.clone());
                            }
                            parsed.unresolved_calls.push(unresolved);
                        }
                    }
                }

                // Use symbol references to detect function references (callbacks,
                // function arguments like `http.HandleFunc("/path", handler)`).
                // If a symbol name matches a defined function AND appears inside
                // another function's body, create a reference edge.
                let call_callees: std::collections::HashSet<&str> =
                    calls.iter().map(|(_, name)| name.as_str()).collect();
                for (ref_line, ref_name) in &symbol_refs {
                    // Only create edges for names that match known function definitions
                    // and aren't already handled by the call extraction above
                    if let Some(&callee_id) = name_to_id.get(ref_name.as_str()) {
                        // Skip if already captured as a direct call on the same line
                        if call_callees.contains(ref_name.as_str()) {
                            continue;
                        }
                        // Skip symbol refs at the callee's definition line.
                        // The function/method name always appears on the first line
                        // of its definition (e.g., `def func`, `void func()`) and
                        // should not be treated as a reference.
                        if let Some(callee_node) = parsed.nodes.iter().find(|n| n.id == callee_id) {
                            if *ref_line == callee_node.location.line_start {
                                continue;
                            }
                        }
                        // Find containing function
                        let caller = parsed.nodes.iter().find(|n| {
                            n.location.line_start <= *ref_line
                                && n.location.line_end >= *ref_line
                                && n.id != callee_id // don't self-reference
                                && !n.name.starts_with("<module:") // don't match synthetic module
                        });
                        // Fall back to synthetic module node for top-level refs
                        let caller_id = caller.map(|n| n.id).or(module_id);
                        if let Some(cid) = caller_id {
                            // Add a high-likelihood edge (not certain — it's a reference, not a call)
                            let edge = crate::core::CallEdge::new(
                                cid,
                                callee_id,
                                crate::core::EdgeConfidence::HighLikely,
                            );
                            // Avoid duplicates
                            if !parsed
                                .edges
                                .iter()
                                .any(|e| e.from == edge.from && e.to == edge.to)
                            {
                                parsed.edges.push(edge);
                            }
                        }
                    }
                }

                // Auto-edge: class → constructor/init methods.
                // When `new ClassName()` resolves to the class node via cross-file
                // imports, the constructor must also be marked as reachable.
                if $lang == crate::core::Language::Rust {
                    // Rust: link Type node → new() method via impl blocks
                    let impl_blocks_for_ctor = crate::parsers::extract_impl_blocks(&tree);
                    for (_, type_name, impl_start, impl_end) in &impl_blocks_for_ctor {
                        let type_node_id = parsed.nodes.iter()
                            .find(|n| n.name == *type_name)
                            .map(|n| n.id);
                        if let Some(tid) = type_node_id {
                            for node in &parsed.nodes {
                                if node.name == "new"
                                    && node.location.line_start >= *impl_start
                                    && node.location.line_end <= *impl_end
                                {
                                    if !parsed.edges.iter().any(|e| e.from == tid && e.to == node.id) {
                                        parsed.edges.push(crate::core::CallEdge::certain(tid, node.id));
                                    }
                                }
                            }
                        }
                    }
                } else {
                    // Non-Rust: link class node → constructor/__init__/initialize
                    let ctor_class_ranges: Vec<(&str, usize, usize)> = functions.iter()
                        .filter(|(name, _, _, _)| {
                            name.chars().next().is_some_and(|c| c.is_uppercase())
                        })
                        .map(|(name, start, end, _)| (name.as_str(), *start, *end))
                        .collect();
                    let constructor_names = ["constructor", "__init__", "initialize"];
                    for &(class_name, cls_start, cls_end) in &ctor_class_ranges {
                        let class_node_id = parsed.nodes.iter()
                            .find(|n| n.name == class_name && n.location.line_start == cls_start)
                            .map(|n| n.id);
                        if let Some(cls_id) = class_node_id {
                            for node in &parsed.nodes {
                                if node.location.line_start >= cls_start
                                    && node.location.line_end <= cls_end
                                    && constructor_names.contains(&node.name.as_str())
                                {
                                    if !parsed.edges.iter().any(|e| e.from == cls_id && e.to == node.id) {
                                        parsed.edges.push(crate::core::CallEdge::certain(cls_id, node.id));
                                    }
                                }
                            }
                        }
                    }
                }

                parsed.parse_duration_ms = start.elapsed().as_millis() as u32;
                Ok(parsed)
            }
        }
    };
}

pub(crate) use define_parser;
