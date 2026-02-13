//! MCP tool: `fossil_detect_scaffolding`
//!
//! Detect AI-generated scaffolding artifacts in source code and file names.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use ignore::WalkBuilder;
use regex::Regex;
use serde_json::{json, Value};

/// Source file extensions to scan for code-level scaffolding patterns.
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "ts", "tsx", "jsx", "go", "java", "rb", "c", "cpp", "h", "hpp", "cs",
    "swift", "kt", "scala", "php", "lua", "sh", "bash", "zsh",
];

fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|ext| SOURCE_EXTENSIONS.contains(&ext))
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Shared regex patterns for phased/temporal/scaffold identifiers
// ---------------------------------------------------------------------------

/// Matches phased names like phase_1, phase1, phase-1, step_2, part_3 (case-insensitive).
const PHASED_PATTERN: &str = r"(?i)\b(phase|step|part)[_\-]?\d+\b";

/// Matches phased references in comments: Phase 2, Phase 2B, step-3, Part 1, etc.
/// Broader than `PHASED_PATTERN` — allows spaces and letter suffixes (identifiers can't have spaces).
const PHASED_COMMENT_PATTERN: &str = r"(?i)\b(phase|step|part)\s?[_\-]?\s?\d+[a-z]?\b";

/// Matches temporal names like week1, week_1, day1, sprint_1.
const TEMPORAL_PATTERN: &str = r"(?i)\b(week|day|sprint)[_\-]?\d+\b";

/// Matches scaffold-y names: scaffold, boilerplate, placeholder, stub, sample_, example_.
const SCAFFOLD_IDENT_PATTERN: &str =
    r"(?i)\b(scaffold|boilerplate|placeholder|stub|sample|example)\b";

/// Matches numbered suffixes: _v2, _v3, _new, _final, _draft.
const NUMBERED_SUFFIX_PATTERN: &str = r"(?i)_(v\d+|final|draft)\b";

// ---------------------------------------------------------------------------
// File/directory name patterns for temp-file detection
// ---------------------------------------------------------------------------

/// Prefix patterns for temp/backup files.
const TEMP_PREFIX_PATTERN: &str = r"(?i)^(temp_|tmp_|backup_|old_)";

/// Suffix patterns for temp/backup files.
const TEMP_SUFFIX_PATTERN: &str = r"(?i)(_bak|\.bak|_old|_backup|_copy|\.orig)$";

/// Scaffold-y file/directory names.
const SCAFFOLD_FILE_PATTERN: &str =
    r"(?i)^(scaffold[_\-]|boilerplate[_\-]|template_|stub_|placeholder[_\-]|sample_|example_)";

/// Numbered suffix patterns on file names.
const NUMBERED_FILE_SUFFIX_PATTERN: &str = r"(?i)_(v\d+|new|final|draft)\.";

// ---------------------------------------------------------------------------
// Source code patterns
// ---------------------------------------------------------------------------

/// TODO/FIXME/HACK/XXX/TEMP comment markers.
const TODO_PATTERN: &str = r"\b(TODO|FIXME|HACK|XXX|TEMP|TEMPORARY)\b";

/// Function/method definitions — captures the name for further regex checks.
/// Covers: def NAME, function NAME, fn NAME, func NAME, fun NAME, void/int/etc NAME(
const FUNC_DEF_PATTERN: &str = r"(?:(?:def|function|fn|func|fun)\s+(\w+)|(?:(?:public|private|protected|static|async|void|int|string|bool|float|double|var|let|const)\s+)+(\w+)\s*\()";

/// Placeholder body patterns (single-statement stubs).
const PLACEHOLDER_PATTERNS: &[&str] = &[
    r"^\s*pass\s*$",
    r"^\s*\.\.\.\s*$",
    r"^\s*unimplemented!\(\)\s*;?\s*$",
    r"^\s*todo!\(.*\)\s*;?\s*$",
    r#"^\s*throw\s+new\s+Error\s*\(\s*"not implemented"\s*\)\s*;?\s*$"#,
    r"^\s*raise\s+NotImplementedError\b",
];

/// Debug print patterns — bare console.log/print/println!/fmt.Println with string literals.
const DEBUG_PRINT_PATTERNS: &[&str] = &[
    r#"console\.log\(\s*["'](?:DEBUG|>>>|\*\*\*)"#,
    r#"(?<!\w)print\(\s*["'](?:DEBUG|>>>|\*\*\*)"#,
    r#"println!\(\s*["'](?:DEBUG|>>>|\*\*\*)"#,
    r#"fmt\.Println\(\s*["'](?:DEBUG|>>>|\*\*\*)"#,
];

// ---------------------------------------------------------------------------
// String literal detection helpers
// ---------------------------------------------------------------------------

/// Compute which lines are inside multi-line string literals.
/// Returns a `Vec<bool>` where `true` means the line is string-literal content.
fn compute_multiline_string_mask(source: &str, ext: &str) -> Vec<bool> {
    let lines: Vec<&str> = source.lines().collect();
    let mut mask = vec![false; lines.len()];
    let mut in_string = false;
    let mut closer: &str = "";

    for (i, line) in lines.iter().enumerate() {
        if in_string {
            mask[i] = true;
            if line.contains(closer) {
                in_string = false;
            }
        } else {
            match ext {
                "rs" => {
                    // Rust raw strings: r##"..."##, r#"..."#
                    if let Some(pos) = line.find("r##\"") {
                        if !line[pos + 4..].contains("\"##") {
                            in_string = true;
                            closer = "\"##";
                        }
                    } else if let Some(pos) = line.find("r#\"") {
                        if !line[pos + 3..].contains("\"#") {
                            in_string = true;
                            closer = "\"#";
                        }
                    }
                }
                "py" => {
                    if line.contains("\"\"\"") && line.matches("\"\"\"").count() % 2 != 0 {
                        in_string = true;
                        closer = "\"\"\"";
                    } else if line.contains("'''") && line.matches("'''").count() % 2 != 0 {
                        in_string = true;
                        closer = "'''";
                    }
                }
                "js" | "ts" | "tsx" | "jsx" => {
                    if line.matches('`').count() % 2 != 0 {
                        in_string = true;
                        closer = "`";
                    }
                }
                _ => {}
            }
        }
    }
    mask
}

/// Check if byte position `pos` in `line` falls inside a double-quoted string literal.
/// Uses simple quote-counting heuristic (handles backslash escapes).
fn is_inside_string_literal(line: &str, pos: usize) -> bool {
    let mut in_str = false;
    let bytes = line.as_bytes();
    for i in 0..pos.min(bytes.len()) {
        if bytes[i] == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
            in_str = !in_str;
        }
    }
    in_str
}

/// Extract the comment portion of a source line, if any.
///
/// Language-aware: recognises `//` (C-family), `#` (Python/Ruby/Shell),
/// `--` (Lua), and `*` continuation lines inside block comments.
/// Tracks quote state so that comment markers inside string literals are
/// ignored.
fn extract_comment<'a>(line: &'a str, ext: &str) -> Option<&'a str> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut in_str = false;
    let mut str_char: u8 = 0;

    // Check for block-comment continuation lines (leading `*`)
    let trimmed = line.trim_start();
    if trimmed.starts_with("* ") || trimmed.starts_with("*\t") || trimmed == "*" {
        return Some(trimmed.get(1..).unwrap_or("").trim_start());
    }

    let mut i = 0;
    while i < len {
        let b = bytes[i];

        // Track string literal state (double and single quotes), skip escaped quotes
        if !in_str && (b == b'"' || b == b'\'') {
            in_str = true;
            str_char = b;
            i += 1;
            continue;
        }
        if in_str {
            if b == str_char && (i == 0 || bytes[i - 1] != b'\\') {
                in_str = false;
            }
            i += 1;
            continue;
        }

        // C-family: //
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            return Some(&line[i + 2..]);
        }

        // Python / Ruby / Shell: #
        if b == b'#' && matches!(ext, "py" | "rb" | "sh" | "bash" | "zsh") {
            return Some(&line[i + 1..]);
        }

        // Lua: --
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' && ext == "lua" {
            return Some(&line[i + 2..]);
        }

        i += 1;
    }

    None
}

/// Check if a file is a test file or shell script where phased comments are normal.
fn is_test_or_script_file(rel_path: &str, ext: &str) -> bool {
    if matches!(ext, "sh" | "bash" | "zsh") {
        return true;
    }
    let lower = rel_path.to_lowercase();
    lower.contains("/test/")
        || lower.contains("/tests/")
        || lower.contains("/__tests__/")
        || lower.contains(".test.")
        || lower.contains(".spec.")
        || lower.contains("_test.")
        || lower.starts_with("test_")
}

// ---------------------------------------------------------------------------
// Tool: fossil_detect_scaffolding
// ---------------------------------------------------------------------------

/// Detect scaffolding artifacts in source code.
///
/// # Arguments
/// - `path` (string, required): Path to the project directory.
/// - `include_todos` (bool, optional): Include TODO/FIXME markers (default: false).
/// - `include_placeholders` (bool, optional): Include placeholder bodies (default: true).
pub fn execute_detect_scaffolding(args: &HashMap<String, Value>) -> Result<Value, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("Missing required argument 'path'")?;

    let root = Path::new(path);
    if !root.exists() {
        return Err(format!("Path does not exist: {path}"));
    }

    let include_todos = args
        .get("include_todos")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let include_placeholders = args
        .get("include_placeholders")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let include_phased_comments = args
        .get("include_phased_comments")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let include_temp_files = args
        .get("include_temp_files")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    // Compile regexes.
    let re_phased = Regex::new(PHASED_PATTERN).map_err(|e| format!("Regex error: {e}"))?;
    let re_temporal = Regex::new(TEMPORAL_PATTERN).map_err(|e| format!("Regex error: {e}"))?;
    let re_scaffold_ident =
        Regex::new(SCAFFOLD_IDENT_PATTERN).map_err(|e| format!("Regex error: {e}"))?;
    let re_numbered =
        Regex::new(NUMBERED_SUFFIX_PATTERN).map_err(|e| format!("Regex error: {e}"))?;
    let re_todo = Regex::new(TODO_PATTERN).map_err(|e| format!("Regex error: {e}"))?;
    let re_func_def = Regex::new(FUNC_DEF_PATTERN).map_err(|e| format!("Regex error: {e}"))?;
    let re_phased_comment =
        Regex::new(PHASED_COMMENT_PATTERN).map_err(|e| format!("Regex error: {e}"))?;

    let placeholder_regexes: Vec<Regex> = PLACEHOLDER_PATTERNS
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();

    let debug_print_regexes: Vec<Regex> = DEBUG_PRINT_PATTERNS
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();

    let name_regexes: Vec<(&str, &Regex)> = vec![
        ("phased", &re_phased),
        ("temporal", &re_temporal),
        ("scaffold", &re_scaffold_ident),
        ("numbered", &re_numbered),
    ];

    let mut findings: Vec<Value> = Vec::new();
    // Track lines with phased scaffolding_name findings to deduplicate against phased comments.
    let mut phased_name_lines: HashSet<(String, usize)> = HashSet::new();

    let walker = WalkBuilder::new(root)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .hidden(true)
        .build();

    for entry in walker.flatten() {
        let entry_path = entry.path();
        if entry_path.is_dir() || !is_source_file(entry_path) {
            continue;
        }

        let source = match fs::read_to_string(entry_path) {
            Ok(s) => s,
            Err(_) => continue, // skip binary / unreadable files
        };

        let rel_path = entry_path
            .strip_prefix(root)
            .unwrap_or(entry_path)
            .to_string_lossy()
            .to_string();

        let ext = entry_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let string_mask = compute_multiline_string_mask(&source, ext);
        let source_lines: Vec<&str> = source.lines().collect();

        for (line_num_0, line) in source_lines.iter().enumerate() {
            let line_num = line_num_0 + 1;

            // Skip lines inside multi-line string literals
            if line_num_0 < string_mask.len() && string_mask[line_num_0] {
                continue;
            }

            /// Helper function to check if a function has a placeholder body
            fn has_placeholder_body(
                source_lines: &[&str],
                function_line: usize,
            ) -> bool {
                // Scan next 10 lines for placeholder patterns
                let end = (function_line + 10).min(source_lines.len());
                for i in function_line..end {
                    let line = source_lines[i];
                    let trimmed = line.trim();
                    if trimmed == "pass"
                        || trimmed == "..."
                        || trimmed.contains("todo!()")
                        || trimmed.contains("unimplemented!()")
                    {
                        return true;
                    }
                    // If we hit closing brace or semicolon at start of line, stop scanning
                    if trimmed.starts_with('}') || (trimmed.starts_with(';') && !trimmed.contains('(')) {
                        break;
                    }
                }
                false
            }

            // --- Function/method name patterns ---
            for caps in re_func_def.captures_iter(line) {
                let cap_match = caps.get(1).or_else(|| caps.get(2));
                let name = cap_match.map(|m| m.as_str()).unwrap_or("");

                // Skip function definitions detected inside string literals
                if let Some(ref m) = cap_match {
                    if is_inside_string_literal(line, m.start()) {
                        continue;
                    }
                }

                for &(category, regex) in &name_regexes {
                    if regex.is_match(name) {
                        if category == "phased" {
                            phased_name_lines.insert((rel_path.clone(), line_num));
                        }

                        // For placeholder identifiers, check if function has real implementation (#26)
                        let confidence = if category == "scaffold" && re_scaffold_ident.is_match(name)
                        {
                            if has_placeholder_body(&source_lines, line_num_0) {
                                "high"
                            } else {
                                "low" // Has real implementation, probably legitimate API
                            }
                        } else if category == "phased" {
                            // Phase N in identifiers is very likely domain-related (phase_count, phase1_latency)
                            // Lower confidence significantly (#24)
                            "low"
                        } else {
                            "high"
                        };

                        findings.push(json!({
                            "file": rel_path,
                            "line": line_num,
                            "category": "scaffolding_name",
                            "match_text": name,
                            "pattern": category,
                            "confidence": confidence,
                        }));
                    }
                }
            }

            // --- TODO/FIXME markers ---
            if include_todos {
                for m in re_todo.find_iter(line) {
                    // Skip matches inside string literals
                    if is_inside_string_literal(line, m.start()) {
                        continue;
                    }
                    // Skip keywords in slash-delimited lists like "TODO/FIXME/HACK"
                    let bytes = line.as_bytes();
                    let before_slash = m.start() > 0 && bytes[m.start() - 1] == b'/';
                    let after_slash = m.end() < bytes.len() && bytes[m.end()] == b'/';
                    if before_slash || after_slash {
                        continue;
                    }

                    // Skip XXX in data format patterns like "XXX-XX-XXXX" (#25)
                    if m.as_str() == "XXX" {
                        let before_char = if m.start() > 0 {
                            line.chars().nth(m.start() - 1)
                        } else {
                            None
                        };
                        let after_char = if m.end() < line.len() {
                            line.chars().nth(m.end())
                        } else {
                            None
                        };

                        // If surrounded by format characters (digits, dashes, or X), skip it
                        if let Some(c) = before_char {
                            if c.is_ascii_digit() || c == '-' || c == 'X' {
                                continue;
                            }
                        }
                        if let Some(c) = after_char {
                            if c.is_ascii_digit() || c == '-' || c == 'X' {
                                continue;
                            }
                        }
                    }

                    findings.push(json!({
                        "file": rel_path,
                        "line": line_num,
                        "category": "todo",
                        "match_text": m.as_str(),
                        "pattern": "todo_marker",
                        "confidence": "low",
                    }));
                }
            }

            // --- Placeholder bodies ---
            if include_placeholders {
                for re in &placeholder_regexes {
                    if re.is_match(line) {
                        // Skip `pass` in Python control-flow blocks (except:, finally:, etc.)
                        if ext == "py" && line.trim() == "pass" {
                            let in_control_block = (0..line_num_0)
                                .rev()
                                .find(|&i| !source_lines[i].trim().is_empty())
                                .map(|i| {
                                    let prev = source_lines[i].trim();
                                    prev.ends_with(':') && {
                                        let lc = prev.to_lowercase();
                                        lc.starts_with("except")
                                            || lc.starts_with("finally")
                                            || lc.starts_with("else")
                                            || lc.starts_with("elif")
                                            || lc.starts_with("if ")
                                            || lc.starts_with("for ")
                                            || lc.starts_with("while ")
                                            || lc.starts_with("with ")
                                            || lc.contains("def __init__")
                                            || lc.contains("def __new__")
                                            || (lc.starts_with("def ")
                                                && (lc.contains("(self") || lc.contains("(cls")))
                                    }
                                })
                                .unwrap_or(false);
                            if in_control_block {
                                break;
                            }
                        }
                        findings.push(json!({
                            "file": rel_path,
                            "line": line_num,
                            "category": "placeholder",
                            "match_text": line.trim(),
                            "pattern": "placeholder_body",
                            "confidence": "high",
                        }));
                        break; // one match per line is enough
                    }
                }
            }

            // --- Debug prints ---
            for re in &debug_print_regexes {
                if let Some(m) = re.find(line) {
                    // Skip matches inside string literals
                    if is_inside_string_literal(line, m.start()) {
                        break;
                    }
                    findings.push(json!({
                        "file": rel_path,
                        "line": line_num,
                        "category": "debug_print",
                        "match_text": line.trim(),
                        "pattern": "debug_print",
                        "confidence": "high",
                    }));
                    break;
                }
            }

            // --- Phased comments (Phase N / Step N / Part N in comments) ---
            // NOTE: "Phase N" is a VERY common domain term (pipelines, protocols, compilation stages).
            // Only flag with LOW confidence, and skip if this appears to be describing system design
            // rather than implementation scaffolding (#24).
            if include_phased_comments && !is_test_or_script_file(&rel_path, ext) {
                if let Some(comment) = extract_comment(line, ext) {
                    if let Some(m) = re_phased_comment.find(comment) {
                        // Deduplicate: skip if this line already has a phased scaffolding_name finding
                        if !phased_name_lines.contains(&(rel_path.clone(), line_num)) {
                            // Check if this looks like a design description vs. scaffolding
                            // Scaffolding: "Phase 1: Implement", "Phase 1: Basic structure"
                            // Design: "Phase 1: Lexing", "Phase 1: Fast regex (0-1ms)", "Phase 1: Handshake"
                            let lower = comment.to_lowercase();
                            let looks_like_scaffolding = lower.contains("implement")
                                || lower.contains("build")
                                || lower.contains("create")
                                || lower.contains("setup")
                                || lower.contains("todo");

                            // Skip if comment describes metrics/performance (strong domain signal)
                            let has_domain_context = lower.contains("ms)")
                                || lower.contains("sec)")
                                || lower.contains("ops)")
                                || lower.contains("latency")
                                || lower.contains("throughput")
                                || lower.contains("handshake")
                                || lower.contains("lexing")
                                || lower.contains("parsing")
                                || lower.contains("compilation")
                                || lower.contains("detection");

                            // Only flag if it looks like scaffolding AND doesn't have domain context
                            if looks_like_scaffolding && !has_domain_context {
                                findings.push(json!({
                                    "file": rel_path,
                                    "line": line_num,
                                    "category": "phased_comment",
                                    "match_text": m.as_str(),
                                    "pattern": "phased_comment",
                                    "confidence": "low",  // Changed from "medium" - very high FP rate
                                }));
                            }
                        }
                    }
                }
            }
        }
    }

    // Append temp-file findings if enabled.
    if include_temp_files {
        findings.extend(detect_temp_file_findings(root));
    }

    let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
    let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let total_count = findings.len();
    let page: Vec<Value> = findings.into_iter().skip(offset).take(limit).collect();
    let has_more = offset + page.len() < total_count;

    let result = json!({
        "path": path,
        "total_findings": total_count,
        "offset": offset,
        "limit": limit,
        "has_more": has_more,
        "findings": page,
    });

    Ok(json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&result).unwrap_or_default()
        }]
    }))
}

// ---------------------------------------------------------------------------
// Temp-file detection helper (merged into fossil_detect_scaffolding)
// ---------------------------------------------------------------------------

/// Detect temporary/scaffolding file and directory names, returning findings.
fn detect_temp_file_findings(root: &Path) -> Vec<Value> {
    let checks: Vec<(&str, Regex)> = vec![
        ("phased", Regex::new(PHASED_PATTERN).unwrap()),
        ("temporal", Regex::new(TEMPORAL_PATTERN).unwrap()),
        ("temp", Regex::new(TEMP_PREFIX_PATTERN).unwrap()),
        ("temp", Regex::new(TEMP_SUFFIX_PATTERN).unwrap()),
        ("scaffold", Regex::new(SCAFFOLD_FILE_PATTERN).unwrap()),
        (
            "numbered",
            Regex::new(NUMBERED_FILE_SUFFIX_PATTERN).unwrap(),
        ),
        ("numbered", Regex::new(NUMBERED_SUFFIX_PATTERN).unwrap()),
    ];

    let mut findings: Vec<Value> = Vec::new();

    let walker = WalkBuilder::new(root)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .hidden(true)
        .build();

    for entry in walker.flatten() {
        let entry_path = entry.path();

        let name = match entry_path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };

        let stem = entry_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(name);

        let rel_path = entry_path
            .strip_prefix(root)
            .unwrap_or(entry_path)
            .to_string_lossy()
            .to_string();

        if rel_path.is_empty() || rel_path == "." {
            continue;
        }

        for (category, regex) in &checks {
            if regex.is_match(name) || regex.is_match(stem) {
                findings.push(json!({
                    "path": rel_path,
                    "name": name,
                    "category": format!("temp_file_{}", category),
                    "pattern": regex.as_str(),
                    "confidence": "medium",
                }));
                break;
            }
        }
    }

    findings
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    // ---- extract_comment tests ----

    #[test]
    fn extract_comment_rust_double_slash() {
        assert_eq!(
            extract_comment("    let x = 1; // Phase 1 init", "rs"),
            Some(" Phase 1 init")
        );
    }

    #[test]
    fn extract_comment_python_hash() {
        assert_eq!(
            extract_comment("x = 1  # Step 2 setup", "py"),
            Some(" Step 2 setup")
        );
    }

    #[test]
    fn extract_comment_lua_double_dash() {
        assert_eq!(
            extract_comment("local x = 1 -- Part 3", "lua"),
            Some(" Part 3")
        );
    }

    #[test]
    fn extract_comment_inside_string_rejected() {
        // The // is inside a string literal, not a real comment
        assert_eq!(
            extract_comment(r#"let s = "http://example.com"; // real comment"#, "rs"),
            Some(" real comment")
        );
        // Entire thing is a string — no comment
        assert_eq!(
            extract_comment(r#"let s = "// not a comment";"#, "rs"),
            None
        );
    }

    #[test]
    fn extract_comment_block_star_continuation() {
        assert_eq!(
            extract_comment("   * Phase 2: continue block", "rs"),
            Some("Phase 2: continue block")
        );
    }

    #[test]
    fn extract_comment_no_comment() {
        assert_eq!(extract_comment("let phase = 1;", "rs"), None);
    }

    // ---- PHASED_COMMENT_PATTERN tests ----

    #[test]
    fn phased_comment_pattern_matches() {
        let re = Regex::new(PHASED_COMMENT_PATTERN).unwrap();
        assert!(re.is_match("Phase 1"));
        assert!(re.is_match("phase 2"));
        assert!(re.is_match("Phase 2B"));
        assert!(re.is_match("step-3"));
        assert!(re.is_match("Step 3"));
        assert!(re.is_match("Part 1"));
        assert!(re.is_match("part_4"));
        assert!(re.is_match("PHASE 10"));
        assert!(re.is_match("Phase2"));
        assert!(re.is_match("step3a"));
    }

    #[test]
    fn phased_comment_pattern_non_matches() {
        let re = Regex::new(PHASED_COMMENT_PATTERN).unwrap();
        assert!(!re.is_match("phaser"));
        assert!(!re.is_match("stepping"));
        assert!(!re.is_match("partial"));
        assert!(!re.is_match("the phase of the moon"));
        assert!(!re.is_match("a step ahead"));
    }

    // ---- Integration test ----

    #[test]
    fn integration_phased_comment_detection() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("main.rs");
        let mut f = fs::File::create(&file_path).unwrap();
        writeln!(f, "fn main() {{").unwrap();
        // Generic "Phase 1: initialization" is NOT flagged — could be legitimate domain
        writeln!(f, "    // Phase 1: initialization").unwrap();
        writeln!(f, "    let x = 1;").unwrap();
        // BUT "Phase 1: Implement basic structure" IS flagged — clear scaffolding
        writeln!(f, "    // Phase 1: Implement basic structure").unwrap();
        writeln!(f, "    let y = x + 1;").unwrap();
        writeln!(f, "}}").unwrap();

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.path().to_string_lossy().to_string()),
        );
        // Disable other categories to isolate phased comments
        args.insert("include_todos".to_string(), Value::Bool(false));
        args.insert("include_placeholders".to_string(), Value::Bool(false));

        let result = execute_detect_scaffolding(&args).unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let findings = parsed["findings"].as_array().unwrap();
        let phased: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "phased_comment")
            .collect();

        // Now only 1 finding: the explicit "Implement" scaffolding
        // (#24: Reduce Phase N false positives by requiring scaffolding keywords)
        assert_eq!(phased.len(), 1);
        assert_eq!(phased[0]["line"], 4);
        assert!(phased[0]["match_text"]
            .as_str()
            .unwrap()
            .contains("Phase 1"));
        assert_eq!(phased[0]["confidence"], "low"); // Low confidence even when flagged
    }

    #[test]
    fn integration_pass_in_except_not_flagged() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("handler.py");
        let mut f = fs::File::create(&file_path).unwrap();
        writeln!(f, "def handle():").unwrap();
        writeln!(f, "    try:").unwrap();
        writeln!(f, "        do_something()").unwrap();
        writeln!(f, "    except:").unwrap();
        writeln!(f, "        pass").unwrap();
        writeln!(f, "    finally:").unwrap();
        writeln!(f, "        pass").unwrap();

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.path().to_string_lossy().to_string()),
        );
        args.insert("include_todos".to_string(), Value::Bool(false));
        args.insert("include_phased_comments".to_string(), Value::Bool(false));

        let result = execute_detect_scaffolding(&args).unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let findings = parsed["findings"].as_array().unwrap();
        let placeholders: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "placeholder")
            .collect();

        assert!(
            placeholders.is_empty(),
            "pass in except/finally blocks should not be flagged as placeholder, got: {:?}",
            placeholders
        );
    }

    #[test]
    fn test_pass_in_init_not_flagged() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("models.py");
        let mut f = fs::File::create(&file_path).unwrap();
        writeln!(f, "class Foo:").unwrap();
        writeln!(f, "    def __init__(self):").unwrap();
        writeln!(f, "        pass").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "class Bar:").unwrap();
        writeln!(f, "    def process(self):").unwrap();
        writeln!(f, "        pass").unwrap();

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.path().to_string_lossy().to_string()),
        );
        args.insert("include_todos".to_string(), Value::Bool(false));
        args.insert("include_phased_comments".to_string(), Value::Bool(false));

        let result = execute_detect_scaffolding(&args).unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let findings = parsed["findings"].as_array().unwrap();
        let placeholders: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "placeholder")
            .collect();

        // pass in __init__ and class methods (with self) should NOT be flagged
        assert!(
            placeholders.is_empty(),
            "pass in __init__ and class methods should not be flagged as placeholder, got: {:?}",
            placeholders
        );
    }

    #[test]
    fn integration_pass_in_bare_function_still_flagged() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("stub.py");
        let mut f = fs::File::create(&file_path).unwrap();
        writeln!(f, "def stub():").unwrap();
        writeln!(f, "    pass").unwrap();

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.path().to_string_lossy().to_string()),
        );
        args.insert("include_todos".to_string(), Value::Bool(false));
        args.insert("include_phased_comments".to_string(), Value::Bool(false));

        let result = execute_detect_scaffolding(&args).unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let findings = parsed["findings"].as_array().unwrap();
        let placeholders: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "placeholder")
            .collect();

        assert_eq!(
            placeholders.len(),
            1,
            "pass in bare function body should still be flagged"
        );
    }

    #[test]
    fn integration_phased_comments_in_test_file_skipped() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("foo.test.ts");
        let mut f = fs::File::create(&file_path).unwrap();
        writeln!(f, "// Step 1: setup").unwrap();
        writeln!(f, "const x = 1;").unwrap();
        writeln!(f, "// Step 2: verify").unwrap();
        writeln!(f, "expect(x).toBe(1);").unwrap();

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.path().to_string_lossy().to_string()),
        );
        args.insert("include_todos".to_string(), Value::Bool(false));
        args.insert("include_placeholders".to_string(), Value::Bool(false));

        let result = execute_detect_scaffolding(&args).unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let findings = parsed["findings"].as_array().unwrap();
        let phased: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "phased_comment")
            .collect();

        assert!(
            phased.is_empty(),
            "phased comments in test files should be skipped, got: {:?}",
            phased
        );
    }

    #[test]
    fn integration_phased_comments_in_shell_script_skipped() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("deploy.sh");
        let mut f = fs::File::create(&file_path).unwrap();
        writeln!(f, "#!/bin/bash").unwrap();
        writeln!(f, "# Step 1: build").unwrap();
        writeln!(f, "make build").unwrap();
        writeln!(f, "# Step 2: deploy").unwrap();
        writeln!(f, "make deploy").unwrap();

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.path().to_string_lossy().to_string()),
        );
        args.insert("include_todos".to_string(), Value::Bool(false));
        args.insert("include_placeholders".to_string(), Value::Bool(false));

        let result = execute_detect_scaffolding(&args).unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let findings = parsed["findings"].as_array().unwrap();
        let phased: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "phased_comment")
            .collect();

        assert!(
            phased.is_empty(),
            "phased comments in shell scripts should be skipped, got: {:?}",
            phased
        );
    }

    #[test]
    fn test_scaffolding_findings_have_confidence() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("main.rs");
        let mut f = fs::File::create(&file_path).unwrap();
        writeln!(f, "fn phase_1_init() {{}}").unwrap();
        writeln!(f, "// Phase 2: processing").unwrap();
        writeln!(f, "// TODO: fix later").unwrap();

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.path().to_string_lossy().to_string()),
        );
        args.insert("include_todos".to_string(), Value::Bool(true));
        args.insert("include_placeholders".to_string(), Value::Bool(true));
        args.insert("include_phased_comments".to_string(), Value::Bool(true));

        let result = execute_detect_scaffolding(&args).unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let findings = parsed["findings"].as_array().unwrap();
        // All findings should have a confidence field
        for finding in findings {
            assert!(
                finding.get("confidence").is_some(),
                "Finding should have confidence field: {:?}",
                finding
            );
            let confidence = finding["confidence"].as_str().unwrap();
            assert!(
                matches!(confidence, "high" | "medium" | "low"),
                "Confidence should be high/medium/low, got: {}",
                confidence
            );
        }

        // Verify specific confidence levels
        let scaffolding_names: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "scaffolding_name")
            .collect();
        for f in &scaffolding_names {
            assert_eq!(f["confidence"], "high");
        }

        let phased: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "phased_comment")
            .collect();
        for f in &phased {
            assert_eq!(f["confidence"], "medium");
        }

        let todos: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "todo")
            .collect();
        for f in &todos {
            assert_eq!(f["confidence"], "low");
        }
    }

    #[test]
    fn test_todos_disabled_by_default() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("main.rs");
        let mut f = fs::File::create(&file_path).unwrap();
        writeln!(f, "fn main() {{}}").unwrap();
        writeln!(f, "// TODO: fix later").unwrap();

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.path().to_string_lossy().to_string()),
        );
        // Do NOT pass include_todos — default should be false
        args.insert("include_placeholders".to_string(), Value::Bool(false));
        args.insert("include_phased_comments".to_string(), Value::Bool(false));

        let result = execute_detect_scaffolding(&args).unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let findings = parsed["findings"].as_array().unwrap();
        let todos: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "todo")
            .collect();

        assert!(
            todos.is_empty(),
            "TODO markers should NOT appear when include_todos defaults to false, got: {:?}",
            todos
        );
    }

    #[test]
    fn integration_phased_comments_disabled() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("main.rs");
        let mut f = fs::File::create(&file_path).unwrap();
        writeln!(f, "// Phase 1: init").unwrap();
        writeln!(f, "fn main() {{}}").unwrap();

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            Value::String(dir.path().to_string_lossy().to_string()),
        );
        args.insert("include_todos".to_string(), Value::Bool(false));
        args.insert("include_placeholders".to_string(), Value::Bool(false));
        args.insert("include_phased_comments".to_string(), Value::Bool(false));

        let result = execute_detect_scaffolding(&args).unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(text).unwrap();

        let findings = parsed["findings"].as_array().unwrap();
        let phased: Vec<&Value> = findings
            .iter()
            .filter(|f| f["category"] == "phased_comment")
            .collect();

        assert!(phased.is_empty(), "phased comments should be suppressed");
    }
}
