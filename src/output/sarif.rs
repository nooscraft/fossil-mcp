//! SARIF 2.1.0 output formatter.

use crate::core::{Confidence, Finding, Reporter, Rule, Severity};
use serde_json::{json, Value};

/// Formats findings as SARIF 2.1.0 JSON.
pub struct SarifFormatter {
    tool_name: String,
    tool_version: String,
    rules: Vec<Rule>,
}

impl SarifFormatter {
    pub fn new() -> Self {
        Self {
            tool_name: "fossil".to_string(),
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            rules: Vec::new(),
        }
    }

    /// Attach rule metadata that will be emitted in `tool.driver.rules`.
    pub fn with_rules(mut self, rules: Vec<Rule>) -> Self {
        self.rules = rules;
        self
    }

    /// Map severity to SARIF level string.
    fn severity_to_sarif(severity: Severity) -> &'static str {
        match severity {
            Severity::Critical | Severity::High => "error",
            Severity::Medium => "warning",
            Severity::Low | Severity::Info => "note",
        }
    }

    /// Map severity to a default CVSS-like security-severity string when no
    /// explicit CVSS score is available on the rule.
    fn severity_to_security_severity(severity: Severity) -> &'static str {
        match severity {
            Severity::Critical => "9.0",
            Severity::High => "7.0",
            Severity::Medium => "4.0",
            Severity::Low => "2.0",
            Severity::Info => "0.0",
        }
    }

    /// Map confidence level to a lowercase string for properties.
    fn confidence_to_string(confidence: Confidence) -> &'static str {
        match confidence {
            Confidence::Certain => "certain",
            Confidence::High => "high",
            Confidence::Medium => "medium",
            Confidence::Low => "low",
        }
    }

    /// Build the `tool.driver.rules` array from attached rules.
    fn build_rules_array(&self) -> Vec<Value> {
        self.rules
            .iter()
            .map(|r| {
                let security_severity = match r.cvss_score {
                    Some(score) => format!("{score:.1}"),
                    None => Self::severity_to_security_severity(r.severity).to_string(),
                };

                let mut properties = serde_json::Map::new();
                properties.insert(
                    "security-severity".to_string(),
                    Value::String(security_severity),
                );
                if !r.tags.is_empty() {
                    properties.insert(
                        "tags".to_string(),
                        Value::Array(r.tags.iter().map(|t| Value::String(t.clone())).collect()),
                    );
                }
                if !r.cve_references.is_empty() {
                    properties.insert(
                        "cve-references".to_string(),
                        Value::Array(
                            r.cve_references
                                .iter()
                                .map(|c| Value::String(c.clone()))
                                .collect(),
                        ),
                    );
                }

                let mut rule_obj = serde_json::Map::new();
                rule_obj.insert("id".to_string(), Value::String(r.id.clone()));
                rule_obj.insert("shortDescription".to_string(), json!({ "text": r.name }));
                if !r.description.is_empty() {
                    rule_obj.insert(
                        "fullDescription".to_string(),
                        json!({ "text": r.description }),
                    );
                }
                if let Some(ref fix) = r.fix_suggestion {
                    rule_obj.insert("help".to_string(), json!({ "text": fix }));
                }
                rule_obj.insert("properties".to_string(), Value::Object(properties));

                Value::Object(rule_obj)
            })
            .collect()
    }

    /// Build the `relatedLocations` array for a finding.
    fn build_related_locations(finding: &Finding) -> Vec<Value> {
        finding
            .related_locations
            .iter()
            .enumerate()
            .map(|(idx, loc)| {
                json!({
                    "id": idx,
                    "physicalLocation": {
                        "artifactLocation": {
                            "uri": loc.file
                        },
                        "region": {
                            "startLine": loc.line_start,
                            "startColumn": loc.column_start + 1,
                            "endLine": loc.line_end,
                            "endColumn": loc.column_end + 1
                        }
                    }
                })
            })
            .collect()
    }

    /// Build the `fixes` array for a finding when `fix_text` is present.
    fn build_fixes(finding: &Finding) -> Vec<Value> {
        match finding.fix_text {
            Some(ref text) => {
                vec![json!({
                    "description": {
                        "text": format!("Suggested fix for {}", finding.rule_id)
                    },
                    "artifactChanges": [{
                        "artifactLocation": {
                            "uri": finding.location.file
                        },
                        "replacements": [{
                            "deletedRegion": {
                                "startLine": finding.location.line_start,
                                "startColumn": finding.location.column_start + 1,
                                "endLine": finding.location.line_end,
                                "endColumn": finding.location.column_end + 1
                            },
                            "insertedContent": {
                                "text": text
                            }
                        }]
                    }]
                })]
            }
            None => Vec::new(),
        }
    }

    /// Build the `properties` object for a result.
    fn build_result_properties(finding: &Finding) -> Value {
        let mut props = serde_json::Map::new();
        props.insert(
            "confidence".to_string(),
            Value::String(Self::confidence_to_string(finding.confidence).to_string()),
        );
        if !finding.tags.is_empty() {
            props.insert(
                "tags".to_string(),
                Value::Array(
                    finding
                        .tags
                        .iter()
                        .map(|t| Value::String(t.clone()))
                        .collect(),
                ),
            );
        }
        if let Some(ref cwe) = finding.cwe {
            props.insert("cweId".to_string(), Value::String(cwe.clone()));
        }
        Value::Object(props)
    }

    /// Build a single SARIF result from a finding.
    fn build_result(&self, finding: &Finding) -> Value {
        let mut result = serde_json::Map::new();

        result.insert("ruleId".to_string(), Value::String(finding.rule_id.clone()));
        result.insert(
            "level".to_string(),
            Value::String(Self::severity_to_sarif(finding.severity).to_string()),
        );
        result.insert(
            "message".to_string(),
            json!({ "text": finding.description }),
        );
        result.insert(
            "locations".to_string(),
            json!([{
                "physicalLocation": {
                    "artifactLocation": {
                        "uri": finding.location.file
                    },
                    "region": {
                        "startLine": finding.location.line_start,
                        "startColumn": finding.location.column_start + 1,
                        "endLine": finding.location.line_end,
                        "endColumn": finding.location.column_end + 1
                    }
                }
            }]),
        );

        // Related locations (only when present)
        let related = Self::build_related_locations(finding);
        if !related.is_empty() {
            result.insert("relatedLocations".to_string(), Value::Array(related));
        }

        // Properties (always present with at least confidence)
        result.insert(
            "properties".to_string(),
            Self::build_result_properties(finding),
        );

        // Fixes (only when fix_text is present)
        let fixes = Self::build_fixes(finding);
        if !fixes.is_empty() {
            result.insert("fixes".to_string(), Value::Array(fixes));
        }

        // Rule index for linking results to rules (only when rules are present)
        if !self.rules.is_empty() {
            if let Some(idx) = self.rules.iter().position(|r| r.id == finding.rule_id) {
                result.insert("ruleIndex".to_string(), Value::Number(idx.into()));
            }
        }

        Value::Object(result)
    }

    /// Build the complete SARIF document.
    fn build_sarif(&self, findings: &[Finding]) -> Value {
        let results: Vec<Value> = findings.iter().map(|f| self.build_result(f)).collect();

        let mut driver = serde_json::Map::new();
        driver.insert("name".to_string(), Value::String(self.tool_name.clone()));
        driver.insert(
            "version".to_string(),
            Value::String(self.tool_version.clone()),
        );
        driver.insert(
            "informationUri".to_string(),
            Value::String("https://github.com/user/fossil".to_string()),
        );

        if !self.rules.is_empty() {
            driver.insert("rules".to_string(), Value::Array(self.build_rules_array()));
        }

        json!({
            "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
            "version": "2.1.0",
            "runs": [{
                "tool": {
                    "driver": Value::Object(driver)
                },
                "results": results
            }]
        })
    }
}

impl Default for SarifFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl Reporter for SarifFormatter {
    fn report(&self, findings: &[Finding]) -> crate::core::Result<String> {
        let sarif = self.build_sarif(findings);
        serde_json::to_string_pretty(&sarif)
            .map_err(|e| crate::core::Error::analysis(format!("SARIF serialization error: {e}")))
    }

    fn format_name(&self) -> &str {
        "sarif"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Confidence, Language, SourceLocation};

    fn make_test_rule(id: &str, name: &str, severity: Severity) -> Rule {
        Rule::new(id, name, "test_pattern", severity, vec![Language::Python])
    }

    #[test]
    fn test_sarif_output() {
        let formatter = SarifFormatter::new();
        let findings = vec![Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("User input flows to SQL query")];

        let output = formatter.report(&findings).unwrap();
        assert!(output.contains("sarif-schema-2.1.0"));
        assert!(output.contains("SEC001"));
        assert!(output.contains("error"));
    }

    #[test]
    fn test_sarif_tool_driver_rules() {
        let mut rule = make_test_rule("SEC001", "SQL Injection", Severity::Critical);
        rule.description = "User input used directly in SQL query".to_string();
        rule.fix_suggestion = Some("Use parameterized queries".to_string());
        rule.tags = vec!["sql".to_string(), "injection".to_string()];
        rule.cvss_score = Some(9.8);

        let formatter = SarifFormatter::new().with_rules(vec![rule]);

        let findings = vec![Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("User input flows to SQL query")];

        let output = formatter.report(&findings).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        // Verify tool.driver.rules exists
        let rules = &parsed["runs"][0]["tool"]["driver"]["rules"];
        assert!(rules.is_array());
        assert_eq!(rules.as_array().unwrap().len(), 1);

        let rule_obj = &rules[0];
        assert_eq!(rule_obj["id"], "SEC001");
        assert_eq!(rule_obj["shortDescription"]["text"], "SQL Injection");
        assert_eq!(
            rule_obj["fullDescription"]["text"],
            "User input used directly in SQL query"
        );
        assert_eq!(rule_obj["help"]["text"], "Use parameterized queries");
        assert_eq!(rule_obj["properties"]["security-severity"], "9.8");

        let tags = rule_obj["properties"]["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0], "sql");
        assert_eq!(tags[1], "injection");

        // Verify result has ruleIndex
        let result = &parsed["runs"][0]["results"][0];
        assert_eq!(result["ruleIndex"], 0);
    }

    #[test]
    fn test_sarif_related_locations() {
        let formatter = SarifFormatter::new();

        let related = vec![
            SourceLocation::new("source.py".to_string(), 10, 10, 0, 20),
            SourceLocation::new("sink.py".to_string(), 50, 50, 5, 30),
        ];

        let findings = vec![Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("Taint flow detected")
        .with_related_locations(related)];

        let output = formatter.report(&findings).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        let result = &parsed["runs"][0]["results"][0];
        let related_locs = result["relatedLocations"].as_array().unwrap();
        assert_eq!(related_locs.len(), 2);

        assert_eq!(related_locs[0]["id"], 0);
        assert_eq!(
            related_locs[0]["physicalLocation"]["artifactLocation"]["uri"],
            "source.py"
        );
        assert_eq!(
            related_locs[0]["physicalLocation"]["region"]["startLine"],
            10
        );

        assert_eq!(related_locs[1]["id"], 1);
        assert_eq!(
            related_locs[1]["physicalLocation"]["artifactLocation"]["uri"],
            "sink.py"
        );
        assert_eq!(
            related_locs[1]["physicalLocation"]["region"]["startLine"],
            50
        );
    }

    #[test]
    fn test_sarif_no_related_locations_when_empty() {
        let formatter = SarifFormatter::new();
        let findings = vec![Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("Simple finding")];

        let output = formatter.report(&findings).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        let result = &parsed["runs"][0]["results"][0];
        // relatedLocations should not be present when empty
        assert!(result.get("relatedLocations").is_none());
    }

    #[test]
    fn test_sarif_fixes() {
        let formatter = SarifFormatter::new();

        let findings = vec![Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("User input in query")
        .with_fix_text("cursor.execute(\"SELECT * FROM users WHERE id = %s\", (user_id,))")];

        let output = formatter.report(&findings).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        let result = &parsed["runs"][0]["results"][0];
        let fixes = result["fixes"].as_array().unwrap();
        assert_eq!(fixes.len(), 1);

        let fix = &fixes[0];
        assert!(fix["description"]["text"]
            .as_str()
            .unwrap()
            .contains("SEC001"));

        let replacement = &fix["artifactChanges"][0]["replacements"][0];
        assert_eq!(replacement["deletedRegion"]["startLine"], 5);
        assert_eq!(replacement["deletedRegion"]["startColumn"], 11);
        assert_eq!(replacement["deletedRegion"]["endLine"], 5);
        assert_eq!(replacement["deletedRegion"]["endColumn"], 31);
        assert_eq!(
            replacement["insertedContent"]["text"],
            "cursor.execute(\"SELECT * FROM users WHERE id = %s\", (user_id,))"
        );
    }

    #[test]
    fn test_sarif_no_fixes_when_absent() {
        let formatter = SarifFormatter::new();
        let findings = vec![Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("No fix available")];

        let output = formatter.report(&findings).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        let result = &parsed["runs"][0]["results"][0];
        // fixes should not be present when no fix_text
        assert!(result.get("fixes").is_none());
    }

    #[test]
    fn test_sarif_result_properties() {
        let formatter = SarifFormatter::new();

        let findings = vec![Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("User input flows to SQL query")
        .with_confidence(Confidence::High)];

        let output = formatter.report(&findings).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        let result = &parsed["runs"][0]["results"][0];
        let props = &result["properties"];
        assert_eq!(props["confidence"], "high");
    }

    #[test]
    fn test_sarif_result_properties_with_tags_and_cwe() {
        let formatter = SarifFormatter::new();

        let mut finding = Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("SQL injection detected")
        .with_confidence(Confidence::Certain);
        finding.tags = vec!["sql".to_string(), "injection".to_string()];
        finding.cwe = Some("CWE-89".to_string());

        let output = formatter.report(&[finding]).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        let result = &parsed["runs"][0]["results"][0];
        let props = &result["properties"];
        assert_eq!(props["confidence"], "certain");
        assert_eq!(props["cweId"], "CWE-89");

        let tags = props["tags"].as_array().unwrap();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0], "sql");
        assert_eq!(tags[1], "injection");
    }

    #[test]
    fn test_sarif_severity_mapping() {
        assert_eq!(
            SarifFormatter::severity_to_sarif(Severity::Critical),
            "error"
        );
        assert_eq!(SarifFormatter::severity_to_sarif(Severity::High), "error");
        assert_eq!(
            SarifFormatter::severity_to_sarif(Severity::Medium),
            "warning"
        );
        assert_eq!(SarifFormatter::severity_to_sarif(Severity::Low), "note");
        assert_eq!(SarifFormatter::severity_to_sarif(Severity::Info), "note");
    }

    #[test]
    fn test_sarif_security_severity_mapping() {
        assert_eq!(
            SarifFormatter::severity_to_security_severity(Severity::Critical),
            "9.0"
        );
        assert_eq!(
            SarifFormatter::severity_to_security_severity(Severity::High),
            "7.0"
        );
        assert_eq!(
            SarifFormatter::severity_to_security_severity(Severity::Medium),
            "4.0"
        );
        assert_eq!(
            SarifFormatter::severity_to_security_severity(Severity::Low),
            "2.0"
        );
        assert_eq!(
            SarifFormatter::severity_to_security_severity(Severity::Info),
            "0.0"
        );
    }

    #[test]
    fn test_sarif_cvss_override_in_rules() {
        let mut rule = make_test_rule("SEC001", "SQL Injection", Severity::Critical);
        rule.cvss_score = Some(9.8);

        let formatter = SarifFormatter::new().with_rules(vec![rule]);
        let findings = vec![Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("test")];

        let output = formatter.report(&findings).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        // CVSS score should override severity-based default
        let rule_props = &parsed["runs"][0]["tool"]["driver"]["rules"][0]["properties"];
        assert_eq!(rule_props["security-severity"], "9.8");
    }

    #[test]
    fn test_sarif_cve_references_in_rules() {
        let mut rule = make_test_rule("SEC001", "Log4Shell", Severity::Critical);
        rule.cve_references = vec!["CVE-2021-44228".to_string(), "CVE-2021-45046".to_string()];

        let formatter = SarifFormatter::new().with_rules(vec![rule]);
        let findings = vec![Finding::new(
            "SEC001",
            "Log4Shell",
            Severity::Critical,
            SourceLocation::new("App.java".to_string(), 10, 10, 0, 40),
        )
        .with_description("Log4j usage detected")];

        let output = formatter.report(&findings).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        let rule_props = &parsed["runs"][0]["tool"]["driver"]["rules"][0]["properties"];
        let cves = rule_props["cve-references"].as_array().unwrap();
        assert_eq!(cves.len(), 2);
        assert_eq!(cves[0], "CVE-2021-44228");
        assert_eq!(cves[1], "CVE-2021-45046");
    }

    #[test]
    fn test_sarif_no_rules_section_without_rules() {
        let formatter = SarifFormatter::new(); // no with_rules call
        let findings = vec![Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("test")];

        let output = formatter.report(&findings).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        // No rules section when no rules attached
        assert!(parsed["runs"][0]["tool"]["driver"].get("rules").is_none());
    }

    #[test]
    fn test_sarif_enriched_full_example() {
        // Full integration test: rules + relatedLocations + fixes + properties
        let mut rule = make_test_rule("SEC001", "SQL Injection", Severity::Critical);
        rule.description = "User input used directly in SQL query".to_string();
        rule.fix_suggestion = Some("Use parameterized queries".to_string());
        rule.tags = vec!["sql".to_string(), "injection".to_string()];
        rule.cvss_score = Some(9.8);

        let formatter = SarifFormatter::new().with_rules(vec![rule]);

        let related = vec![SourceLocation::new("source.py".to_string(), 10, 10, 0, 20)];

        let mut finding = Finding::new(
            "SEC001",
            "SQL Injection",
            Severity::Critical,
            SourceLocation::new("test.py".to_string(), 5, 5, 10, 30),
        )
        .with_description("User input flows to SQL query")
        .with_confidence(Confidence::High)
        .with_related_locations(related)
        .with_fix_text("cursor.execute(\"SELECT * FROM users WHERE id = %s\", (user_id,))");
        finding.tags = vec!["sql".to_string(), "injection".to_string()];
        finding.cwe = Some("CWE-89".to_string());

        let output = formatter.report(&[finding]).unwrap();
        let parsed: Value = serde_json::from_str(&output).unwrap();

        // Verify structure
        assert_eq!(parsed["version"], "2.1.0");

        // tool.driver.rules
        let rules = &parsed["runs"][0]["tool"]["driver"]["rules"];
        assert_eq!(rules.as_array().unwrap().len(), 1);
        assert_eq!(rules[0]["properties"]["security-severity"], "9.8");

        // Result
        let result = &parsed["runs"][0]["results"][0];
        assert_eq!(result["ruleId"], "SEC001");
        assert_eq!(result["level"], "error");
        assert_eq!(result["ruleIndex"], 0);

        // Related locations
        assert_eq!(result["relatedLocations"].as_array().unwrap().len(), 1);

        // Fixes
        assert_eq!(result["fixes"].as_array().unwrap().len(), 1);

        // Properties
        assert_eq!(result["properties"]["confidence"], "high");
        assert_eq!(result["properties"]["cweId"], "CWE-89");
        assert_eq!(result["properties"]["tags"].as_array().unwrap().len(), 2);
    }
}
