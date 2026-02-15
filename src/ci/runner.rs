//! CI orchestration: runs analyses and evaluates thresholds.
//!
//! The CiRunner coordinates dead code detection, clone detection, and optional
//! scaffolding detection, then applies threshold evaluation to produce a CheckResult.

use std::collections::HashMap;
use std::path::Path;

use crate::clones::detector::{CloneConfig, CloneDetector};
use crate::config::{CiConfig, FossilConfig};
use crate::core::{Error, Finding};
use crate::dead_code::detector::{Detector as DeadCodeDetector, DetectorConfig};

use super::{CheckResult, DiffFilter, DiffScope, ThresholdEvaluator};

/// Orchestrates CI analysis: dead code → clones → optional scaffolding → thresholds.
pub struct CiRunner {
    config: CiConfig,
    fossil_config: FossilConfig,
    diff_filter: Option<DiffFilter>,
}

impl CiRunner {
    /// Create a new CI runner with optional diff filter.
    pub fn new(
        config: CiConfig,
        fossil_config: FossilConfig,
        diff_filter: Option<DiffFilter>,
    ) -> Self {
        Self {
            config,
            fossil_config,
            diff_filter,
        }
    }

    /// Run the full CI analysis pipeline.
    pub fn run(&self, path: &Path) -> Result<CheckResult, Error> {
        let mut findings = Vec::new();

        // Run dead code detection
        let dead_code_findings = self.detect_dead_code(path)?;
        let dead_code_count = dead_code_findings.len();
        findings.extend(dead_code_findings);

        // Run clone detection
        let clone_findings = self.detect_clones(path)?;
        let clone_count = clone_findings.len();
        findings.extend(clone_findings);

        // Run scaffolding detection
        let scaffolding_count = self.detect_scaffolding(path);

        // Apply confidence filter if configured
        if let Some(min_conf) = ThresholdEvaluator::new(self.config.clone()).min_confidence() {
            findings.retain(|f| f.confidence >= min_conf);
        }

        // Evaluate thresholds
        let evaluator = ThresholdEvaluator::new(self.config.clone());
        let violations = evaluator.evaluate(dead_code_count, clone_count, scaffolding_count);
        let passed = violations.is_empty();

        let diff_scope: Option<DiffScope> = self.diff_filter.as_ref().map(|f| f.scope());

        Ok(CheckResult {
            dead_code_count,
            clone_count,
            scaffolding_count,
            findings,
            violations,
            passed,
            diff_scope,
        })
    }

    /// Run dead code detection, optionally filtered by diff.
    fn detect_dead_code(&self, path: &Path) -> Result<Vec<Finding>, Error> {
        // Build detector config
        let entry_point_rules = crate::config::ResolvedEntryPointRules::from_config(
            &self.fossil_config.entry_points,
            Some(path),
        );

        let config = DetectorConfig {
            include_tests: true,
            min_confidence: crate::core::Confidence::Low,
            min_lines: 0,
            exclude_patterns: Vec::new(),
            detect_dead_stores: true,
            use_rta: true,
            use_sdg: false,
            entry_point_rules: Some(entry_point_rules),
        };

        let detector = DeadCodeDetector::new(config);
        let result = detector.detect(path)?;

        // Convert to findings
        let mut findings: Vec<Finding> = result
            .findings
            .iter()
            .map(|f| {
                let location = crate::core::SourceLocation::new(
                    f.file.clone(),
                    f.line_start,
                    f.line_end,
                    0,
                    0,
                );
                Finding::new(
                    format!("DEAD-{}", f.fossil_type),
                    f.name.clone(),
                    f.severity,
                    location,
                )
                .with_confidence(f.confidence)
                .with_description(f.reason.clone())
            })
            .collect();

        // Filter by diff if configured
        if let Some(ref diff) = self.diff_filter {
            findings.retain(|f| diff.contains(&f.location.file));
        }

        Ok(findings)
    }

    /// Run scaffolding detection, returning the number of findings.
    fn detect_scaffolding(&self, path: &Path) -> usize {
        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            serde_json::Value::String(path.to_string_lossy().to_string()),
        );
        // Include phased comments and placeholders, skip TODOs by default
        args.insert("include_todos".to_string(), serde_json::Value::Bool(false));
        args.insert(
            "include_placeholders".to_string(),
            serde_json::Value::Bool(true),
        );
        args.insert(
            "include_phased_comments".to_string(),
            serde_json::Value::Bool(true),
        );
        args.insert(
            "include_temp_files".to_string(),
            serde_json::Value::Bool(true),
        );

        match crate::mcp::tools::scaffolding::execute_detect_scaffolding(&args) {
            Ok(result) => {
                // Extract total_findings from the nested JSON response
                result
                    .pointer("/content/0/text")
                    .and_then(|v| v.as_str())
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
                    .and_then(|parsed| parsed.get("total_findings")?.as_u64())
                    .unwrap_or(0) as usize
            }
            Err(_) => 0,
        }
    }

    /// Run clone detection, optionally filtered by diff.
    fn detect_clones(&self, path: &Path) -> Result<Vec<Finding>, Error> {
        let config = CloneConfig {
            min_lines: self.fossil_config.clones.min_lines,
            min_nodes: 5,
            similarity_threshold: self.fossil_config.clones.similarity_threshold,
            detect_type1: true,
            detect_type2: true,
            detect_type3: true,
            detect_cross_language: true,
        };

        let detector = CloneDetector::new(config);
        let result = detector.detect(path)?;

        // Convert clone groups to findings
        let mut findings = Vec::new();
        for group in result.groups {
            for (idx, instance) in group.instances.iter().enumerate() {
                // Filter by diff if configured
                if let Some(ref diff) = self.diff_filter {
                    if !diff.contains(&instance.file) {
                        continue;
                    }
                }

                let location = crate::core::SourceLocation::new(
                    instance.file.clone(),
                    instance.start_line,
                    instance.end_line,
                    0,
                    0,
                );

                let finding = Finding::new(
                    format!("CLONE-{:?}", group.clone_type),
                    format!(
                        "Clone instance {} (similarity: {:.0}%)",
                        idx + 1,
                        group.similarity * 100.0
                    ),
                    crate::core::Severity::Medium,
                    location,
                )
                .with_confidence(crate::core::Confidence::High)
                .with_description(format!(
                    "{:?} clone in {} at lines {}-{}",
                    group.clone_type, instance.file, instance.start_line, instance.end_line
                ));

                findings.push(finding);
            }
        }

        Ok(findings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ci_runner_no_diff_filter() {
        let config = CiConfig::default();
        let fossil_config = FossilConfig::default();
        let runner = CiRunner::new(config, fossil_config, None);
        // Basic smoke test: runner is created
        assert!(runner.diff_filter.is_none());
    }
}
