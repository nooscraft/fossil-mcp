//! Cache store for analysis results across CI/CD runs.
//!
//! Enables incremental analysis: only re-analyze changed files and their dependents.
//! Cache key: (file_hash, language, config_hash) -> AnalysisResult

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Cache configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// Enable caching (default: true)
    pub enabled: bool,
    /// Cache directory path (default: ~/.fossil-cache)
    pub cache_dir: Option<String>,
    /// Cache TTL in hours (default: 168 = 1 week)
    pub ttl_hours: u32,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            cache_dir: None,
            ttl_hours: 168, // 1 week
        }
    }
}

/// Cached analysis result for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedFileAnalysis {
    /// File hash (SHA256 first 16 bytes)
    pub file_hash: String,
    /// Language (e.g., "rust", "typescript")
    pub language: String,
    /// Config hash (serialized config's SHA256 first 16 bytes)
    pub config_hash: String,
    /// Timestamp (UNIX seconds)
    pub timestamp: u64,
    /// Node definitions extracted from file
    pub nodes: Vec<CachedNode>,
    /// Edge definitions extracted from file
    pub edges: Vec<CachedEdge>,
}

/// Cached node definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedNode {
    pub id: String,
    pub name: String,
    pub full_name: String,
    pub kind: String,
    pub file: String,
    pub language: String,
    pub visibility: String,
}

/// Cached edge definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedEdge {
    pub from_id: String,
    pub to_id: String,
    pub confidence: String,
}

/// Cached dead code findings for a file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedDeadCodeFindings {
    pub file_hash: String,
    pub config_hash: String,
    pub timestamp: u64,
    /// Dead code findings (node IDs)
    pub dead_nodes: Vec<String>,
    /// Confidence scores (node_id -> confidence)
    pub confidences: HashMap<String, f64>,
}

/// Cache store for persisting analysis results.
pub struct CacheStore {
    cache_dir: PathBuf,
    enabled: bool,
}

impl CacheStore {
    /// Create a new cache store.
    pub fn new(config: &CacheConfig) -> Result<Self, crate::core::Error> {
        let cache_dir = if let Some(dir) = &config.cache_dir {
            PathBuf::from(dir)
        } else {
            // Default to ~/.fossil-cache
            let home = dirs::home_dir().ok_or_else(|| {
                crate::core::Error::config("Cannot find home directory".to_string())
            })?;
            home.join(".fossil-cache")
        };

        if config.enabled {
            fs::create_dir_all(&cache_dir)
                .map_err(|e| crate::core::Error::config(format!("Cannot create cache dir: {e}")))?;
        }

        Ok(Self {
            cache_dir,
            enabled: config.enabled,
        })
    }

    /// Get cache key for a file.
    pub fn cache_key(file_hash: &str, language: &str, config_hash: &str) -> String {
        format!("{}_{}_{}", file_hash, language, config_hash)
    }

    /// Store file analysis in cache.
    pub fn store_file_analysis(
        &self,
        analysis: &CachedFileAnalysis,
    ) -> Result<(), crate::core::Error> {
        if !self.enabled {
            return Ok(());
        }

        let key = Self::cache_key(
            &analysis.file_hash,
            &analysis.language,
            &analysis.config_hash,
        );
        let cache_file = self.cache_dir.join(format!("{}.json", key));

        let json = serde_json::to_string(analysis)
            .map_err(|e| crate::core::Error::config(format!("Cannot serialize cache: {e}")))?;

        fs::write(cache_file, json)
            .map_err(|e| crate::core::Error::config(format!("Cannot write cache file: {e}")))?;

        Ok(())
    }

    /// Retrieve file analysis from cache.
    pub fn get_file_analysis(
        &self,
        file_hash: &str,
        language: &str,
        config_hash: &str,
    ) -> Result<Option<CachedFileAnalysis>, crate::core::Error> {
        if !self.enabled {
            return Ok(None);
        }

        let key = Self::cache_key(file_hash, language, config_hash);
        let cache_file = self.cache_dir.join(format!("{}.json", key));

        if !cache_file.exists() {
            return Ok(None);
        }

        let json = fs::read_to_string(&cache_file)
            .map_err(|e| crate::core::Error::config(format!("Cannot read cache file: {e}")))?;

        let analysis: CachedFileAnalysis = serde_json::from_str(&json)
            .map_err(|e| crate::core::Error::config(format!("Cannot deserialize cache: {e}")))?;

        Ok(Some(analysis))
    }

    /// Store dead code findings in cache.
    pub fn store_dead_code_findings(
        &self,
        findings: &CachedDeadCodeFindings,
    ) -> Result<(), crate::core::Error> {
        if !self.enabled {
            return Ok(());
        }

        let key = format!("deadcode_{}_{}", findings.file_hash, findings.config_hash);
        let cache_file = self.cache_dir.join(format!("{}.json", key));

        let json = serde_json::to_string(findings)
            .map_err(|e| crate::core::Error::config(format!("Cannot serialize cache: {e}")))?;

        fs::write(cache_file, json)
            .map_err(|e| crate::core::Error::config(format!("Cannot write cache file: {e}")))?;

        Ok(())
    }

    /// Retrieve dead code findings from cache.
    pub fn get_dead_code_findings(
        &self,
        file_hash: &str,
        config_hash: &str,
    ) -> Result<Option<CachedDeadCodeFindings>, crate::core::Error> {
        if !self.enabled {
            return Ok(None);
        }

        let key = format!("deadcode_{}_{}", file_hash, config_hash);
        let cache_file = self.cache_dir.join(format!("{}.json", key));

        if !cache_file.exists() {
            return Ok(None);
        }

        let json = fs::read_to_string(&cache_file)
            .map_err(|e| crate::core::Error::config(format!("Cannot read cache file: {e}")))?;

        let findings: CachedDeadCodeFindings = serde_json::from_str(&json)
            .map_err(|e| crate::core::Error::config(format!("Cannot deserialize cache: {e}")))?;

        Ok(Some(findings))
    }

    /// Clear entire cache.
    pub fn clear(&self) -> Result<(), crate::core::Error> {
        if self.enabled && self.cache_dir.exists() {
            fs::remove_dir_all(&self.cache_dir)
                .map_err(|e| crate::core::Error::config(format!("Cannot clear cache: {e}")))?;
            fs::create_dir_all(&self.cache_dir).map_err(|e| {
                crate::core::Error::config(format!("Cannot recreate cache dir: {e}"))
            })?;
        }
        Ok(())
    }

    /// Remove cache files older than `max_age_hours`.
    pub fn cleanup(&self, max_age_hours: u32) -> Result<usize, crate::core::Error> {
        if !self.enabled || !self.cache_dir.exists() {
            return Ok(0);
        }

        let max_age = std::time::Duration::from_secs(u64::from(max_age_hours) * 3600);
        let now = std::time::SystemTime::now();
        let mut removed = 0;

        let entries = fs::read_dir(&self.cache_dir)
            .map_err(|e| crate::core::Error::config(format!("Cannot read cache dir: {e}")))?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            if let Ok(metadata) = fs::metadata(&path) {
                let is_expired = metadata
                    .modified()
                    .ok()
                    .and_then(|mtime| now.duration_since(mtime).ok())
                    .is_some_and(|age| age > max_age);
                if is_expired {
                    let _ = fs::remove_file(&path);
                    removed += 1;
                }
            }
        }

        Ok(removed)
    }

    /// Get cache statistics.
    pub fn get_stats(&self) -> Result<CacheStats, crate::core::Error> {
        let mut stats = CacheStats::default();

        if !self.cache_dir.exists() {
            return Ok(stats);
        }

        let entries = fs::read_dir(&self.cache_dir)
            .map_err(|e| crate::core::Error::config(format!("Cannot read cache dir: {e}")))?;

        for entry in entries {
            let entry = entry
                .map_err(|e| crate::core::Error::config(format!("Cannot read cache entry: {e}")))?;
            let path = entry.path();

            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                stats.total_files += 1;

                if let Ok(metadata) = fs::metadata(&path) {
                    stats.total_size_bytes += metadata.len() as usize;
                }
            }
        }

        Ok(stats)
    }
}

/// Cache statistics.
#[derive(Debug, Default, Clone)]
pub struct CacheStats {
    pub total_files: usize,
    pub total_size_bytes: usize,
    pub hits: usize,
    pub misses: usize,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }

    pub fn total_size_kb(&self) -> f64 {
        self.total_size_bytes as f64 / 1024.0
    }

    pub fn total_size_mb(&self) -> f64 {
        self.total_size_bytes as f64 / (1024.0 * 1024.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_key_format() {
        let key = CacheStore::cache_key("abc123", "rust", "def456");
        assert_eq!(key, "abc123_rust_def456");
    }

    #[test]
    fn test_cache_stats_hit_rate() {
        let stats = CacheStats {
            hits: 90,
            misses: 10,
            ..Default::default()
        };
        assert_eq!(stats.hit_rate(), 90.0);
    }

    #[test]
    fn test_cache_stats_empty() {
        let stats = CacheStats::default();
        assert_eq!(stats.hit_rate(), 0.0);
    }
}
