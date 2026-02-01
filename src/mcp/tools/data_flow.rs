//! MCP tool: `fossil_get_data_flow`
//!
//! Returns def-use chain information for a function. Extracts variable
//! definitions and uses from the function's parsed source and CFG.

use std::collections::HashMap;

use serde_json::{json, Value};

use crate::mcp::context::AnalysisContext;

/// Get data flow (def-use) information for a function.
///
/// # Arguments
/// - `function_name` (string, required): Name of the function to analyze.
/// - `variable` (string, optional): Filter to a specific variable name.
pub fn execute(args: &HashMap<String, Value>, context: &AnalysisContext) -> Result<Value, String> {
    let function_name = args
        .get("function_name")
        .and_then(|v| v.as_str())
        .ok_or("Missing required argument 'function_name'")?;

    let variable_filter = args.get("variable").and_then(|v| v.as_str());

    let graph = &context.graph;

    // Find the function node in the code graph.
    let node_idx = graph
        .find_node_by_name(function_name)
        .ok_or_else(|| format!("Function '{}' not found in the code graph", function_name))?;

    let node = graph
        .get_node(node_idx)
        .ok_or("Internal error: node not found")?;

    // Try to find the function's source from parsed files.
    let mut function_source = None;
    for pf in &context.parsed_files {
        if pf.path == node.location.file {
            // Extract the function's lines from the source.
            let lines: Vec<&str> = pf.source.lines().collect();
            let start = node.location.line_start.saturating_sub(1);
            let end = node.location.line_end.min(lines.len());
            if start < end {
                function_source = Some(lines[start..end].join("\n"));
            }
            break;
        }
    }

    // Extract variable information from the source.
    let mut variables: HashMap<String, VariableInfo> = HashMap::new();

    if let Some(ref source) = function_source {
        // Simple text-based extraction of definitions and uses.
        for (line_idx, line) in source.lines().enumerate() {
            let line_num = node.location.line_start + line_idx;
            let trimmed = line.trim();

            // Skip comments and empty lines.
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed.starts_with("//")
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
            {
                continue;
            }

            // Look for assignment patterns (simple heuristic).
            if let Some(eq_pos) = find_simple_assignment(trimmed) {
                let lhs = trimmed[..eq_pos].trim();
                // Strip declaration keywords.
                let var_name = strip_declaration_keywords(lhs);
                if is_valid_identifier(var_name) {
                    let entry =
                        variables
                            .entry(var_name.to_string())
                            .or_insert_with(|| VariableInfo {
                                name: var_name.to_string(),
                                defs: Vec::new(),
                                uses: Vec::new(),
                            });
                    entry.defs.push(json!({
                        "line": line_num,
                        "snippet": trimmed,
                    }));

                    // The RHS may contain uses of other variables.
                    let rhs = trimmed[eq_pos + 1..].trim();
                    extract_identifier_uses(rhs, line_num, &mut variables, Some(var_name));
                }
            } else {
                // Line without assignment: extract identifier uses.
                extract_identifier_uses(trimmed, line_num, &mut variables, None);
            }
        }
    }

    // Also include callers/callees as data flow context.
    let callers: Vec<String> = graph
        .callers_of(node_idx)
        .filter_map(|idx| graph.get_node(idx))
        .map(|n| n.full_name.clone())
        .collect();

    let callees: Vec<String> = graph
        .calls_from(node_idx)
        .filter_map(|idx| graph.get_node(idx))
        .map(|n| n.full_name.clone())
        .collect();

    // Apply variable filter if specified.
    let variable_list: Vec<Value> = variables
        .values()
        .filter(|v| {
            if let Some(filter) = variable_filter {
                v.name == filter
            } else {
                true
            }
        })
        .map(|v| {
            json!({
                "name": v.name,
                "defs": v.defs,
                "uses": v.uses,
            })
        })
        .collect();

    let result = json!({
        "function": function_name,
        "full_name": node.full_name,
        "file": node.location.file,
        "line_start": node.location.line_start,
        "line_end": node.location.line_end,
        "variables": variable_list,
        "callers": callers,
        "callees": callees,
        "has_source": function_source.is_some(),
    });

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
        }]
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct VariableInfo {
    name: String,
    defs: Vec<Value>,
    uses: Vec<Value>,
}

/// Find a simple assignment operator (`=`) that is not `==`, `!=`, `<=`, `>=`, `=>`.
fn find_simple_assignment(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        if bytes[i] == b'=' {
            let prev = if i > 0 { bytes[i - 1] } else { 0 };
            let next = if i + 1 < len { bytes[i + 1] } else { 0 };

            // Skip ==, !=, <=, >=, =>
            if prev == b'!' || prev == b'<' || prev == b'>' || prev == b'=' {
                i += 1;
                continue;
            }
            if next == b'=' || next == b'>' {
                i += 2;
                continue;
            }

            return Some(i);
        }
        // Skip string literals.
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            while i < len && bytes[i] != quote {
                if bytes[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
        }
        i += 1;
    }

    None
}

/// Strip common declaration keywords (let, const, var, mut, etc.) from a
/// variable name candidate.
fn strip_declaration_keywords(lhs: &str) -> &str {
    let trimmed = lhs.trim();
    for prefix in &["let mut ", "let ", "const ", "var ", "mut "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.trim();
        }
    }
    // Java/Go style: `Type varname` -- take last token
    let tokens: Vec<&str> = trimmed.split_whitespace().collect();
    if tokens.len() >= 2 {
        tokens.last().unwrap_or(&trimmed)
    } else {
        trimmed
    }
}

/// Check if a string looks like a valid identifier.
fn is_valid_identifier(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let first = s.chars().next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return false;
    }
    s.chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
}

/// Extract identifier-like tokens from an expression and record them as uses.
fn extract_identifier_uses(
    expr: &str,
    line_num: usize,
    variables: &mut HashMap<String, VariableInfo>,
    skip_name: Option<&str>,
) {
    // Split on non-identifier characters.
    for token in expr.split(|c: char| !c.is_alphanumeric() && c != '_') {
        let token = token.trim();
        if token.is_empty() || !is_valid_identifier(token) {
            continue;
        }
        // Skip keywords and the LHS variable itself.
        if is_keyword(token) {
            continue;
        }
        if let Some(skip) = skip_name {
            if token == skip {
                continue;
            }
        }
        let entry = variables
            .entry(token.to_string())
            .or_insert_with(|| VariableInfo {
                name: token.to_string(),
                defs: Vec::new(),
                uses: Vec::new(),
            });
        entry.uses.push(json!({
            "line": line_num,
        }));
    }
}

fn is_keyword(token: &str) -> bool {
    matches!(
        token,
        "if" | "else"
            | "elif"
            | "for"
            | "while"
            | "return"
            | "def"
            | "class"
            | "fn"
            | "func"
            | "function"
            | "import"
            | "from"
            | "let"
            | "const"
            | "var"
            | "mut"
            | "pub"
            | "self"
            | "this"
            | "true"
            | "false"
            | "True"
            | "False"
            | "None"
            | "null"
            | "undefined"
            | "new"
            | "in"
            | "not"
            | "and"
            | "or"
            | "is"
            | "as"
            | "with"
            | "try"
            | "except"
            | "catch"
            | "finally"
            | "throw"
            | "raise"
            | "break"
            | "continue"
            | "pass"
            | "async"
            | "await"
            | "yield"
            | "static"
            | "void"
            | "int"
            | "float"
            | "str"
            | "string"
            | "bool"
            | "boolean"
    )
}
