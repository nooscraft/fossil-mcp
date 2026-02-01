//! MCP tool: `fossil_explain_finding`
//!
//! Returns rich context about a finding at a specific file and line location.
//! Handles both security findings (via rule database) and dead code findings
//! (via the analysis context's code graph).

use std::collections::HashMap;
use std::fs;

use serde_json::{json, Value};

use crate::mcp::context::AnalysisContext;

/// Explain a finding at a specific source location.
///
/// # Arguments
/// - `file` (string, required): Path to the source file.
/// - `line` (number, required): Line number of the finding (1-indexed).
/// - `rule_id` (string, optional): Specific rule ID to look up. If not
///   provided, the tool returns the code context and any matching rules.
pub fn execute(
    args: &HashMap<String, Value>,
    context: Option<&AnalysisContext>,
) -> Result<Value, String> {
    let file = args
        .get("file")
        .and_then(|v| v.as_str())
        .ok_or("Missing required argument 'file'")?;

    let line = args
        .get("line")
        .and_then(|v| v.as_u64())
        .ok_or("Missing required argument 'line'")? as usize;

    let rule_id = args.get("rule_id").and_then(|v| v.as_str());

    // Read source file and extract context lines.
    let source =
        fs::read_to_string(file).map_err(|e| format!("Failed to read file '{}': {}", file, e))?;

    let lines: Vec<&str> = source.lines().collect();
    let total_lines = lines.len();

    if line == 0 || line > total_lines {
        return Err(format!(
            "Line {} is out of range (file has {} lines)",
            line, total_lines
        ));
    }

    // Extract context: +/- 10 lines around the target line.
    let context_radius = 10;
    let start = line.saturating_sub(context_radius).max(1);
    let end = (line + context_radius).min(total_lines);

    let mut context_lines = Vec::new();
    for i in start..=end {
        let prefix = if i == line { ">>> " } else { "    " };
        context_lines.push(format!("{}{:>4} | {}", prefix, i, lines[i - 1]));
    }
    let code_context = context_lines.join("\n");

    // Look up rule information from the rule database.
    let db = crate::rules::RuleDatabase::with_defaults();

    let rule_info = if let Some(id) = rule_id {
        db.get_rule(id).map(|rule| {
            json!({
                "id": rule.id,
                "title": rule.name,
                "description": rule.description,
                "severity": format!("{}", rule.severity),
                "cwe": rule.cwe,
                "owasp": rule.owasp,
                "confidence": format!("{}", rule.confidence),
                "tags": rule.tags,
            })
        })
    } else {
        None
    };

    let suggested_fix = if let Some(id) = rule_id {
        db.get_rule(id).and_then(|r| r.fix_suggestion.clone())
    } else {
        None
    };

    // If no specific rule was requested, scan the line for potential matches.
    let potential_rules: Vec<Value> = if rule_id.is_none() {
        db.all_rules()
            .iter()
            .filter(|r| r.enabled)
            .filter(|r| {
                let line_content = lines[line - 1];
                match r.id.as_str() {
                    "SEC001" => line_content.contains("execute") || line_content.contains("query"),
                    "SEC002" => {
                        line_content.contains("innerHTML")
                            || line_content.contains("document.write")
                    }
                    "SEC003" => line_content.contains("os.system") || line_content.contains("exec"),
                    "SEC004" => {
                        line_content.contains("password")
                            || line_content.contains("secret")
                            || line_content.contains("api_key")
                    }
                    "SEC005" => line_content.contains("open") && line_content.contains("request"),
                    "SEC006" => {
                        line_content.contains("pickle") || line_content.contains("yaml.unsafe_load")
                    }
                    "SEC007" => line_content.contains("md5") || line_content.contains("sha1"),
                    _ => false,
                }
            })
            .map(|r| {
                json!({
                    "id": r.id,
                    "title": r.name,
                    "severity": format!("{}", r.severity),
                    "description": r.description,
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    // Dead code analysis: if we have an analysis context, look up the node
    // at this file+line and provide dead code-specific reasoning.
    let dead_code_info = context.and_then(|ctx| explain_dead_code(ctx, file, line));

    // Determine the effective rule and suggested_fix: prefer security rule,
    // fall back to dead code analysis.
    let effective_rule = rule_info.or_else(|| {
        dead_code_info.as_ref().map(|dc| {
            json!({
                "id": "DEAD-CODE",
                "title": "Unreachable code",
                "description": dc.reason,
                "severity": "info",
                "confidence": dc.confidence,
                "tags": ["dead-code", "maintainability"],
            })
        })
    });

    let effective_fix =
        suggested_fix.or_else(|| dead_code_info.as_ref().map(|dc| dc.suggested_fix.clone()));

    let result = json!({
        "file": file,
        "line": line,
        "code_context": code_context,
        "rule": effective_rule,
        "suggested_fix": effective_fix,
        "potential_rules": potential_rules,
        "total_file_lines": total_lines,
        "dead_code_analysis": dead_code_info.as_ref().map(|dc| json!({
            "function_name": dc.function_name,
            "is_reachable": dc.is_reachable,
            "callers": dc.callers,
            "callees": dc.callees,
            "attributes": dc.attributes,
            "reason": dc.reason,
        })),
    });

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
        }]
    }))
}

/// Dead code explanation details.
struct DeadCodeInfo {
    function_name: String,
    is_reachable: bool,
    callers: Vec<String>,
    callees: Vec<String>,
    attributes: Vec<String>,
    reason: String,
    suggested_fix: String,
    confidence: String,
}

/// Look up the node at file+line in the graph and explain why it's dead.
fn explain_dead_code(ctx: &AnalysisContext, file: &str, line: usize) -> Option<DeadCodeInfo> {
    // Find the node where line is within the definition range.
    let target_node = ctx
        .graph
        .nodes()
        .filter(|(_, node)| {
            (node.location.file.ends_with(file)
                || file.ends_with(&node.location.file)
                || node.location.file == file)
                && node.location.line_start <= line
                && node.location.line_end >= line
        })
        // Prefer the most specific (smallest span) node
        .min_by_key(|(_, node)| node.location.line_end - node.location.line_start);

    // Fall back to any node starting at this line
    let found = target_node.or_else(|| {
        ctx.graph.nodes().find(|(_, node)| {
            (node.location.file.ends_with(file)
                || file.ends_with(&node.location.file)
                || node.location.file == file)
                && node.location.line_start == line
        })
    });

    // If no node found, return partial info with coverage gap explanation
    let (node_idx, node) = match found {
        Some(pair) => pair,
        None => {
            // Check if we have any nodes in this file at all
            let file_has_nodes = ctx.graph.nodes().any(|(_, n)| {
                n.location.file.ends_with(file)
                    || file.ends_with(&n.location.file)
                    || n.location.file == file
            });
            let reason = if file_has_nodes {
                format!(
                    "No function/class definition found at line {}. \
                     The line may be inside a function body, or the code may not have been \
                     recognized as a definition by the parser.",
                    line
                )
            } else {
                format!(
                    "No analysis data for file '{}'. The file may not have been included \
                     in the analysis scan, or its language may not be supported.",
                    file
                )
            };
            return Some(DeadCodeInfo {
                function_name: String::new(),
                is_reachable: false,
                callers: Vec::new(),
                callees: Vec::new(),
                attributes: Vec::new(),
                reason,
                suggested_fix: "Run a full analysis scan to ensure this file is included."
                    .to_string(),
                confidence: "unknown".to_string(),
            });
        }
    };

    // Suppress for module-level synthetic nodes
    if node.name.starts_with("<module:") {
        return None;
    }

    // Check reachability from entry points
    let entry_points: Vec<_> = ctx.graph.entry_points().iter().copied().collect();
    let is_reachable = entry_points.iter().any(|&ep| {
        // BFS from entry point to see if target is reachable
        let mut visited = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(ep);
        visited.insert(ep);
        while let Some(current) = queue.pop_front() {
            if current == node_idx {
                return true;
            }
            for neighbor in ctx.graph.calls_from(current) {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }
        false
    });

    // Collect callers
    let callers: Vec<String> = ctx
        .graph
        .callers_of(node_idx)
        .filter_map(|caller_idx| {
            ctx.graph.get_node(caller_idx).map(|n| {
                format!(
                    "{}() at {}:{}",
                    n.name, n.location.file, n.location.line_start
                )
            })
        })
        .collect();

    // Collect callees
    let callees: Vec<String> = ctx
        .graph
        .calls_from(node_idx)
        .filter_map(|callee_idx| {
            ctx.graph.get_node(callee_idx).map(|n| {
                format!(
                    "{}() at {}:{}",
                    n.name, n.location.file, n.location.line_start
                )
            })
        })
        .collect();

    let attributes = node.attributes.clone();

    // Build reason string
    let reason = if is_reachable {
        format!(
            "{}() is reachable from entry points and is NOT dead code.",
            node.name
        )
    } else if callers.is_empty() {
        format!(
            "{}() has zero callers and is not reachable from any entry point. \
             No other function in the project calls this function.",
            node.name
        )
    } else {
        let caller_names: Vec<&str> = callers.iter().map(|s| s.as_str()).collect();
        format!(
            "{}() has {} caller(s) ({}) but none of them are reachable from entry points. \
             The entire call chain is disconnected from the application's execution flow.",
            node.name,
            callers.len(),
            caller_names.join(", ")
        )
    };

    // Build suggested fix
    let has_trait_impl = attributes.iter().any(|a| a.starts_with("impl_trait:"));
    let has_extends = attributes
        .iter()
        .any(|a| a.starts_with("extends:") || a.starts_with("implements:"));
    let is_test = node.is_test || attributes.iter().any(|a| a == "test" || a == "cfg_test");

    let suggested_fix = if is_reachable {
        "This function is reachable and not dead code. No action needed.".to_string()
    } else if is_test {
        format!(
            "{}() appears to be a test function. It may be invoked by the test runner \
             and not directly from application code. Consider if this is a false positive.",
            node.name
        )
    } else if has_trait_impl {
        let trait_name = attributes
            .iter()
            .find(|a| a.starts_with("impl_trait:"))
            .map(|a| &a["impl_trait:".len()..])
            .unwrap_or("unknown");
        format!(
            "{}() implements trait '{}'. It may be called via dynamic dispatch (dyn {}) \
             or generic bounds. Verify it's not needed before removing.",
            node.name, trait_name, trait_name
        )
    } else if has_extends {
        format!(
            "{}() overrides a parent class method. It may be called via polymorphic dispatch. \
             Verify it's not needed before removing.",
            node.name
        )
    } else if callers.is_empty() {
        format!(
            "Remove {}() and its associated code. No callers reference this function.",
            node.name
        )
    } else {
        format!(
            "The callers of {}() are themselves dead. Consider removing the entire \
             dead call chain: {}",
            node.name,
            callers.join(", ")
        )
    };

    let confidence = if is_reachable {
        "not_dead"
    } else if has_trait_impl || has_extends || is_test {
        "low"
    } else if callers.is_empty() {
        "high"
    } else {
        "medium"
    };

    Some(DeadCodeInfo {
        function_name: node.name.clone(),
        is_reachable,
        callers,
        callees,
        attributes,
        reason,
        suggested_fix,
        confidence: confidence.to_string(),
    })
}
