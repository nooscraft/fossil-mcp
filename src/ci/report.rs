//! Check report formatting: human-readable summaries and GitHub Actions annotations.

use super::CheckResult;
use crate::cli::C;

/// Format a check result as human-readable text.
pub fn format_text(result: &CheckResult, use_colors: bool) -> String {
    let colors = C { enabled: use_colors };
    let mut output = String::new();

    // Status line
    let status = if result.passed {
        colors.green("✓ PASS")
    } else {
        colors.red("✗ FAIL")
    };
    output.push_str(&format!("{}\n", status));

    // Summary line
    output.push_str(&format!(
        "  Dead code: {} | Clones: {} | Scaffolding: {}\n",
        colors.cyan(&result.dead_code_count.to_string()),
        colors.cyan(&result.clone_count.to_string()),
        colors.cyan(&result.scaffolding_count.to_string())
    ));

    // Diff scope if present
    if let Some(ref diff) = result.diff_scope {
        output.push_str(&format!(
            "  Diff scope: {} ({} files changed)\n",
            colors.dim(&diff.base_branch),
            diff.total_changed
        ));
    }

    // Violations
    if !result.violations.is_empty() {
        output.push_str(&colors.red("\n✗ Threshold violations:\n"));
        for violation in &result.violations {
            output.push_str(&format!(
                "  - {}: {} (threshold: {})\n",
                colors.yellow(&violation.category),
                colors.red(&violation.actual.to_string()),
                colors.dim(&violation.threshold.to_string())
            ));
        }
    }

    output
}

/// Format a check result as GitHub Actions annotations.
/// Only emits annotations if $GITHUB_ACTIONS env var is set.
pub fn format_github_actions(result: &CheckResult) -> String {
    if std::env::var("GITHUB_ACTIONS").is_err() {
        return String::new();
    }

    let mut output = String::new();

    if !result.passed {
        for violation in &result.violations {
            output.push_str(&format!(
                "::error::Fossil check failed: {} (actual: {}, threshold: {})\n",
                violation.message, violation.actual, violation.threshold
            ));
        }
    }

    output
}

/// Format check result summary with optional GitHub Actions annotations.
pub fn format_summary(result: &CheckResult, use_colors: bool) -> String {
    let mut output = format_text(result, use_colors);
    output.push_str(&format_github_actions(result));
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_text_pass() {
        let result = CheckResult {
            dead_code_count: 0,
            clone_count: 0,
            scaffolding_count: 0,
            findings: vec![],
            violations: vec![],
            passed: true,
            diff_scope: None,
        };

        let output = format_text(&result, false);
        assert!(output.contains("PASS"));
        assert!(output.contains("0"));
    }

    #[test]
    fn test_format_text_fail() {
        let result = CheckResult {
            dead_code_count: 5,
            clone_count: 2,
            scaffolding_count: 0,
            findings: vec![],
            violations: vec![
                crate::ci::ThresholdViolation {
                    category: "dead_code".to_string(),
                    threshold: 0,
                    actual: 5,
                    message: "Dead code exceeds threshold".to_string(),
                },
            ],
            passed: false,
            diff_scope: None,
        };

        let output = format_text(&result, false);
        assert!(output.contains("FAIL"));
        assert!(output.contains("5"));
        assert!(output.contains("Threshold violations"));
    }

    #[test]
    fn test_format_text_with_diff_scope() {
        let result = CheckResult {
            dead_code_count: 1,
            clone_count: 0,
            scaffolding_count: 0,
            findings: vec![],
            violations: vec![],
            passed: true,
            diff_scope: Some(crate::ci::DiffScope {
                base_branch: "main".to_string(),
                changed_files: vec!["src/main.rs".to_string()],
                total_changed: 1,
            }),
        };

        let output = format_text(&result, false);
        assert!(output.contains("main"));
        assert!(output.contains("1 files changed"));
    }
}
