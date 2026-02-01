//! Persistent on-disk cache for incremental analysis.
//!
//! Stores per-file analysis results (nodes, edges, entry points) keyed by
//! content hash so that unchanged files can skip re-parsing on subsequent runs.
//! The cache is serialized as JSON to `{project_root}/.fossil/cache.json`.

use std::collections::HashMap;
use std::path::Path;

use crate::core::{CallEdge, CodeNode, NodeId};
use serde::{Deserialize, Serialize};
use tracing::warn;

/// A single cached analysis entry for one source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFileEntry {
    /// Relative or absolute path of the source file.
    pub path: String,
    /// xxh3_64 hash of the file content at the time of caching.
    pub content_hash: u64,
    /// Extracted code nodes from the file.
    pub nodes: Vec<CodeNode>,
    /// Intra-file call edges.
    pub edges: Vec<CallEdge>,
    /// Entry-point node IDs detected in this file.
    pub entry_points: Vec<NodeId>,
}

/// Persistent JSON-backed cache of per-file analysis results.
///
/// Keyed by file path (string). Supports load/save to a JSON file on disk,
/// content-hash-based invalidation, and entry-level CRUD operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistentCache {
    entries: HashMap<String, CachedFileEntry>,
}

impl PersistentCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Load a cache from a JSON file on disk.
    ///
    /// Returns an empty cache if the file does not exist or cannot be parsed.
    pub fn load(cache_path: &Path) -> Self {
        if !cache_path.exists() {
            return Self::new();
        }

        match std::fs::read_to_string(cache_path) {
            Ok(contents) => match serde_json::from_str::<PersistentCache>(&contents) {
                Ok(cache) => cache,
                Err(e) => {
                    warn!("Failed to parse cache file {}: {}", cache_path.display(), e);
                    Self::new()
                }
            },
            Err(e) => {
                warn!("Failed to read cache file {}: {}", cache_path.display(), e);
                Self::new()
            }
        }
    }

    /// Save the cache to a JSON file on disk.
    ///
    /// Creates parent directories if they do not exist.
    pub fn save(&self, cache_path: &Path) -> Result<(), crate::core::Error> {
        if let Some(parent) = cache_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                crate::core::Error::analysis(format!(
                    "Failed to create cache directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }

        let json = serde_json::to_string_pretty(self)
            .map_err(|e| crate::core::Error::analysis(format!("Failed to serialize cache: {e}")))?;

        std::fs::write(cache_path, json).map_err(|e| {
            crate::core::Error::analysis(format!(
                "Failed to write cache file {}: {}",
                cache_path.display(),
                e
            ))
        })?;

        Ok(())
    }

    /// Check whether a file has changed relative to its cached entry.
    ///
    /// Returns `true` if the file is not in the cache or the content hash differs.
    pub fn is_file_changed(&self, path: &str, content_hash: u64) -> bool {
        match self.entries.get(path) {
            Some(entry) => entry.content_hash != content_hash,
            None => true,
        }
    }

    /// Retrieve a cached entry by file path.
    pub fn get_entry(&self, path: &str) -> Option<&CachedFileEntry> {
        self.entries.get(path)
    }

    /// Insert or update a cached entry.
    pub fn update_entry(&mut self, entry: CachedFileEntry) {
        self.entries.insert(entry.path.clone(), entry);
    }

    /// Remove an entry by file path (e.g. for deleted files).
    pub fn remove_entry(&mut self, path: &str) {
        self.entries.remove(path);
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterator over all cached entries.
    pub fn entries(&self) -> impl Iterator<Item = (&String, &CachedFileEntry)> {
        self.entries.iter()
    }
}

impl Default for PersistentCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute an xxh3_64 hash of file contents.
///
/// This is the canonical hashing function for content-based cache invalidation.
pub fn hash_content(content: &[u8]) -> u64 {
    xxhash_rust::xxh3::xxh3_64(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Language, NodeKind, SourceLocation, Visibility};
    use tempfile::TempDir;

    fn make_test_entry(path: &str, hash: u64) -> CachedFileEntry {
        let loc = SourceLocation::new(path.to_string(), 1, 5, 0, 0);
        let node = CodeNode::new(
            "test_fn".to_string(),
            NodeKind::Function,
            loc,
            Language::Python,
            Visibility::Public,
        );
        let node_id = node.id;
        CachedFileEntry {
            path: path.to_string(),
            content_hash: hash,
            nodes: vec![node],
            edges: vec![],
            entry_points: vec![node_id],
        }
    }

    #[test]
    fn test_cache_save_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join(".fossil").join("cache.json");

        let mut cache = PersistentCache::new();
        cache.update_entry(make_test_entry("src/main.py", 12345));
        cache.update_entry(make_test_entry("src/helper.py", 67890));

        cache.save(&cache_path).unwrap();
        assert!(cache_path.exists());

        let loaded = PersistentCache::load(&cache_path);
        assert_eq!(loaded.len(), 2);

        let entry = loaded.get_entry("src/main.py").unwrap();
        assert_eq!(entry.content_hash, 12345);
        assert_eq!(entry.nodes.len(), 1);
        assert_eq!(entry.nodes[0].name, "test_fn");
        assert_eq!(entry.entry_points.len(), 1);

        let entry2 = loaded.get_entry("src/helper.py").unwrap();
        assert_eq!(entry2.content_hash, 67890);
    }

    #[test]
    fn test_load_nonexistent_returns_empty() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("nonexistent").join("cache.json");

        let cache = PersistentCache::load(&cache_path);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_load_corrupt_returns_empty() {
        let dir = TempDir::new().unwrap();
        let cache_path = dir.path().join("cache.json");
        std::fs::write(&cache_path, "not valid json").unwrap();

        let cache = PersistentCache::load(&cache_path);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_is_file_changed() {
        let mut cache = PersistentCache::new();
        cache.update_entry(make_test_entry("a.py", 100));

        // Same hash: not changed
        assert!(!cache.is_file_changed("a.py", 100));
        // Different hash: changed
        assert!(cache.is_file_changed("a.py", 999));
        // Unknown file: changed
        assert!(cache.is_file_changed("unknown.py", 100));
    }

    #[test]
    fn test_update_entry_overwrites() {
        let mut cache = PersistentCache::new();
        cache.update_entry(make_test_entry("a.py", 100));
        assert_eq!(cache.get_entry("a.py").unwrap().content_hash, 100);

        cache.update_entry(make_test_entry("a.py", 200));
        assert_eq!(cache.get_entry("a.py").unwrap().content_hash, 200);
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_remove_entry() {
        let mut cache = PersistentCache::new();
        cache.update_entry(make_test_entry("a.py", 100));
        assert_eq!(cache.len(), 1);

        cache.remove_entry("a.py");
        assert!(cache.is_empty());
        assert!(cache.get_entry("a.py").is_none());
    }

    #[test]
    fn test_hash_content_deterministic() {
        let data = b"def main(): pass";
        let h1 = hash_content(data);
        let h2 = hash_content(data);
        assert_eq!(h1, h2);

        // Different content produces different hash
        let h3 = hash_content(b"def main(): return 42");
        assert_ne!(h1, h3);
    }
}
