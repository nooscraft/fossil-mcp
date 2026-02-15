//! Differential analysis: Detect changed files and re-analyze only affected code.
//!
//! Enables incremental analysis in CI/CD pipelines by:
//! 1. Detecting which files changed via git diff
//! 2. Loading cached results for unchanged files
//! 3. Re-analyzing only changed files + their dependents
//! 4. Merging cached and fresh results

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Changed files information from git diff.
#[derive(Debug, Clone)]
pub struct DiffInfo {
    /// Files that were added or modified
    pub changed_files: HashSet<PathBuf>,
    /// Files that were deleted
    pub deleted_files: HashSet<PathBuf>,
}

impl DiffInfo {
    /// Parse git diff output to detect changed files.
    ///
    /// # Arguments
    /// * `diff_output` - Output from `git diff --name-status`
    ///
    /// # Format
    /// ```text
    /// M    src/file1.rs         (Modified)
    /// A    src/file2.rs         (Added)
    /// D    src/file3.rs         (Deleted)
    /// R100 old.rs    new.rs   (Renamed)
    /// ```
    pub fn from_git_diff(diff_output: &str) -> Result<Self, String> {
        let mut changed_files = HashSet::new();
        let mut deleted_files = HashSet::new();

        for line in diff_output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Parse status and filename
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }

            let status = parts[0];
            let file_path = if status.starts_with('R') || status.starts_with('C') {
                // Rename or copy: use new filename (second column)
                parts.get(2).unwrap_or(&parts[1])
            } else {
                // Add, modify, delete: use filename (second column)
                parts.get(1).unwrap_or(&"")
            };

            if file_path.is_empty() {
                continue;
            }

            let path = PathBuf::from(file_path);

            match status.chars().next() {
                Some('D') => {
                    deleted_files.insert(path);
                }
                Some('M') | Some('A') | Some('R') | Some('C') => {
                    changed_files.insert(path);
                }
                _ => {}
            }
        }

        Ok(Self {
            changed_files,
            deleted_files,
        })
    }

    /// Check if a file path has changed.
    pub fn is_changed(&self, file_path: &Path) -> bool {
        self.changed_files.contains(&file_path.to_path_buf())
    }

    /// Check if a file path was deleted.
    pub fn is_deleted(&self, file_path: &Path) -> bool {
        self.deleted_files.contains(&file_path.to_path_buf())
    }

    /// Get all changed files as strings.
    pub fn changed_file_strings(&self) -> Vec<String> {
        self.changed_files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect()
    }
}

/// Analyzer for determining which functions are affected by changes.
pub struct DependentAnalyzer {
    /// Maps from function to files it's called in
    function_call_sites: HashMap<String, HashSet<String>>,
    /// Maps from file to functions defined in it
    file_to_functions: HashMap<String, HashSet<String>>,
}

impl Default for DependentAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl DependentAnalyzer {
    /// Create a new dependent analyzer.
    pub fn new() -> Self {
        Self {
            function_call_sites: HashMap::new(),
            file_to_functions: HashMap::new(),
        }
    }

    /// Register a function defined in a file.
    pub fn add_function_definition(&mut self, function: String, file: String) {
        self.file_to_functions
            .entry(file)
            .or_default()
            .insert(function);
    }

    /// Register a function call site.
    pub fn add_function_call(&mut self, function: String, called_in_file: String) {
        self.function_call_sites
            .entry(function)
            .or_default()
            .insert(called_in_file);
    }

    /// Find all files that need re-analysis due to changed files.
    ///
    /// Returns files that:
    /// 1. Were directly changed
    /// 2. Call functions defined in changed files
    pub fn find_affected_files(&self, changed_files: &HashSet<PathBuf>) -> HashSet<String> {
        let mut affected = HashSet::new();

        // Convert changed files to strings for lookup
        let changed_file_strings: HashSet<String> = changed_files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        // 1. Add directly changed files
        for file in &changed_file_strings {
            affected.insert(file.clone());
        }

        // 2. Find files that call functions from changed files
        for changed_file in &changed_file_strings {
            if let Some(functions) = self.file_to_functions.get(changed_file) {
                for function in functions {
                    if let Some(call_sites) = self.function_call_sites.get(function) {
                        affected.extend(call_sites.clone());
                    }
                }
            }
        }

        affected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_git_diff_modified() {
        let diff = "M\tsrc/file1.rs\nM\tsrc/file2.rs";
        let info = DiffInfo::from_git_diff(diff).unwrap();

        assert_eq!(info.changed_files.len(), 2);
        assert!(info.is_changed(Path::new("src/file1.rs")));
        assert!(info.is_changed(Path::new("src/file2.rs")));
    }

    #[test]
    fn test_parse_git_diff_added_deleted() {
        let diff = "A\tsrc/new.rs\nD\tsrc/old.rs";
        let info = DiffInfo::from_git_diff(diff).unwrap();

        assert!(info.is_changed(Path::new("src/new.rs")));
        assert!(info.is_deleted(Path::new("src/old.rs")));
    }

    #[test]
    fn test_parse_git_diff_renamed() {
        let diff = "R100\tsrc/old.rs\tsrc/new.rs";
        let info = DiffInfo::from_git_diff(diff).unwrap();

        assert!(info.is_changed(Path::new("src/new.rs")));
    }

    #[test]
    fn test_find_affected_files() {
        let mut analyzer = DependentAnalyzer::new();

        // Set up functions and call sites
        analyzer.add_function_definition("foo".to_string(), "src/a.rs".to_string());
        analyzer.add_function_definition("bar".to_string(), "src/b.rs".to_string());

        analyzer.add_function_call("foo".to_string(), "src/c.rs".to_string());
        analyzer.add_function_call("bar".to_string(), "src/d.rs".to_string());

        let mut changed = HashSet::new();
        changed.insert(PathBuf::from("src/a.rs"));

        let affected = analyzer.find_affected_files(&changed);

        // Should include: src/a.rs (changed) + src/c.rs (calls foo from changed file)
        assert!(affected.contains("src/a.rs"));
        assert!(affected.contains("src/c.rs"));
        assert!(!affected.contains("src/b.rs")); // Not changed, not affected
    }

    #[test]
    fn test_empty_diff() {
        let diff = "";
        let info = DiffInfo::from_git_diff(diff).unwrap();

        assert!(info.changed_files.is_empty());
        assert!(info.deleted_files.is_empty());
    }
}
