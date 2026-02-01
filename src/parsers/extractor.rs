//! Shared extractor utilities for all languages.
//!
//! These functions traverse the tree-sitter CST to extract functions, calls,
//! and symbol references. Language-agnostic where possible, with
//! per-language node-type mappings.

use super::ZeroCopyParseTree;

/// Extract function/method definitions as (name, start_line, end_line, is_public).
pub fn extract_functions(tree: &ZeroCopyParseTree) -> Vec<(String, usize, usize, bool)> {
    let mut results = Vec::new();
    let root = tree.ts_tree().root_node();
    collect_functions(root, tree.source_code(), tree.language(), &mut results);
    results
}

fn collect_functions(
    node: tree_sitter::Node,
    source: &str,
    language: crate::core::Language,
    results: &mut Vec<(String, usize, usize, bool)>,
) {
    let kind = node.kind();

    // Handle arrow functions specially: get name from parent variable_declarator
    if kind == "arrow_function" {
        if let Some(name) = extract_arrow_function_name(node, source) {
            let start_line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;
            let is_public = check_visibility(node, source, language, &name);
            results.push((name, start_line, end_line, is_public));
        }
        // Still recurse into the body for nested function definitions
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            collect_functions(child, source, language, results);
        }
        return;
    }

    let is_func_def = matches!(
        kind,
        "function_definition"
            | "function_declaration"
            | "method_definition"
            | "method_declaration"
            | "function_item"
            | "function"
            | "func_literal"
    );

    // Ruby uses "method" for def/end blocks
    let is_ruby_method = kind == "method" && language == crate::core::Language::Ruby;

    if is_func_def || is_ruby_method {
        if let Some(name) = extract_name_from_def(node, source) {
            let start_line = node.start_position().row + 1; // 1-indexed
            let end_line = node.end_position().row + 1;
            let is_public = check_visibility(node, source, language, &name);
            results.push((name, start_line, end_line, is_public));
        }
    }

    // Also handle class/struct/trait definitions to extract method names
    let is_class_def = matches!(
        kind,
        "class_definition" | "class_declaration" | "struct_item" | "impl_item" | "trait_item"
    );
    if is_class_def {
        if let Some(name) = extract_name_from_def(node, source) {
            let start_line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;
            let is_public = check_visibility(node, source, language, &name);
            results.push((name, start_line, end_line, is_public));
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_functions(child, source, language, results);
    }
}

/// Extract the name of an arrow function from its parent variable_declarator.
/// Returns `Some("handler")` for `const handler = () => { ... }`.
/// Returns `None` for anonymous arrows or destructured assignments.
fn extract_arrow_function_name(node: tree_sitter::Node, source: &str) -> Option<String> {
    let parent = node.parent()?;
    if parent.kind() == "variable_declarator" {
        if let Some(name_node) = parent.child_by_field_name("name") {
            let name = source[name_node.byte_range()].to_string();
            // Only return name if it's a simple identifier (not destructuring)
            if !name.contains('{') && !name.contains('[') {
                return Some(name);
            }
        }
    }
    None
}

fn extract_name_from_def(node: tree_sitter::Node, source: &str) -> Option<String> {
    // Try common child field names: "name", "declarator"
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(source[name_node.byte_range()].to_string());
    }
    if let Some(decl) = node.child_by_field_name("declarator") {
        if let Some(name_node) = decl.child_by_field_name("name") {
            return Some(source[name_node.byte_range()].to_string());
        }
        return Some(source[decl.byte_range()].to_string());
    }
    None
}

fn check_visibility(
    node: tree_sitter::Node,
    source: &str,
    language: crate::core::Language,
    name: &str,
) -> bool {
    // Check for explicit visibility modifiers (Rust `pub`, Java `public`, JS `export`)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "visibility_modifier" || kind == "access_modifier" || kind == "modifier" {
            let text = &source[child.byte_range()];
            return text.contains("pub") || text.contains("public") || text.contains("export");
        }
    }

    // Language-specific visibility rules
    match language {
        // Rust: explicit `pub` keyword required. No modifier = private.
        // If we reach here, no visibility_modifier was found above, so it's private.
        crate::core::Language::Rust => false,

        // Go: uppercase first letter = exported, lowercase = unexported
        crate::core::Language::Go => name.starts_with(|c: char| c.is_uppercase()),

        // Java/C#/Kotlin/Scala: explicit access modifiers. No modifier varies by language,
        // but we treat it as package-private (not public).
        crate::core::Language::Java
        | crate::core::Language::CSharp
        | crate::core::Language::Kotlin
        | crate::core::Language::Scala => {
            // Check for Java-style modifiers in the node text
            let node_text = &source[node.byte_range()];
            let first_line = node_text.lines().next().unwrap_or("");
            first_line.contains("public") || first_line.contains("export")
        }

        // Python: leading underscore = private convention
        crate::core::Language::Python => !name.starts_with('_'),

        // Ruby: functions starting with _ or inside private block are private
        crate::core::Language::Ruby => !name.starts_with('_'),

        // PHP: check for public/private/protected keywords
        crate::core::Language::PHP => {
            let node_text = &source[node.byte_range()];
            let first_line = node_text.lines().next().unwrap_or("");
            // PHP: explicit private/protected = not public, everything else is public
            !first_line.contains("private") && !first_line.contains("protected")
        }

        // JavaScript/TypeScript: export keyword makes it public
        // Without export, it's module-scoped (private-ish)
        crate::core::Language::JavaScript | crate::core::Language::TypeScript => {
            // Walk up ancestors looking for export_statement.
            // For arrow functions: arrow_function → variable_declarator → lexical_declaration → export_statement
            // For regular functions: function_declaration → export_statement
            let mut current = node.parent();
            while let Some(n) = current {
                if n.kind() == "export_statement" {
                    return true;
                }
                current = n.parent();
            }
            // Top-level functions without export — treat as module-private
            false
        }

        // Default: assume public for languages we haven't specialized
        _ => true,
    }
}

/// Extract function calls as (caller_line, callee_name).
pub fn extract_calls(tree: &ZeroCopyParseTree) -> Vec<(usize, String)> {
    let mut results = Vec::new();
    let root = tree.ts_tree().root_node();
    collect_calls(root, tree.source_code(), &mut results);
    results
}

fn collect_calls(node: tree_sitter::Node, source: &str, results: &mut Vec<(usize, String)>) {
    let kind = node.kind();
    let line = node.start_position().row + 1;

    match kind {
        // Python: call(function=..., arguments=...)
        // JavaScript/TypeScript/Go/Rust/C/C++/Swift: call_expression(function=...)
        "call_expression" | "call" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let callee = extract_callee_name(func_node, source);
                if !callee.is_empty() {
                    results.push((line, callee));
                }
            } else if let Some(name_node) = node.child_by_field_name("name") {
                // Some languages use "name" field
                let callee = source[name_node.byte_range()].to_string();
                if !callee.is_empty() {
                    results.push((line, callee));
                }
            } else {
                // Ruby: call(identifier, argument_list) — callee is first identifier child
                extract_callee_from_first_child(node, source, line, results);
            }
        }

        // Java: method_invocation(object?, identifier, argument_list)
        "method_invocation" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let callee = source[name_node.byte_range()].to_string();
                if !callee.is_empty() {
                    results.push((line, callee));
                }
            } else {
                extract_callee_from_first_child(node, source, line, results);
            }
        }

        // C#: invocation_expression(identifier/member_access, argument_list)
        "invocation_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let callee = extract_callee_name(func_node, source);
                if !callee.is_empty() {
                    results.push((line, callee));
                }
            } else {
                extract_callee_from_first_child(node, source, line, results);
            }
        }

        // PHP: function_call_expression(name=..., arguments=...)
        "function_call_expression" => {
            if let Some(name_node) = node.child_by_field_name("function") {
                let callee = extract_callee_name(name_node, source);
                if !callee.is_empty() {
                    results.push((line, callee));
                }
            } else if let Some(name_node) = node.child_by_field_name("name") {
                let callee = source[name_node.byte_range()].to_string();
                if !callee.is_empty() {
                    results.push((line, callee));
                }
            }
        }

        // PHP: member_call_expression (e.g. $obj->method())
        "member_call_expression" | "scoped_call_expression" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let callee = source[name_node.byte_range()].to_string();
                if !callee.is_empty() {
                    results.push((line, callee));
                }
            }
        }

        // Java: object_creation_expression (new Foo())
        "object_creation_expression" => {
            if let Some(type_node) = node.child_by_field_name("type") {
                let callee = source[type_node.byte_range()].to_string();
                if !callee.is_empty() {
                    results.push((line, callee));
                }
            }
        }

        // JSX: <Component /> or <Component>...</Component> (TSX grammar)
        "jsx_self_closing_element" | "jsx_opening_element" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "identifier" {
                    let name = source[child.byte_range()].to_string();
                    // Only treat uppercase identifiers as component calls
                    // (lowercase like <div> are HTML tags, not components)
                    if !name.is_empty() && name.chars().next().is_some_and(|c| c.is_uppercase()) {
                        results.push((line, name));
                    }
                    break;
                }
            }
        }

        // JS/TS: new_expression (new Foo())
        "new_expression" => {
            if let Some(constructor) = node.child_by_field_name("constructor") {
                let callee = extract_callee_name(constructor, source);
                if !callee.is_empty() {
                    // Emit call to the class name (links to class definition)
                    results.push((line, callee.clone()));
                    // Emit scoped constructor call so "Foo.constructor" matches
                    // the full_name of the constructor method node
                    results.push((line, format!("{}.constructor", callee)));
                }
            }
        }

        // Rust: struct_expression (StructName { field: value })
        "struct_expression" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let callee = source[name_node.byte_range()].to_string();
                if !callee.is_empty() {
                    results.push((line, callee));
                }
            }
        }

        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls(child, source, results);
    }
}

/// Extract callee name from the first identifier child of a node.
/// Used for languages where the callee is a direct child (Ruby, Java fallback).
fn extract_callee_from_first_child(
    node: tree_sitter::Node,
    source: &str,
    line: usize,
    results: &mut Vec<(usize, String)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let child_kind = child.kind();
        if child_kind == "identifier"
            || child_kind == "constant"
            || child_kind == "simple_identifier"
        {
            let callee = source[child.byte_range()].to_string();
            if !callee.is_empty() {
                results.push((line, callee));
            }
            return;
        }
        if child_kind == "member_expression"
            || child_kind == "member_access_expression"
            || child_kind == "field_expression"
        {
            let callee = extract_callee_name(child, source);
            if !callee.is_empty() {
                results.push((line, callee));
            }
            return;
        }
    }
}

fn extract_callee_name(node: tree_sitter::Node, source: &str) -> String {
    let kind = node.kind();
    match kind {
        "identifier" | "type_identifier" => source[node.byte_range()].to_string(),
        "member_expression" | "attribute" | "field_expression" => {
            // For `obj.method()`, extract the method name.
            // If the receiver is `this`/`self`, return just the method name
            // so it can match class method definitions.
            // Otherwise, return `receiver.method` to avoid false matches
            // (e.g., `console.log()` should NOT match `OldLogger.log`).
            let method_name = if let Some(prop) = node.child_by_field_name("property") {
                source[prop.byte_range()].to_string()
            } else if let Some(field) = node.child_by_field_name("field") {
                source[field.byte_range()].to_string()
            } else if let Some(attr) = node.child_by_field_name("attribute") {
                source[attr.byte_range()].to_string()
            } else {
                return source[node.byte_range()].to_string();
            };

            // Check if the receiver is this/self — if so, return bare method name
            let receiver_node = node
                .child_by_field_name("object")
                .or_else(|| node.child_by_field_name("value"));
            if let Some(recv) = receiver_node {
                if is_self_receiver(recv, source) {
                    return method_name;
                }
                // Non-self receiver: qualify so we don't create false edges
                let receiver_text = source[recv.byte_range()].to_string();
                return format!("{}.{}", receiver_text, method_name);
            }

            method_name
        }
        "scoped_identifier" => {
            // Rust `module::function`
            if let Some(name) = node.child_by_field_name("name") {
                return source[name.byte_range()].to_string();
            }
            source[node.byte_range()].to_string()
        }
        _ => source[node.byte_range()].to_string(),
    }
}

/// Check whether a node is a `this`/`self`/`@` receiver,
/// handling chained member expressions like `this.field`.
fn is_self_receiver(node: tree_sitter::Node, source: &str) -> bool {
    match node.kind() {
        "this" | "self" => true,
        "identifier" => {
            let text = &source[node.byte_range()];
            text == "this" || text == "self"
        }
        // For chained access like `this.field`, check the root object
        "member_expression" | "attribute" | "field_expression" => {
            if let Some(obj) = node.child_by_field_name("object") {
                is_self_receiver(obj, source)
            } else if let Some(val) = node.child_by_field_name("value") {
                is_self_receiver(val, source)
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Extract attributes/decorators for definitions.
///
/// Returns `Vec<(definition_name, start_line, Vec<attribute_string>)>`.
/// Attributes are normalized:
/// - Rust `#[test]` / `#[tokio::test]` → `"test"`
/// - Rust `#[cfg(test)]` → `"cfg_test"`
/// - Rust `#[serde(default = "fn")]` → `"serde_default:fn"`
/// - Rust `#[serde(serialize_with = "fn")]` → `"serde_serialize_with:fn"`
/// - Rust `#[serde(deserialize_with = "fn")]` → `"serde_deserialize_with:fn"`
/// - Python `@pytest.mark.*` → `"test"`
/// - Python `@app.route(...)` → `"route"`
/// - Java/C# `@Test` → `"test"`
/// - Java `@Bean`, `@Controller` etc. → pass through
///
/// Also extracts trait impl context for Rust `impl Trait for Type` blocks:
/// - Functions inside `impl From<A> for B` get `"impl_trait:From"` attribute
pub fn extract_attributes(tree: &ZeroCopyParseTree) -> Vec<(String, usize, Vec<String>)> {
    let mut results = Vec::new();
    let root = tree.ts_tree().root_node();
    collect_attributes(
        root,
        tree.source_code(),
        tree.language(),
        &mut results,
        None,
        false,
    );
    results
}

fn collect_attributes(
    node: tree_sitter::Node,
    source: &str,
    language: crate::core::Language,
    results: &mut Vec<(String, usize, Vec<String>)>,
    impl_trait: Option<&str>,
    in_cfg_test: bool,
) {
    let kind = node.kind();

    // Track impl trait context for Rust
    let mut current_impl_trait: Option<String> = impl_trait.map(|s| s.to_string());
    if kind == "impl_item" && language == crate::core::Language::Rust {
        current_impl_trait = extract_impl_trait_name_ast(node, source);
    }

    // Track cfg(test) context for Rust modules
    let mut current_cfg_test = in_cfg_test;
    if kind == "mod_item" && language == crate::core::Language::Rust {
        let mut sibling = node.prev_sibling();
        while let Some(sib) = sibling {
            if sib.kind() == "attribute_item" {
                let text = source[sib.byte_range()].trim();
                if text.contains("cfg(test)") {
                    current_cfg_test = true;
                    break;
                }
            } else if sib.kind() != "line_comment" && sib.kind() != "block_comment" {
                break;
            }
            sibling = sib.prev_sibling();
        }
    }

    let is_func_def = matches!(
        kind,
        "function_definition"
            | "function_declaration"
            | "method_definition"
            | "method_declaration"
            | "function_item"
            | "function"
    );
    let is_class_def = matches!(
        kind,
        "class_definition" | "class_declaration" | "struct_item" | "impl_item"
    );

    if is_func_def || is_class_def {
        if let Some(name) = extract_name_from_def(node, source) {
            let start_line = node.start_position().row + 1;
            let mut attrs = Vec::new();

            match language {
                crate::core::Language::Rust => {
                    collect_rust_attributes(node, source, &mut attrs);
                    // For structs, also extract serde attributes from field declarations
                    if is_class_def && kind == "struct_item" {
                        collect_rust_field_serde_attributes(node, source, &mut attrs);
                    }
                }
                crate::core::Language::Python => {
                    collect_python_decorators(node, source, &mut attrs);
                }
                crate::core::Language::Java
                | crate::core::Language::Kotlin
                | crate::core::Language::CSharp => {
                    collect_java_annotations(node, source, &mut attrs);
                }
                crate::core::Language::TypeScript | crate::core::Language::JavaScript => {
                    collect_typescript_decorators(node, source, &mut attrs);
                }
                _ => {}
            }

            // Add impl trait context for Rust functions inside impl blocks
            if let Some(ref trait_name) = current_impl_trait {
                if is_func_def {
                    attrs.push(format!("impl_trait:{trait_name}"));
                }
            }

            // Propagate cfg(test) to functions inside #[cfg(test)] mod blocks
            if current_cfg_test && is_func_def && !attrs.contains(&"cfg_test".to_string()) {
                attrs.push("cfg_test".to_string());
            }

            if !attrs.is_empty() {
                results.push((name, start_line, attrs));
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_attributes(
            child,
            source,
            language,
            results,
            current_impl_trait.as_deref(),
            current_cfg_test,
        );
    }
}

/// Extract the trait name from a Rust `impl Trait for Type` using AST traversal.
/// More robust than text parsing — handles multi-line signatures.
///
/// In tree-sitter-rust, `impl_item` with a trait has children:
///   "impl" [type_parameters] TRAIT_TYPE "for" SELF_TYPE block
fn extract_impl_trait_name_ast(node: tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    let mut trait_type_node: Option<tree_sitter::Node> = None;

    for child in node.children(&mut cursor) {
        if child.kind() == "for" {
            // The trait type is the node we captured before "for"
            return trait_type_node.map(|n| extract_type_base_name(n, source));
        }
        // Capture type nodes (these are potential trait type)
        if is_type_node(child.kind()) {
            trait_type_node = Some(child);
        }
    }
    None
}

/// Extract the implementing type name from a Rust `impl [Trait for] Type` block.
/// Returns the type name after "for" (or the sole type for plain impl blocks).
fn extract_impl_type_name(node: tree_sitter::Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    let mut after_for = false;
    let mut last_type_node: Option<tree_sitter::Node> = None;

    for child in node.children(&mut cursor) {
        if child.kind() == "for" {
            after_for = true;
            continue;
        }
        if after_for && is_type_node(child.kind()) {
            return Some(extract_type_base_name(child, source));
        }
        // For plain impl blocks (no trait), capture the last type before the block
        if !after_for && is_type_node(child.kind()) {
            last_type_node = Some(child);
        }
    }
    // Plain impl block: return the type name
    if !after_for {
        return last_type_node.map(|n| extract_type_base_name(n, source));
    }
    None
}

/// Check if a tree-sitter node kind represents a type.
fn is_type_node(kind: &str) -> bool {
    matches!(
        kind,
        "type_identifier" | "generic_type" | "scoped_type_identifier" | "primitive_type"
    )
}

/// Extract the base name from a type node (strips generics).
/// "From" from `From<Foo>`, "Vec" from `Vec<T>`, "MyType" from `MyType`.
fn extract_type_base_name(node: tree_sitter::Node, source: &str) -> String {
    match node.kind() {
        "type_identifier" | "primitive_type" => source[node.byte_range()].to_string(),
        "generic_type" => {
            // generic_type has a "name" or "type" child
            if let Some(name_child) = node.child_by_field_name("type") {
                return extract_type_base_name(name_child, source);
            }
            // Fallback: first child is usually the type identifier
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "type_identifier" || child.kind() == "scoped_type_identifier" {
                    return extract_type_base_name(child, source);
                }
            }
            source[node.byte_range()].to_string()
        }
        "scoped_type_identifier" => {
            // e.g. std::fmt::Display → take last component "Display"
            if let Some(name) = node.child_by_field_name("name") {
                return source[name.byte_range()].to_string();
            }
            source[node.byte_range()].to_string()
        }
        _ => source[node.byte_range()].to_string(),
    }
}

/// Extract impl block info for setting full_name on methods.
/// Returns Vec<(trait_name, type_name, start_line, end_line)>.
pub fn extract_impl_blocks(
    tree: &ZeroCopyParseTree,
) -> Vec<(Option<String>, String, usize, usize)> {
    let mut results = Vec::new();
    if tree.language() != crate::core::Language::Rust {
        return results;
    }
    let root = tree.ts_tree().root_node();
    collect_impl_blocks(root, tree.source_code(), &mut results);
    results
}

fn collect_impl_blocks(
    node: tree_sitter::Node,
    source: &str,
    results: &mut Vec<(Option<String>, String, usize, usize)>,
) {
    if node.kind() == "impl_item" {
        let trait_name = extract_impl_trait_name_ast(node, source);
        // For trait impls, get the type after "for"
        // For plain impls, get the sole type
        let type_name = extract_impl_type_name(node, source);
        if let Some(type_name) = type_name {
            let start_line = node.start_position().row + 1;
            let end_line = node.end_position().row + 1;
            results.push((trait_name, type_name, start_line, end_line));
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_impl_blocks(child, source, results);
    }
}

/// Collect Rust attributes from preceding sibling nodes.
fn collect_rust_attributes(node: tree_sitter::Node, source: &str, attrs: &mut Vec<String>) {
    // Check preceding siblings for attribute_item nodes
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "attribute_item" || sib.kind() == "inner_attribute_item" {
            let text = source[sib.byte_range()].trim().to_string();
            normalize_rust_attribute(&text, attrs);
        } else if sib.kind() != "line_comment" && sib.kind() != "block_comment" {
            break;
        }
        sibling = sib.prev_sibling();
    }
}

/// Scan Rust struct field declarations for serde attributes.
/// Extracts `#[serde(default = "fn_name")]` etc. from fields and attaches
/// them to the struct's attribute list so the entry point detector can see them.
///
/// In tree-sitter Rust, field attributes are siblings of `field_declaration`
/// inside `field_declaration_list`, NOT children of `field_declaration`:
///   field_declaration_list { attribute_item, field_declaration, attribute_item, field_declaration, ... }
fn collect_rust_field_serde_attributes(
    struct_node: tree_sitter::Node,
    source: &str,
    attrs: &mut Vec<String>,
) {
    let mut cursor = struct_node.walk();
    for child in struct_node.children(&mut cursor) {
        if child.kind() == "field_declaration_list" {
            let mut cursor2 = child.walk();
            for item in child.children(&mut cursor2) {
                if item.kind() == "attribute_item" {
                    let text = source[item.byte_range()].trim();
                    if text.contains("serde(") {
                        normalize_rust_attribute(text, attrs);
                    }
                }
            }
        }
    }
}

fn normalize_rust_attribute(text: &str, attrs: &mut Vec<String>) {
    // Strip #[ and ]
    let inner = text
        .trim_start_matches("#[")
        .trim_start_matches("#![")
        .trim_end_matches(']');

    if inner == "test" || inner == "tokio::test" || inner == "async_std::test" {
        attrs.push("test".to_string());
    } else if inner == "cfg(test)" {
        attrs.push("cfg_test".to_string());
    } else if inner.starts_with("derive(") {
        let derive_inner = inner.trim_start_matches("derive(").trim_end_matches(')');
        for trait_name in derive_inner.split(',') {
            let t = trait_name.trim();
            if !t.is_empty() {
                attrs.push(format!("derive:{t}"));
            }
        }
    } else if inner.starts_with("serde(") {
        // Parse serde attributes
        let serde_inner = inner.trim_start_matches("serde(").trim_end_matches(')');
        for part in serde_inner.split(',') {
            let part = part.trim();
            if let Some(rest) = part.strip_prefix("default = ") {
                let fn_name = rest.trim_matches('"').trim_matches('\'');
                attrs.push(format!("serde_default:{fn_name}"));
            } else if part.trim() == "default" {
                attrs.push("serde_uses_default".to_string());
            } else if let Some(rest) = part.strip_prefix("serialize_with = ") {
                let fn_name = rest.trim_matches('"').trim_matches('\'');
                attrs.push(format!("serde_serialize_with:{fn_name}"));
            } else if let Some(rest) = part.strip_prefix("deserialize_with = ") {
                let fn_name = rest.trim_matches('"').trim_matches('\'');
                attrs.push(format!("serde_deserialize_with:{fn_name}"));
            }
        }
    } else {
        // Pass through other attributes as-is
        attrs.push(inner.to_string());
    }
}

/// Collect Python decorator attributes.
fn collect_python_decorators(node: tree_sitter::Node, source: &str, attrs: &mut Vec<String>) {
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "decorator" {
            let text = source[sib.byte_range()].trim().to_string();
            // Strip leading @
            let decorator = text.trim_start_matches('@');
            // Strip arguments
            let name = decorator.split('(').next().unwrap_or(decorator).trim();

            if name.starts_with("pytest.mark") || name == "unittest.skip" {
                attrs.push("test".to_string());
            } else if name.contains("route") || name.starts_with("app.") {
                attrs.push("route".to_string());
            } else {
                attrs.push(name.to_string());
            }
        } else if sib.kind() != "comment" {
            break;
        }
        sibling = sib.prev_sibling();
    }
}

/// Collect Java/C#/Kotlin annotation attributes.
fn collect_java_annotations(node: tree_sitter::Node, source: &str, attrs: &mut Vec<String>) {
    // Java: annotations are children of the definition node or preceding siblings
    // Check children first (modifiers node may contain annotations)
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifiers"
            || child.kind() == "annotation"
            || child.kind() == "marker_annotation"
        {
            collect_annotations_from_node(child, source, attrs);
        }
    }
    // Also check preceding siblings
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "annotation" || sib.kind() == "marker_annotation" {
            let text = source[sib.byte_range()].trim().to_string();
            let name = text
                .trim_start_matches('@')
                .split('(')
                .next()
                .unwrap_or("")
                .trim();
            if name == "Test" || name == "ParameterizedTest" || name == "RepeatedTest" {
                attrs.push("test".to_string());
            } else if !name.is_empty() {
                attrs.push(name.to_string());
            }
        } else if sib.kind() != "line_comment" && sib.kind() != "block_comment" {
            break;
        }
        sibling = sib.prev_sibling();
    }
}

/// Collect TypeScript/JavaScript decorator attributes.
fn collect_typescript_decorators(node: tree_sitter::Node, source: &str, attrs: &mut Vec<String>) {
    // tree-sitter-typescript: decorators are children of the decorated node
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let text = source[child.byte_range()].trim();
            let name = text
                .trim_start_matches('@')
                .split('(')
                .next()
                .unwrap_or("")
                .trim();
            normalize_ts_decorator(name, attrs);
        }
    }
    // Also check preceding siblings (handles some grammar variants)
    let mut sibling = node.prev_sibling();
    while let Some(sib) = sibling {
        if sib.kind() == "decorator" {
            let text = source[sib.byte_range()].trim();
            let name = text
                .trim_start_matches('@')
                .split('(')
                .next()
                .unwrap_or("")
                .trim();
            normalize_ts_decorator(name, attrs);
        } else if !matches!(sib.kind(), "comment" | "line_comment" | "block_comment") {
            break;
        }
        sibling = sib.prev_sibling();
    }
}

fn normalize_ts_decorator(name: &str, attrs: &mut Vec<String>) {
    match name {
        "Get" | "Post" | "Put" | "Delete" | "Patch" | "Head" | "Options" | "All" | "Controller"
        | "RequestMapping" | "Route" => attrs.push("route".into()),
        "Injectable" | "Component" | "Service" | "Module" | "Pipe" | "Guard" | "Interceptor"
        | "Middleware" => attrs.push("component".into()),
        "Test" | "Suite" | "Fixture" => attrs.push("test".into()),
        _ if !name.is_empty() => attrs.push(name.into()),
        _ => {}
    }
}

fn collect_annotations_from_node(node: tree_sitter::Node, source: &str, attrs: &mut Vec<String>) {
    let kind = node.kind();
    if kind == "annotation" || kind == "marker_annotation" {
        let text = source[node.byte_range()].trim().to_string();
        let name = text
            .trim_start_matches('@')
            .split('(')
            .next()
            .unwrap_or("")
            .trim();
        if name == "Test" || name == "ParameterizedTest" || name == "RepeatedTest" {
            attrs.push("test".to_string());
        } else if !name.is_empty() {
            attrs.push(name.to_string());
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_annotations_from_node(child, source, attrs);
    }
}

/// Extract import statements from source code.
///
/// Returns `Vec<(imported_name, source_path, alias, line)>`.
/// Handles JS/TS imports, Python imports, and re-exports.
pub fn extract_imports(tree: &ZeroCopyParseTree) -> Vec<(String, String, Option<String>, usize)> {
    let mut results = Vec::new();
    let root = tree.ts_tree().root_node();
    collect_imports(root, tree.source_code(), tree.language(), &mut results);
    results
}

fn collect_imports(
    node: tree_sitter::Node,
    source: &str,
    language: crate::core::Language,
    results: &mut Vec<(String, String, Option<String>, usize)>,
) {
    let kind = node.kind();
    let line = node.start_position().row + 1;

    match language {
        crate::core::Language::JavaScript | crate::core::Language::TypeScript => {
            match kind {
                // import { X, Y as Z } from './path'
                "import_statement" => {
                    let source_path = extract_import_source(node, source);
                    if let Some(ref path) = source_path {
                        extract_js_import_names(node, source, path, line, results);
                    }
                }
                // export { X } from './path' or export * from './path'
                "export_statement" => {
                    // Only re-exports (with source)
                    let source_path = extract_import_source(node, source);
                    if let Some(ref path) = source_path {
                        // Check for "export *" (namespace re-export)
                        let text = &source[node.byte_range()];
                        if text.contains("* from") || text.contains("*from") {
                            results.push(("*".to_string(), path.clone(), None, line));
                        } else {
                            extract_js_import_names(node, source, path, line, results);
                        }
                    }
                }
                _ => {}
            }
        }
        crate::core::Language::Python => {
            // from module import name [as alias]
            if kind == "import_from_statement" {
                let module_name = node
                    .child_by_field_name("module_name")
                    .map(|n| source[n.byte_range()].to_string());
                if let Some(ref module) = module_name {
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
                            let name = if child.kind() == "aliased_import" {
                                child
                                    .child_by_field_name("name")
                                    .map(|n| source[n.byte_range()].to_string())
                            } else {
                                Some(source[child.byte_range()].to_string())
                            };
                            let alias = if child.kind() == "aliased_import" {
                                child
                                    .child_by_field_name("alias")
                                    .map(|n| source[n.byte_range()].to_string())
                            } else {
                                None
                            };
                            if let Some(n) = name {
                                // Skip the module name itself (first dotted_name is the module)
                                if n != *module {
                                    results.push((n, module.clone(), alias, line));
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_imports(child, source, language, results);
    }
}

/// Extract the source/from path from an import/export statement.
fn extract_import_source(node: tree_sitter::Node, source: &str) -> Option<String> {
    // Look for "source" field or string child
    if let Some(src) = node.child_by_field_name("source") {
        let text = source[src.byte_range()].trim();
        let cleaned = text.trim_matches('"').trim_matches('\'');
        return Some(cleaned.to_string());
    }
    // Fallback: look for string node children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            let text = source[child.byte_range()].trim();
            let cleaned = text.trim_matches('"').trim_matches('\'');
            return Some(cleaned.to_string());
        }
    }
    None
}

/// Extract imported names from JS/TS import/export statements.
fn extract_js_import_names(
    node: tree_sitter::Node,
    source: &str,
    source_path: &str,
    line: usize,
    results: &mut Vec<(String, String, Option<String>, usize)>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "import_clause" | "named_imports" | "export_clause" => {
                extract_js_import_names(child, source, source_path, line, results);
            }
            "import_specifier" | "export_specifier" => {
                let name = child
                    .child_by_field_name("name")
                    .map(|n| source[n.byte_range()].to_string());
                let alias = child
                    .child_by_field_name("alias")
                    .map(|n| source[n.byte_range()].to_string());
                if let Some(n) = name {
                    results.push((n, source_path.to_string(), alias, line));
                }
            }
            "identifier" => {
                // default import: import Foo from './bar'
                let name = source[child.byte_range()].to_string();
                results.push((name, source_path.to_string(), None, line));
            }
            "namespace_import" => {
                // import * as Foo from './bar'
                results.push(("*".to_string(), source_path.to_string(), None, line));
            }
            _ => {}
        }
    }
}

/// Extract class hierarchy as (class_name, parent_classes, line).
///
/// Identifies class/interface definitions and their inheritance relationships
/// from the tree-sitter AST. Supports Python, JS/TS, Java, C#, Ruby, PHP, C++.
/// Rust is handled separately via `impl_trait:X` attributes.
pub fn extract_class_hierarchy(tree: &ZeroCopyParseTree) -> Vec<(String, Vec<String>, usize)> {
    let mut results = Vec::new();
    let root = tree.ts_tree().root_node();
    collect_class_hierarchy(root, tree.source_code(), tree.language(), &mut results);
    results
}

fn collect_class_hierarchy(
    node: tree_sitter::Node,
    source: &str,
    language: crate::core::Language,
    results: &mut Vec<(String, Vec<String>, usize)>,
) {
    let kind = node.kind();
    let line = node.start_position().row + 1;

    match language {
        crate::core::Language::Python => {
            if kind == "class_definition" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    let mut parents = Vec::new();
                    // superclasses are in argument_list or superclasses field
                    if let Some(superclasses) = node.child_by_field_name("superclasses") {
                        let mut cursor = superclasses.walk();
                        for child in superclasses.children(&mut cursor) {
                            let ck = child.kind();
                            if ck == "identifier" || ck == "attribute" || ck == "dotted_name" {
                                let parent_name = source[child.byte_range()].to_string();
                                if parent_name != "object" && parent_name != "ABC" {
                                    parents.push(parent_name);
                                }
                            }
                        }
                    }
                    if !parents.is_empty() {
                        results.push((class_name, parents, line));
                    }
                }
            }
        }
        crate::core::Language::JavaScript | crate::core::Language::TypeScript => {
            if kind == "class_declaration" || kind == "class" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    let mut parents = Vec::new();
                    // Check for class_heritage / extends_clause
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        let ck = child.kind();
                        if ck == "class_heritage" || ck == "extends_clause" {
                            extract_heritage_names(child, source, &mut parents);
                        }
                        // TS: implements_clause
                        if ck == "implements_clause" {
                            extract_heritage_names(child, source, &mut parents);
                        }
                    }
                    if !parents.is_empty() {
                        results.push((class_name, parents, line));
                    }
                }
            }
        }
        crate::core::Language::Java | crate::core::Language::Kotlin => {
            if kind == "class_declaration" || kind == "interface_declaration" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    let mut parents = Vec::new();
                    // Java: superclass field, interfaces field
                    if let Some(super_node) = node.child_by_field_name("superclass") {
                        let parent_name = extract_type_name(super_node, source);
                        if !parent_name.is_empty() {
                            parents.push(parent_name);
                        }
                    }
                    // interfaces field or super_interfaces
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        let ck = child.kind();
                        if ck == "super_interfaces" || ck == "extends_interfaces" {
                            extract_type_list_names(child, source, &mut parents);
                        }
                    }
                    if !parents.is_empty() {
                        results.push((class_name, parents, line));
                    }
                }
            }
        }
        crate::core::Language::CSharp => {
            if kind == "class_declaration"
                || kind == "interface_declaration"
                || kind == "struct_declaration"
            {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    let mut parents = Vec::new();
                    // C#: base_list
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "base_list" {
                            extract_type_list_names(child, source, &mut parents);
                        }
                    }
                    if !parents.is_empty() {
                        results.push((class_name, parents, line));
                    }
                }
            }
        }
        crate::core::Language::Ruby => {
            if kind == "class" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    let mut parents = Vec::new();
                    if let Some(super_node) = node.child_by_field_name("superclass") {
                        let parent_name = source[super_node.byte_range()].to_string();
                        if !parent_name.is_empty() {
                            parents.push(parent_name);
                        }
                    }
                    if !parents.is_empty() {
                        results.push((class_name, parents, line));
                    }
                }
            }
        }
        crate::core::Language::PHP => {
            if kind == "class_declaration" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    let mut parents = Vec::new();
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        let ck = child.kind();
                        if ck == "base_clause" {
                            extract_type_list_names(child, source, &mut parents);
                        }
                        if ck == "class_interface_clause" {
                            extract_type_list_names(child, source, &mut parents);
                        }
                    }
                    if !parents.is_empty() {
                        results.push((class_name, parents, line));
                    }
                }
            }
        }
        crate::core::Language::Cpp => {
            if kind == "class_specifier" || kind == "struct_specifier" {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let class_name = source[name_node.byte_range()].to_string();
                    let mut parents = Vec::new();
                    let mut cursor = node.walk();
                    for child in node.children(&mut cursor) {
                        if child.kind() == "base_class_clause" {
                            extract_type_list_names(child, source, &mut parents);
                        }
                    }
                    if !parents.is_empty() {
                        results.push((class_name, parents, line));
                    }
                }
            }
        }
        crate::core::Language::Rust => {
            // Extract trait implementations: impl Trait for Type → ClassRelation
            if kind == "impl_item" {
                if let Some(trait_name) = extract_impl_trait_name_ast(node, source) {
                    if let Some(type_name) = extract_impl_type_name(node, source) {
                        results.push((type_name, vec![trait_name], line));
                    }
                }
            }
            // Note: trait definitions are extracted as nodes by extract_functions
            // (trait_item is in is_class_def), so ClassHierarchy registers them as types.
        }
        // Go/others don't have class inheritance
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_class_hierarchy(child, source, language, results);
    }
}

/// Extract type names from a heritage/extends/implements clause.
fn extract_heritage_names(node: tree_sitter::Node, source: &str, parents: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let ck = child.kind();
        if ck == "identifier" || ck == "type_identifier" {
            let name = source[child.byte_range()].to_string();
            parents.push(name);
        } else if ck == "extends_clause"
            || ck == "implements_clause"
            || ck == "generic_type"
            || ck == "type_annotation"
        {
            // For generic_type, extract the base name
            if ck == "generic_type" {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = source[name_node.byte_range()].to_string();
                    parents.push(name);
                    continue;
                }
            }
            extract_heritage_names(child, source, parents);
        }
    }
}

/// Extract a type name from a type node (e.g. Java superclass).
fn extract_type_name(node: tree_sitter::Node, source: &str) -> String {
    let kind = node.kind();
    if kind == "type_identifier" || kind == "identifier" {
        return source[node.byte_range()].to_string();
    }
    // For generic types, extract the base name
    if let Some(name_node) = node.child_by_field_name("name") {
        return source[name_node.byte_range()].to_string();
    }
    // Recurse into children to find identifier
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let ck = child.kind();
        if ck == "type_identifier" || ck == "identifier" {
            return source[child.byte_range()].to_string();
        }
    }
    source[node.byte_range()].to_string()
}

/// Extract type names from a list (e.g. implements list, base_list).
fn extract_type_list_names(node: tree_sitter::Node, source: &str, parents: &mut Vec<String>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let ck = child.kind();
        if ck == "type_identifier" || ck == "identifier" || ck == "name" || ck == "qualified_name" {
            let name = source[child.byte_range()].to_string();
            // Take just the last component for qualified names
            let simple_name = name.rsplit('.').next().unwrap_or(&name);
            parents.push(simple_name.to_string());
        } else if ck == "generic_type" {
            if let Some(name_node) = child.child_by_field_name("name") {
                parents.push(source[name_node.byte_range()].to_string());
            } else {
                // Recurse
                extract_type_list_names(child, source, parents);
            }
        } else if ck != ","
            && ck != ":"
            && ck != "extends"
            && ck != "implements"
            && ck != "public"
            && ck != "private"
            && ck != "protected"
            && ck != "comment"
            && ck != "line_comment"
            && ck != "block_comment"
        {
            // Recurse into containers
            extract_type_list_names(child, source, parents);
        }
    }
}

/// Extract all symbol references as (line, name).
pub fn extract_symbol_refs(tree: &ZeroCopyParseTree) -> Vec<(usize, String)> {
    let mut results = Vec::new();
    let root = tree.ts_tree().root_node();
    collect_symbol_refs(root, tree.source_code(), &mut results);
    results
}

fn collect_symbol_refs(node: tree_sitter::Node, source: &str, results: &mut Vec<(usize, String)>) {
    if node.kind() == "identifier"
        && !node.parent().is_some_and(|p| {
            // Skip identifiers that are definitions (not references)
            matches!(
                p.kind(),
                "function_definition"
                    | "function_declaration"
                    | "class_definition"
                    | "class_declaration"
                    | "function_item"
                    | "struct_item"
                    | "method_definition"
            ) && p
                .child_by_field_name("name")
                .is_some_and(|n| n.id() == node.id())
        })
    {
        let name = source[node.byte_range()].to_string();
        if !name.is_empty() {
            results.push((node.start_position().row + 1, name));
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_symbol_refs(child, source, results);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parsers::parsers::PythonParser;

    #[test]
    fn test_extract_webapp_calls() {
        let parser = PythonParser::new().unwrap();
        let source = r#"
def main():
    app = create_app()
    app.run()

def create_app():
    db = connect_database()
    return {"db": db}

def connect_database():
    return {}

def handle_search(request):
    query = request.params["q"]
    db = connect_database()
    results = db.execute(query)
    return render_results(results)

def render_results(results):
    return str(results)

def unused():
    pass
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let funcs = extract_functions(&tree);
        let calls = extract_calls(&tree);

        assert_eq!(funcs.len(), 6);
        let callee_names: Vec<&str> = calls.iter().map(|(_, n)| n.as_str()).collect();
        assert!(callee_names.contains(&"create_app"));
        assert!(callee_names.contains(&"connect_database"));
        assert!(callee_names.contains(&"render_results"));
    }

    #[test]
    fn test_extract_java_calls() {
        let parser = crate::parsers::parsers::JavaParser::new().unwrap();
        let source = r#"
public class UserService {
    public String findUser(String userId) {
        String result = processQuery(userId);
        return result;
    }

    private String processQuery(String id) {
        return helperMethod(id);
    }

    private String helperMethod(String data) {
        return data;
    }
}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let calls = extract_calls(&tree);
        let callee_names: Vec<&str> = calls.iter().map(|(_, n)| n.as_str()).collect();
        assert!(
            callee_names.contains(&"processQuery"),
            "Java method_invocation must be extracted"
        );
        assert!(
            callee_names.contains(&"helperMethod"),
            "Java method_invocation must be extracted"
        );
    }

    #[test]
    fn test_extract_go_calls() {
        let parser = crate::parsers::parsers::GoParser::new().unwrap();
        let source = r#"
package main

func main() {
    result := helper()
    process(result)
}

func helper() string {
    return "ok"
}

func process(data string) {
    fmt.Println(data)
}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let calls = extract_calls(&tree);
        let callee_names: Vec<&str> = calls.iter().map(|(_, n)| n.as_str()).collect();
        assert!(callee_names.contains(&"helper"));
        assert!(callee_names.contains(&"process"));
    }

    #[test]
    fn test_extract_js_calls() {
        let parser = crate::parsers::parsers::JavaScriptParser::new().unwrap();
        let source = r#"
function main() {
    const result = helper();
    process(result);
}

function helper() {
    return "ok";
}

function process(data) {
    console.log(data);
}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let calls = extract_calls(&tree);
        let callee_names: Vec<&str> = calls.iter().map(|(_, n)| n.as_str()).collect();
        assert!(callee_names.contains(&"helper"));
        assert!(callee_names.contains(&"process"));
    }

    #[test]
    fn test_extract_ruby_calls() {
        let parser = crate::parsers::parsers::RubyParser::new().unwrap();
        let source = r#"
def main
  result = helper()
  process(result)
end

def helper
  "ok"
end

def process(data)
  puts data
end
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let calls = extract_calls(&tree);
        let callee_names: Vec<&str> = calls.iter().map(|(_, n)| n.as_str()).collect();
        assert!(callee_names.contains(&"helper"));
        assert!(callee_names.contains(&"process"));
    }

    #[test]
    fn test_extract_python_functions() {
        let parser = PythonParser::new().unwrap();
        let source = r#"
def hello():
    pass

def _private():
    pass

class MyClass:
    def method(self):
        pass
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let funcs = extract_functions(&tree);

        let names: Vec<&str> = funcs.iter().map(|(n, _, _, _)| n.as_str()).collect();
        assert!(names.contains(&"hello"));
        assert!(names.contains(&"_private"));
        assert!(names.contains(&"MyClass"));
    }

    #[test]
    fn test_extract_python_calls() {
        let parser = PythonParser::new().unwrap();
        let source = r#"
def main():
    hello()
    world()
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let calls = extract_calls(&tree);

        let callee_names: Vec<&str> = calls.iter().map(|(_, n)| n.as_str()).collect();
        assert!(callee_names.contains(&"hello"));
        assert!(callee_names.contains(&"world"));
    }

    #[test]
    fn test_extract_rust_functions() {
        let parser = crate::parsers::parsers::RustParser::new().unwrap();
        let source = r#"
pub fn public_fn() {}
fn private_fn() {}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let funcs = extract_functions(&tree);

        assert_eq!(funcs.len(), 2);
        let public_fn = funcs.iter().find(|(n, _, _, _)| n == "public_fn").unwrap();
        assert!(public_fn.3); // is_public
        let private_fn = funcs.iter().find(|(n, _, _, _)| n == "private_fn").unwrap();
        assert!(!private_fn.3); // not public
    }

    #[test]
    fn test_extract_rust_attributes() {
        let parser = crate::parsers::parsers::RustParser::new().unwrap();
        let source = r#"
#[test]
fn test_something() {}

#[tokio::test]
async fn test_async() {}

fn regular_fn() {}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let attrs = super::extract_attributes(&tree);

        let test_something = attrs.iter().find(|(n, _, _)| n == "test_something");
        assert!(test_something.is_some(), "Should find test_something attrs");
        assert!(test_something.unwrap().2.contains(&"test".to_string()));

        let test_async = attrs.iter().find(|(n, _, _)| n == "test_async");
        assert!(test_async.is_some(), "Should find test_async attrs");
        assert!(test_async.unwrap().2.contains(&"test".to_string()));

        let regular = attrs.iter().find(|(n, _, _)| n == "regular_fn");
        assert!(regular.is_none(), "regular_fn should have no attributes");
    }

    #[test]
    fn test_extract_rust_serde_attributes() {
        let parser = crate::parsers::parsers::RustParser::new().unwrap();
        let source = r#"
#[serde(default = "default_page_size")]
fn default_page_size() -> usize { 10 }

#[serde(serialize_with = "serialize_date")]
fn serialize_date() {}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let attrs = super::extract_attributes(&tree);

        let page_size = attrs.iter().find(|(n, _, _)| n == "default_page_size");
        assert!(page_size.is_some(), "Should find default_page_size attrs");
        assert!(
            page_size
                .unwrap()
                .2
                .contains(&"serde_default:default_page_size".to_string()),
            "Should have serde_default attr, got: {:?}",
            page_size.unwrap().2
        );

        let ser = attrs.iter().find(|(n, _, _)| n == "serialize_date");
        assert!(ser.is_some());
        assert!(ser
            .unwrap()
            .2
            .contains(&"serde_serialize_with:serialize_date".to_string()));
    }

    #[test]
    fn test_extract_rust_impl_trait() {
        let parser = crate::parsers::parsers::RustParser::new().unwrap();
        let source = r#"
struct MyType;
impl From<String> for MyType {
    fn from(s: String) -> Self {
        MyType
    }
}

impl Default for MyType {
    fn default() -> Self {
        MyType
    }
}

impl MyType {
    fn new() -> Self {
        MyType
    }
}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let attrs = super::extract_attributes(&tree);

        let from_fn = attrs.iter().find(|(n, _, _)| n == "from");
        assert!(from_fn.is_some(), "Should find from() attrs");
        assert!(
            from_fn.unwrap().2.contains(&"impl_trait:From".to_string()),
            "from() should have impl_trait:From, got: {:?}",
            from_fn.unwrap().2
        );

        let default_fn = attrs.iter().find(|(n, _, _)| n == "default");
        assert!(default_fn.is_some());
        assert!(default_fn
            .unwrap()
            .2
            .contains(&"impl_trait:Default".to_string()));

        // new() is in a plain impl, should NOT have impl_trait
        let new_fn = attrs.iter().find(|(n, _, _)| n == "new");
        assert!(new_fn.is_none(), "new() in plain impl should have no attrs");
    }

    #[test]
    fn test_extract_python_decorators() {
        let parser = PythonParser::new().unwrap();
        let source = r#"
@pytest.mark.parametrize("x", [1, 2])
def test_something(x):
    pass

@app.route("/api/users")
def get_users():
    pass

def regular():
    pass
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let attrs = super::extract_attributes(&tree);

        let test_fn = attrs.iter().find(|(n, _, _)| n == "test_something");
        assert!(test_fn.is_some(), "Should find test_something attrs");
        assert!(test_fn.unwrap().2.contains(&"test".to_string()));

        let route_fn = attrs.iter().find(|(n, _, _)| n == "get_users");
        assert!(route_fn.is_some(), "Should find get_users attrs");
        assert!(route_fn.unwrap().2.contains(&"route".to_string()));
    }

    #[test]
    fn test_extract_js_imports() {
        let parser = crate::parsers::parsers::JavaScriptParser::new().unwrap();
        let source = r#"
import { foo, bar as baz } from './module';
import defaultExport from './other';

function main() {
    foo();
    baz();
    defaultExport();
}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let imports = super::extract_imports(&tree);

        let import_names: Vec<&str> = imports.iter().map(|(n, _, _, _)| n.as_str()).collect();
        assert!(
            import_names.contains(&"foo"),
            "Should extract 'foo' import, got: {:?}",
            import_names
        );
        assert!(
            import_names.contains(&"bar"),
            "Should extract 'bar' import (aliased as baz), got: {:?}",
            import_names
        );

        // Check alias
        let bar_import = imports.iter().find(|(n, _, _, _)| n == "bar");
        assert!(bar_import.is_some());
        assert_eq!(bar_import.unwrap().2, Some("baz".to_string()));

        // Check source paths
        let foo_import = imports.iter().find(|(n, _, _, _)| n == "foo");
        assert_eq!(foo_import.unwrap().1, "./module");
    }

    #[test]
    fn test_extract_ts_reexports() {
        let parser = crate::parsers::parsers::TypeScriptParser::new().unwrap();
        let source = r#"
export { MyComponent } from './components/MyComponent';
export * from './utils';
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let imports = super::extract_imports(&tree);

        let import_names: Vec<&str> = imports.iter().map(|(n, _, _, _)| n.as_str()).collect();
        assert!(
            import_names.contains(&"MyComponent"),
            "Should extract re-exported 'MyComponent', got: {:?}",
            import_names
        );
        assert!(
            import_names.contains(&"*"),
            "Should extract wildcard re-export, got: {:?}",
            import_names
        );
    }

    #[test]
    fn test_extract_python_imports() {
        let parser = PythonParser::new().unwrap();
        let source = r#"
from os.path import join, exists
from collections import defaultdict as dd
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let imports = super::extract_imports(&tree);

        let import_names: Vec<&str> = imports.iter().map(|(n, _, _, _)| n.as_str()).collect();
        assert!(
            import_names.contains(&"join"),
            "Should extract 'join' import, got: {:?}",
            import_names
        );
        assert!(
            import_names.contains(&"exists"),
            "Should extract 'exists' import, got: {:?}",
            import_names
        );
    }

    #[test]
    fn test_extract_ruby_auth_functions() {
        let parser = crate::parsers::parsers::RubyParser::new().unwrap();
        let source =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/auth.rb"))
                .unwrap();
        let tree = parser.parse_to_tree(&source).unwrap();
        let funcs = extract_functions(&tree);
        let names: Vec<&str> = funcs.iter().map(|(n, _, _, _)| n.as_str()).collect();
        eprintln!("Ruby auth.rb extracted functions: {:?}", names);
        // Top-level functions
        assert!(
            names.contains(&"start_app"),
            "Missing start_app, got: {:?}",
            names
        );
        assert!(
            names.contains(&"old_login_handler"),
            "Missing old_login_handler"
        );
        assert!(
            names.contains(&"format_currency"),
            "Missing format_currency"
        );
        assert!(names.contains(&"format_money"), "Missing format_money");
        assert!(
            names.contains(&"validate_password"),
            "Missing validate_password"
        );
        assert!(
            names.contains(&"check_password_strength"),
            "Missing check_password_strength"
        );
        // Class methods
        assert!(names.contains(&"initialize"), "Missing initialize");
        assert!(names.contains(&"get"), "Missing get");
    }

    #[test]
    fn test_extract_java_userservice_functions() {
        let parser = crate::parsers::parsers::JavaParser::new().unwrap();
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/UserService.java"
        ))
        .unwrap();
        let tree = parser.parse_to_tree(&source).unwrap();
        let funcs = extract_functions(&tree);
        let names: Vec<&str> = funcs.iter().map(|(n, _, _, _)| n.as_str()).collect();
        eprintln!("Java UserService.java extracted functions: {:?}", names);
        // All methods
        assert!(
            names.contains(&"findUser"),
            "Missing findUser, got: {:?}",
            names
        );
        assert!(names.contains(&"formatUserName"), "Missing formatUserName");
        assert!(
            names.contains(&"formatDisplayName"),
            "Missing formatDisplayName"
        );
        assert!(names.contains(&"migrateUsers"), "Missing migrateUsers");
        assert!(names.contains(&"exportUsers"), "Missing exportUsers");
        // Inner class methods
        assert!(names.contains(&"authenticate"), "Missing authenticate");
        assert!(names.contains(&"generateToken"), "Missing generateToken");
    }

    // =========================================================================
    // Class hierarchy extraction tests
    // =========================================================================

    #[test]
    fn test_extract_python_class_hierarchy() {
        let parser = PythonParser::new().unwrap();
        let source = r#"
class Animal:
    def speak(self):
        pass

class Dog(Animal):
    def speak(self):
        return "woof"

class Cat(Animal):
    def speak(self):
        return "meow"

class GuideDog(Dog):
    def guide(self):
        pass
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let hierarchy = extract_class_hierarchy(&tree);

        // Dog extends Animal
        let dog = hierarchy.iter().find(|(name, _, _)| name == "Dog");
        assert!(
            dog.is_some(),
            "Should find Dog in hierarchy, got: {:?}",
            hierarchy
        );
        assert!(
            dog.unwrap().1.contains(&"Animal".to_string()),
            "Dog should extend Animal"
        );

        // Cat extends Animal
        let cat = hierarchy.iter().find(|(name, _, _)| name == "Cat");
        assert!(cat.is_some(), "Should find Cat in hierarchy");
        assert!(cat.unwrap().1.contains(&"Animal".to_string()));

        // GuideDog extends Dog
        let guide_dog = hierarchy.iter().find(|(name, _, _)| name == "GuideDog");
        assert!(guide_dog.is_some(), "Should find GuideDog in hierarchy");
        assert!(guide_dog.unwrap().1.contains(&"Dog".to_string()));

        // Animal has no parents (not in results since parents is empty)
        let animal = hierarchy.iter().find(|(name, _, _)| name == "Animal");
        assert!(
            animal.is_none(),
            "Animal has no parents, should not be in results"
        );
    }

    #[test]
    fn test_extract_ts_class_hierarchy() {
        let parser = crate::parsers::parsers::TypeScriptParser::new().unwrap();
        let source = r#"
class BaseService {
    init() {}
}

class UserService extends BaseService {
    findUser(id: string) {}
}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let hierarchy = extract_class_hierarchy(&tree);

        let user_svc = hierarchy.iter().find(|(name, _, _)| name == "UserService");
        assert!(
            user_svc.is_some(),
            "Should find UserService, got: {:?}",
            hierarchy
        );
        assert!(
            user_svc.unwrap().1.contains(&"BaseService".to_string()),
            "UserService should extend BaseService"
        );
    }

    #[test]
    fn test_extract_java_class_hierarchy() {
        let parser = crate::parsers::parsers::JavaParser::new().unwrap();
        let source = r#"
public class Animal {
    public void speak() {}
}

public class Dog extends Animal {
    public void speak() { System.out.println("woof"); }
}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let hierarchy = extract_class_hierarchy(&tree);

        let dog = hierarchy.iter().find(|(name, _, _)| name == "Dog");
        assert!(
            dog.is_some(),
            "Should find Dog in Java hierarchy, got: {:?}",
            hierarchy
        );
        assert!(
            dog.unwrap().1.contains(&"Animal".to_string()),
            "Dog should extend Animal, got: {:?}",
            dog.unwrap().1
        );
    }

    #[test]
    fn test_extract_class_hierarchy_no_inheritance() {
        let parser = PythonParser::new().unwrap();
        let source = r#"
class Standalone:
    def method(self):
        pass
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let hierarchy = extract_class_hierarchy(&tree);

        // Standalone class with no parents should not appear
        assert!(
            hierarchy.is_empty(),
            "Class with no parents should not be in hierarchy, got: {:?}",
            hierarchy
        );
    }

    #[test]
    fn test_extract_rust_serde_field_attributes() {
        let parser = crate::parsers::parsers::RustParser::new().unwrap();
        let source = r#"
fn default_port() -> u16 { 8080 }
fn default_host() -> String { "localhost".to_string() }

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    pub name: String,
}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let attrs = super::extract_attributes(&tree);

        // The Config struct should have serde_default attributes from its fields
        let config_attrs = attrs.iter().find(|(n, _, _)| n == "Config");
        assert!(
            config_attrs.is_some(),
            "Should find Config struct attrs. All attrs: {:?}",
            attrs
        );
        let config_attr_list = &config_attrs.unwrap().2;
        assert!(
            config_attr_list.contains(&"serde_default:default_port".to_string()),
            "Config should have serde_default:default_port, got: {:?}",
            config_attr_list
        );
        assert!(
            config_attr_list.contains(&"serde_default:default_host".to_string()),
            "Config should have serde_default:default_host, got: {:?}",
            config_attr_list
        );
    }

    #[test]
    fn test_extract_rust_cfg_test_module_propagation() {
        let parser = crate::parsers::parsers::RustParser::new().unwrap();
        let source = r#"
fn regular_fn() {}

#[cfg(test)]
mod tests {
    fn test_helper() {}

    #[test]
    fn test_something() {}
}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let attrs = super::extract_attributes(&tree);

        // test_helper inside #[cfg(test)] mod should get cfg_test attribute
        let test_helper = attrs.iter().find(|(n, _, _)| n == "test_helper");
        assert!(
            test_helper.is_some(),
            "Should find test_helper attrs, got: {:?}",
            attrs
        );
        assert!(
            test_helper.unwrap().2.contains(&"cfg_test".to_string()),
            "test_helper should have cfg_test attribute, got: {:?}",
            test_helper.unwrap().2
        );

        // test_something should have both test and cfg_test
        let test_something = attrs.iter().find(|(n, _, _)| n == "test_something");
        assert!(test_something.is_some());
        assert!(test_something.unwrap().2.contains(&"test".to_string()));
        assert!(test_something.unwrap().2.contains(&"cfg_test".to_string()));

        // regular_fn should NOT have cfg_test
        let regular = attrs.iter().find(|(n, _, _)| n == "regular_fn");
        assert!(regular.is_none(), "regular_fn should have no attributes");
    }

    #[test]
    fn test_extract_rust_impl_trait_multiline() {
        let parser = crate::parsers::parsers::RustParser::new().unwrap();
        // Multi-line impl signature where "for" is on the second line
        let source = r#"
impl<T: Clone + Send + Sync>
    From<Vec<T>>
    for MyCollection<T>
{
    fn from(items: Vec<T>) -> Self {
        MyCollection { items }
    }
}
"#;
        let tree = parser.parse_to_tree(source).unwrap();
        let attrs = super::extract_attributes(&tree);

        let from_method = attrs.iter().find(|(n, _, _)| n == "from");
        assert!(
            from_method.is_some(),
            "Should find 'from' method, got: {:?}",
            attrs
        );
        assert!(
            from_method
                .unwrap()
                .2
                .contains(&"impl_trait:From".to_string()),
            "from() should have impl_trait:From attribute, got: {:?}",
            from_method.unwrap().2
        );
    }
}
