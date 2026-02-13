//! Git diff integration: determine changed files in the current PR.
//!
//! This module handles shell integration with `git diff` to scope analysis to only
//! files that changed in the current PR. Without this, existing tech debt blocks every build.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::core::Error;

use super::DiffScope;

/// Filters findings to only those in files changed vs. a base branch.
pub struct DiffFilter {
    base_branch: String,
    changed_files: Vec<PathBuf>,
}

impl DiffFilter {
    /// Create a new diff filter by running `git diff <base-branch>...HEAD --name-only`.
    pub fn new(base_branch: &str, project_root: &Path) -> Result<Self, Error> {
        let output = Command::new("git")
            .arg("diff")
            .arg(format!("{}...HEAD", base_branch))
            .arg("--name-only")
            .current_dir(project_root)
            .output()
            .map_err(|e| {
                Error::analysis(format!(
                    "Failed to run git diff: {}. Make sure git is installed and you're in a git repository.",
                    e
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::analysis(format!(
                "git diff failed: {}",
                stderr.trim()
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let changed_files: Vec<PathBuf> = stdout
            .lines()
            .map(|line| PathBuf::from(line.trim()))
            .filter(|line| !line.as_os_str().is_empty())
            .collect();

        Ok(Self {
            base_branch: base_branch.to_string(),
            changed_files,
        })
    }

    /// Get the diff scope information.
    pub fn scope(&self) -> DiffScope {
        DiffScope {
            base_branch: self.base_branch.clone(),
            changed_files: self
                .changed_files
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            total_changed: self.changed_files.len(),
        }
    }

    /// Check if a file is in the diff scope.
    pub fn contains(&self, file_path: &str) -> bool {
        self.changed_files.iter().any(|p| {
            let file_str = p.to_string_lossy();
            // Normalize paths for comparison (handle both relative and absolute)
            file_path.ends_with(file_str.as_ref())
                || file_str.as_ref().ends_with(file_path)
                || file_path == file_str.as_ref()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a test DiffFilter with predefined files (for testing only).
    #[cfg(test)]
    pub(crate) fn create_test_diff_filter(
        base_branch: &str,
        changed_files: Vec<&str>,
    ) -> DiffFilter {
        DiffFilter {
            base_branch: base_branch.to_string(),
            changed_files: changed_files.iter().map(|f| PathBuf::from(f)).collect(),
        }
    }

    #[test]
    fn test_diff_filter_contains() {
        let filter = create_test_diff_filter("main", vec!["src/main.rs", "src/lib.rs"]);

        assert!(filter.contains("src/main.rs"));
        assert!(filter.contains("src/lib.rs"));
        assert!(!filter.contains("tests/test.rs"));
    }

    #[test]
    fn test_diff_filter_scope() {
        let filter = create_test_diff_filter("origin/main", vec!["src/a.rs", "src/b.rs"]);

        let scope = filter.scope();
        assert_eq!(scope.base_branch, "origin/main");
        assert_eq!(scope.total_changed, 2);
    }
}
