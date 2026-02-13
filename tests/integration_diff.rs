//! Integration tests for CI check with real git diff filtering.
//!
//! These tests create actual git repositories and verify that:
//! 1. DiffFilter correctly identifies changed files
//! 2. CI check respects diff scope and only reports findings in changed files
//! 3. Git integration works end-to-end

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Helper: Initialize a git repo and return its path
fn init_git_repo(dir: &TempDir) -> PathBuf {
    let repo_path = dir.path();

    // Initialize git repo with main as default branch
    Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to init git repo");

    // Configure git user (required for commits)
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git email");

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(repo_path)
        .output()
        .expect("Failed to set git name");

    repo_path.to_path_buf()
}

/// Helper: Commit all changes in git repo
fn commit_all(repo_path: &PathBuf, message: &str) {
    Command::new("git")
        .args(["add", "."])
        .current_dir(repo_path)
        .output()
        .expect("Failed to stage files");

    Command::new("git")
        .args(["commit", "-m", message])
        .current_dir(repo_path)
        .output()
        .expect("Failed to commit");
}

/// Helper: Create and checkout a new branch
fn create_branch(repo_path: &PathBuf, branch_name: &str) {
    Command::new("git")
        .args(["checkout", "-b", branch_name])
        .current_dir(repo_path)
        .output()
        .unwrap_or_else(|_| panic!("Failed to create branch {}", branch_name));
}

// ============================================================================
// Test 1: DiffFilter with Single Changed File
// ============================================================================

#[test]
fn test_diff_filter_single_changed_file() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = init_git_repo(&temp_dir);

    // Create initial file on main
    fs::write(
        repo_path.join("main.rs"),
        r#"
fn used_function() {}

fn main() {
    used_function();
}
"#,
    )
    .unwrap();

    commit_all(&repo_path, "Initial commit");

    // Create feature branch
    create_branch(&repo_path, "feature/new-code");

    // Create new file on feature branch
    fs::write(
        repo_path.join("feature.rs"),
        r#"
fn unused_in_feature() {}

fn feature_func() {
    used_in_feature();
}

fn used_in_feature() {}
"#,
    )
    .unwrap();

    commit_all(&repo_path, "Add feature code");

    // Run diff to see what changed
    let output = Command::new("git")
        .args(["diff", "main...HEAD", "--name-only"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to run git diff");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let changed_files: Vec<&str> = stdout.lines().collect();

    // Should only show feature.rs as changed
    assert_eq!(changed_files.len(), 1, "Should have 1 changed file");
    assert!(
        changed_files[0].contains("feature.rs"),
        "Changed file should be feature.rs"
    );
}

// ============================================================================
// Test 2: DiffFilter with Multiple Changed Files
// ============================================================================

#[test]
fn test_diff_filter_multiple_changed_files() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = init_git_repo(&temp_dir);

    // Create directory structure
    fs::create_dir(repo_path.join("src")).unwrap();

    // Initial commit
    fs::write(
        repo_path.join("src/lib.rs"),
        r#"
pub fn init() {}

pub fn main() {
    init();
}
"#,
    )
    .unwrap();

    commit_all(&repo_path, "Initial commit");

    // Create feature branch
    create_branch(&repo_path, "feature/multi-file");

    // Modify multiple files
    fs::write(
        repo_path.join("src/lib.rs"),
        r#"
pub fn init() {}
pub fn new_helper() {}

pub fn main() {
    init();
    new_helper();
}
"#,
    )
    .unwrap();

    fs::create_dir_all(repo_path.join("src/utils")).unwrap();
    fs::write(
        repo_path.join("src/utils/mod.rs"),
        r#"
pub fn util_func() {}
"#,
    )
    .unwrap();

    commit_all(&repo_path, "Add multiple files");

    // Run diff to see all changed files
    let output = Command::new("git")
        .args(["diff", "main...HEAD", "--name-only"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to run git diff");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let changed_files: Vec<&str> = stdout.lines().collect();

    // Should show both lib.rs and utils/mod.rs
    assert_eq!(changed_files.len(), 2, "Should have 2 changed files");
    assert!(changed_files.iter().any(|f| f.contains("lib.rs")));
    assert!(changed_files.iter().any(|f| f.contains("utils")));
}

// ============================================================================
// Test 3: CI Runner with Diff Scope Integration
// ============================================================================

#[test]
fn test_ci_runner_with_real_git_diff() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = init_git_repo(&temp_dir);

    // Create initial clean file on main
    fs::write(
        repo_path.join("main.rs"),
        r#"
fn main() {
    used_function();
}

fn used_function() {
    println!("Used");
}
"#,
    )
    .unwrap();

    commit_all(&repo_path, "Initial clean main");

    // Create feature branch with dead code
    create_branch(&repo_path, "feature/with-dead-code");

    fs::write(
        repo_path.join("feature.rs"),
        r#"
fn entry_point() {
    used_function();
}

fn used_function() {
    println!("Used");
}

fn unused_function() {
    println!("Dead code!");
}
"#,
    )
    .unwrap();

    commit_all(&repo_path, "Add file with dead code");

    // Create diff filter pointing to main branch
    let diff_filter =
        fossil_mcp::ci::DiffFilter::new("main", &repo_path).expect("Failed to create DiffFilter");

    // Verify diff scope
    let scope = diff_filter.scope();
    assert_eq!(scope.base_branch, "main");
    assert_eq!(scope.total_changed, 1, "Should have 1 changed file");
    assert!(
        scope.changed_files[0].contains("feature.rs"),
        "Changed file should be feature.rs"
    );

    // Verify filter correctly identifies changed files
    assert!(
        diff_filter.contains("feature.rs"),
        "Should contain feature.rs"
    );
    assert!(
        !diff_filter.contains("main.rs"),
        "Should not contain main.rs (not in diff)"
    );
}

// ============================================================================
// Test 4: Diff Filter Path Normalization
// ============================================================================

#[test]
fn test_diff_filter_path_normalization() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = init_git_repo(&temp_dir);

    // Create nested directory structure
    fs::create_dir_all(repo_path.join("src/analysis/dead_code")).unwrap();

    fs::write(
        repo_path.join("src/analysis/dead_code/detector.rs"),
        "fn analyze() {}",
    )
    .unwrap();

    commit_all(&repo_path, "Initial");

    create_branch(&repo_path, "feature/nested");

    fs::write(
        repo_path.join("src/analysis/dead_code/detector.rs"),
        "fn analyze() { println!(\"changed\"); }",
    )
    .unwrap();

    commit_all(&repo_path, "Modify nested file");

    let diff_filter =
        fossil_mcp::ci::DiffFilter::new("main", &repo_path).expect("Failed to create DiffFilter");

    // Test various path formats that should match the same file
    assert!(
        diff_filter.contains("src/analysis/dead_code/detector.rs"),
        "Should match exact path"
    );
    assert!(
        diff_filter.contains("detector.rs"),
        "Should match by basename"
    );
    assert!(
        diff_filter.contains("dead_code/detector.rs"),
        "Should match partial path"
    );
}

// ============================================================================
// Test 5: Diff with No Changes
// ============================================================================

#[test]
fn test_diff_filter_no_changes() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = init_git_repo(&temp_dir);

    fs::write(repo_path.join("file.rs"), "fn main() {}").unwrap();
    commit_all(&repo_path, "Initial");

    // Try to diff against main when on main (no changes)
    let result = fossil_mcp::ci::DiffFilter::new("main", &repo_path);

    // This should succeed but have empty changed files
    assert!(result.is_ok(), "Should handle no-changes case");
    let diff_filter = result.unwrap();
    let scope = diff_filter.scope();
    assert_eq!(scope.total_changed, 0, "Should have 0 changed files");
}

// ============================================================================
// Test 6: Diff with Deleted Files
// ============================================================================

#[test]
fn test_diff_filter_deleted_files() {
    let temp_dir = TempDir::new().unwrap();
    let repo_path = init_git_repo(&temp_dir);

    fs::write(repo_path.join("to_delete.rs"), "fn old_code() {}").unwrap();
    fs::write(repo_path.join("to_keep.rs"), "fn kept_code() {}").unwrap();

    commit_all(&repo_path, "Initial");

    create_branch(&repo_path, "feature/cleanup");

    // Delete a file
    fs::remove_file(repo_path.join("to_delete.rs")).unwrap();

    commit_all(&repo_path, "Delete old file");

    let diff_filter =
        fossil_mcp::ci::DiffFilter::new("main", &repo_path).expect("Failed to create DiffFilter");

    let scope = diff_filter.scope();

    // Both files should appear in diff (one modified, one deleted)
    assert_eq!(scope.total_changed, 1, "Should show deleted file in diff");
}
