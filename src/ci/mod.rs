//! CI/CD mode: fail builds on configurable thresholds with diff-aware support.
//!
//! This module provides infrastructure for running Fossil in CI pipelines,
//! allowing teams to gate PRs and block merges based on code quality thresholds.
//!
//! Key features:
//! - Configurable thresholds for dead code, clones, and scaffolding
//! - Diff-aware mode: only check files changed in the current PR (critical for adoption)
//! - GitHub Actions integration via SARIF
//! - Simple exit codes: 0 (pass), 1 (threshold exceeded), 2 (error)

pub mod diff;
pub mod report;
pub mod runner;
pub mod threshold;

// Re-export key types for convenience
pub use diff::DiffFilter;
pub use report::{format_github_actions, format_summary, format_text};
pub use runner::CiRunner;
pub use threshold::ThresholdEvaluator;

use serde::{Deserialize, Serialize};

/// Result of a CI check: pass/fail with violation details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    /// Number of dead code findings in scope.
    pub dead_code_count: usize,

    /// Number of clone groups in scope.
    pub clone_count: usize,

    /// Number of scaffolding artifacts in scope.
    pub scaffolding_count: usize,

    /// All findings (findings filtered by diff and thresholds).
    pub findings: Vec<crate::core::Finding>,

    /// Threshold violations that caused the check to fail.
    pub violations: Vec<ThresholdViolation>,

    /// Whether all thresholds were met (true = pass, false = fail).
    pub passed: bool,

    /// Information about the diff scope (if --diff was used).
    pub diff_scope: Option<DiffScope>,
}

impl CheckResult {
    /// Exit code for this result: 0 (pass), 1 (threshold exceeded), 2 (error).
    pub fn exit_code(&self) -> i32 {
        if self.passed {
            0
        } else {
            1
        }
    }
}

/// A single threshold violation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdViolation {
    /// Category of the violation: "dead_code", "clones", "scaffolding".
    pub category: String,

    /// The threshold that was exceeded.
    pub threshold: usize,

    /// Actual count that exceeded the threshold.
    pub actual: usize,

    /// Human-readable message.
    pub message: String,
}

/// Scope of analysis when using --diff mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffScope {
    /// Base branch (e.g., "main", "origin/main").
    pub base_branch: String,

    /// List of files changed between base and HEAD.
    pub changed_files: Vec<String>,

    /// Total number of changed files.
    pub total_changed: usize,
}
