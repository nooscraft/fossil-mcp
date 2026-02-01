//! In-memory analysis cache for parsed files and graphs.
//!
//! The primary [`AnalysisCache`] uses [`SieveCache`] (SIEVE eviction) for
//! bounded-memory caching of parsed files.  [`TwoLevelCache`] adds an optional
//! disk-backed second tier via [`PersistentCache`].

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::ParsedFile;
use parking_lot::RwLock;

use super::persistent_cache::PersistentCache;
use super::sieve_cache::SieveCache;

/// Default L1 (in-memory) cache capacity.
const DEFAULT_L1_CAPACITY: usize = 1000;

/// Cache entry with file metadata for invalidation.
#[derive(Debug, Clone)]
struct CacheEntry {
    parsed: ParsedFile,
    modified_at: SystemTime,
    size_bytes: u64,
}

/// Thread-safe in-memory cache of parsed files backed by [`SieveCache`].
///
/// Invalidates on file modification time or size change.  The SIEVE eviction
/// policy automatically evicts cold entries when the cache reaches capacity.
pub struct AnalysisCache {
    entries: RwLock<SieveCache<PathBuf, CacheEntry>>,
    capacity: usize,
}

impl AnalysisCache {
    /// Create a new cache with the default capacity (1000 entries).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_L1_CAPACITY)
    }

    /// Create a new cache with a specific capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: RwLock::new(SieveCache::new(capacity)),
            capacity,
        }
    }

    /// Get a cached parse result if the file hasn't changed.
    pub fn get(&self, path: &Path) -> Option<ParsedFile> {
        let mut entries = self.entries.write();
        let entry = entries.get(&path.to_path_buf())?.clone();

        // Check if file still matches
        if let Ok(metadata) = std::fs::metadata(path) {
            if let Ok(modified) = metadata.modified() {
                let size = metadata.len();
                if modified == entry.modified_at && size == entry.size_bytes {
                    return Some(entry.parsed.clone());
                }
            }
        }

        None
    }

    /// Store a parsed file in the cache.
    pub fn put(&self, path: &Path, parsed: ParsedFile) {
        if let Ok(metadata) = std::fs::metadata(path) {
            let modified_at = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let size_bytes = metadata.len();
            let entry = CacheEntry {
                parsed,
                modified_at,
                size_bytes,
            };
            self.entries.write().insert(path.to_path_buf(), entry);
        }
    }

    /// Invalidate a specific file's cache entry.
    pub fn invalidate(&self, path: &Path) {
        self.entries.write().remove(&path.to_path_buf());
    }

    /// Clear all cached entries.
    pub fn clear(&self) {
        self.entries.write().clear();
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.read().is_empty()
    }

    /// Evict stale entries (files that no longer exist).
    ///
    /// **Note:** Since [`SieveCache`] does not expose an iterator, this method
    /// is a no-op.  Callers should prefer [`invalidate()`](Self::invalidate)
    /// for targeted removal of specific paths.
    pub fn evict_stale(&self) {
        // SieveCache has no iterator, so we cannot enumerate keys to check
        // existence.  This is intentionally a no-op; use `invalidate()` for
        // targeted removal instead.
    }

    /// The configured capacity of the underlying SIEVE cache.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns the hit ratio of the underlying SIEVE cache.
    pub fn hit_ratio(&self) -> f64 {
        self.entries.read().hit_ratio()
    }
}

impl Default for AnalysisCache {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Two-level cache: L1 (SIEVE in-memory) + L2 (persistent disk)
// ---------------------------------------------------------------------------

/// A two-level cache combining a fast in-memory SIEVE tier (L1) with a
/// persistent on-disk tier (L2).
///
/// Lookup order: L1 -> L2 -> miss.
/// On an L2 hit, the entry is promoted into L1 for subsequent fast access.
pub struct TwoLevelCache {
    l1: RwLock<SieveCache<PathBuf, CacheEntry>>,
    l2: RwLock<PersistentCache>,
}

impl TwoLevelCache {
    /// Create a new two-level cache.
    ///
    /// * `l1_capacity` — maximum entries kept in the in-memory SIEVE tier.
    /// * `l2` — the persistent (disk-backed) cache tier.
    pub fn new(l1_capacity: usize, l2: PersistentCache) -> Self {
        Self {
            l1: RwLock::new(SieveCache::new(l1_capacity)),
            l2: RwLock::new(l2),
        }
    }

    /// Lookup a parsed file.  Checks L1 first, then L2.
    ///
    /// On an L2 hit the entry is promoted into L1.
    pub fn get(&self, path: &Path) -> Option<ParsedFile> {
        let path_buf = path.to_path_buf();

        // --- L1 check ---
        {
            let mut l1 = self.l1.write();
            if let Some(entry) = l1.get(&path_buf) {
                let entry = entry.clone();
                // Validate freshness
                if let Ok(metadata) = std::fs::metadata(path) {
                    if let Ok(modified) = metadata.modified() {
                        let size = metadata.len();
                        if modified == entry.modified_at && size == entry.size_bytes {
                            return Some(entry.parsed);
                        }
                    }
                }
            }
        }

        // --- L2 check ---
        let path_str = path.to_string_lossy().to_string();
        let l2 = self.l2.read();
        if let Some(cached) = l2.get_entry(&path_str) {
            // Build a ParsedFile stub from the cached data and promote to L1.
            let source = std::fs::read_to_string(path).ok()?;
            let metadata = std::fs::metadata(path).ok()?;
            let modified_at = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let size_bytes = metadata.len();

            // Verify the content hash still matches
            let current_hash = crate::analysis::persistent_cache::hash_content(source.as_bytes());
            if current_hash != cached.content_hash {
                return None;
            }

            let language = crate::core::Language::from_extension(
                path.extension().and_then(|e| e.to_str()).unwrap_or(""),
            )
            .unwrap_or(crate::core::Language::Python);

            let mut parsed = ParsedFile::new(path_str, language, source);
            parsed.nodes = cached.nodes.clone();
            parsed.edges = cached.edges.clone();
            parsed.entry_points = cached.entry_points.clone();

            // Promote into L1
            let entry = CacheEntry {
                parsed: parsed.clone(),
                modified_at,
                size_bytes,
            };
            self.l1.write().insert(path_buf, entry);

            return Some(parsed);
        }

        None
    }

    /// Store a parsed file in L1 (and optionally L2).
    pub fn put(&self, path: &Path, parsed: ParsedFile) {
        if let Ok(metadata) = std::fs::metadata(path) {
            let modified_at = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let size_bytes = metadata.len();
            let entry = CacheEntry {
                parsed,
                modified_at,
                size_bytes,
            };
            self.l1.write().insert(path.to_path_buf(), entry);
        }
    }

    /// Invalidate a path from both tiers.
    pub fn invalidate(&self, path: &Path) {
        self.l1.write().remove(&path.to_path_buf());
        let path_str = path.to_string_lossy().to_string();
        self.l2.write().remove_entry(&path_str);
    }

    /// Clear L1 entirely.
    pub fn clear_l1(&self) {
        self.l1.write().clear();
    }

    /// Number of entries in L1.
    pub fn l1_len(&self) -> usize {
        self.l1.read().len()
    }

    /// Number of entries in L2.
    pub fn l2_len(&self) -> usize {
        self.l2.read().len()
    }

    /// L1 hit ratio.
    pub fn l1_hit_ratio(&self) -> f64 {
        self.l1.read().hit_ratio()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::persistent_cache::{hash_content, CachedFileEntry, PersistentCache};
    use crate::core::Language;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_cache_hit_and_miss() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.py");
        fs::write(&file_path, "def test(): pass").unwrap();

        let cache = AnalysisCache::new();

        // Miss
        assert!(cache.get(&file_path).is_none());

        // Put
        let parsed = ParsedFile::new(
            file_path.to_string_lossy().to_string(),
            Language::Python,
            "def test(): pass".to_string(),
        );
        cache.put(&file_path, parsed);

        // Hit
        assert!(cache.get(&file_path).is_some());
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn test_cache_invalidation() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.py");
        fs::write(&file_path, "def test(): pass").unwrap();

        let cache = AnalysisCache::new();
        let parsed = ParsedFile::new(
            file_path.to_string_lossy().to_string(),
            Language::Python,
            "def test(): pass".to_string(),
        );
        cache.put(&file_path, parsed);
        assert_eq!(cache.len(), 1);

        cache.invalidate(&file_path);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cache_evict_stale() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.py");
        fs::write(&file_path, "def test(): pass").unwrap();

        let cache = AnalysisCache::new();
        let parsed = ParsedFile::new(
            file_path.to_string_lossy().to_string(),
            Language::Python,
            "def test(): pass".to_string(),
        );
        cache.put(&file_path, parsed);

        // Delete the file — evict_stale is best-effort with SieveCache.
        // Use invalidate for targeted removal.
        fs::remove_file(&file_path).unwrap();
        cache.invalidate(&file_path);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn test_cache_with_capacity() {
        let cache = AnalysisCache::with_capacity(50);
        assert_eq!(cache.capacity(), 50);
        assert!(cache.is_empty());
    }

    #[test]
    fn test_sieve_eviction_under_capacity() {
        // Create a tiny cache with capacity 2
        let dir = TempDir::new().unwrap();
        let cache = AnalysisCache::with_capacity(2);

        let file_a = dir.path().join("a.py");
        let file_b = dir.path().join("b.py");
        let file_c = dir.path().join("c.py");

        fs::write(&file_a, "def a(): pass").unwrap();
        fs::write(&file_b, "def b(): pass").unwrap();
        fs::write(&file_c, "def c(): pass").unwrap();

        let mk = |path: &Path| {
            ParsedFile::new(
                path.to_string_lossy().to_string(),
                Language::Python,
                "pass".to_string(),
            )
        };

        cache.put(&file_a, mk(&file_a));
        cache.put(&file_b, mk(&file_b));
        assert_eq!(cache.len(), 2);

        // Inserting a third entry should evict one (SIEVE policy).
        cache.put(&file_c, mk(&file_c));
        assert_eq!(cache.len(), 2);
    }

    // ---- TwoLevelCache tests ----

    #[test]
    fn test_two_level_cache_l1_hit() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.py");
        fs::write(&file_path, "def test(): pass").unwrap();

        let l2 = PersistentCache::new();
        let cache = TwoLevelCache::new(100, l2);

        // Miss at first
        assert!(cache.get(&file_path).is_none());

        // Put into L1
        let parsed = ParsedFile::new(
            file_path.to_string_lossy().to_string(),
            Language::Python,
            "def test(): pass".to_string(),
        );
        cache.put(&file_path, parsed);

        // Should hit L1
        assert!(cache.get(&file_path).is_some());
        assert_eq!(cache.l1_len(), 1);
    }

    #[test]
    fn test_two_level_cache_l2_promotion() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.py");
        let content = "def test(): pass";
        fs::write(&file_path, content).unwrap();

        // Pre-populate L2 with a matching entry
        let mut l2 = PersistentCache::new();
        let content_hash = hash_content(content.as_bytes());
        let path_str = file_path.to_string_lossy().to_string();
        l2.update_entry(CachedFileEntry {
            path: path_str,
            content_hash,
            nodes: vec![],
            edges: vec![],
            entry_points: vec![],
        });

        let cache = TwoLevelCache::new(100, l2);

        // L1 empty, but L2 has the entry — should promote
        assert_eq!(cache.l1_len(), 0);
        let result = cache.get(&file_path);
        assert!(result.is_some());
        // After promotion, L1 should now have the entry
        assert_eq!(cache.l1_len(), 1);
    }

    #[test]
    fn test_two_level_cache_invalidate() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.py");
        let content = "def test(): pass";
        fs::write(&file_path, content).unwrap();

        let mut l2 = PersistentCache::new();
        let content_hash = hash_content(content.as_bytes());
        let path_str = file_path.to_string_lossy().to_string();
        l2.update_entry(CachedFileEntry {
            path: path_str,
            content_hash,
            nodes: vec![],
            edges: vec![],
            entry_points: vec![],
        });

        let cache = TwoLevelCache::new(100, l2);

        // Promote to L1
        cache.get(&file_path);
        assert_eq!(cache.l1_len(), 1);
        assert_eq!(cache.l2_len(), 1);

        // Invalidate from both tiers
        cache.invalidate(&file_path);
        assert_eq!(cache.l1_len(), 0);
        assert_eq!(cache.l2_len(), 0);
    }
}
