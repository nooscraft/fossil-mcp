//! Semgrep YAML rule format converter.
//!
//! Converts Semgrep-format rules into Fossil's internal Rule format.
//! Supports: pattern, patterns, pattern-not, pattern-inside, metavariable-regex.

use crate::core::{Confidence, Language, PatternType, Rule, Severity};
use serde::Deserialize;

// =============================================================================
// Format detection
// =============================================================================

/// Detected rule format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleFormat {
    /// Native Fossil rule format (has `pattern_type` field).
    Fossil,
    /// Semgrep rule format (has `message` / `pattern` / `patterns` / `languages`).
    Semgrep,
}

/// Auto-detect whether YAML content is Fossil or Semgrep format.
///
/// Heuristic:
/// - Fossil rules contain `pattern_type:` at the item level.
/// - Semgrep rules contain `message:` and either `pattern:`, `patterns:`,
///   `pattern-either:` or `pattern-regex:` at the item level.
pub fn detect_format(yaml: &str) -> RuleFormat {
    // Quick string-level heuristic -- avoids a full parse.
    let has_pattern_type = yaml.contains("pattern_type:");
    let has_message = yaml.contains("message:");
    let has_semgrep_keys = yaml.contains("pattern-either:")
        || yaml.contains("pattern-inside:")
        || yaml.contains("pattern-not:")
        || yaml.contains("pattern-regex:")
        || yaml.contains("metavariable-regex:");

    // Presence of `pattern_type:` is unique to Fossil; Semgrep never uses it.
    if has_pattern_type && !has_semgrep_keys {
        return RuleFormat::Fossil;
    }

    // If the file has `message:` (Semgrep-specific description field) together
    // with Semgrep-only combinators, it is clearly Semgrep.
    if has_message && has_semgrep_keys {
        return RuleFormat::Semgrep;
    }

    // If there is `message:` but no `pattern_type:`, lean toward Semgrep.
    if has_message && !has_pattern_type {
        return RuleFormat::Semgrep;
    }

    // Default: Fossil
    RuleFormat::Fossil
}

// =============================================================================
// Semgrep YAML deserialization types
// =============================================================================

#[derive(Debug, Deserialize)]
struct SemgrepRuleFile {
    rules: Vec<SemgrepRule>,
}

#[derive(Debug, Deserialize)]
struct SemgrepRule {
    id: String,
    #[serde(default)]
    message: String,
    #[serde(default = "default_semgrep_severity")]
    severity: String,
    #[serde(default)]
    languages: Vec<String>,

    // Pattern variants -- Semgrep rules must have exactly one of these.
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    patterns: Option<Vec<SemgrepPatternItem>>,
    #[serde(default, rename = "pattern-either")]
    pattern_either: Option<Vec<SemgrepPatternItem>>,
    #[serde(default, rename = "pattern-regex")]
    pattern_regex: Option<String>,

    // Metadata
    #[serde(default)]
    metadata: Option<SemgrepMetadata>,

    // Fix suggestion
    #[serde(default)]
    fix: Option<String>,
}

fn default_semgrep_severity() -> String {
    "WARNING".to_string()
}

/// A single condition inside `patterns:` or `pattern-either:`.
#[derive(Debug, Deserialize)]
struct SemgrepPatternItem {
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default, rename = "pattern-not")]
    pattern_not: Option<String>,
    #[serde(default, rename = "pattern-inside")]
    pattern_inside: Option<String>,
    #[serde(default, rename = "pattern-not-inside")]
    pattern_not_inside: Option<String>,
    #[serde(default, rename = "pattern-regex")]
    pattern_regex: Option<String>,
    #[serde(default, rename = "pattern-either")]
    pattern_either: Option<Vec<SemgrepPatternItem>>,
    #[serde(default, rename = "metavariable-regex")]
    metavariable_regex: Option<MetavariableRegex>,
}

#[derive(Debug, Deserialize)]
struct MetavariableRegex {
    /// The metavariable this constraint applies to (e.g. `$FUNC`).
    /// Kept for potential future use and full deserialization fidelity.
    #[serde(default)]
    #[allow(dead_code)]
    metavariable: String,
    #[serde(default)]
    regex: String,
}

#[derive(Debug, Deserialize)]
struct SemgrepMetadata {
    #[serde(default)]
    cwe: Option<SemgrepCwe>,
    #[serde(default)]
    owasp: Option<SemgrepOwasp>,
    #[serde(default)]
    confidence: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    subcategory: Option<Vec<String>>,
}

/// CWE can be a single string or a list of strings.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SemgrepCwe {
    Single(String),
    List(Vec<String>),
}

/// OWASP can be a single string or a list of strings.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SemgrepOwasp {
    Single(String),
    List(Vec<String>),
}

// =============================================================================
// Converter
// =============================================================================

/// Converts Semgrep YAML rules to Fossil's internal [`Rule`] format.
///
/// # Conversion details
///
/// | Semgrep                       | Fossil                                         |
/// |-------------------------------|-------------------------------------------------|
/// | `$VARNAME`                    | `(?P<VARNAME>[a-zA-Z_]\w*)`                    |
/// | `...`                         | `[\s\S]*?`                                     |
/// | `severity: ERROR`             | `Severity::Critical`                            |
/// | `severity: WARNING`           | `Severity::High`                                |
/// | `severity: INFO`              | `Severity::Medium`                              |
/// | `patterns:` (conjunction)     | Combined with `(?=[\s\S]*pattern)` lookaheads   |
/// | `pattern-either:` (disjoin)   | Combined with `(?:p1|p2|...)`                   |
/// | `pattern-not:`                | Wrapped in `(?![\s\S]*pattern)`                 |
/// | `metadata.cwe`                | `rule.cwe`                                      |
/// | `metadata.owasp`              | `rule.owasp`                                    |
/// | `fix:`                        | `rule.fix_suggestion`                           |
pub struct SemgrepConverter;

impl SemgrepConverter {
    /// Convert Semgrep YAML content to a list of Fossil [`Rule`]s.
    pub fn convert(yaml: &str) -> Result<Vec<Rule>, crate::core::Error> {
        let file: SemgrepRuleFile = serde_yaml_ng::from_str(yaml)
            .map_err(|e| crate::core::Error::config(format!("Semgrep YAML parse error: {e}")))?;

        let mut rules = Vec::with_capacity(file.rules.len());
        for semgrep_rule in file.rules {
            let rule = Self::convert_rule(semgrep_rule)?;
            rules.push(rule);
        }
        Ok(rules)
    }

    /// Convert a single Semgrep rule to a Fossil [`Rule`].
    fn convert_rule(rule: SemgrepRule) -> Result<Rule, crate::core::Error> {
        let severity = Self::map_severity(&rule.severity);
        let languages = Self::map_languages(&rule.languages);
        let (pattern, pattern_type) = Self::build_pattern(&rule)?;

        let (cwe, owasp, confidence, tags) = Self::extract_metadata(&rule.metadata);

        let id = format!("semgrep-{}", rule.id);

        Ok(Rule {
            id,
            name: rule.id.clone(),
            description: rule.message,
            severity,
            confidence,
            languages,
            pattern,
            pattern_type,
            cwe,
            owasp,
            fix_suggestion: rule.fix,
            tags,
            enabled: true,
            cvss_score: None,
            cve_references: Vec::new(),
        })
    }

    // -------------------------------------------------------------------------
    // Severity mapping
    // -------------------------------------------------------------------------

    fn map_severity(s: &str) -> Severity {
        match s.to_uppercase().as_str() {
            "ERROR" => Severity::Critical,
            "WARNING" => Severity::High,
            "INFO" => Severity::Medium,
            _ => Severity::Medium,
        }
    }

    // -------------------------------------------------------------------------
    // Language mapping
    // -------------------------------------------------------------------------

    fn map_languages(langs: &[String]) -> Vec<Language> {
        langs
            .iter()
            .filter_map(|l| match l.to_lowercase().as_str() {
                "python" | "py" => Some(Language::Python),
                "javascript" | "js" => Some(Language::JavaScript),
                "typescript" | "ts" => Some(Language::TypeScript),
                "java" => Some(Language::Java),
                "go" | "golang" => Some(Language::Go),
                "rust" => Some(Language::Rust),
                "csharp" | "c#" => Some(Language::CSharp),
                "ruby" | "rb" => Some(Language::Ruby),
                "php" => Some(Language::PHP),
                "cpp" | "c++" => Some(Language::Cpp),
                "c" => Some(Language::C),
                "kotlin" | "kt" => Some(Language::Kotlin),
                "swift" => Some(Language::Swift),
                "bash" | "shell" | "sh" => Some(Language::Bash),
                "scala" => Some(Language::Scala),
                "dart" => Some(Language::Dart),
                // Semgrep supports more languages; we silently drop unsupported ones.
                _ => None,
            })
            .collect()
    }

    // -------------------------------------------------------------------------
    // Pattern building
    // -------------------------------------------------------------------------

    fn build_pattern(rule: &SemgrepRule) -> Result<(String, PatternType), crate::core::Error> {
        // 1. Explicit `pattern-regex:` -> use as-is (already a regex).
        if let Some(ref pr) = rule.pattern_regex {
            return Ok((pr.clone(), PatternType::Regex));
        }

        // 2. Simple `pattern:` -> convert Semgrep pattern syntax to regex.
        if let Some(ref p) = rule.pattern {
            let converted = Self::semgrep_pattern_to_regex(p);
            return Ok((converted, PatternType::Regex));
        }

        // 3. `patterns:` (conjunction) -> combine with lookaheads.
        if let Some(ref items) = rule.patterns {
            let regex = Self::patterns_conjunction_to_regex(items)?;
            return Ok((regex, PatternType::Regex));
        }

        // 4. `pattern-either:` (disjunction) -> combine with alternation.
        if let Some(ref items) = rule.pattern_either {
            let regex = Self::pattern_either_to_regex(items)?;
            return Ok((regex, PatternType::Regex));
        }

        Err(crate::core::Error::config(format!(
            "Semgrep rule '{}' has no recognisable pattern field",
            rule.id
        )))
    }

    /// Convert `patterns:` (conjunction) items into a single regex using
    /// positive and negative lookaheads.
    fn patterns_conjunction_to_regex(
        items: &[SemgrepPatternItem],
    ) -> Result<String, crate::core::Error> {
        let mut parts: Vec<String> = Vec::new();

        for item in items {
            if let Some(ref p) = item.pattern {
                let converted = Self::semgrep_pattern_to_regex(p);
                parts.push(format!("(?=[\\s\\S]*{converted})"));
            }
            if let Some(ref p) = item.pattern_not {
                let converted = Self::semgrep_pattern_to_regex(p);
                parts.push(format!("(?![\\s\\S]*{converted})"));
            }
            if let Some(ref p) = item.pattern_inside {
                let converted = Self::semgrep_pattern_to_regex(p);
                parts.push(format!("(?=[\\s\\S]*{converted})"));
            }
            if let Some(ref p) = item.pattern_not_inside {
                let converted = Self::semgrep_pattern_to_regex(p);
                parts.push(format!("(?![\\s\\S]*{converted})"));
            }
            if let Some(ref p) = item.pattern_regex {
                parts.push(format!("(?=[\\s\\S]*{p})"));
            }
            if let Some(ref mvr) = item.metavariable_regex {
                // Embed the metavariable regex constraint directly.
                parts.push(format!("(?=[\\s\\S]*{})", mvr.regex));
            }
            if let Some(ref either) = item.pattern_either {
                let alt = Self::pattern_either_to_regex(either)?;
                parts.push(format!("(?=[\\s\\S]*{alt})"));
            }
        }

        if parts.is_empty() {
            return Ok(".*".to_string());
        }

        // Anchor: all lookaheads must match from start-of-input.
        Ok(parts.join(""))
    }

    /// Convert `pattern-either:` (disjunction) items into a single regex
    /// using alternation.
    fn pattern_either_to_regex(items: &[SemgrepPatternItem]) -> Result<String, crate::core::Error> {
        let mut alternatives: Vec<String> = Vec::new();

        for item in items {
            if let Some(ref p) = item.pattern {
                alternatives.push(Self::semgrep_pattern_to_regex(p));
            }
            if let Some(ref p) = item.pattern_regex {
                alternatives.push(p.clone());
            }
        }

        if alternatives.is_empty() {
            return Ok(".*".to_string());
        }

        Ok(format!("(?:{})", alternatives.join("|")))
    }

    /// Convert a single Semgrep pattern string to a regex.
    ///
    /// Transformations:
    /// - `$VARNAME` -> `(?P<VARNAME>[a-zA-Z_]\w*)`
    /// - `...`      -> `[\s\S]*?`
    /// - Literal special characters are escaped (except the above).
    fn semgrep_pattern_to_regex(pattern: &str) -> String {
        // We process the pattern in chunks, handling `...` and `$VAR` specially.
        let mut result = String::with_capacity(pattern.len() * 2);
        let chars: Vec<char> = pattern.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            // Check for `...` (Semgrep ellipsis).
            if i + 2 < len && chars[i] == '.' && chars[i + 1] == '.' && chars[i + 2] == '.' {
                result.push_str("[\\s\\S]*?");
                i += 3;
                continue;
            }

            // Check for `$VARNAME` (Semgrep metavariable).
            if chars[i] == '$'
                && i + 1 < len
                && (chars[i + 1].is_ascii_uppercase() || chars[i + 1] == '_')
            {
                let start = i + 1;
                let mut end = start;
                while end < len && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
                    end += 1;
                }
                let var_name: String = chars[start..end].iter().collect();
                if var_name.is_empty() {
                    // Just a bare `$` -- escape it.
                    result.push_str("\\$");
                    i += 1;
                } else {
                    result.push_str(&format!("(?P<{var_name}>[a-zA-Z_]\\w*)"));
                    i = end;
                }
                continue;
            }

            // Escape regex-special characters in literal text.
            let c = chars[i];
            if is_regex_meta(c) {
                result.push('\\');
            }
            result.push(c);
            i += 1;
        }

        result
    }

    // -------------------------------------------------------------------------
    // Metadata extraction
    // -------------------------------------------------------------------------

    fn extract_metadata(
        meta: &Option<SemgrepMetadata>,
    ) -> (Option<String>, Option<String>, Confidence, Vec<String>) {
        let Some(meta) = meta else {
            return (None, None, Confidence::Medium, Vec::new());
        };

        // CWE
        let cwe = match &meta.cwe {
            Some(SemgrepCwe::Single(s)) => Some(s.clone()),
            Some(SemgrepCwe::List(v)) => v.first().cloned(),
            None => None,
        };

        // OWASP
        let owasp = match &meta.owasp {
            Some(SemgrepOwasp::Single(s)) => Some(s.clone()),
            Some(SemgrepOwasp::List(v)) => v.first().cloned(),
            None => None,
        };

        // Confidence
        let confidence = meta
            .confidence
            .as_deref()
            .map(|c| match c.to_uppercase().as_str() {
                "HIGH" => Confidence::High,
                "MEDIUM" => Confidence::Medium,
                "LOW" => Confidence::Low,
                _ => Confidence::Medium,
            })
            .unwrap_or(Confidence::Medium);

        // Tags: merge category, subcategory, and explicit tags.
        let mut tags: Vec<String> = Vec::new();
        if let Some(ref t) = meta.tags {
            tags.extend(t.clone());
        }
        if let Some(ref cat) = meta.category {
            if !tags.contains(cat) {
                tags.push(cat.clone());
            }
        }
        if let Some(ref sub) = meta.subcategory {
            for s in sub {
                if !tags.contains(s) {
                    tags.push(s.clone());
                }
            }
        }
        // Always tag converted rules.
        let semgrep_tag = "semgrep-import".to_string();
        if !tags.contains(&semgrep_tag) {
            tags.push(semgrep_tag);
        }

        (cwe, owasp, confidence, tags)
    }
}

// =============================================================================
// Helpers
// =============================================================================

/// Returns `true` if `c` is a regex metacharacter that should be escaped in
/// literal context.
fn is_regex_meta(c: char) -> bool {
    matches!(
        c,
        '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '|'
    )
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;

    // ---- Format detection ---------------------------------------------------

    #[test]
    fn test_detect_fossil_format() {
        let yaml = r#"
rules:
  - id: TEST001
    name: Test
    pattern: "eval\\("
    pattern_type: regex
    severity: high
"#;
        assert_eq!(detect_format(yaml), RuleFormat::Fossil);
    }

    #[test]
    fn test_detect_semgrep_format() {
        let yaml = r#"
rules:
  - id: my-rule
    message: Do not use eval
    severity: ERROR
    languages: [python]
    pattern: eval(...)
"#;
        assert_eq!(detect_format(yaml), RuleFormat::Semgrep);
    }

    #[test]
    fn test_detect_semgrep_with_patterns() {
        let yaml = r#"
rules:
  - id: my-rule
    message: Dangerous
    severity: WARNING
    languages: [python]
    patterns:
      - pattern: eval($X)
      - pattern-not: eval("safe")
"#;
        assert_eq!(detect_format(yaml), RuleFormat::Semgrep);
    }

    // ---- Simple pattern conversion ------------------------------------------

    #[test]
    fn test_convert_simple_rule() {
        let yaml = r#"
rules:
  - id: use-of-eval
    message: Avoid using eval
    severity: ERROR
    languages: [python, javascript]
    pattern: "eval(...)"
"#;
        let rules = SemgrepConverter::convert(yaml).unwrap();
        assert_eq!(rules.len(), 1);

        let rule = &rules[0];
        assert_eq!(rule.id, "semgrep-use-of-eval");
        assert_eq!(rule.name, "use-of-eval");
        assert_eq!(rule.description, "Avoid using eval");
        assert_eq!(rule.severity, Severity::Critical); // ERROR -> Critical
        assert_eq!(rule.languages.len(), 2);
        assert!(rule.languages.contains(&Language::Python));
        assert!(rule.languages.contains(&Language::JavaScript));
        assert_eq!(rule.pattern_type, PatternType::Regex);
        // `eval(...)` -> `eval\([\s\S]*?\)`
        assert!(rule.pattern.contains("eval"));
        assert!(rule.pattern.contains("[\\s\\S]*?"));
    }

    // ---- Metavariable conversion --------------------------------------------

    #[test]
    fn test_convert_metavariable() {
        let yaml = r#"
rules:
  - id: dangerous-call
    message: Dangerous function call
    severity: WARNING
    languages: [python]
    pattern: "dangerous_function($X)"
"#;
        let rules = SemgrepConverter::convert(yaml).unwrap();
        let rule = &rules[0];
        // $X -> (?P<X>[a-zA-Z_]\w*)
        assert!(rule.pattern.contains("(?P<X>[a-zA-Z_]\\w*)"));
    }

    // ---- Severity mapping ---------------------------------------------------

    #[test]
    fn test_severity_mapping() {
        assert_eq!(SemgrepConverter::map_severity("ERROR"), Severity::Critical);
        assert_eq!(SemgrepConverter::map_severity("WARNING"), Severity::High);
        assert_eq!(SemgrepConverter::map_severity("INFO"), Severity::Medium);
        assert_eq!(SemgrepConverter::map_severity("unknown"), Severity::Medium);
    }

    // ---- CWE extraction from metadata ---------------------------------------

    #[test]
    fn test_cwe_extraction_single() {
        let yaml = r#"
rules:
  - id: sqli
    message: SQL injection
    severity: ERROR
    languages: [python]
    pattern: "execute($QUERY)"
    metadata:
      cwe: "CWE-89: SQL Injection"
      owasp: "A03:2021"
      confidence: HIGH
"#;
        let rules = SemgrepConverter::convert(yaml).unwrap();
        let rule = &rules[0];
        assert_eq!(rule.cwe.as_deref(), Some("CWE-89: SQL Injection"));
        assert_eq!(rule.owasp.as_deref(), Some("A03:2021"));
        assert_eq!(rule.confidence, Confidence::High);
    }

    #[test]
    fn test_cwe_extraction_list() {
        let yaml = r#"
rules:
  - id: multi-cwe
    message: Multiple CWEs
    severity: WARNING
    languages: [java]
    pattern: "foo($X)"
    metadata:
      cwe:
        - "CWE-89"
        - "CWE-79"
      owasp:
        - "A03:2021"
        - "A07:2021"
"#;
        let rules = SemgrepConverter::convert(yaml).unwrap();
        let rule = &rules[0];
        // First CWE/OWASP is used.
        assert_eq!(rule.cwe.as_deref(), Some("CWE-89"));
        assert_eq!(rule.owasp.as_deref(), Some("A03:2021"));
    }

    // ---- patterns: conjunction -----------------------------------------------

    #[test]
    fn test_patterns_conjunction() {
        let yaml = r#"
rules:
  - id: conjunction-rule
    message: Conjunction test
    severity: WARNING
    languages: [python]
    patterns:
      - pattern: "os.system($CMD)"
      - pattern-not: 'os.system("safe")'
"#;
        let rules = SemgrepConverter::convert(yaml).unwrap();
        let rule = &rules[0];
        // Should contain positive and negative lookaheads.
        assert!(rule.pattern.contains("(?=[\\s\\S]*"));
        assert!(rule.pattern.contains("(?![\\s\\S]*"));
    }

    // ---- pattern-either: disjunction -----------------------------------------

    #[test]
    fn test_pattern_either() {
        let yaml = r#"
rules:
  - id: either-rule
    message: Either test
    severity: INFO
    languages: [javascript]
    pattern-either:
      - pattern: "eval(...)"
      - pattern: "Function(...)"
"#;
        let rules = SemgrepConverter::convert(yaml).unwrap();
        let rule = &rules[0];
        // Should be wrapped in (?:...|...)
        assert!(rule.pattern.starts_with("(?:"));
        assert!(rule.pattern.contains('|'));
    }

    // ---- pattern-regex: pass-through -----------------------------------------

    #[test]
    fn test_pattern_regex_passthrough() {
        let yaml = r#"
rules:
  - id: regex-rule
    message: Regex test
    severity: WARNING
    languages: [python]
    pattern-regex: "eval\\(.*\\)"
"#;
        let rules = SemgrepConverter::convert(yaml).unwrap();
        let rule = &rules[0];
        assert_eq!(rule.pattern, "eval\\(.*\\)");
    }

    // ---- Fix suggestion mapping ----------------------------------------------

    #[test]
    fn test_fix_suggestion() {
        let yaml = r#"
rules:
  - id: with-fix
    message: Use safe function
    severity: ERROR
    languages: [python]
    pattern: "dangerous($X)"
    fix: "safe($X)"
"#;
        let rules = SemgrepConverter::convert(yaml).unwrap();
        assert_eq!(rules[0].fix_suggestion.as_deref(), Some("safe($X)"));
    }

    // ---- Tags from metadata -------------------------------------------------

    #[test]
    fn test_tags_from_metadata() {
        let yaml = r#"
rules:
  - id: tagged-rule
    message: Tagged
    severity: INFO
    languages: [python]
    pattern: "foo()"
    metadata:
      category: security
      subcategory:
        - audit
        - vuln
      tags:
        - custom-tag
"#;
        let rules = SemgrepConverter::convert(yaml).unwrap();
        let tags = &rules[0].tags;
        assert!(tags.contains(&"custom-tag".to_string()));
        assert!(tags.contains(&"security".to_string()));
        assert!(tags.contains(&"audit".to_string()));
        assert!(tags.contains(&"semgrep-import".to_string()));
    }

    // ---- metavariable-regex inside patterns ----------------------------------

    #[test]
    fn test_metavariable_regex() {
        let yaml = r#"
rules:
  - id: mvar-rule
    message: Metavar regex test
    severity: WARNING
    languages: [python]
    patterns:
      - pattern: "$FUNC($ARG)"
      - metavariable-regex:
          metavariable: "$FUNC"
          regex: "(eval|exec)"
"#;
        let rules = SemgrepConverter::convert(yaml).unwrap();
        let rule = &rules[0];
        // The metavariable-regex constraint should appear in the output.
        assert!(rule.pattern.contains("(eval|exec)"));
    }

    // ---- Edge cases ----------------------------------------------------------

    #[test]
    fn test_empty_rules() {
        let yaml = "rules: []\n";
        let rules = SemgrepConverter::convert(yaml).unwrap();
        assert!(rules.is_empty());
    }

    #[test]
    fn test_semgrep_pattern_to_regex_ellipsis() {
        let result = SemgrepConverter::semgrep_pattern_to_regex("foo(...)");
        assert_eq!(result, "foo\\([\\s\\S]*?\\)");
    }

    #[test]
    fn test_semgrep_pattern_to_regex_metavar() {
        let result = SemgrepConverter::semgrep_pattern_to_regex("$FUNC($ARG)");
        assert_eq!(
            result,
            "(?P<FUNC>[a-zA-Z_]\\w*)\\((?P<ARG>[a-zA-Z_]\\w*)\\)"
        );
    }

    #[test]
    fn test_converted_pattern_is_valid_regex() {
        // Verify that the converted patterns compile as valid regex.
        let patterns = vec![
            "eval(...)",
            "$FUNC($X, $Y)",
            "os.system($CMD)",
            "dangerous_function(...)",
        ];
        for p in patterns {
            let converted = SemgrepConverter::semgrep_pattern_to_regex(p);
            assert!(
                Regex::new(&converted).is_ok(),
                "Pattern '{}' converted to invalid regex: '{}'",
                p,
                converted
            );
        }
    }
}
