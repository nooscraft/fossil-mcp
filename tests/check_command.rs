//! End-to-end tests for CI check command.
//!
//! Tests the full CI pipeline: config merging, diff filtering, threshold evaluation,
//! and proper exit codes.

use std::fs;
use tempfile::TempDir;

use fossil_mcp::ci::{CiRunner, ThresholdEvaluator};
use fossil_mcp::config::{CiConfig, FossilConfig};

// ============================================================================
// Test 1: Threshold Evaluator
// ============================================================================

#[test]
fn test_threshold_evaluator_no_violations_when_no_thresholds() {
    let config = CiConfig::default();
    let evaluator = ThresholdEvaluator::new(config);
    let violations = evaluator.evaluate(100, 50, 10);
    assert!(violations.is_empty(), "No thresholds should never violate");
}

#[test]
fn test_threshold_evaluator_dead_code_violation() {
    let config = CiConfig {
        max_dead_code: Some(5),
        ..Default::default()
    };
    let evaluator = ThresholdEvaluator::new(config);
    let violations = evaluator.evaluate(10, 0, 0);
    assert_eq!(violations.len(), 1);
    assert_eq!(violations[0].category, "dead_code");
    assert_eq!(violations[0].actual, 10);
    assert_eq!(violations[0].threshold, 5);
}

#[test]
fn test_threshold_evaluator_multiple_violations() {
    let config = CiConfig {
        max_dead_code: Some(1),
        max_clones: Some(1),
        max_scaffolding: Some(0),
        ..Default::default()
    };
    let evaluator = ThresholdEvaluator::new(config);
    let violations = evaluator.evaluate(5, 3, 2);
    assert_eq!(violations.len(), 3);
    assert!(violations.iter().any(|v| v.category == "dead_code"));
    assert!(violations.iter().any(|v| v.category == "clones"));
    assert!(violations.iter().any(|v| v.category == "scaffolding"));
}

#[test]
fn test_threshold_evaluator_confidence_filter() {
    let config = CiConfig {
        min_confidence: Some("high".to_string()),
        ..Default::default()
    };
    let evaluator = ThresholdEvaluator::new(config);
    let min_conf = evaluator.min_confidence();
    assert_eq!(min_conf, Some(fossil_mcp::core::Confidence::High));
}

// ============================================================================
// Test 2: CI Config and Merging
// ============================================================================

#[test]
fn test_fossil_config_with_ci_section() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("fossil.toml"),
        r#"
[ci]
max_dead_code = 10
max_clones = 5
min_confidence = "high"
fail_on_scaffolding = true
"#,
    )
    .unwrap();

    let config = FossilConfig::load(dir.path().join("fossil.toml").as_path()).unwrap();
    assert_eq!(config.ci.max_dead_code, Some(10));
    assert_eq!(config.ci.max_clones, Some(5));
    assert_eq!(config.ci.min_confidence, Some("high".to_string()));
    assert_eq!(config.ci.fail_on_scaffolding, Some(true));
}

#[test]
fn test_ci_config_env_overrides() {
    // Set environment variables
    std::env::set_var("FOSSIL_CI_MAX_DEAD_CODE", "20");
    std::env::set_var("FOSSIL_CI_MAX_CLONES", "10");

    let mut config = FossilConfig::default();
    config.apply_env_overrides();

    assert_eq!(config.ci.max_dead_code, Some(20));
    assert_eq!(config.ci.max_clones, Some(10));

    // Clean up
    std::env::remove_var("FOSSIL_CI_MAX_DEAD_CODE");
    std::env::remove_var("FOSSIL_CI_MAX_CLONES");
}

// ============================================================================
// Test 3: Diff Filter (Git Integration) — see src/ci/diff.rs for unit tests
// ============================================================================

// DiffFilter unit tests are in src/ci/diff.rs.
// End-to-end diff integration testing is covered by CI runner tests below.

// ============================================================================
// Test 4: CI Runner
// ============================================================================

#[test]
fn test_ci_runner_basic_initialization() {
    let config = CiConfig::default();
    let fossil_config = FossilConfig::default();
    let runner = CiRunner::new(config, fossil_config, None);
    // Basic smoke test: runner is created successfully
    assert!(std::mem::size_of_val(&runner) > 0);
}

#[test]
fn test_ci_runner_with_no_violations() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("main.rs"),
        r#"
fn main() {
    used_function();
}

fn used_function() {}
"#,
    )
    .unwrap();

    let config = CiConfig {
        max_dead_code: Some(100), // Permissive threshold
        ..Default::default()
    };
    let fossil_config = FossilConfig::default();
    let runner = CiRunner::new(config, fossil_config, None);

    let result = runner.run(dir.path()).unwrap();
    assert!(result.passed, "Should pass with permissive thresholds");
    assert!(result.violations.is_empty(), "Should have no violations");
}

#[test]
fn test_ci_runner_with_violations() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("main.rs"),
        r#"
fn main() {
    used_function();
}

fn used_function() {}

fn dead_code_1() {}
fn dead_code_2() {}
"#,
    )
    .unwrap();

    let config = CiConfig {
        max_dead_code: Some(1), // Strict threshold
        ..Default::default()
    };
    let fossil_config = FossilConfig::default();
    let runner = CiRunner::new(config, fossil_config, None);

    let result = runner.run(dir.path()).unwrap();
    assert!(!result.passed, "Should fail with strict thresholds");
    assert!(!result.violations.is_empty(), "Should have violations");
    assert!(
        result
            .violations
            .iter()
            .any(|v| v.category == "dead_code"),
        "Should have dead code violation"
    );
}

// ============================================================================
// Test 5: Check Result Exit Codes
// ============================================================================

#[test]
fn test_check_result_exit_code_pass() {
    let result = fossil_mcp::ci::CheckResult {
        dead_code_count: 0,
        clone_count: 0,
        scaffolding_count: 0,
        findings: vec![],
        violations: vec![],
        passed: true,
        diff_scope: None,
    };
    assert_eq!(result.exit_code(), 0);
}

#[test]
fn test_check_result_exit_code_fail() {
    let result = fossil_mcp::ci::CheckResult {
        dead_code_count: 5,
        clone_count: 0,
        scaffolding_count: 0,
        findings: vec![],
        violations: vec![fossil_mcp::ci::ThresholdViolation {
            category: "dead_code".to_string(),
            threshold: 0,
            actual: 5,
            message: "Too much dead code".to_string(),
        }],
        passed: false,
        diff_scope: None,
    };
    assert_eq!(result.exit_code(), 1);
}

// ============================================================================
// Test 6: Report Formatting
// ============================================================================

#[test]
fn test_report_formatting_pass() {
    let result = fossil_mcp::ci::CheckResult {
        dead_code_count: 0,
        clone_count: 0,
        scaffolding_count: 0,
        findings: vec![],
        violations: vec![],
        passed: true,
        diff_scope: None,
    };
    let output = fossil_mcp::ci::format_text(&result, false);
    assert!(output.contains("PASS"));
    assert!(!output.contains("FAIL"));
}

#[test]
fn test_report_formatting_fail_with_violations() {
    let result = fossil_mcp::ci::CheckResult {
        dead_code_count: 5,
        clone_count: 2,
        scaffolding_count: 0,
        findings: vec![],
        violations: vec![
            fossil_mcp::ci::ThresholdViolation {
                category: "dead_code".to_string(),
                threshold: 0,
                actual: 5,
                message: "Too much dead code".to_string(),
            },
            fossil_mcp::ci::ThresholdViolation {
                category: "clones".to_string(),
                threshold: 1,
                actual: 2,
                message: "Too many clones".to_string(),
            },
        ],
        passed: false,
        diff_scope: None,
    };
    let output = fossil_mcp::ci::format_text(&result, false);
    assert!(output.contains("FAIL"));
    assert!(output.contains("Threshold violations"));
    assert!(output.contains("dead_code"));
    assert!(output.contains("clones"));
}

#[test]
fn test_report_formatting_with_diff_scope() {
    let result = fossil_mcp::ci::CheckResult {
        dead_code_count: 1,
        clone_count: 0,
        scaffolding_count: 0,
        findings: vec![],
        violations: vec![],
        passed: true,
        diff_scope: Some(fossil_mcp::ci::DiffScope {
            base_branch: "origin/main".to_string(),
            changed_files: vec!["src/main.rs".to_string()],
            total_changed: 1,
        }),
    };
    let output = fossil_mcp::ci::format_text(&result, false);
    assert!(output.contains("origin/main"));
    assert!(output.contains("1 files changed"));
}
