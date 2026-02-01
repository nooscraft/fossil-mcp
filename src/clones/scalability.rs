//! Scalable clone detection via N-gram indexing with stop-N-gram filtering.
//!
//! Combines [`NgramIndex`] for sub-quadratic candidate generation with
//! [`StopNgramFilter`] to prune ubiquitous N-grams (e.g., common boilerplate
//! patterns like `{ return $ID }`) that would cause excessive false-positive
//! pairings.

use std::collections::HashMap;

use super::ngram_index::NgramIndex;

/// Filters out N-grams that appear in more than a configurable fraction of documents.
///
/// Common patterns like `{ return x ; }` appear in nearly every function and
/// provide no discriminative signal for clone detection. This filter identifies
/// and removes them after a two-phase process:
///
/// 1. **Recording phase**: Call `record_ngram` for each N-gram occurrence.
/// 2. **Finalization phase**: Call `finalize` to compute the stop set.
/// 3. **Query phase**: Call `is_stop_ngram` to check membership.
#[derive(Debug)]
pub struct StopNgramFilter {
    /// Number of documents that contain each N-gram hash.
    doc_frequencies: HashMap<u64, usize>,
    /// Total number of documents in the corpus.
    total_docs: usize,
    /// Fraction threshold: N-grams appearing in more than this fraction are stop-N-grams.
    threshold_fraction: f64,
    /// The computed set of stop-N-gram hashes (populated after `finalize`).
    stop_set: HashMap<u64, bool>,
    /// Whether `finalize` has been called.
    finalized: bool,
}

impl StopNgramFilter {
    /// Create a new filter.
    ///
    /// - `total_docs`: the total number of documents that will be analyzed.
    /// - `threshold_fraction`: N-grams appearing in more than this fraction of
    ///   documents are considered stop-N-grams. Default is 0.5 (50%).
    pub fn new(total_docs: usize, threshold_fraction: f64) -> Self {
        Self {
            doc_frequencies: HashMap::new(),
            total_docs,
            threshold_fraction: threshold_fraction.clamp(0.0, 1.0),
            stop_set: HashMap::new(),
            finalized: false,
        }
    }

    /// Record that `ngram_hash` appears in one more document.
    ///
    /// Must be called before `finalize`. Each N-gram should be recorded at most
    /// once per document (callers should deduplicate per-document N-grams).
    pub fn record_ngram(&mut self, ngram_hash: u64) {
        *self.doc_frequencies.entry(ngram_hash).or_insert(0) += 1;
    }

    /// Compute the stop-N-gram set from recorded document frequencies.
    ///
    /// After calling this, `is_stop_ngram` will return accurate results.
    pub fn finalize(&mut self) {
        // "Appears in >50% of documents" => count > total_docs * threshold_fraction.
        // Using floor ensures strict greater-than semantics: for 10 docs at 0.5,
        // threshold_count = 5, so only N-grams in 6+ docs are stop-N-grams.
        // For 2 docs at 0.5, threshold_count = 1, so only N-grams in 2 docs are stop-N-grams.
        let threshold_count = (self.total_docs as f64 * self.threshold_fraction).floor() as usize;

        self.stop_set.clear();
        for (&hash, &count) in &self.doc_frequencies {
            if count > threshold_count {
                self.stop_set.insert(hash, true);
            }
        }

        self.finalized = true;
    }

    /// Check whether the given N-gram hash is a stop-N-gram.
    ///
    /// Returns `false` if `finalize` has not been called yet.
    pub fn is_stop_ngram(&self, ngram_hash: u64) -> bool {
        self.finalized && self.stop_set.contains_key(&ngram_hash)
    }

    /// Return the number of N-grams identified as stop-N-grams.
    pub fn num_stop_ngrams(&self) -> usize {
        self.stop_set.len()
    }
}

/// Scalable clone detector combining N-gram indexing with stop-N-gram filtering.
///
/// Usage:
/// 1. Call `add_function` for each function/code block in the corpus. This
///    records N-gram frequencies for stop-N-gram detection.
/// 2. Call `build_index` to finalize the stop filter and build the N-gram index
///    (excluding stop-N-grams).
/// 3. Call `find_clone_candidates` to retrieve candidate pairs with estimated
///    similarity scores.
#[derive(Debug)]
pub struct ScalableCloneDetector {
    /// The N-gram inverted index (populated during `build_index`).
    index: NgramIndex,
    /// Stop-N-gram filter.
    stop_filter: StopNgramFilter,
    /// Per-document raw N-gram hashes (before stop filtering), stored for index build.
    raw_ngrams: Vec<(usize, Vec<u64>)>,
    /// Whether the index has been built.
    built: bool,
}

impl ScalableCloneDetector {
    /// Create a new scalable clone detector with default stop-N-gram threshold (0.5).
    ///
    /// - `estimated_docs`: approximate number of documents (used to size the stop filter).
    /// - `ngram_size`: retained for API compatibility (N-gram size is passed per-call).
    pub fn new(estimated_docs: usize, _ngram_size: usize) -> Self {
        Self {
            index: NgramIndex::new(),
            stop_filter: StopNgramFilter::new(estimated_docs, 0.5),
            raw_ngrams: Vec::new(),
            built: false,
        }
    }

    /// Set a custom stop-N-gram threshold fraction.
    ///
    /// N-grams appearing in more than this fraction of documents are filtered out.
    /// Default is 0.5 (50%). Set to 1.0 to disable stop filtering entirely.
    pub fn with_threshold(mut self, threshold_fraction: f64) -> Self {
        self.stop_filter = StopNgramFilter::new(self.stop_filter.total_docs, threshold_fraction);
        self
    }

    /// Add a function's tokens to the detector.
    ///
    /// Extracts N-grams from the token sequence and records their document frequencies
    /// for stop-N-gram detection.
    pub fn add_function(&mut self, doc_id: usize, tokens: &[&str], ngram_size: usize) {
        let ngrams = NgramIndex::ngrams_from_tokens(tokens, ngram_size);

        // Record each *distinct* N-gram for this document in the stop filter.
        let mut seen = std::collections::HashSet::new();
        for &hash in &ngrams {
            if seen.insert(hash) {
                self.stop_filter.record_ngram(hash);
            }
        }

        self.raw_ngrams.push((doc_id, ngrams));
    }

    /// Finalize the stop filter and build the N-gram index.
    ///
    /// Must be called after all documents have been added via `add_function`
    /// and before calling `find_clone_candidates`.
    pub fn build_index(&mut self) {
        // Update total_docs to the actual count.
        self.stop_filter.total_docs = self.raw_ngrams.len();
        self.stop_filter.finalize();

        // Build the index, excluding stop-N-grams.
        self.index = NgramIndex::new();
        for (doc_id, ngrams) in &self.raw_ngrams {
            let filtered: Vec<u64> = ngrams
                .iter()
                .filter(|&&h| !self.stop_filter.is_stop_ngram(h))
                .copied()
                .collect();
            self.index.add_document(*doc_id, &filtered);
        }

        self.built = true;
    }

    /// Find clone candidates exceeding the minimum overlap threshold.
    ///
    /// Returns `(doc_a, doc_b, estimated_similarity)` tuples where the estimated
    /// similarity is the overlap count normalized by the average N-gram count
    /// of the two documents (a rough Jaccard-like estimate).
    ///
    /// Panics (debug) if `build_index` has not been called.
    pub fn find_clone_candidates(&self, min_overlap: usize) -> Vec<(usize, usize, f64)> {
        assert!(
            self.built,
            "build_index() must be called before find_clone_candidates()"
        );

        // Build a lookup for *distinct* filtered N-gram counts per document.
        // Must match the distinct counting used by NgramIndex::find_candidates.
        let mut doc_ngram_counts: HashMap<usize, usize> = HashMap::new();
        for (doc_id, ngrams) in &self.raw_ngrams {
            let distinct: std::collections::HashSet<u64> = ngrams
                .iter()
                .filter(|&&h| !self.stop_filter.is_stop_ngram(h))
                .copied()
                .collect();
            doc_ngram_counts.insert(*doc_id, distinct.len());
        }

        let raw_candidates = self.index.find_candidates(min_overlap);

        raw_candidates
            .into_iter()
            .map(|(a, b, overlap)| {
                let count_a = doc_ngram_counts.get(&a).copied().unwrap_or(1).max(1);
                let count_b = doc_ngram_counts.get(&b).copied().unwrap_or(1).max(1);
                // Jaccard-like estimate: overlap / (|A| + |B| - overlap)
                let union_estimate = (count_a + count_b).saturating_sub(overlap).max(1);
                let similarity = overlap as f64 / union_estimate as f64;
                (a, b, similarity.clamp(0.0, 1.0))
            })
            .collect()
    }

    /// Return a reference to the stop-N-gram filter (for inspection/testing).
    pub fn stop_filter(&self) -> &StopNgramFilter {
        &self.stop_filter
    }

    /// Return a reference to the underlying N-gram index (for inspection/testing).
    pub fn ngram_index(&self) -> &NgramIndex {
        &self.index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- StopNgramFilter tests ----

    #[test]
    fn test_stop_filter_basic() {
        let mut filter = StopNgramFilter::new(4, 0.5);

        // N-gram 100 appears in 3 out of 4 docs (75%) -- should be a stop-N-gram.
        filter.record_ngram(100);
        filter.record_ngram(100);
        filter.record_ngram(100);

        // N-gram 200 appears in 1 out of 4 docs (25%) -- should NOT be a stop-N-gram.
        filter.record_ngram(200);

        filter.finalize();

        assert!(filter.is_stop_ngram(100), "75% > 50% threshold");
        assert!(!filter.is_stop_ngram(200), "25% < 50% threshold");
        assert!(
            !filter.is_stop_ngram(999),
            "Unknown N-gram is not a stop-N-gram"
        );
    }

    #[test]
    fn test_stop_filter_not_finalized() {
        let mut filter = StopNgramFilter::new(2, 0.5);
        filter.record_ngram(42);
        filter.record_ngram(42);
        // Not finalized -- should always return false.
        assert!(!filter.is_stop_ngram(42));
    }

    #[test]
    fn test_stop_filter_boundary_threshold() {
        let mut filter = StopNgramFilter::new(10, 0.5);

        // N-gram appearing in 6 out of 10 docs (60% > 50%) -- should be a stop-N-gram.
        for _ in 0..6 {
            filter.record_ngram(111);
        }

        // N-gram appearing in exactly 5 out of 10 docs (50%, not >50%) -- NOT a stop-N-gram.
        for _ in 0..5 {
            filter.record_ngram(222);
        }

        // N-gram appearing in 4 out of 10 docs (40%) -- NOT a stop-N-gram.
        for _ in 0..4 {
            filter.record_ngram(333);
        }

        filter.finalize();

        assert!(filter.is_stop_ngram(111), "60% > 50% should be stop");
        assert!(
            !filter.is_stop_ngram(222),
            "Exactly 50% should not be stop (strict >50%)"
        );
        assert!(!filter.is_stop_ngram(333), "40% should not be stop");
    }

    #[test]
    fn test_stop_filter_all_stop() {
        let mut filter = StopNgramFilter::new(2, 0.5);
        // Both N-grams appear in both docs.
        filter.record_ngram(1);
        filter.record_ngram(1);
        filter.record_ngram(2);
        filter.record_ngram(2);
        filter.finalize();

        assert!(filter.is_stop_ngram(1));
        assert!(filter.is_stop_ngram(2));
        assert_eq!(filter.num_stop_ngrams(), 2);
    }

    #[test]
    fn test_stop_filter_empty() {
        let mut filter = StopNgramFilter::new(10, 0.5);
        filter.finalize();
        assert_eq!(filter.num_stop_ngrams(), 0);
        assert!(!filter.is_stop_ngram(42));
    }

    // ---- ScalableCloneDetector tests ----

    #[test]
    fn test_scalable_detector_similar_functions() {
        // Use with_threshold(1.0) to disable stop filtering for this small-scale test.
        // In production, stop filtering shines with hundreds+ of documents.
        let mut detector = ScalableCloneDetector::new(2, 3).with_threshold(1.0);

        let tokens_a: Vec<&str> = vec![
            "fn", "compute", "(", "x", ",", "y", ")", "{", "let", "result", "=", "x", "+", "y",
            ";", "return", "result", ";", "}",
        ];
        let tokens_b: Vec<&str> = vec![
            "fn",
            "calculate",
            "(",
            "a",
            ",",
            "b",
            ")",
            "{",
            "let",
            "sum",
            "=",
            "a",
            "+",
            "b",
            ";",
            "return",
            "sum",
            ";",
            "}",
        ];

        detector.add_function(0, &tokens_a, 3);
        detector.add_function(1, &tokens_b, 3);
        detector.build_index();

        let candidates = detector.find_clone_candidates(1);
        assert!(
            !candidates.is_empty(),
            "Similar functions should be detected as candidates"
        );

        let (a, b, sim) = &candidates[0];
        assert_eq!(*a, 0);
        assert_eq!(*b, 1);
        assert!(
            *sim > 0.0,
            "Estimated similarity should be positive, got {sim}"
        );
    }

    #[test]
    fn test_scalable_detector_dissimilar_functions() {
        let mut detector = ScalableCloneDetector::new(2, 3).with_threshold(1.0);

        let tokens_a: Vec<&str> = vec![
            "fn", "sort", "(", "arr", ")", "{", "for", "i", "in", "range", "(", "len", ")", "{",
            "swap", "(", "arr", "[", "i", "]", ")", ";", "}", "}",
        ];
        let tokens_b: Vec<&str> = vec![
            "class",
            "Database",
            "extends",
            "Connection",
            "{",
            "query",
            "(",
            "sql",
            ")",
            "{",
            "return",
            "execute",
            "(",
            "sql",
            ")",
            ";",
            "}",
            "}",
        ];

        detector.add_function(0, &tokens_a, 3);
        detector.add_function(1, &tokens_b, 3);
        detector.build_index();

        let candidates = detector.find_clone_candidates(3);
        assert!(
            candidates.is_empty(),
            "Dissimilar functions should not be candidates"
        );
    }

    #[test]
    fn test_scalable_detector_stop_filtering() {
        let mut detector = ScalableCloneDetector::new(10, 3);

        // Create 10 functions that all share the boilerplate "{ return result ; }"
        // but are otherwise very different.
        let shared_suffix: Vec<&str> = vec!["{", "return", "result", ";", "}"];

        for i in 0..10 {
            // Each function has a unique prefix + the shared boilerplate.
            let unique_prefix: Vec<&str> = match i {
                0 => vec!["fn", "alpha", "(", "x", ")"],
                1 => vec!["fn", "beta", "(", "y", ")"],
                2 => vec!["fn", "gamma", "(", "z", ")"],
                3 => vec!["fn", "delta", "(", "w", ")"],
                4 => vec!["fn", "epsilon", "(", "v", ")"],
                5 => vec!["fn", "zeta", "(", "u", ")"],
                6 => vec!["fn", "eta", "(", "t", ")"],
                7 => vec!["fn", "theta", "(", "s", ")"],
                8 => vec!["fn", "iota", "(", "r", ")"],
                9 => vec!["fn", "kappa", "(", "q", ")"],
                _ => unreachable!(),
            };

            let mut tokens = unique_prefix;
            tokens.extend_from_slice(&shared_suffix);
            detector.add_function(i, &tokens, 3);
        }

        detector.build_index();

        // The shared N-grams (from boilerplate) should be filtered as stop-N-grams
        // since they appear in >50% of documents.
        assert!(
            detector.stop_filter().num_stop_ngrams() > 0,
            "Common boilerplate N-grams should be identified as stop-N-grams"
        );
    }

    #[test]
    fn test_scalable_detector_end_to_end() {
        // With 4 documents at default threshold 0.5: N-grams shared by >2 docs are stop.
        // The two pairs are structurally different, so cross-pair N-grams should be rare.
        let mut detector = ScalableCloneDetector::new(4, 3).with_threshold(1.0);

        // Two pairs: (0, 1) are similar, (2, 3) are similar, but the pairs are different.
        let pair_a_1: Vec<&str> = vec![
            "fn", "add", "(", "a", ",", "b", ")", "{", "return", "a", "+", "b", ";", "}",
        ];
        let pair_a_2: Vec<&str> = vec![
            "fn", "sum", "(", "x", ",", "y", ")", "{", "return", "x", "+", "y", ";", "}",
        ];
        let pair_b_1: Vec<&str> = vec![
            "for", "item", "in", "list", "{", "if", "item", ">", "max", "{", "max", "=", "item",
            ";", "}", "}",
        ];
        let pair_b_2: Vec<&str> = vec![
            "for", "elem", "in", "array", "{", "if", "elem", ">", "largest", "{", "largest", "=",
            "elem", ";", "}", "}",
        ];

        detector.add_function(0, &pair_a_1, 3);
        detector.add_function(1, &pair_a_2, 3);
        detector.add_function(2, &pair_b_1, 3);
        detector.add_function(3, &pair_b_2, 3);
        detector.build_index();

        let candidates = detector.find_clone_candidates(1);

        // We should find at least some candidates.
        assert!(
            !candidates.is_empty(),
            "End-to-end should produce candidates from similar function pairs"
        );

        // Verify that all returned similarities are in valid range.
        for &(_, _, sim) in &candidates {
            assert!(
                (0.0..=1.0).contains(&sim),
                "Similarity must be in [0, 1], got {sim}"
            );
        }
    }

    #[test]
    fn test_scalable_detector_end_to_end_with_stop_filtering() {
        // Test that stop filtering correctly prunes boilerplate while keeping
        // discriminative N-grams for genuine clone pairs.
        let mut detector = ScalableCloneDetector::new(6, 3);

        // 4 functions with shared boilerplate (will be stop-filtered).
        // Plus 2 genuinely similar functions that share unique patterns.
        let boilerplate_funcs: [Vec<&str>; 4] = [
            vec!["fn", "a", "(", ")", "{", "return", "0", ";", "}"],
            vec!["fn", "b", "(", ")", "{", "return", "1", ";", "}"],
            vec!["fn", "c", "(", ")", "{", "return", "2", ";", "}"],
            vec!["fn", "d", "(", ")", "{", "return", "3", ";", "}"],
        ];

        // Two functions sharing unique structure not present in the boilerplate.
        let clone_1: Vec<&str> = vec![
            "while", "running", "{", "data", "=", "fetch", "(", "url", ")", ";", "process", "(",
            "data", ")", ";", "count", "+=", "1", ";", "}",
        ];
        let clone_2: Vec<&str> = vec![
            "while", "active", "{", "result", "=", "fetch", "(", "endpoint", ")", ";", "process",
            "(", "result", ")", ";", "total", "+=", "1", ";", "}",
        ];

        for (i, func) in boilerplate_funcs.iter().enumerate() {
            detector.add_function(i, func, 3);
        }
        detector.add_function(4, &clone_1, 3);
        detector.add_function(5, &clone_2, 3);
        detector.build_index();

        // The boilerplate N-grams shared by 4+ docs should be filtered.
        assert!(
            detector.stop_filter().num_stop_ngrams() > 0,
            "Boilerplate N-grams should be identified as stop-N-grams"
        );

        let candidates = detector.find_clone_candidates(1);
        // (4, 5) should still appear because they share unique N-grams.
        let has_clone_pair = candidates.iter().any(|&(a, b, _)| a == 4 && b == 5);
        assert!(
            has_clone_pair,
            "Clone pair (4, 5) should survive stop filtering due to unique shared N-grams"
        );
    }

    #[test]
    fn test_scalable_detector_empty() {
        let mut detector = ScalableCloneDetector::new(0, 3);
        detector.build_index();
        let candidates = detector.find_clone_candidates(1);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_scalable_detector_single_function() {
        let mut detector = ScalableCloneDetector::new(1, 3);
        let tokens: Vec<&str> = vec!["fn", "foo", "(", ")", "{", "}"];
        detector.add_function(0, &tokens, 3);
        detector.build_index();
        let candidates = detector.find_clone_candidates(1);
        assert!(candidates.is_empty(), "Single function cannot form a pair");
    }

    #[test]
    fn test_scalable_detector_identical_functions() {
        // With only 2 identical documents, all N-grams appear in 100% of docs.
        // Use with_threshold(1.0) to disable stop filtering for this edge case,
        // since stop filtering is designed for larger corpora.
        let mut detector = ScalableCloneDetector::new(2, 3).with_threshold(1.0);

        let tokens: Vec<&str> = vec![
            "fn",
            "process",
            "(",
            "data",
            ")",
            "{",
            "let",
            "result",
            "=",
            "transform",
            "(",
            "data",
            ")",
            ";",
            "return",
            "result",
            ";",
            "}",
        ];

        detector.add_function(0, &tokens, 3);
        detector.add_function(1, &tokens, 3);
        detector.build_index();

        let candidates = detector.find_clone_candidates(1);
        assert!(!candidates.is_empty(), "Identical functions should match");

        let (_, _, sim) = candidates[0];
        assert!(
            (sim - 1.0).abs() < f64::EPSILON,
            "Identical functions should have similarity 1.0, got {sim}"
        );
    }
}
