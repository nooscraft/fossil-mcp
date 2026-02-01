//! Feature-flag detection for dead code analysis.
//!
//! Scans source files for feature-flag patterns (compile-time and runtime) and
//! identifies conditional blocks that may be always-dead based on static analysis
//! of the flag conditions.
//!
//! Supported patterns per language:
//! - **Rust**: `#[cfg(...)]`, `#[cfg(not(...))]`, `#[cfg(test)]`, `#[cfg(target_os = "...")]`
//! - **C/C++**: `#ifdef`, `#ifndef`, `#if defined(...)`, `#if 0`
//! - **Python**: `os.environ.get(...)`, `settings.FEATURE_FLAG`, `if DEBUG:`
//! - **JavaScript/TypeScript**: `process.env.FEATURE`, `config.featureFlag`, `import.meta.env.FEATURE`

use crate::core::Language;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

// =============================================================================
// Types
// =============================================================================

/// The kind of feature flag detected.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlagType {
    /// Rust `#[cfg(...)]` attribute.
    RustCfg,
    /// C/C++ `#ifdef` / `#ifndef` / `#if defined(...)` / `#if 0`.
    CppIfdef,
    /// Runtime conditional feature check (Python `settings.FLAG`, etc.).
    ConditionalFeatureCheck,
    /// Environment variable check (`os.environ`, `process.env`, `import.meta.env`).
    EnvironmentVariable,
}

/// A single detected feature flag in source code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureFlag {
    /// Name or expression of the flag (e.g. `"feature = \"foo\""`, `"MY_FEATURE"`).
    pub name: String,
    /// File path where the flag was found.
    pub file: String,
    /// 1-based line number of the flag.
    pub line: usize,
    /// Classification of the flag pattern.
    pub flag_type: FlagType,
    /// Whether static analysis determines the flag to be always dead.
    pub is_always_dead: bool,
}

/// A block of code controlled by a feature flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionalBlock {
    /// The feature flag that controls this block.
    pub flag: FeatureFlag,
    /// 1-based start line of the conditional block.
    pub start_line: usize,
    /// 1-based end line of the conditional block.
    pub end_line: usize,
    /// Number of non-empty source lines within the block.
    pub lines_of_code: usize,
}

// =============================================================================
// Compiled regex cache (one-time init)
// =============================================================================

struct RustPatterns {
    cfg_feature: Regex,
    cfg_not: Regex,
    cfg_test: Regex,
    cfg_target_os: Regex,
    cfg_generic: Regex,
}

struct CppPatterns {
    ifdef: Regex,
    ifndef: Regex,
    if_defined: Regex,
    if_zero: Regex,
    endif: Regex,
    else_directive: Regex,
    elif_directive: Regex,
}

struct PythonPatterns {
    environ_get: Regex,
    environ_bracket: Regex,
    settings_flag: Regex,
    if_debug: Regex,
    if_false: Regex,
    if_zero: Regex,
}

struct JsPatterns {
    process_env: Regex,
    config_flag: Regex,
    import_meta_env: Regex,
}

fn rust_patterns() -> &'static RustPatterns {
    static INSTANCE: OnceLock<RustPatterns> = OnceLock::new();
    INSTANCE.get_or_init(|| RustPatterns {
        cfg_feature: Regex::new(r#"#\[cfg\(feature\s*=\s*"([^"]+)"\)\]"#).unwrap(),
        cfg_not: Regex::new(r#"#\[cfg\(not\((.+?)\)\)\]"#).unwrap(),
        cfg_test: Regex::new(r"#\[cfg\(test\)\]").unwrap(),
        cfg_target_os: Regex::new(r#"#\[cfg\(target_os\s*=\s*"([^"]+)"\)\]"#).unwrap(),
        cfg_generic: Regex::new(r"#\[cfg\((.+?)\)\]").unwrap(),
    })
}

fn cpp_patterns() -> &'static CppPatterns {
    static INSTANCE: OnceLock<CppPatterns> = OnceLock::new();
    INSTANCE.get_or_init(|| CppPatterns {
        ifdef: Regex::new(r"^\s*#\s*ifdef\s+(\w+)").unwrap(),
        ifndef: Regex::new(r"^\s*#\s*ifndef\s+(\w+)").unwrap(),
        if_defined: Regex::new(r"^\s*#\s*if\s+defined\s*\(\s*(\w+)\s*\)").unwrap(),
        if_zero: Regex::new(r"^\s*#\s*if\s+0\s*$").unwrap(),
        endif: Regex::new(r"^\s*#\s*endif").unwrap(),
        else_directive: Regex::new(r"^\s*#\s*else").unwrap(),
        elif_directive: Regex::new(r"^\s*#\s*elif").unwrap(),
    })
}

fn python_patterns() -> &'static PythonPatterns {
    static INSTANCE: OnceLock<PythonPatterns> = OnceLock::new();
    INSTANCE.get_or_init(|| PythonPatterns {
        environ_get: Regex::new(r#"os\.environ\.get\(\s*["'](\w+)["']"#).unwrap(),
        environ_bracket: Regex::new(r#"os\.environ\[\s*["'](\w+)["']"#).unwrap(),
        settings_flag: Regex::new(r"settings\.([A-Z_][A-Z_0-9]*)").unwrap(),
        if_debug: Regex::new(r"^\s*if\s+DEBUG\s*:").unwrap(),
        if_false: Regex::new(r"^\s*if\s+False\s*:").unwrap(),
        if_zero: Regex::new(r"^\s*if\s+0\s*:").unwrap(),
    })
}

fn js_patterns() -> &'static JsPatterns {
    static INSTANCE: OnceLock<JsPatterns> = OnceLock::new();
    INSTANCE.get_or_init(|| JsPatterns {
        process_env: Regex::new(r"process\.env\.(\w+)").unwrap(),
        config_flag: Regex::new(r"config\.([a-zA-Z_]\w*)").unwrap(),
        import_meta_env: Regex::new(r"import\.meta\.env\.(\w+)").unwrap(),
    })
}

// =============================================================================
// FeatureFlagDetector
// =============================================================================

/// Detects feature flags and conditional blocks in source code.
pub struct FeatureFlagDetector;

impl FeatureFlagDetector {
    /// Scan source text for feature flag patterns appropriate to the given language.
    pub fn detect_flags(source: &str, file_path: &str, language: Language) -> Vec<FeatureFlag> {
        match language {
            Language::Rust => Self::detect_rust_flags(source, file_path),
            Language::C | Language::Cpp => Self::detect_cpp_flags(source, file_path),
            Language::Python => Self::detect_python_flags(source, file_path),
            Language::JavaScript | Language::TypeScript => Self::detect_js_flags(source, file_path),
            _ => Vec::new(),
        }
    }

    /// Return only the flags that are statically determined to be always dead.
    pub fn find_always_dead_flags(flags: &[FeatureFlag]) -> Vec<&FeatureFlag> {
        flags.iter().filter(|f| f.is_always_dead).collect()
    }

    /// Identify conditional blocks controlled by feature flags.
    pub fn find_conditional_blocks(
        source: &str,
        file_path: &str,
        language: Language,
    ) -> Vec<ConditionalBlock> {
        match language {
            Language::Rust => Self::find_rust_conditional_blocks(source, file_path),
            Language::C | Language::Cpp => Self::find_cpp_conditional_blocks(source, file_path),
            Language::Python => Self::find_python_conditional_blocks(source, file_path),
            Language::JavaScript | Language::TypeScript => {
                Self::find_js_conditional_blocks(source, file_path)
            }
            _ => Vec::new(),
        }
    }

    // =========================================================================
    // Rust
    // =========================================================================

    fn detect_rust_flags(source: &str, file_path: &str) -> Vec<FeatureFlag> {
        let pats = rust_patterns();
        let mut flags = Vec::new();

        for (line_idx, line) in source.lines().enumerate() {
            let line_num = line_idx + 1;

            // #[cfg(feature = "...")]
            if let Some(cap) = pats.cfg_feature.captures(line) {
                flags.push(FeatureFlag {
                    name: format!("feature = \"{}\"", &cap[1]),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::RustCfg,
                    is_always_dead: false,
                });
                continue;
            }

            // #[cfg(test)]
            if pats.cfg_test.is_match(line) {
                flags.push(FeatureFlag {
                    name: "test".to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::RustCfg,
                    is_always_dead: false,
                });
                continue;
            }

            // #[cfg(target_os = "...")]
            if let Some(cap) = pats.cfg_target_os.captures(line) {
                flags.push(FeatureFlag {
                    name: format!("target_os = \"{}\"", &cap[1]),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::RustCfg,
                    is_always_dead: false,
                });
                continue;
            }

            // #[cfg(not(...))] -- check for always-dead contradictions
            if let Some(cap) = pats.cfg_not.captures(line) {
                let inner = cap[1].trim();
                let is_always_dead = Self::is_rust_cfg_not_always_dead(inner);
                flags.push(FeatureFlag {
                    name: format!("not({})", inner),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::RustCfg,
                    is_always_dead,
                });
                continue;
            }

            // Generic #[cfg(...)] that wasn't caught above
            if let Some(cap) = pats.cfg_generic.captures(line) {
                let inner = cap[1].trim();
                // Skip if we already matched a more specific pattern
                if !flags.iter().any(|f| f.line == line_num) {
                    flags.push(FeatureFlag {
                        name: inner.to_string(),
                        file: file_path.to_string(),
                        line: line_num,
                        flag_type: FlagType::RustCfg,
                        is_always_dead: false,
                    });
                }
            }
        }

        flags
    }

    /// Heuristic: `not(any(...))` with contradictory conditions is always dead.
    ///
    /// For example `not(any(unix, windows, target_os = "macos"))` is considered
    /// always dead because the disjunction covers all practical targets.
    fn is_rust_cfg_not_always_dead(inner: &str) -> bool {
        // `not(any(unix, windows))` -- covers all mainstream platforms
        if inner.starts_with("any(") && inner.ends_with(')') {
            let body = &inner[4..inner.len() - 1];
            let parts: Vec<&str> = body.split(',').map(|s| s.trim()).collect();
            let has_unix = parts.contains(&"unix");
            let has_windows = parts.contains(&"windows");
            if has_unix && has_windows {
                return true;
            }
        }
        false
    }

    // =========================================================================
    // C / C++
    // =========================================================================

    fn detect_cpp_flags(source: &str, file_path: &str) -> Vec<FeatureFlag> {
        let pats = cpp_patterns();
        let mut flags = Vec::new();

        for (line_idx, line) in source.lines().enumerate() {
            let line_num = line_idx + 1;

            // #if 0  (always dead)
            if pats.if_zero.is_match(line) {
                flags.push(FeatureFlag {
                    name: "0".to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::CppIfdef,
                    is_always_dead: true,
                });
                continue;
            }

            // #ifdef FEATURE
            if let Some(cap) = pats.ifdef.captures(line) {
                flags.push(FeatureFlag {
                    name: cap[1].to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::CppIfdef,
                    is_always_dead: false,
                });
                continue;
            }

            // #ifndef FEATURE
            if let Some(cap) = pats.ifndef.captures(line) {
                flags.push(FeatureFlag {
                    name: cap[1].to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::CppIfdef,
                    is_always_dead: false,
                });
                continue;
            }

            // #if defined(FEATURE)
            if let Some(cap) = pats.if_defined.captures(line) {
                flags.push(FeatureFlag {
                    name: cap[1].to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::CppIfdef,
                    is_always_dead: false,
                });
            }
        }

        flags
    }

    // =========================================================================
    // Python
    // =========================================================================

    fn detect_python_flags(source: &str, file_path: &str) -> Vec<FeatureFlag> {
        let pats = python_patterns();
        let mut flags = Vec::new();

        for (line_idx, line) in source.lines().enumerate() {
            let line_num = line_idx + 1;

            // if False:  (always dead)
            if pats.if_false.is_match(line) {
                flags.push(FeatureFlag {
                    name: "False".to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::ConditionalFeatureCheck,
                    is_always_dead: true,
                });
                continue;
            }

            // if 0:  (always dead)
            if pats.if_zero.is_match(line) {
                flags.push(FeatureFlag {
                    name: "0".to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::ConditionalFeatureCheck,
                    is_always_dead: true,
                });
                continue;
            }

            // if DEBUG:
            if pats.if_debug.is_match(line) {
                flags.push(FeatureFlag {
                    name: "DEBUG".to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::ConditionalFeatureCheck,
                    is_always_dead: false,
                });
                continue;
            }

            // os.environ.get("FEATURE")
            if let Some(cap) = pats.environ_get.captures(line) {
                flags.push(FeatureFlag {
                    name: cap[1].to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::EnvironmentVariable,
                    is_always_dead: false,
                });
                continue;
            }

            // os.environ["FEATURE"]
            if let Some(cap) = pats.environ_bracket.captures(line) {
                flags.push(FeatureFlag {
                    name: cap[1].to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::EnvironmentVariable,
                    is_always_dead: false,
                });
                continue;
            }

            // settings.FEATURE_FLAG
            if let Some(cap) = pats.settings_flag.captures(line) {
                flags.push(FeatureFlag {
                    name: cap[1].to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::ConditionalFeatureCheck,
                    is_always_dead: false,
                });
            }
        }

        flags
    }

    // =========================================================================
    // JavaScript / TypeScript
    // =========================================================================

    fn detect_js_flags(source: &str, file_path: &str) -> Vec<FeatureFlag> {
        let pats = js_patterns();
        let mut flags = Vec::new();

        for (line_idx, line) in source.lines().enumerate() {
            let line_num = line_idx + 1;

            // import.meta.env.FEATURE  (must come before config.* to avoid false match)
            if let Some(cap) = pats.import_meta_env.captures(line) {
                flags.push(FeatureFlag {
                    name: cap[1].to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::EnvironmentVariable,
                    is_always_dead: false,
                });
                continue;
            }

            // process.env.FEATURE
            if let Some(cap) = pats.process_env.captures(line) {
                flags.push(FeatureFlag {
                    name: cap[1].to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::EnvironmentVariable,
                    is_always_dead: false,
                });
                continue;
            }

            // config.featureFlag
            if let Some(cap) = pats.config_flag.captures(line) {
                flags.push(FeatureFlag {
                    name: cap[1].to_string(),
                    file: file_path.to_string(),
                    line: line_num,
                    flag_type: FlagType::ConditionalFeatureCheck,
                    is_always_dead: false,
                });
            }
        }

        flags
    }

    // =========================================================================
    // Conditional block detection — Rust
    // =========================================================================

    fn find_rust_conditional_blocks(source: &str, file_path: &str) -> Vec<ConditionalBlock> {
        let flags = Self::detect_rust_flags(source, file_path);
        let lines: Vec<&str> = source.lines().collect();
        let mut blocks = Vec::new();

        for flag in flags {
            let start_line = flag.line;
            // The block controlled by a #[cfg(..)] starts on the line after the attribute.
            // Look for the end of the item: balanced braces or single-line item.
            let block_start = start_line; // inclusive, attribute line
            if let Some(end_line) = Self::find_rust_item_end(&lines, start_line) {
                let loc = Self::count_non_empty_lines(&lines, start_line, end_line);
                blocks.push(ConditionalBlock {
                    flag,
                    start_line: block_start,
                    end_line,
                    lines_of_code: loc,
                });
            }
        }

        blocks
    }

    /// Starting from the line after the attribute, find the end of the Rust item.
    fn find_rust_item_end(lines: &[&str], attr_line: usize) -> Option<usize> {
        // attr_line is 1-based; start scanning from the item line itself
        let item_start_idx = attr_line; // 0-based index = attr_line (because attr_line is 1-based, so next line is index attr_line)
        if item_start_idx >= lines.len() {
            return None;
        }

        // If the next line after the attribute opens a brace block, find closing brace
        let mut depth: i32 = 0;
        let mut found_open = false;
        for (idx, line) in lines.iter().enumerate().skip(item_start_idx) {
            for ch in line.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        found_open = true;
                    }
                    '}' => {
                        depth -= 1;
                    }
                    _ => {}
                }
            }
            if found_open && depth <= 0 {
                return Some(idx + 1); // 1-based
            }
            // Single-line item (e.g. `fn foo();` or `use bar;`)
            if !found_open && line.trim_end().ends_with(';') {
                return Some(idx + 1); // 1-based
            }
        }

        // Fallback: just the attribute line plus one
        if !found_open {
            return Some(attr_line + 1);
        }

        None
    }

    // =========================================================================
    // Conditional block detection -- C/C++
    // =========================================================================

    fn find_cpp_conditional_blocks(source: &str, file_path: &str) -> Vec<ConditionalBlock> {
        let flags = Self::detect_cpp_flags(source, file_path);
        let pats = cpp_patterns();
        let lines: Vec<&str> = source.lines().collect();
        let mut blocks = Vec::new();

        for flag in flags {
            let start_line = flag.line;
            // Scan forward from the directive to find matching #endif, tracking nesting
            let start_idx = start_line; // 0-based index of first line inside the block
            let mut depth: u32 = 1;
            let mut end_line = None;

            for (idx, line) in lines.iter().enumerate().skip(start_idx) {
                // Nested #if / #ifdef / #ifndef
                if pats.ifdef.is_match(line)
                    || pats.ifndef.is_match(line)
                    || pats.if_defined.is_match(line)
                    || pats.if_zero.is_match(line)
                {
                    depth += 1;
                }
                if pats.endif.is_match(line) {
                    depth -= 1;
                    if depth == 0 {
                        end_line = Some(idx + 1); // 1-based
                        break;
                    }
                }
            }

            if let Some(end) = end_line {
                // For always-dead #if 0 blocks, the controlled region is between the
                // #if 0 and the matching #endif (or #else/#elif).
                let effective_end = Self::find_cpp_effective_end(&lines, start_line, end, pats);
                let loc = Self::count_non_empty_lines(&lines, start_line, effective_end);
                blocks.push(ConditionalBlock {
                    flag,
                    start_line,
                    end_line: effective_end,
                    lines_of_code: loc,
                });
            }
        }

        blocks
    }

    /// For `#if 0`, the dead region ends at the first `#else` or `#elif` at the
    /// same nesting depth, or at `#endif` if neither is present.
    fn find_cpp_effective_end(
        lines: &[&str],
        start_line: usize,
        endif_line: usize,
        pats: &CppPatterns,
    ) -> usize {
        let start_idx = start_line; // 0-based index of line after directive
        let end_idx = endif_line - 1; // 0-based index of #endif line
        let mut depth: u32 = 0;

        for (idx, line) in lines.iter().enumerate().take(end_idx).skip(start_idx) {
            if pats.ifdef.is_match(line)
                || pats.ifndef.is_match(line)
                || pats.if_defined.is_match(line)
                || pats.if_zero.is_match(line)
            {
                depth += 1;
            }
            if pats.endif.is_match(line) {
                depth = depth.saturating_sub(1);
            }
            if depth == 0
                && (pats.else_directive.is_match(line) || pats.elif_directive.is_match(line))
            {
                return idx + 1; // 1-based
            }
        }

        endif_line
    }

    // =========================================================================
    // Conditional block detection -- Python
    // =========================================================================

    fn find_python_conditional_blocks(source: &str, file_path: &str) -> Vec<ConditionalBlock> {
        let flags = Self::detect_python_flags(source, file_path);
        let lines: Vec<&str> = source.lines().collect();
        let mut blocks = Vec::new();

        for flag in flags {
            let start_line = flag.line;
            if let Some(end_line) = Self::find_python_block_end(&lines, start_line) {
                let loc = Self::count_non_empty_lines(&lines, start_line, end_line);
                blocks.push(ConditionalBlock {
                    flag,
                    start_line,
                    end_line,
                    lines_of_code: loc,
                });
            }
        }

        blocks
    }

    /// Find the end of a Python indented block starting from the `if ...:` line.
    fn find_python_block_end(lines: &[&str], if_line: usize) -> Option<usize> {
        let if_idx = if_line - 1; // 0-based
        if if_idx >= lines.len() {
            return None;
        }

        // Determine the indentation of the `if` line
        let if_indent = Self::leading_spaces(lines[if_idx]);

        // Everything indented more than the `if` line belongs to its block
        let mut last_body_line = if_line; // 1-based
        for (idx, line) in lines.iter().enumerate().skip(if_idx + 1) {
            if line.trim().is_empty() {
                continue; // blank lines don't end the block
            }
            let indent = Self::leading_spaces(line);
            if indent <= if_indent {
                break;
            }
            last_body_line = idx + 1; // 1-based
        }

        Some(last_body_line)
    }

    // =========================================================================
    // Conditional block detection -- JS/TS
    // =========================================================================

    fn find_js_conditional_blocks(source: &str, file_path: &str) -> Vec<ConditionalBlock> {
        let flags = Self::detect_js_flags(source, file_path);
        let lines: Vec<&str> = source.lines().collect();
        let mut blocks = Vec::new();

        for flag in flags {
            let start_line = flag.line;
            if let Some(end_line) = Self::find_js_block_end(&lines, start_line) {
                let loc = Self::count_non_empty_lines(&lines, start_line, end_line);
                blocks.push(ConditionalBlock {
                    flag,
                    start_line,
                    end_line,
                    lines_of_code: loc,
                });
            }
        }

        blocks
    }

    /// Find the end of a JS/TS block by balanced braces starting from the flag line.
    fn find_js_block_end(lines: &[&str], flag_line: usize) -> Option<usize> {
        let start_idx = flag_line - 1; // 0-based
        if start_idx >= lines.len() {
            return None;
        }

        let mut depth: i32 = 0;
        let mut found_open = false;

        for (idx, line) in lines.iter().enumerate().skip(start_idx) {
            for ch in line.chars() {
                match ch {
                    '{' => {
                        depth += 1;
                        found_open = true;
                    }
                    '}' => depth -= 1,
                    _ => {}
                }
            }
            if found_open && depth <= 0 {
                return Some(idx + 1); // 1-based
            }
        }

        // No braces found -- single-line conditional
        Some(flag_line)
    }

    // =========================================================================
    // Helpers
    // =========================================================================

    /// Count non-empty lines between `start` and `end` (both 1-based, inclusive).
    fn count_non_empty_lines(lines: &[&str], start: usize, end: usize) -> usize {
        let from = start.saturating_sub(1); // to 0-based
        let to = end.min(lines.len()); // exclusive upper bound
        lines[from..to]
            .iter()
            .filter(|l| !l.trim().is_empty())
            .count()
    }

    /// Count leading space characters (spaces and tabs, where tab = 4 spaces).
    fn leading_spaces(line: &str) -> usize {
        let mut count = 0;
        for ch in line.chars() {
            match ch {
                ' ' => count += 1,
                '\t' => count += 4,
                _ => break,
            }
        }
        count
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Rust cfg detection
    // -------------------------------------------------------------------------

    #[test]
    fn test_rust_cfg_feature() {
        let source = r#"
#[cfg(feature = "serde")]
fn serialize() {}

#[cfg(feature = "async")]
fn async_run() {}
"#;
        let flags = FeatureFlagDetector::detect_flags(source, "lib.rs", Language::Rust);
        assert_eq!(flags.len(), 2);
        assert_eq!(flags[0].name, r#"feature = "serde""#);
        assert_eq!(flags[0].flag_type, FlagType::RustCfg);
        assert_eq!(flags[0].line, 2);
        assert!(!flags[0].is_always_dead);
        assert_eq!(flags[1].name, r#"feature = "async""#);
    }

    #[test]
    fn test_rust_cfg_test() {
        let source = r#"
#[cfg(test)]
mod tests {
    fn test_something() {}
}
"#;
        let flags = FeatureFlagDetector::detect_flags(source, "lib.rs", Language::Rust);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "test");
        assert_eq!(flags[0].flag_type, FlagType::RustCfg);
    }

    #[test]
    fn test_rust_cfg_target_os() {
        let source = r#"#[cfg(target_os = "linux")]
fn linux_only() {}
"#;
        let flags = FeatureFlagDetector::detect_flags(source, "lib.rs", Language::Rust);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, r#"target_os = "linux""#);
    }

    #[test]
    fn test_rust_cfg_not_always_dead() {
        let source = r#"
#[cfg(not(any(unix, windows)))]
fn exotic_platform() {}
"#;
        let flags = FeatureFlagDetector::detect_flags(source, "lib.rs", Language::Rust);
        assert_eq!(flags.len(), 1);
        assert!(
            flags[0].is_always_dead,
            "not(any(unix, windows)) should be always dead"
        );
        assert_eq!(flags[0].name, "not(any(unix, windows))");
    }

    #[test]
    fn test_rust_cfg_not_not_always_dead() {
        // not(unix) alone is NOT always dead -- Windows exists
        let source = r#"
#[cfg(not(unix))]
fn non_unix() {}
"#;
        let flags = FeatureFlagDetector::detect_flags(source, "lib.rs", Language::Rust);
        assert_eq!(flags.len(), 1);
        assert!(!flags[0].is_always_dead);
    }

    // -------------------------------------------------------------------------
    // C/C++ ifdef detection
    // -------------------------------------------------------------------------

    #[test]
    fn test_cpp_ifdef() {
        let source = r#"
#ifdef ENABLE_LOGGING
    log("hello");
#endif

#ifndef NDEBUG
    assert(x > 0);
#endif
"#;
        let flags = FeatureFlagDetector::detect_flags(source, "main.c", Language::C);
        assert_eq!(flags.len(), 2);
        assert_eq!(flags[0].name, "ENABLE_LOGGING");
        assert_eq!(flags[0].flag_type, FlagType::CppIfdef);
        assert!(!flags[0].is_always_dead);
        assert_eq!(flags[1].name, "NDEBUG");
    }

    #[test]
    fn test_cpp_if_defined() {
        let source = "#if defined(MY_FEATURE)\n    do_stuff();\n#endif\n";
        let flags = FeatureFlagDetector::detect_flags(source, "main.cpp", Language::Cpp);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "MY_FEATURE");
        assert_eq!(flags[0].flag_type, FlagType::CppIfdef);
    }

    #[test]
    fn test_cpp_if_zero_always_dead() {
        let source = "#if 0\n    dead_code();\n#endif\n";
        let flags = FeatureFlagDetector::detect_flags(source, "main.c", Language::C);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "0");
        assert!(flags[0].is_always_dead, "#if 0 should be always dead");
    }

    #[test]
    fn test_cpp_always_dead_filter() {
        let source = "#ifdef FOO\nstuff();\n#endif\n#if 0\ndead();\n#endif\n";
        let flags = FeatureFlagDetector::detect_flags(source, "main.c", Language::C);
        let dead = FeatureFlagDetector::find_always_dead_flags(&flags);
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].name, "0");
    }

    // -------------------------------------------------------------------------
    // Python feature flag patterns
    // -------------------------------------------------------------------------

    #[test]
    fn test_python_environ_get() {
        let source = r#"
if os.environ.get("FEATURE_X"):
    enable_feature()
"#;
        let flags = FeatureFlagDetector::detect_flags(source, "app.py", Language::Python);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "FEATURE_X");
        assert_eq!(flags[0].flag_type, FlagType::EnvironmentVariable);
    }

    #[test]
    fn test_python_environ_bracket() {
        let source = "val = os.environ[\"MY_VAR\"]\n";
        let flags = FeatureFlagDetector::detect_flags(source, "app.py", Language::Python);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "MY_VAR");
        assert_eq!(flags[0].flag_type, FlagType::EnvironmentVariable);
    }

    #[test]
    fn test_python_settings_flag() {
        let source = "if settings.FEATURE_FLAG:\n    do_thing()\n";
        let flags = FeatureFlagDetector::detect_flags(source, "views.py", Language::Python);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "FEATURE_FLAG");
        assert_eq!(flags[0].flag_type, FlagType::ConditionalFeatureCheck);
    }

    #[test]
    fn test_python_if_debug() {
        let source = "if DEBUG:\n    print('debug mode')\n";
        let flags = FeatureFlagDetector::detect_flags(source, "app.py", Language::Python);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "DEBUG");
        assert_eq!(flags[0].flag_type, FlagType::ConditionalFeatureCheck);
        assert!(!flags[0].is_always_dead);
    }

    #[test]
    fn test_python_if_false_always_dead() {
        let source = "if False:\n    dead_code()\n";
        let flags = FeatureFlagDetector::detect_flags(source, "app.py", Language::Python);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "False");
        assert!(flags[0].is_always_dead);
    }

    #[test]
    fn test_python_if_zero_always_dead() {
        let source = "if 0:\n    dead_code()\n";
        let flags = FeatureFlagDetector::detect_flags(source, "app.py", Language::Python);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "0");
        assert!(flags[0].is_always_dead);
    }

    // -------------------------------------------------------------------------
    // JavaScript / TypeScript feature flag patterns
    // -------------------------------------------------------------------------

    #[test]
    fn test_js_process_env() {
        let source = "if (process.env.ENABLE_CACHE) {\n    setupCache();\n}\n";
        let flags = FeatureFlagDetector::detect_flags(source, "app.js", Language::JavaScript);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "ENABLE_CACHE");
        assert_eq!(flags[0].flag_type, FlagType::EnvironmentVariable);
    }

    #[test]
    fn test_js_config_flag() {
        let source = "if (config.featureFlag) {\n    doSomething();\n}\n";
        let flags = FeatureFlagDetector::detect_flags(source, "app.js", Language::JavaScript);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "featureFlag");
        assert_eq!(flags[0].flag_type, FlagType::ConditionalFeatureCheck);
    }

    #[test]
    fn test_ts_import_meta_env() {
        let source = "if (import.meta.env.VITE_FEATURE) {\n    activate();\n}\n";
        let flags = FeatureFlagDetector::detect_flags(source, "app.ts", Language::TypeScript);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "VITE_FEATURE");
        assert_eq!(flags[0].flag_type, FlagType::EnvironmentVariable);
    }

    #[test]
    fn test_ts_process_env() {
        let source = "const x = process.env.NODE_ENV;\n";
        let flags = FeatureFlagDetector::detect_flags(source, "server.ts", Language::TypeScript);
        assert_eq!(flags.len(), 1);
        assert_eq!(flags[0].name, "NODE_ENV");
    }

    // -------------------------------------------------------------------------
    // Always-dead detection (cross-language)
    // -------------------------------------------------------------------------

    #[test]
    fn test_find_always_dead_flags_mixed() {
        let mut flags = vec![
            FeatureFlag {
                name: "FOO".to_string(),
                file: "a.c".to_string(),
                line: 1,
                flag_type: FlagType::CppIfdef,
                is_always_dead: false,
            },
            FeatureFlag {
                name: "0".to_string(),
                file: "a.c".to_string(),
                line: 5,
                flag_type: FlagType::CppIfdef,
                is_always_dead: true,
            },
            FeatureFlag {
                name: "False".to_string(),
                file: "b.py".to_string(),
                line: 3,
                flag_type: FlagType::ConditionalFeatureCheck,
                is_always_dead: true,
            },
        ];
        let dead = FeatureFlagDetector::find_always_dead_flags(&flags);
        assert_eq!(dead.len(), 2);
        assert!(dead.iter().all(|f| f.is_always_dead));

        // Mutate to verify independence
        flags[1].is_always_dead = false;
        let dead2 = FeatureFlagDetector::find_always_dead_flags(&flags);
        assert_eq!(dead2.len(), 1);
    }

    // -------------------------------------------------------------------------
    // Conditional block detection
    // -------------------------------------------------------------------------

    #[test]
    fn test_rust_conditional_blocks() {
        let source = r#"
#[cfg(feature = "serde")]
fn serialize() {
    do_serialize();
}
"#;
        let blocks = FeatureFlagDetector::find_conditional_blocks(source, "lib.rs", Language::Rust);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].flag.name, r#"feature = "serde""#);
        assert_eq!(blocks[0].start_line, 2);
        assert_eq!(blocks[0].end_line, 5);
        assert!(blocks[0].lines_of_code >= 3); // attribute + fn + body + closing brace
    }

    #[test]
    fn test_cpp_conditional_blocks() {
        let source = "\
#if 0
    dead_code();
    more_dead();
#endif
";
        let blocks = FeatureFlagDetector::find_conditional_blocks(source, "main.c", Language::C);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].flag.name, "0");
        assert!(blocks[0].flag.is_always_dead);
        assert_eq!(blocks[0].start_line, 1);
        assert_eq!(blocks[0].end_line, 4);
        assert!(blocks[0].lines_of_code >= 3);
    }

    #[test]
    fn test_cpp_conditional_block_with_else() {
        let source = "\
#if 0
    dead_code();
#else
    live_code();
#endif
";
        let blocks = FeatureFlagDetector::find_conditional_blocks(source, "main.c", Language::C);
        assert_eq!(blocks.len(), 1);
        // The dead region should end at #else, not at #endif
        assert_eq!(blocks[0].start_line, 1);
        assert_eq!(blocks[0].end_line, 3);
    }

    #[test]
    fn test_python_conditional_blocks() {
        let source = "\
if False:
    dead_a()
    dead_b()
live_code()
";
        let blocks =
            FeatureFlagDetector::find_conditional_blocks(source, "app.py", Language::Python);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start_line, 1);
        assert_eq!(blocks[0].end_line, 3);
        assert_eq!(blocks[0].lines_of_code, 3); // if False: + 2 body lines
    }

    #[test]
    fn test_js_conditional_blocks() {
        let source = "\
if (process.env.FEATURE) {
    enableFeature();
    setupStuff();
}
";
        let blocks =
            FeatureFlagDetector::find_conditional_blocks(source, "app.js", Language::JavaScript);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].start_line, 1);
        assert_eq!(blocks[0].end_line, 4);
        assert_eq!(blocks[0].lines_of_code, 4);
    }

    // -------------------------------------------------------------------------
    // Unsupported languages return empty
    // -------------------------------------------------------------------------

    #[test]
    fn test_unsupported_language() {
        let flags = FeatureFlagDetector::detect_flags("fn main() {}", "foo.go", Language::Go);
        assert!(flags.is_empty());
        let blocks =
            FeatureFlagDetector::find_conditional_blocks("fn main() {}", "foo.go", Language::Go);
        assert!(blocks.is_empty());
    }

    // -------------------------------------------------------------------------
    // Edge cases
    // -------------------------------------------------------------------------

    #[test]
    fn test_empty_source() {
        let flags = FeatureFlagDetector::detect_flags("", "empty.rs", Language::Rust);
        assert!(flags.is_empty());
    }

    #[test]
    fn test_no_flags_in_source() {
        let source = "fn main() {\n    println!(\"Hello\");\n}\n";
        let flags = FeatureFlagDetector::detect_flags(source, "main.rs", Language::Rust);
        assert!(flags.is_empty());
    }

    #[test]
    fn test_multiple_flags_same_file() {
        let source = r#"
#[cfg(feature = "a")]
fn feat_a() {}

#[cfg(feature = "b")]
fn feat_b() {}

#[cfg(test)]
mod tests {}
"#;
        let flags = FeatureFlagDetector::detect_flags(source, "lib.rs", Language::Rust);
        assert_eq!(flags.len(), 3);
        assert!(flags.iter().all(|f| f.file == "lib.rs"));
    }

    #[test]
    fn test_cpp_nested_ifdef() {
        let source = "\
#ifdef OUTER
    #ifdef INNER
        inner_code();
    #endif
    outer_code();
#endif
";
        let flags = FeatureFlagDetector::detect_flags(source, "test.c", Language::C);
        assert_eq!(flags.len(), 2);
        assert_eq!(flags[0].name, "OUTER");
        assert_eq!(flags[1].name, "INNER");

        let blocks = FeatureFlagDetector::find_conditional_blocks(source, "test.c", Language::C);
        assert_eq!(blocks.len(), 2);
        // Outer block should span from #ifdef OUTER to the matching #endif
        assert_eq!(blocks[0].start_line, 1);
        assert_eq!(blocks[0].end_line, 6);
    }
}
