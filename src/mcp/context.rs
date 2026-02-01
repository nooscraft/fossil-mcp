//! Shared analysis context for the Fossil MCP server.
//!
//! Provides a lazily-initialized, thread-safe cache of the analysis pipeline
//! result. Multiple MCP tool calls reuse this cached state instead of
//! re-parsing the project each time.
//!
//! Uses [`IncrementalAnalyzer`] under the hood so that after files change,
//! only modified files are re-parsed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::Instant;

use crate::analysis::incremental::IncrementalAnalyzer;
use crate::core::ParsedFile;
use crate::graph::{CfgEdgeKind, CodeGraph, ControlFlowGraph};

/// Summary returned by [`SharedContext::refresh`].
pub struct RefreshResult {
    pub files_changed: usize,
    pub files_unchanged: usize,
    pub files_deleted: usize,
    pub duration_ms: u64,
}

/// Cached result of running the analysis pipeline on a project root.
pub struct AnalysisContext {
    pub root: PathBuf,
    pub graph: CodeGraph,
    pub parsed_files: Vec<ParsedFile>,
    pub cfgs: HashMap<String, ControlFlowGraph>,
    pub files_parsed: usize,
    pub total_lines: usize,
    /// `(path, source_content)` pairs for clone detection reuse.
    pub source_files: Vec<(String, String)>,
}

/// Thread-safe wrapper around an optional [`AnalysisContext`].
///
/// Uses `std::sync::RwLock` so that many readers can access the cached
/// context concurrently, while initialization and invalidation take an
/// exclusive write lock.
///
/// Holds an [`IncrementalAnalyzer`] behind a `Mutex` so that successive
/// calls to [`refresh`](Self::refresh) only re-parse changed files.
pub struct SharedContext {
    inner: RwLock<Option<AnalysisContext>>,
    analyzer: Mutex<Option<IncrementalAnalyzer>>,
    /// Monotonically increasing counter. Bumped every time [`refresh`]
    /// detects actual file changes.
    generation: AtomicU64,
}

impl SharedContext {
    /// Create an empty (uninitialized) shared context.
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(None),
            analyzer: Mutex::new(None),
            generation: AtomicU64::new(0),
        }
    }

    /// Current generation counter.
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Ensure that the context has been initialized for the given project
    /// root. If the context is already populated and the root matches, this
    /// is a no-op. Otherwise the incremental analysis pipeline is executed
    /// and the result is cached.
    pub fn ensure_initialized(&self, root: &Path) -> Result<(), String> {
        // Fast path: read lock to check if already initialized for this root.
        {
            let read_guard = self
                .inner
                .read()
                .map_err(|e| format!("RwLock poisoned (read): {e}"))?;
            if let Some(ctx) = read_guard.as_ref() {
                if ctx.root == root {
                    return Ok(());
                }
            }
        }

        // Slow path: run incremental analysis and store the result.
        self.run_and_store(root)?;
        Ok(())
    }

    /// Re-run incremental analysis on `root`, updating the cached context.
    ///
    /// This is always safe to call — if nothing changed on disk the
    /// incremental analyzer will detect that and return quickly.
    ///
    /// Returns a [`RefreshResult`] summarising what changed.
    pub fn refresh(&self, root: &Path) -> Result<RefreshResult, String> {
        self.run_and_store(root)
    }

    /// Run a closure with shared (read) access to the cached context.
    ///
    /// Returns an error if the context has not been initialized yet
    /// (call [`ensure_initialized`](Self::ensure_initialized) first) or if
    /// the lock is poisoned.
    pub fn with_context<F, R>(&self, f: F) -> Result<R, String>
    where
        F: FnOnce(&AnalysisContext) -> R,
    {
        let read_guard = self
            .inner
            .read()
            .map_err(|e| format!("RwLock poisoned (read): {e}"))?;
        match read_guard.as_ref() {
            Some(ctx) => Ok(f(ctx)),
            None => Err("Analysis context not initialized".to_string()),
        }
    }

    /// Clear the cached context, forcing re-analysis on the next call to
    /// [`ensure_initialized`](Self::ensure_initialized).
    pub fn invalidate(&self) {
        if let Ok(mut write_guard) = self.inner.write() {
            *write_guard = None;
        }
        if let Ok(mut analyzer_guard) = self.analyzer.lock() {
            *analyzer_guard = None;
        }
    }

    // ------------------------------------------------------------------
    // Internal
    // ------------------------------------------------------------------

    /// Run incremental analysis and store the result in `self.inner`.
    ///
    /// If the root changed the analyzer is re-created from scratch.
    /// Returns a `RefreshResult` with change counts.
    fn run_and_store(&self, root: &Path) -> Result<RefreshResult, String> {
        let start = Instant::now();

        // 1. Acquire the analyzer lock and get-or-create the analyzer.
        let mut analyzer_guard = self
            .analyzer
            .lock()
            .map_err(|e| format!("Analyzer mutex poisoned: {e}"))?;

        // If root changed, drop the old analyzer so it gets rebuilt.
        let root_changed = {
            let read_guard = self
                .inner
                .read()
                .map_err(|e| format!("RwLock poisoned (read): {e}"))?;
            match read_guard.as_ref() {
                Some(ctx) => ctx.root != root,
                None => true, // not initialized yet
            }
        };

        if root_changed {
            *analyzer_guard = None;
        }

        let analyzer =
            analyzer_guard.get_or_insert_with(|| IncrementalAnalyzer::from_project_root(root));

        // 2. Run the incremental pipeline.
        let result = analyzer
            .run_incremental(root)
            .map_err(|e| format!("Incremental pipeline error: {e}"))?;

        let files_parsed = result.files_parsed;
        let files_scanned = result.files_scanned;
        let files_unchanged = files_scanned.saturating_sub(files_parsed);

        // Count deleted files: files that were in the previous context but
        // are no longer present in the new result.
        let files_deleted = {
            let read_guard = self
                .inner
                .read()
                .map_err(|e| format!("RwLock poisoned (read): {e}"))?;
            match read_guard.as_ref() {
                Some(prev) => {
                    let new_paths: std::collections::HashSet<&str> = result
                        .parsed_files
                        .iter()
                        .map(|pf| pf.path.as_str())
                        .collect();
                    prev.parsed_files
                        .iter()
                        .filter(|pf| !new_paths.contains(pf.path.as_str()))
                        .count()
                }
                None => 0,
            }
        };

        // 3. Extract source files for clone detection reuse.
        let source_files: Vec<(String, String)> = result
            .parsed_files
            .iter()
            .map(|pf| (pf.path.clone(), pf.source.clone()))
            .collect();

        // 4. Build CFGs.
        let cfgs = build_cfgs(&result.graph);

        // 5. Bump generation if anything actually changed.
        let has_changes = files_parsed > 0 || files_deleted > 0 || root_changed;
        if has_changes {
            self.generation.fetch_add(1, Ordering::Release);
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        // 6. Store in the RwLock.
        {
            let mut write_guard = self
                .inner
                .write()
                .map_err(|e| format!("RwLock poisoned (write): {e}"))?;
            *write_guard = Some(AnalysisContext {
                root: root.to_path_buf(),
                graph: result.graph,
                parsed_files: result.parsed_files,
                cfgs,
                files_parsed: result.files_parsed,
                total_lines: result.total_lines,
                source_files,
            });
        }

        Ok(RefreshResult {
            files_changed: files_parsed,
            files_unchanged,
            files_deleted,
            duration_ms,
        })
    }
}

impl Default for SharedContext {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a simple (entry -> exit) CFG for each function-like node in the
/// graph.  Real intra-procedural CFGs are constructed inside individual
/// analysis passes; here we just cache a skeleton so that downstream MCP
/// tools have something to work with.
fn build_cfgs(graph: &CodeGraph) -> HashMap<String, ControlFlowGraph> {
    use crate::core::NodeKind;

    let mut cfgs = HashMap::new();

    for (_idx, node) in graph.nodes() {
        let is_function_like = matches!(
            node.kind,
            NodeKind::Function
                | NodeKind::Method
                | NodeKind::AsyncFunction
                | NodeKind::AsyncMethod
                | NodeKind::Constructor
                | NodeKind::StaticMethod
                | NodeKind::Lambda
                | NodeKind::Closure
        );

        if !is_function_like {
            continue;
        }

        let name = if node.full_name.is_empty() {
            node.name.clone()
        } else {
            node.full_name.clone()
        };

        // Avoid overwriting if we already have a CFG for this name
        // (e.g. overloaded functions across files).
        if cfgs.contains_key(&name) {
            continue;
        }

        let mut cfg = ControlFlowGraph::new(&name);
        let entry = cfg.create_entry();
        let exit = cfg.create_exit();
        cfg.add_edge(entry, exit, CfgEdgeKind::FallThrough);

        cfgs.insert(name, cfg);
    }

    cfgs
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_shared_context_lifecycle() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("main.py"),
            "def main():\n    helper()\n\ndef helper():\n    pass\n",
        )
        .unwrap();

        let ctx = SharedContext::new();

        // Should not be initialized yet.
        assert!(ctx.with_context(|_| ()).is_err());
        assert_eq!(ctx.generation(), 0);

        // Initialize.
        ctx.ensure_initialized(dir.path()).unwrap();

        // Should succeed now.
        let node_count = ctx.with_context(|c| c.graph.node_count()).unwrap();
        assert!(node_count >= 2);

        // CFGs should have been built for the two functions.
        let cfg_count = ctx.with_context(|c| c.cfgs.len()).unwrap();
        assert!(cfg_count >= 2, "Expected >= 2 CFGs, got {cfg_count}");

        // Generation should have bumped (first init counts as a change).
        assert!(ctx.generation() >= 1);

        // Re-initialize with the same root should be a no-op.
        let gen_before = ctx.generation();
        ctx.ensure_initialized(dir.path()).unwrap();
        assert_eq!(ctx.generation(), gen_before);

        // Invalidate and verify.
        ctx.invalidate();
        assert!(ctx.with_context(|_| ()).is_err());
    }

    #[test]
    fn test_shared_context_root_change() {
        let dir1 = TempDir::new().unwrap();
        fs::write(dir1.path().join("a.py"), "def a():\n    pass\n").unwrap();

        let dir2 = TempDir::new().unwrap();
        fs::write(dir2.path().join("b.py"), "def b():\n    pass\n").unwrap();

        let ctx = SharedContext::new();
        ctx.ensure_initialized(dir1.path()).unwrap();

        let root1 = ctx.with_context(|c| c.root.clone()).unwrap();
        assert_eq!(root1, dir1.path());

        // Switching to a different root should re-run the pipeline.
        ctx.ensure_initialized(dir2.path()).unwrap();

        let root2 = ctx.with_context(|c| c.root.clone()).unwrap();
        assert_eq!(root2, dir2.path());
    }

    #[test]
    fn test_shared_context_refresh() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.py"), "def main():\n    pass\n").unwrap();

        let ctx = SharedContext::new();

        // First refresh should parse everything.
        let r1 = ctx.refresh(dir.path()).unwrap();
        assert!(r1.files_changed > 0);
        assert_eq!(r1.files_deleted, 0);

        // source_files should be populated.
        let source_count = ctx.with_context(|c| c.source_files.len()).unwrap();
        assert!(source_count >= 1);

        // Second refresh with no changes — nothing re-parsed.
        let gen_before = ctx.generation();
        let r2 = ctx.refresh(dir.path()).unwrap();
        assert_eq!(r2.files_changed, 0);
        assert_eq!(ctx.generation(), gen_before);
    }
}
