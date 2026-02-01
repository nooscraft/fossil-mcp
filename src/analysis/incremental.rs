//! Incremental analysis: only re-parses files whose content has changed.
//!
//! Uses [`PersistentCache`] for content-hash-based invalidation and merges
//! cached results for unchanged files with freshly-parsed results for changed
//! files into a single [`PipelineResult`].
//!
//! A [`sketch_oxide::membership::BinaryFuseFilter`] is optionally maintained as
//! a fast pre-check: if a file's content hash is *not* in the filter, it is
//! definitely new/changed and we can skip the persistent cache lookup entirely.

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use crate::core::ParsedFile;
use crate::graph::GraphBuilder;
use crate::parsers::ParserRegistry;
use sketch_oxide::membership::BinaryFuseFilter;
use tracing::info;

use super::persistent_cache::{hash_content, CachedFileEntry, PersistentCache};
use super::pipeline::PipelineResult;
use super::scanner::{FileScanner, SourceFile};

/// Classification of source files into changed, deleted, and unchanged sets.
#[derive(Debug, Clone)]
pub struct ChangeSet {
    /// Files that are new or whose content hash differs from the cache.
    pub changed_files: Vec<String>,
    /// Files that were previously cached but no longer appear on disk.
    pub deleted_files: Vec<String>,
    /// Files whose content hash matches the cache (no re-parse needed).
    pub unchanged_files: Vec<String>,
}

/// Incremental analyzer that skips re-parsing unchanged files.
///
/// Wraps a [`PersistentCache`] and orchestrates scan -> diff -> parse -> merge.
///
/// Optionally maintains a [`BinaryFuseFilter`] of all known content hashes so
/// that change detection can short-circuit: if the filter says "definitely not
/// present", the file is new/changed without needing to consult the persistent
/// cache.
pub struct IncrementalAnalyzer {
    cache: PersistentCache,
    /// Binary fuse filter built from all content hashes currently in the cache.
    /// `None` until the first full scan completes.
    content_filter: Option<BinaryFuseFilter>,
}

impl IncrementalAnalyzer {
    /// Create an incremental analyzer backed by the given persistent cache.
    pub fn new(cache: PersistentCache) -> Self {
        let content_filter = Self::build_filter_from_cache(&cache);
        Self {
            cache,
            content_filter,
        }
    }

    /// Load (or create) an incremental analyzer from the default cache location
    /// under the given project root: `{root}/.fossil/cache.json`.
    pub fn from_project_root(root: &Path) -> Self {
        let cache_path = root.join(".fossil").join("cache.json");
        let cache = PersistentCache::load(&cache_path);
        let content_filter = Self::build_filter_from_cache(&cache);
        Self {
            cache,
            content_filter,
        }
    }

    /// Build a `BinaryFuseFilter` from all content hashes in the persistent
    /// cache.  Returns `None` if the cache is empty or filter construction
    /// fails.
    fn build_filter_from_cache(cache: &PersistentCache) -> Option<BinaryFuseFilter> {
        if cache.is_empty() {
            return None;
        }
        let hashes: HashSet<u64> = cache
            .entries()
            .map(|(_, entry)| entry.content_hash)
            .collect();
        BinaryFuseFilter::from_items(hashes, 8).ok()
    }

    /// Rebuild the content filter from the current cache state.
    pub fn rebuild_content_filter(&mut self) {
        self.content_filter = Self::build_filter_from_cache(&self.cache);
    }

    /// Returns `true` if a content filter is currently available.
    pub fn has_content_filter(&self) -> bool {
        self.content_filter.is_some()
    }

    /// Detect which files have changed, been deleted, or remain unchanged
    /// relative to the current cache state.
    ///
    /// When a content filter is available, it is used as a fast pre-check:
    /// if the filter says the hash is definitely absent, the file is marked
    /// changed without consulting the persistent cache.
    pub fn detect_changes(&self, _root: &Path, source_files: &[SourceFile]) -> ChangeSet {
        let mut changed_files = Vec::new();
        let mut unchanged_files = Vec::new();

        // Build a set of current file paths for deletion detection
        let mut current_paths = std::collections::HashSet::new();

        for file in source_files {
            let path_str = file.path.to_string_lossy().to_string();
            current_paths.insert(path_str.clone());

            // Read file content and hash it
            let content_hash = match std::fs::read(&file.path) {
                Ok(bytes) => hash_content(&bytes),
                Err(_) => {
                    // Cannot read file — treat as changed (will fail later during parse)
                    changed_files.push(path_str);
                    continue;
                }
            };

            // Fast path: if the content filter is available and says this hash
            // is definitely not present, the file must be new or changed.
            if let Some(ref filter) = self.content_filter {
                if !filter.contains(&content_hash) {
                    changed_files.push(path_str);
                    continue;
                }
            }

            // Slow path: consult the persistent cache (handles hash collisions
            // in the filter and path-level checks).
            if self.cache.is_file_changed(&path_str, content_hash) {
                changed_files.push(path_str);
            } else {
                unchanged_files.push(path_str);
            }
        }

        // Detect deleted files: in cache but not in current scan
        let deleted_files: Vec<String> = self
            .cache
            .entries()
            .filter(|(path, _)| !current_paths.contains(*path))
            .map(|(path, _)| path.clone())
            .collect();

        ChangeSet {
            changed_files,
            deleted_files,
            unchanged_files,
        }
    }

    /// Run an incremental analysis on the project at `root`.
    ///
    /// 1. Scan for source files.
    /// 2. Detect changes against the cache.
    /// 3. Parse only changed/new files.
    /// 4. Merge cached results for unchanged files.
    /// 5. Build a unified CodeGraph via GraphBuilder.
    /// 6. Update and save the cache.
    pub fn run_incremental(&mut self, root: &Path) -> Result<PipelineResult, crate::core::Error> {
        let start = Instant::now();

        // Scan for source files
        let scanner = FileScanner::new();
        let source_files = scanner.scan(root)?;
        let files_scanned = source_files.len();
        info!("Scanned {} source files", files_scanned);

        // Detect changes
        let changes = self.detect_changes(root, &source_files);
        info!(
            "Changes: {} changed, {} unchanged, {} deleted",
            changes.changed_files.len(),
            changes.unchanged_files.len(),
            changes.deleted_files.len(),
        );

        // Parse changed files
        let registry = ParserRegistry::with_defaults()?;
        let mut all_parsed: Vec<ParsedFile> = Vec::new();
        let mut errors: Vec<(String, String)> = Vec::new();
        let mut total_lines = 0usize;

        // Build a lookup from path to SourceFile for changed files
        let source_file_map: std::collections::HashMap<String, &SourceFile> = source_files
            .iter()
            .map(|sf| (sf.path.to_string_lossy().to_string(), sf))
            .collect();

        for path_str in &changes.changed_files {
            let Some(source_file) = source_file_map.get(path_str) else {
                errors.push((path_str.clone(), "File not found in scan".to_string()));
                continue;
            };

            let source = match std::fs::read_to_string(&source_file.path) {
                Ok(s) => s,
                Err(e) => {
                    errors.push((path_str.clone(), format!("IO error: {e}")));
                    continue;
                }
            };

            let ext = source_file
                .path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let parser = match registry
                .get_parser_for_extension(ext)
                .or_else(|| registry.get_parser(source_file.language))
            {
                Some(p) => p,
                None => {
                    errors.push((
                        path_str.clone(),
                        format!("No parser for {}", source_file.language.name()),
                    ));
                    continue;
                }
            };

            match parser.parse_file(path_str, &source) {
                Ok(parsed) => {
                    total_lines += parsed.source.lines().count();

                    // Update cache entry for this file
                    let content_hash = hash_content(source.as_bytes());
                    self.cache.update_entry(CachedFileEntry {
                        path: path_str.clone(),
                        content_hash,
                        nodes: parsed.nodes.clone(),
                        edges: parsed.edges.clone(),
                        entry_points: parsed.entry_points.clone(),
                    });

                    all_parsed.push(parsed);
                }
                Err(e) => {
                    errors.push((path_str.clone(), e.to_string()));
                }
            }
        }

        let files_parsed = all_parsed.len();

        // Reconstruct ParsedFile stubs for unchanged files from cache
        for path_str in &changes.unchanged_files {
            if let Some(cached) = self.cache.get_entry(path_str) {
                let source_file = source_file_map.get(path_str);
                let language = source_file
                    .map(|sf| sf.language)
                    .unwrap_or(crate::core::Language::Python);

                // Read the source for line counting (lightweight)
                let source = std::fs::read_to_string(path_str).unwrap_or_default();
                total_lines += source.lines().count();

                let mut pf = ParsedFile::new(path_str.clone(), language, source);
                pf.nodes = cached.nodes.clone();
                pf.edges = cached.edges.clone();
                pf.entry_points = cached.entry_points.clone();
                all_parsed.push(pf);
            }
        }

        // Remove deleted files from cache
        for path_str in &changes.deleted_files {
            self.cache.remove_entry(path_str);
        }

        // Build project graph
        let builder = GraphBuilder::new()?;
        let graph = builder.build_project_graph(&all_parsed)?;
        info!(
            "Built graph: {} nodes, {} edges",
            graph.node_count(),
            graph.edge_count()
        );

        // Rebuild the binary fuse filter from the updated cache
        self.rebuild_content_filter();

        // Save cache
        let cache_path = root.join(".fossil").join("cache.json");
        self.cache.save(&cache_path)?;

        let duration_ms = start.elapsed().as_millis() as u64;

        Ok(PipelineResult {
            graph,
            parsed_files: all_parsed,
            errors,
            files_scanned,
            files_parsed,
            total_lines,
            duration_ms,
        })
    }

    /// Access the underlying persistent cache.
    pub fn cache(&self) -> &PersistentCache {
        &self.cache
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_detect_changes_new_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.py"), "def a(): pass").unwrap();
        fs::write(dir.path().join("b.py"), "def b(): pass").unwrap();

        let scanner = FileScanner::new();
        let files = scanner.scan(dir.path()).unwrap();

        let cache = PersistentCache::new();
        let analyzer = IncrementalAnalyzer::new(cache);
        let changes = analyzer.detect_changes(dir.path(), &files);

        // Both files are new (not in cache), so both should be changed
        assert_eq!(changes.changed_files.len(), 2);
        assert!(changes.unchanged_files.is_empty());
        assert!(changes.deleted_files.is_empty());
    }

    #[test]
    fn test_detect_changes_unchanged_file() {
        let dir = TempDir::new().unwrap();
        let content = "def a(): pass";
        fs::write(dir.path().join("a.py"), content).unwrap();

        let scanner = FileScanner::new();
        let files = scanner.scan(dir.path()).unwrap();

        // Pre-populate cache with the correct hash
        let mut cache = PersistentCache::new();
        let content_hash = hash_content(content.as_bytes());
        let path_str = dir.path().join("a.py").to_string_lossy().to_string();
        cache.update_entry(CachedFileEntry {
            path: path_str,
            content_hash,
            nodes: vec![],
            edges: vec![],
            entry_points: vec![],
        });

        let analyzer = IncrementalAnalyzer::new(cache);
        let changes = analyzer.detect_changes(dir.path(), &files);

        assert!(changes.changed_files.is_empty());
        assert_eq!(changes.unchanged_files.len(), 1);
        assert!(changes.deleted_files.is_empty());
    }

    #[test]
    fn test_detect_changes_modified_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.py"), "def a(): return 42").unwrap();

        let scanner = FileScanner::new();
        let files = scanner.scan(dir.path()).unwrap();

        // Cache has a different hash for the same file
        let mut cache = PersistentCache::new();
        let path_str = dir.path().join("a.py").to_string_lossy().to_string();
        cache.update_entry(CachedFileEntry {
            path: path_str,
            content_hash: 99999, // stale hash
            nodes: vec![],
            edges: vec![],
            entry_points: vec![],
        });

        let analyzer = IncrementalAnalyzer::new(cache);
        let changes = analyzer.detect_changes(dir.path(), &files);

        assert_eq!(changes.changed_files.len(), 1);
        assert!(changes.unchanged_files.is_empty());
    }

    #[test]
    fn test_detect_changes_deleted_file() {
        let dir = TempDir::new().unwrap();
        // Only a.py exists on disk
        fs::write(dir.path().join("a.py"), "def a(): pass").unwrap();

        let scanner = FileScanner::new();
        let files = scanner.scan(dir.path()).unwrap();

        // Cache has entries for both a.py and deleted.py
        let mut cache = PersistentCache::new();
        let a_path = dir.path().join("a.py").to_string_lossy().to_string();
        let a_hash = hash_content(b"def a(): pass");
        cache.update_entry(CachedFileEntry {
            path: a_path,
            content_hash: a_hash,
            nodes: vec![],
            edges: vec![],
            entry_points: vec![],
        });
        cache.update_entry(CachedFileEntry {
            path: "deleted.py".to_string(),
            content_hash: 111,
            nodes: vec![],
            edges: vec![],
            entry_points: vec![],
        });

        let analyzer = IncrementalAnalyzer::new(cache);
        let changes = analyzer.detect_changes(dir.path(), &files);

        assert!(changes.changed_files.is_empty());
        assert_eq!(changes.unchanged_files.len(), 1);
        assert_eq!(changes.deleted_files.len(), 1);
        assert_eq!(changes.deleted_files[0], "deleted.py");
    }

    #[test]
    fn test_incremental_run_first_time() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("main.py"),
            "def main():\n    helper()\n\ndef helper():\n    pass\n",
        )
        .unwrap();

        let mut analyzer = IncrementalAnalyzer::from_project_root(dir.path());
        let result = analyzer.run_incremental(dir.path()).unwrap();

        assert_eq!(result.files_scanned, 1);
        assert_eq!(result.files_parsed, 1); // First run: all files parsed
        assert!(result.graph.node_count() >= 2);
        assert!(result.errors.is_empty());

        // Cache should now have the entry
        assert_eq!(analyzer.cache().len(), 1);

        // Verify cache was written to disk
        let cache_path = dir.path().join(".fossil").join("cache.json");
        assert!(cache_path.exists());
    }

    #[test]
    fn test_incremental_run_skips_unchanged() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("main.py"),
            "def main():\n    helper()\n\ndef helper():\n    pass\n",
        )
        .unwrap();

        // First run: populates cache
        let mut analyzer = IncrementalAnalyzer::from_project_root(dir.path());
        let result1 = analyzer.run_incremental(dir.path()).unwrap();
        assert_eq!(result1.files_parsed, 1);

        // Second run: should skip parsing (file unchanged)
        let mut analyzer2 = IncrementalAnalyzer::from_project_root(dir.path());
        let result2 = analyzer2.run_incremental(dir.path()).unwrap();
        assert_eq!(result2.files_parsed, 0); // No files re-parsed
        assert_eq!(result2.files_scanned, 1);
        assert!(result2.graph.node_count() >= 2); // Graph still built from cache
    }

    #[test]
    fn test_incremental_run_reparses_changed() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.py"), "def main():\n    pass\n").unwrap();

        // First run: populates cache
        let mut analyzer = IncrementalAnalyzer::from_project_root(dir.path());
        let _result1 = analyzer.run_incremental(dir.path()).unwrap();

        // Modify the file
        fs::write(
            dir.path().join("main.py"),
            "def main():\n    helper()\n\ndef helper():\n    pass\n",
        )
        .unwrap();

        // Second run: should detect change and re-parse
        let mut analyzer2 = IncrementalAnalyzer::from_project_root(dir.path());
        let result2 = analyzer2.run_incremental(dir.path()).unwrap();
        assert_eq!(result2.files_parsed, 1); // Re-parsed the changed file
        assert!(result2.graph.node_count() >= 2);
    }

    // ---- BinaryFuseFilter tests ----

    #[test]
    fn test_empty_cache_has_no_filter() {
        let cache = PersistentCache::new();
        let analyzer = IncrementalAnalyzer::new(cache);
        assert!(!analyzer.has_content_filter());
    }

    #[test]
    fn test_populated_cache_has_filter() {
        let mut cache = PersistentCache::new();
        cache.update_entry(CachedFileEntry {
            path: "a.py".to_string(),
            content_hash: hash_content(b"def a(): pass"),
            nodes: vec![],
            edges: vec![],
            entry_points: vec![],
        });

        let analyzer = IncrementalAnalyzer::new(cache);
        assert!(analyzer.has_content_filter());
    }

    #[test]
    fn test_filter_detects_new_files_fast() {
        let dir = TempDir::new().unwrap();
        let content_a = "def a(): pass";
        let content_b = "def b(): pass";
        fs::write(dir.path().join("a.py"), content_a).unwrap();
        fs::write(dir.path().join("b.py"), content_b).unwrap();

        let scanner = FileScanner::new();
        let files = scanner.scan(dir.path()).unwrap();

        // Cache only knows about a.py
        let mut cache = PersistentCache::new();
        let a_path = dir.path().join("a.py").to_string_lossy().to_string();
        cache.update_entry(CachedFileEntry {
            path: a_path,
            content_hash: hash_content(content_a.as_bytes()),
            nodes: vec![],
            edges: vec![],
            entry_points: vec![],
        });

        let analyzer = IncrementalAnalyzer::new(cache);
        assert!(analyzer.has_content_filter());

        let changes = analyzer.detect_changes(dir.path(), &files);

        // b.py should be detected as changed (new file, hash not in filter)
        assert_eq!(changes.changed_files.len(), 1);
        assert!(changes.changed_files[0].contains("b.py"));
        assert_eq!(changes.unchanged_files.len(), 1);
    }

    #[test]
    fn test_filter_rebuilt_after_incremental_run() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("main.py"), "def main():\n    pass\n").unwrap();

        let mut analyzer = IncrementalAnalyzer::from_project_root(dir.path());

        // Before first run: empty cache, no filter
        assert!(!analyzer.has_content_filter());

        // First run populates cache and rebuilds filter
        let _result = analyzer.run_incremental(dir.path()).unwrap();
        assert!(analyzer.has_content_filter());
    }
}
