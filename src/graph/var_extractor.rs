//! Variable definition/use extraction from tree-sitter nodes.
//!
//! Language-specific patterns for extracting assignments and reads.

use super::cfg::CfgNodeId;
use super::dataflow::{DefPoint, UsePoint, VarRef};
use crate::core::Language;

/// Extract definitions and uses from a tree-sitter node within a block.
pub fn extract_defs_and_uses(
    node: tree_sitter::Node<'_>,
    source: &str,
    language: Language,
    block_id: CfgNodeId,
    stmt_index: usize,
) -> (Vec<DefPoint>, Vec<UsePoint>) {
    let mut defs = Vec::new();
    let mut uses = Vec::new();

    extract_recursive(
        node, source, language, block_id, stmt_index, &mut defs, &mut uses,
    );

    (defs, uses)
}

fn extract_recursive(
    node: tree_sitter::Node<'_>,
    source: &str,
    language: Language,
    block_id: CfgNodeId,
    stmt_index: usize,
    defs: &mut Vec<DefPoint>,
    uses: &mut Vec<UsePoint>,
) {
    let kind = node.kind();
    let text = node
        .utf8_text(source.as_bytes())
        .unwrap_or_default()
        .to_string();

    match language {
        Language::Python => {
            extract_python(node, source, kind, &text, block_id, stmt_index, defs, uses);
        }
        Language::JavaScript | Language::TypeScript => {
            extract_js_ts(node, source, kind, &text, block_id, stmt_index, defs, uses);
        }
        Language::Java | Language::CSharp => {
            extract_java_csharp(node, source, kind, &text, block_id, stmt_index, defs, uses);
        }
        Language::Go => {
            extract_go(node, source, kind, &text, block_id, stmt_index, defs, uses);
        }
        Language::Rust => {
            extract_rust(node, source, kind, &text, block_id, stmt_index, defs, uses);
        }
        _ => {
            // Generic: treat assignments as defs, identifiers as uses
            extract_generic(node, source, kind, &text, block_id, stmt_index, defs, uses);
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_recursive(child, source, language, block_id, stmt_index, defs, uses);
    }
}

fn make_def(
    name: &str,
    node: tree_sitter::Node<'_>,
    block_id: CfgNodeId,
    stmt_index: usize,
) -> DefPoint {
    DefPoint {
        var: VarRef::new(name),
        block: block_id,
        stmt_index,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    }
}

fn make_use(
    name: &str,
    node: tree_sitter::Node<'_>,
    block_id: CfgNodeId,
    stmt_index: usize,
) -> UsePoint {
    UsePoint {
        var: VarRef::new(name),
        block: block_id,
        stmt_index,
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_python(
    node: tree_sitter::Node<'_>,
    source: &str,
    kind: &str,
    _text: &str,
    block_id: CfgNodeId,
    stmt_index: usize,
    defs: &mut Vec<DefPoint>,
    uses: &mut Vec<UsePoint>,
) {
    match kind {
        "assignment" => {
            // left = right: left is def, right contains uses
            if let Some(left) = node.child_by_field_name("left") {
                if let Ok(name) = left.utf8_text(source.as_bytes()) {
                    defs.push(make_def(name, left, block_id, stmt_index));
                }
            }
        }
        "augmented_assignment" => {
            // x += expr: x is both use and def
            if let Some(left) = node.child_by_field_name("left") {
                if let Ok(name) = left.utf8_text(source.as_bytes()) {
                    uses.push(make_use(name, left, block_id, stmt_index));
                    defs.push(make_def(name, left, block_id, stmt_index));
                }
            }
        }
        "identifier" => {
            // Check parent to determine if it's a use or def context
            if let Some(parent) = node.parent() {
                let parent_kind = parent.kind();
                if parent_kind != "assignment"
                    || parent.child_by_field_name("left").map(|n| n.id()) != Some(node.id())
                {
                    if let Ok(name) = node.utf8_text(source.as_bytes()) {
                        if !is_builtin(name) {
                            uses.push(make_use(name, node, block_id, stmt_index));
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_js_ts(
    node: tree_sitter::Node<'_>,
    source: &str,
    kind: &str,
    _text: &str,
    block_id: CfgNodeId,
    stmt_index: usize,
    defs: &mut Vec<DefPoint>,
    uses: &mut Vec<UsePoint>,
) {
    match kind {
        "variable_declarator" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    defs.push(make_def(name, name_node, block_id, stmt_index));
                }
            }
        }
        "assignment_expression" => {
            if let Some(left) = node.child_by_field_name("left") {
                if let Ok(name) = left.utf8_text(source.as_bytes()) {
                    defs.push(make_def(name, left, block_id, stmt_index));
                }
            }
        }
        "identifier" => {
            if let Some(parent) = node.parent() {
                let pk = parent.kind();
                if pk != "variable_declarator" && pk != "assignment_expression"
                    || parent.child_by_field_name("name").map(|n| n.id()) != Some(node.id())
                        && parent.child_by_field_name("left").map(|n| n.id()) != Some(node.id())
                {
                    if let Ok(name) = node.utf8_text(source.as_bytes()) {
                        uses.push(make_use(name, node, block_id, stmt_index));
                    }
                }
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_java_csharp(
    node: tree_sitter::Node<'_>,
    source: &str,
    kind: &str,
    _text: &str,
    block_id: CfgNodeId,
    stmt_index: usize,
    defs: &mut Vec<DefPoint>,
    uses: &mut Vec<UsePoint>,
) {
    match kind {
        "variable_declarator" | "local_variable_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                if let Ok(name) = name_node.utf8_text(source.as_bytes()) {
                    defs.push(make_def(name, name_node, block_id, stmt_index));
                }
            }
        }
        "assignment_expression" => {
            if let Some(left) = node.child_by_field_name("left") {
                if let Ok(name) = left.utf8_text(source.as_bytes()) {
                    defs.push(make_def(name, left, block_id, stmt_index));
                }
            }
        }
        "identifier" => {
            if let Ok(name) = node.utf8_text(source.as_bytes()) {
                uses.push(make_use(name, node, block_id, stmt_index));
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_go(
    node: tree_sitter::Node<'_>,
    source: &str,
    kind: &str,
    _text: &str,
    block_id: CfgNodeId,
    stmt_index: usize,
    defs: &mut Vec<DefPoint>,
    uses: &mut Vec<UsePoint>,
) {
    match kind {
        "short_var_declaration" | "var_declaration" => {
            if let Some(left) = node.child_by_field_name("left") {
                if let Ok(name) = left.utf8_text(source.as_bytes()) {
                    defs.push(make_def(name, left, block_id, stmt_index));
                }
            }
        }
        "assignment_statement" => {
            if let Some(left) = node.child_by_field_name("left") {
                if let Ok(name) = left.utf8_text(source.as_bytes()) {
                    defs.push(make_def(name, left, block_id, stmt_index));
                }
            }
        }
        "identifier" => {
            if let Ok(name) = node.utf8_text(source.as_bytes()) {
                uses.push(make_use(name, node, block_id, stmt_index));
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_rust(
    node: tree_sitter::Node<'_>,
    source: &str,
    kind: &str,
    _text: &str,
    block_id: CfgNodeId,
    stmt_index: usize,
    defs: &mut Vec<DefPoint>,
    uses: &mut Vec<UsePoint>,
) {
    match kind {
        "let_declaration" => {
            if let Some(pat) = node.child_by_field_name("pattern") {
                if let Ok(name) = pat.utf8_text(source.as_bytes()) {
                    defs.push(make_def(name, pat, block_id, stmt_index));
                }
            }
        }
        "assignment_expression" => {
            if let Some(left) = node.child_by_field_name("left") {
                if let Ok(name) = left.utf8_text(source.as_bytes()) {
                    defs.push(make_def(name, left, block_id, stmt_index));
                }
            }
        }
        "identifier" => {
            if let Ok(name) = node.utf8_text(source.as_bytes()) {
                uses.push(make_use(name, node, block_id, stmt_index));
            }
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn extract_generic(
    node: tree_sitter::Node<'_>,
    source: &str,
    kind: &str,
    _text: &str,
    block_id: CfgNodeId,
    stmt_index: usize,
    defs: &mut Vec<DefPoint>,
    uses: &mut Vec<UsePoint>,
) {
    if kind.contains("assignment") || kind.contains("declarator") || kind.contains("declaration") {
        if let Some(left) = node
            .child_by_field_name("left")
            .or(node.child_by_field_name("name"))
            .or(node.child_by_field_name("pattern"))
        {
            if let Ok(name) = left.utf8_text(source.as_bytes()) {
                defs.push(make_def(name, left, block_id, stmt_index));
            }
        }
    } else if kind == "identifier" {
        if let Ok(name) = node.utf8_text(source.as_bytes()) {
            uses.push(make_use(name, node, block_id, stmt_index));
        }
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "True"
            | "False"
            | "None"
            | "self"
            | "cls"
            | "print"
            | "len"
            | "range"
            | "int"
            | "str"
            | "float"
            | "list"
            | "dict"
            | "set"
            | "tuple"
            | "type"
            | "isinstance"
            | "super"
    )
}
