//! YAML/JSON rule loader.

use std::path::Path;

use crate::core::{Confidence, Language, PatternType, Rule, Severity};
use serde::Deserialize;
use walkdir::WalkDir;

/// Loads rules from YAML/JSON files.
pub struct RuleLoader;

impl RuleLoader {
    /// Load rules from a directory (recursively scans for .yml/.yaml/.json files).
    pub fn load_from_dir(dir: &Path) -> Result<Vec<Rule>, crate::core::Error> {
        let mut rules = Vec::new();

        if !dir.exists() {
            return Ok(rules);
        }

        for entry in WalkDir::new(dir).follow_links(true).into_iter().flatten() {
            let path = entry.path();
            if path.is_file() {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                match ext {
                    "yml" | "yaml" => {
                        let content = std::fs::read_to_string(path).map_err(|e| {
                            crate::core::Error::config(format!(
                                "Cannot read {}: {e}",
                                path.display()
                            ))
                        })?;
                        let format = crate::rules::semgrep_converter::detect_format(&content);
                        match format {
                            crate::rules::semgrep_converter::RuleFormat::Semgrep => {
                                let loaded =
                                    crate::rules::semgrep_converter::SemgrepConverter::convert(
                                        &content,
                                    )?;
                                rules.extend(loaded);
                            }
                            crate::rules::semgrep_converter::RuleFormat::Fossil => {
                                let loaded = Self::parse_yaml(&content)?;
                                rules.extend(loaded);
                            }
                        }
                    }
                    "json" => {
                        let loaded = Self::load_json_file(path)?;
                        rules.extend(loaded);
                    }
                    _ => {}
                }
            }
        }

        Ok(rules)
    }

    /// Load rules from a single YAML file.
    pub fn load_yaml_file(path: &Path) -> Result<Vec<Rule>, crate::core::Error> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            crate::core::Error::config(format!("Cannot read {}: {e}", path.display()))
        })?;
        Self::parse_yaml(&content)
    }

    /// Load rules from a single JSON file.
    pub fn load_json_file(path: &Path) -> Result<Vec<Rule>, crate::core::Error> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            crate::core::Error::config(format!("Cannot read {}: {e}", path.display()))
        })?;
        Self::parse_json(&content)
    }

    /// Load rules from a Semgrep-format YAML file.
    pub fn load_semgrep_file(path: &Path) -> Result<Vec<Rule>, crate::core::Error> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            crate::core::Error::config(format!("Cannot read {}: {e}", path.display()))
        })?;
        crate::rules::semgrep_converter::SemgrepConverter::convert(&content)
    }

    /// Parse rules from YAML string.
    pub fn parse_yaml(yaml: &str) -> Result<Vec<Rule>, crate::core::Error> {
        let file: RuleFile = serde_yaml_ng::from_str(yaml)
            .map_err(|e| crate::core::Error::config(format!("YAML parse error: {e}")))?;
        Ok(file.rules.into_iter().map(|r| r.into_rule()).collect())
    }

    /// Parse rules from JSON string.
    pub fn parse_json(json: &str) -> Result<Vec<Rule>, crate::core::Error> {
        let file: RuleFile = serde_json::from_str(json)
            .map_err(|e| crate::core::Error::config(format!("JSON parse error: {e}")))?;
        Ok(file.rules.into_iter().map(|r| r.into_rule()).collect())
    }
}

/// Deserialization format for a rule file.
#[derive(Debug, Deserialize)]
struct RuleFile {
    rules: Vec<RuleSpec>,
}

/// Deserialization format for a single rule.
#[derive(Debug, Deserialize)]
struct RuleSpec {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_severity")]
    severity: String,
    #[serde(default = "default_confidence")]
    confidence: String,
    #[serde(default)]
    languages: Vec<String>,
    pattern: String,
    #[serde(default = "default_pattern_type")]
    pattern_type: String,
    #[serde(default)]
    cwe: Option<String>,
    #[serde(default)]
    owasp: Option<String>,
    #[serde(default)]
    fix_suggestion: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

fn default_severity() -> String {
    "medium".to_string()
}
fn default_confidence() -> String {
    "medium".to_string()
}
fn default_pattern_type() -> String {
    "regex".to_string()
}
fn default_true() -> bool {
    true
}

impl RuleSpec {
    fn into_rule(self) -> Rule {
        let severity = match self.severity.to_lowercase().as_str() {
            "critical" => Severity::Critical,
            "high" => Severity::High,
            "medium" => Severity::Medium,
            "low" => Severity::Low,
            "info" => Severity::Info,
            _ => Severity::Medium,
        };

        let confidence = match self.confidence.to_lowercase().as_str() {
            "certain" => Confidence::Certain,
            "high" => Confidence::High,
            "medium" => Confidence::Medium,
            "low" => Confidence::Low,
            _ => Confidence::Medium,
        };

        let pattern_type = match self.pattern_type.to_lowercase().as_str() {
            "regex" => PatternType::Regex,
            "tree-sitter" | "treesitter" => PatternType::TreeSitterQuery,
            "structural" => PatternType::Structural,
            "taint" => PatternType::Taint,
            _ => PatternType::Regex,
        };

        let languages: Vec<Language> = self
            .languages
            .iter()
            .filter_map(|l| match l.to_lowercase().as_str() {
                "python" => Some(Language::Python),
                "javascript" | "js" => Some(Language::JavaScript),
                "typescript" | "ts" => Some(Language::TypeScript),
                "java" => Some(Language::Java),
                "go" => Some(Language::Go),
                "rust" => Some(Language::Rust),
                "csharp" | "c#" => Some(Language::CSharp),
                "ruby" => Some(Language::Ruby),
                "php" => Some(Language::PHP),
                "cpp" | "c++" => Some(Language::Cpp),
                "c" => Some(Language::C),
                "swift" => Some(Language::Swift),
                "bash" | "shell" => Some(Language::Bash),
                "scala" => Some(Language::Scala),
                _ => None,
            })
            .collect();

        Rule {
            id: self.id,
            name: self.name,
            description: self.description,
            severity,
            confidence,
            languages,
            pattern: self.pattern,
            pattern_type,
            cwe: self.cwe,
            owasp: self.owasp,
            fix_suggestion: self.fix_suggestion,
            tags: self.tags,
            enabled: self.enabled,
            cvss_score: None,
            cve_references: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_yaml_rules() {
        let yaml = r#"
rules:
  - id: TEST001
    name: Test Rule
    severity: high
    confidence: high
    languages: [python, javascript]
    pattern: "eval\\("
    cwe: CWE-95
    tags: [injection]
"#;
        let rules = RuleLoader::parse_yaml(yaml).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "TEST001");
        assert_eq!(rules[0].severity, Severity::High);
        assert_eq!(rules[0].languages.len(), 2);
    }

    #[test]
    fn test_parse_json_rules() {
        let json = r#"{
    "rules": [
        {
            "id": "TEST002",
            "name": "JSON Rule",
            "severity": "critical",
            "languages": ["python"],
            "pattern": "pickle\\.loads"
        }
    ]
}"#;
        let rules = RuleLoader::parse_json(json).unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].severity, Severity::Critical);
    }
}
