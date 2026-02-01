//! sketch_oxide integration for enriched clone and code analysis features.
//!
//! This module combines multiple probabilistic data structures from `sketch_oxide`
//! to provide efficient, approximate analysis of code tokens and function-level metrics.
//!
//! ## Overview
//!
//! - [`SketchFeatures`] captures per-function token statistics:
//!   unique token cardinality, token frequency, complexity distribution, and
//!   a probabilistic token set for Jaccard similarity.
//!
//! - [`ProjectSketchStats`] captures project-wide aggregates:
//!   function call frequencies and top-K most referenced code snippets.

use sketch_oxide::quantiles::DDSketch;
use sketch_oxide::{CountMinSketch, HeavyKeeper, ThetaSketch, UltraLogLog};
use xxhash_rust::xxh3::xxh3_64;

// ---------------------------------------------------------------------------
// Per-function sketch features
// ---------------------------------------------------------------------------

/// Per-function sketch features for enriched clone analysis.
///
/// Combines several sketch data structures to capture different statistical
/// properties of a function's token stream:
///
/// - **`token_cardinality`** ([`UltraLogLog`]): estimates the number of unique
///   tokens (variables, keywords, operators, etc.) in a function body.
/// - **`token_frequencies`** ([`CountMinSketch`]): tracks approximate frequency
///   of each distinct token, useful for weighting similarity computations.
/// - **`complexity_sketch`** ([`DDSketch`]): records a distribution of per-token
///   "complexity" values (e.g. nesting depth, cyclomatic contribution) so that
///   quantiles such as median and p99 complexity can be retrieved later.
/// - **`token_set`** ([`ThetaSketch`]): a probabilistic set representation that
///   supports Jaccard similarity estimation between two functions' token sets.
pub struct SketchFeatures {
    /// Estimates unique token count per function.
    pub token_cardinality: UltraLogLog,
    /// Tracks approximate token frequency.
    pub token_frequencies: CountMinSketch,
    /// Distribution of complexity values.
    pub complexity_sketch: DDSketch,
    /// Probabilistic set for Jaccard similarity.
    pub token_set: ThetaSketch,
}

impl SketchFeatures {
    /// Creates a new `SketchFeatures` with sensible defaults.
    ///
    /// # Errors
    ///
    /// Returns an error string if any underlying sketch fails to initialize
    /// (should not happen with the hardcoded parameters).
    pub fn new() -> Result<Self, String> {
        let token_cardinality =
            UltraLogLog::new(12).map_err(|e| format!("UltraLogLog init: {e}"))?;
        let token_frequencies =
            CountMinSketch::new(0.01, 0.01).map_err(|e| format!("CountMinSketch init: {e}"))?;
        let complexity_sketch = DDSketch::new(0.01).map_err(|e| format!("DDSketch init: {e}"))?;
        let token_set = ThetaSketch::new(12).map_err(|e| format!("ThetaSketch init: {e}"))?;

        Ok(Self {
            token_cardinality,
            token_frequencies,
            complexity_sketch,
            token_set,
        })
    }

    /// Builds a `SketchFeatures` from a token slice.
    ///
    /// Each token string is hashed with xxh3_64 and inserted into all four
    /// sketches.  The complexity sketch receives the hash cast to a positive
    /// f64 value (modulo-mapped to the range \[1, 1001\]) so that quantile
    /// queries remain meaningful.
    ///
    /// # Arguments
    ///
    /// * `tokens` - slice of token string references extracted from a function body.
    ///
    /// # Errors
    ///
    /// Propagates initialization errors from [`SketchFeatures::new`].
    pub fn from_tokens(tokens: &[&str]) -> Result<Self, String> {
        let mut features = Self::new()?;

        for &token in tokens {
            let hash = xxh3_64(token.as_bytes());

            // UltraLogLog — cardinality estimation (accepts any Hash type)
            features.token_cardinality.add(&hash);

            // CountMinSketch — frequency tracking (accepts any Hash type)
            features.token_frequencies.update(&hash);

            // DDSketch — complexity distribution (needs a positive f64 value)
            // Map the hash into a positive range [1.0, 1001.0] to keep
            // quantile queries well-behaved.
            let complexity_value = (hash % 1000) as f64 + 1.0;
            features.complexity_sketch.add(complexity_value);

            // ThetaSketch — probabilistic set membership (accepts any Hash type)
            features.token_set.update(&hash);
        }

        Ok(features)
    }

    /// Returns the estimated number of unique tokens.
    pub fn estimated_unique_tokens(&self) -> f64 {
        self.token_cardinality.cardinality()
    }

    /// Returns the estimated frequency of a specific token.
    pub fn token_frequency(&self, token: &str) -> u64 {
        let hash = xxh3_64(token.as_bytes());
        self.token_frequencies.estimate(&hash)
    }

    /// Returns the estimated median complexity value.
    pub fn median_complexity(&self) -> Option<f64> {
        self.complexity_sketch.quantile(0.5)
    }

    /// Returns the estimated p99 complexity value.
    pub fn p99_complexity(&self) -> Option<f64> {
        self.complexity_sketch.quantile(0.99)
    }
}

/// Computes an approximate Jaccard similarity between two [`ThetaSketch`] instances.
///
/// Jaccard similarity J(A, B) = |A intersect B| / |A union B|.
///
/// Both sketches must have been created with the same `lg_k` and seed (the
/// default constructor satisfies this).
///
/// Returns 0.0 if the union is empty, or if the sketches are incompatible.
pub fn jaccard_similarity(a: &ThetaSketch, b: &ThetaSketch) -> f64 {
    let intersection = match a.intersect(b) {
        Ok(s) => s,
        Err(_) => return 0.0,
    };
    let union = match a.union(b) {
        Ok(s) => s,
        Err(_) => return 0.0,
    };

    let union_est = union.estimate();
    if union_est <= 0.0 {
        return 0.0;
    }

    let intersection_est = intersection.estimate();
    // Clamp to [0, 1] to guard against floating point drift.
    (intersection_est / union_est).clamp(0.0, 1.0)
}

// ---------------------------------------------------------------------------
// Project-level sketch statistics
// ---------------------------------------------------------------------------

/// Project-wide sketch statistics for code analysis.
///
/// - **`call_frequency`** ([`CountMinSketch`]): tracks how often each function
///   name appears as a call target across the entire project.
/// - **`heaviest_snippets`** ([`HeavyKeeper`]): identifies the top-K most
///   referenced code snippets (by byte representation) in the project.
pub struct ProjectSketchStats {
    /// Approximate frequency of each function being called.
    pub call_frequency: CountMinSketch,
    /// Top-K most referenced code snippets.
    pub heaviest_snippets: HeavyKeeper,
}

impl ProjectSketchStats {
    /// Creates a new `ProjectSketchStats` with sensible defaults.
    ///
    /// Tracks the top-100 heaviest snippets.
    ///
    /// # Errors
    ///
    /// Returns an error string if any underlying sketch fails to initialize.
    pub fn new() -> Result<Self, String> {
        let call_frequency =
            CountMinSketch::new(0.001, 0.01).map_err(|e| format!("CountMinSketch init: {e}"))?;
        let heaviest_snippets =
            HeavyKeeper::new(100, 0.001, 0.01).map_err(|e| format!("HeavyKeeper init: {e}"))?;

        Ok(Self {
            call_frequency,
            heaviest_snippets,
        })
    }

    /// Records a function call.
    ///
    /// The function name is hashed to a `u64` via xxh3_64 and inserted into
    /// the `call_frequency` sketch.
    pub fn record_call(&mut self, function_name: &str) {
        let hash = xxh3_64(function_name.as_bytes());
        self.call_frequency.update(&hash);
    }

    /// Records a code snippet reference.
    ///
    /// The snippet bytes are inserted directly into the `heaviest_snippets`
    /// HeavyKeeper.
    pub fn record_snippet(&mut self, snippet: &[u8]) {
        self.heaviest_snippets.update(snippet);
    }

    /// Returns the estimated call frequency for a given function name.
    pub fn estimated_calls(&self, function_name: &str) -> u64 {
        let hash = xxh3_64(function_name.as_bytes());
        self.call_frequency.estimate(&hash)
    }

    /// Returns the top-K heaviest snippets as `(item_hash, count)` pairs.
    pub fn top_snippets(&self) -> Vec<(u64, u32)> {
        self.heaviest_snippets.top_k()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sketch_features_new() {
        let features = SketchFeatures::new();
        assert!(features.is_ok());
    }

    #[test]
    fn test_sketch_features_from_empty_tokens() {
        let features = SketchFeatures::from_tokens(&[]).unwrap();
        // No tokens inserted -- cardinality should be 0.
        assert!(features.estimated_unique_tokens() < 1.0);
    }

    #[test]
    fn test_sketch_features_from_tokens_cardinality() {
        let tokens = vec!["fn", "main", "(", ")", "{", "let", "x", "=", "1", ";", "}"];
        let features = SketchFeatures::from_tokens(&tokens).unwrap();

        // 11 unique tokens -- UltraLogLog should be reasonably close.
        let est = features.estimated_unique_tokens();
        assert!(
            (5.0..=20.0).contains(&est),
            "Expected cardinality near 11, got {est}"
        );
    }

    #[test]
    fn test_sketch_features_token_frequency() {
        let tokens = vec!["let", "let", "let", "x", "y"];
        let features = SketchFeatures::from_tokens(&tokens).unwrap();

        // "let" appears 3 times -- CountMinSketch never underestimates.
        let freq = features.token_frequency("let");
        assert!(freq >= 3, "Expected frequency >= 3 for 'let', got {freq}");

        // "x" appears once.
        let freq_x = features.token_frequency("x");
        assert!(freq_x >= 1, "Expected frequency >= 1 for 'x', got {freq_x}");
    }

    #[test]
    fn test_sketch_features_complexity_quantiles() {
        let tokens: Vec<&str> = (0..100).map(|_| "tok").collect();
        let features = SketchFeatures::from_tokens(&tokens).unwrap();

        let median = features.median_complexity();
        assert!(median.is_some(), "Expected median to be Some");

        let p99 = features.p99_complexity();
        assert!(p99.is_some(), "Expected p99 to be Some");
    }

    #[test]
    fn test_jaccard_similarity_identical_sets() {
        let tokens = vec!["a", "b", "c", "d", "e"];
        let a = SketchFeatures::from_tokens(&tokens).unwrap();
        let b = SketchFeatures::from_tokens(&tokens).unwrap();

        let sim = jaccard_similarity(&a.token_set, &b.token_set);
        assert!(
            sim > 0.8,
            "Identical token sets should have high similarity, got {sim}"
        );
    }

    #[test]
    fn test_jaccard_similarity_disjoint_sets() {
        let tokens_a = vec!["a", "b", "c"];
        let tokens_b = vec!["x", "y", "z"];
        let a = SketchFeatures::from_tokens(&tokens_a).unwrap();
        let b = SketchFeatures::from_tokens(&tokens_b).unwrap();

        let sim = jaccard_similarity(&a.token_set, &b.token_set);
        assert!(
            sim < 0.3,
            "Disjoint token sets should have low similarity, got {sim}"
        );
    }

    #[test]
    fn test_jaccard_similarity_partial_overlap() {
        let tokens_a = vec!["a", "b", "c", "d"];
        let tokens_b = vec!["c", "d", "e", "f"];
        let a = SketchFeatures::from_tokens(&tokens_a).unwrap();
        let b = SketchFeatures::from_tokens(&tokens_b).unwrap();

        let sim = jaccard_similarity(&a.token_set, &b.token_set);
        // Exact Jaccard = 2/6 ~ 0.33 -- allow generous tolerance.
        assert!(
            sim > 0.1 && sim < 0.7,
            "Partial overlap should yield moderate similarity, got {sim}"
        );
    }

    #[test]
    fn test_jaccard_similarity_empty_sets() {
        let a = SketchFeatures::from_tokens(&[]).unwrap();
        let b = SketchFeatures::from_tokens(&[]).unwrap();

        let sim = jaccard_similarity(&a.token_set, &b.token_set);
        assert!(
            (0.0..=1.0).contains(&sim),
            "Empty set similarity should be in [0, 1], got {sim}"
        );
    }

    #[test]
    fn test_project_sketch_stats_new() {
        let stats = ProjectSketchStats::new();
        assert!(stats.is_ok());
    }

    #[test]
    fn test_project_sketch_stats_record_call() {
        let mut stats = ProjectSketchStats::new().unwrap();
        for _ in 0..50 {
            stats.record_call("process_data");
        }
        stats.record_call("init");

        let freq = stats.estimated_calls("process_data");
        assert!(freq >= 50, "Expected call frequency >= 50, got {freq}");

        let freq_init = stats.estimated_calls("init");
        assert!(
            freq_init >= 1,
            "Expected call frequency >= 1 for 'init', got {freq_init}"
        );

        // Non-existent function should have 0.
        let freq_none = stats.estimated_calls("nonexistent_function");
        assert_eq!(freq_none, 0, "Non-recorded function should have freq 0");
    }

    #[test]
    fn test_project_sketch_stats_record_snippet() {
        let mut stats = ProjectSketchStats::new().unwrap();
        for _ in 0..200 {
            stats.record_snippet(b"fn main() {}");
        }
        for _ in 0..10 {
            stats.record_snippet(b"println!(\"hello\")");
        }

        let top = stats.top_snippets();
        assert!(!top.is_empty(), "Top snippets should not be empty");
    }

    #[test]
    fn test_sketch_features_repeated_tokens() {
        // Repeated tokens should give cardinality close to 1.
        let tokens: Vec<&str> = (0..500).map(|_| "same_token").collect();
        let features = SketchFeatures::from_tokens(&tokens).unwrap();

        let est = features.estimated_unique_tokens();
        assert!(
            est < 5.0,
            "Repeated token cardinality should be near 1, got {est}"
        );

        let freq = features.token_frequency("same_token");
        assert!(freq >= 500, "Expected frequency >= 500, got {freq}");
    }
}
