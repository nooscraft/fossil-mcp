//! Persistent fingerprint storage for clone evolution tracking.
//!
//! Stores per-function fingerprints (MinHash sketch, SimHash, content hash) across
//! multiple snapshots (e.g. git commits). Supports JSON serialization for offline
//! storage and roundtrip fidelity.

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Unique identifier for a codebase snapshot (e.g. git commit hash or ISO timestamp).
pub type SnapshotId = String;

/// Fingerprint of a single function at a specific snapshot in time.
///
/// Captures enough information to identify the function across snapshots and
/// compare its content via probabilistic sketches.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionFingerprint {
    /// Path of the file containing the function.
    pub file: String,
    /// Name of the function (e.g. `process_data`, `MyClass.run`).
    pub name: String,
    /// Start line (1-indexed).
    pub start_line: usize,
    /// End line (1-indexed, inclusive).
    pub end_line: usize,
    /// MinHash sketch stored as a vector of shingle hashes.
    ///
    /// Used for Jaccard similarity estimation between function pairs.
    /// Computed from normalized token k-grams via xxh3.
    pub minhash_sketch: Vec<u64>,
    /// SimHash fingerprint (64-bit).
    ///
    /// Used for fast Hamming-distance screening before expensive comparisons.
    pub simhash: u64,
    /// Content hash (xxh3_64 of the raw source text).
    ///
    /// Detects exact content changes between snapshots.
    pub content_hash: u64,
    /// Timestamp or label for when this fingerprint was captured.
    pub timestamp: String,
}

/// Persistent store mapping snapshot IDs to their function fingerprints.
///
/// Provides save/load via JSON serialization and lookup by snapshot ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FingerprintStore {
    snapshots: HashMap<SnapshotId, Vec<FunctionFingerprint>>,
}

impl FingerprintStore {
    /// Creates an empty fingerprint store.
    pub fn new() -> Self {
        Self {
            snapshots: HashMap::new(),
        }
    }

    /// Adds a snapshot with its function fingerprints.
    ///
    /// If a snapshot with the same ID already exists, it is replaced.
    pub fn add_snapshot(&mut self, id: SnapshotId, fingerprints: Vec<FunctionFingerprint>) {
        self.snapshots.insert(id, fingerprints);
    }

    /// Retrieves fingerprints for a given snapshot ID.
    pub fn get_snapshot(&self, id: &str) -> Option<&Vec<FunctionFingerprint>> {
        self.snapshots.get(id)
    }

    /// Serializes the store to JSON and writes it to the given path.
    ///
    /// # Errors
    ///
    /// Returns an error string if file creation or serialization fails.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let json =
            serde_json::to_string_pretty(self).map_err(|e| format!("Serialization error: {e}"))?;
        std::fs::write(path, json).map_err(|e| format!("Write error: {e}"))
    }

    /// Deserializes a store from a JSON file at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error string if reading or deserialization fails.
    pub fn load(path: &Path) -> Result<Self, String> {
        let contents = std::fs::read_to_string(path).map_err(|e| format!("Read error: {e}"))?;
        serde_json::from_str(&contents).map_err(|e| format!("Deserialization error: {e}"))
    }

    /// Returns all snapshot IDs, sorted lexicographically.
    pub fn list_snapshots(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.snapshots.keys().map(|s| s.as_str()).collect();
        ids.sort_unstable();
        ids
    }
}

impl Default for FingerprintStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fingerprint(file: &str, name: &str) -> FunctionFingerprint {
        FunctionFingerprint {
            file: file.to_string(),
            name: name.to_string(),
            start_line: 1,
            end_line: 10,
            minhash_sketch: vec![100, 200, 300],
            simhash: 0xDEAD_BEEF,
            content_hash: 0xCAFE_BABE,
            timestamp: "2025-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_store_add_and_get_snapshot() {
        let mut store = FingerprintStore::new();
        let fps = vec![sample_fingerprint("a.py", "foo")];
        store.add_snapshot("abc123".to_string(), fps.clone());

        let retrieved = store.get_snapshot("abc123");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().len(), 1);
        assert_eq!(retrieved.unwrap()[0].name, "foo");

        assert!(store.get_snapshot("nonexistent").is_none());
    }

    #[test]
    fn test_store_list_snapshots_sorted() {
        let mut store = FingerprintStore::new();
        store.add_snapshot("zzz".to_string(), vec![]);
        store.add_snapshot("aaa".to_string(), vec![]);
        store.add_snapshot("mmm".to_string(), vec![]);

        let ids = store.list_snapshots();
        assert_eq!(ids, vec!["aaa", "mmm", "zzz"]);
    }

    #[test]
    fn test_store_save_load_roundtrip() {
        let dir = tempfile::tempdir().expect("Failed to create temp dir");
        let path = dir.path().join("fingerprints.json");

        let mut store = FingerprintStore::new();
        store.add_snapshot(
            "commit_a".to_string(),
            vec![
                sample_fingerprint("src/lib.rs", "parse"),
                sample_fingerprint("src/lib.rs", "analyze"),
            ],
        );
        store.add_snapshot(
            "commit_b".to_string(),
            vec![sample_fingerprint("src/main.rs", "main")],
        );

        store.save(&path).expect("Failed to save");
        let loaded = FingerprintStore::load(&path).expect("Failed to load");

        assert_eq!(loaded.list_snapshots(), store.list_snapshots());

        let a_fps = loaded.get_snapshot("commit_a").unwrap();
        assert_eq!(a_fps.len(), 2);
        assert_eq!(a_fps[0], store.get_snapshot("commit_a").unwrap()[0]);
        assert_eq!(a_fps[1], store.get_snapshot("commit_a").unwrap()[1]);

        let b_fps = loaded.get_snapshot("commit_b").unwrap();
        assert_eq!(b_fps.len(), 1);
        assert_eq!(b_fps[0], store.get_snapshot("commit_b").unwrap()[0]);
    }

    #[test]
    fn test_store_replace_snapshot() {
        let mut store = FingerprintStore::new();
        store.add_snapshot("snap".to_string(), vec![sample_fingerprint("a.py", "old")]);
        store.add_snapshot("snap".to_string(), vec![sample_fingerprint("b.py", "new")]);

        let fps = store.get_snapshot("snap").unwrap();
        assert_eq!(fps.len(), 1);
        assert_eq!(fps[0].name, "new");
    }

    #[test]
    fn test_store_default() {
        let store = FingerprintStore::default();
        assert!(store.list_snapshots().is_empty());
    }

    #[test]
    fn test_load_nonexistent_file() {
        let result = FingerprintStore::load(Path::new("/nonexistent/path/store.json"));
        assert!(result.is_err());
    }
}
