//! N-gram inverted index for sub-quadratic clone candidate generation.
//!
//! Instead of comparing all O(N^2) document pairs, this module builds an inverted
//! index mapping N-gram hashes to the documents that contain them. Candidate pairs
//! are generated only from documents that share at least `min_overlap` N-grams,
//! dramatically reducing comparisons for large codebases.
//!
//! Includes optional `BinaryFuseFilter`-based pre-screening: each document builds
//! a compact membership filter over its N-gram hashes. Before computing full
//! overlap via the inverted index, a candidate can be quickly rejected at ~22 ns
//! per query if its N-grams are absent from the other document's filter.

use std::collections::HashMap;

use sketch_oxide::membership::BinaryFuseFilter;
use xxhash_rust::xxh3::xxh3_64;

/// An entry in the inverted index: which document contains this N-gram and at what positions.
#[derive(Debug, Clone)]
pub struct NgramEntry {
    /// Unique identifier for the document (function, code block, etc.).
    pub doc_id: usize,
    /// Positions within the document's token stream where this N-gram occurs.
    pub positions: Vec<usize>,
}

/// Per-document binary fuse filter for fast N-gram membership pre-screening.
///
/// Built from all distinct N-gram hashes of a document. Supports O(1) `contains`
/// queries (~22 ns) with a very low false-positive rate (~1/256 at 8 bits/entry).
pub struct DocumentFilter {
    /// Document identifier.
    pub doc_id: usize,
    /// Binary fuse filter over the document's N-gram hashes.
    filter: BinaryFuseFilter,
    /// Number of distinct N-grams in this document (for threshold checks).
    pub ngram_count: usize,
}

impl DocumentFilter {
    /// Build a filter from a document's N-gram hashes.
    ///
    /// Returns `None` if the N-gram set is empty or filter construction fails.
    pub fn build(doc_id: usize, ngrams: &[u64]) -> Option<Self> {
        if ngrams.is_empty() {
            return None;
        }
        let unique: std::collections::HashSet<u64> = ngrams.iter().copied().collect();
        let ngram_count = unique.len();
        let filter = BinaryFuseFilter::from_items(unique, 8).ok()?;
        Some(Self {
            doc_id,
            filter,
            ngram_count,
        })
    }

    /// Check if this document's filter contains the given N-gram hash.
    pub fn contains(&self, hash: &u64) -> bool {
        self.filter.contains(hash)
    }

    /// Estimate the overlap of another document's N-grams against this filter.
    ///
    /// Returns the number of N-grams from `other_ngrams` that pass the filter.
    /// This is an upper bound on true overlap (false positives are possible
    /// but rare at ~1/256).
    pub fn estimate_overlap(&self, other_ngrams: &[u64]) -> usize {
        let unique: std::collections::HashSet<u64> = other_ngrams.iter().copied().collect();
        unique.iter().filter(|h| self.filter.contains(h)).count()
    }
}

impl std::fmt::Debug for DocumentFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocumentFilter")
            .field("doc_id", &self.doc_id)
            .field("ngram_count", &self.ngram_count)
            .finish()
    }
}

/// Inverted index mapping N-gram hashes to the documents containing them.
///
/// This provides sub-quadratic candidate generation: only document pairs sharing
/// at least one N-gram are ever compared, avoiding the full O(N^2) pairwise scan.
#[derive(Debug)]
pub struct NgramIndex {
    /// Hash of N-gram -> list of documents containing it.
    index: HashMap<u64, Vec<NgramEntry>>,
    /// Total number of documents added to the index.
    num_docs: usize,
    /// Per-document binary fuse filters for pre-screening.
    filters: HashMap<usize, DocumentFilter>,
}

impl NgramIndex {
    /// Create an empty N-gram index.
    pub fn new() -> Self {
        Self {
            index: HashMap::new(),
            num_docs: 0,
            filters: HashMap::new(),
        }
    }

    /// Return the number of documents in the index.
    pub fn num_docs(&self) -> usize {
        self.num_docs
    }

    /// Return the number of distinct N-gram hashes in the index.
    pub fn num_ngrams(&self) -> usize {
        self.index.len()
    }

    /// Add a document's N-gram hashes to the index.
    ///
    /// Each hash is recorded with its position(s) within the document's N-gram
    /// sequence, enabling both overlap counting and positional analysis.
    /// Also builds a `BinaryFuseFilter` for fast pre-screening of this document.
    pub fn add_document(&mut self, doc_id: usize, ngrams: &[u64]) {
        // Group positions by N-gram hash for this document.
        let mut positions_by_hash: HashMap<u64, Vec<usize>> = HashMap::new();
        for (pos, &hash) in ngrams.iter().enumerate() {
            positions_by_hash.entry(hash).or_default().push(pos);
        }

        for (hash, positions) in positions_by_hash {
            self.index
                .entry(hash)
                .or_default()
                .push(NgramEntry { doc_id, positions });
        }

        // Build binary fuse filter for this document
        if let Some(filter) = DocumentFilter::build(doc_id, ngrams) {
            self.filters.insert(doc_id, filter);
        }

        self.num_docs += 1;
    }

    /// Get the binary fuse filter for a document, if one was built.
    pub fn get_filter(&self, doc_id: usize) -> Option<&DocumentFilter> {
        self.filters.get(&doc_id)
    }

    /// Find all document pairs that share at least `min_overlap` distinct N-gram hashes.
    ///
    /// Returns a vector of `(doc_a, doc_b, overlap_count)` tuples where `doc_a < doc_b`
    /// and `overlap_count >= min_overlap`. The overlap count is the number of distinct
    /// N-gram hashes shared between the two documents.
    pub fn find_candidates(&self, min_overlap: usize) -> Vec<(usize, usize, usize)> {
        // Count shared N-grams for each document pair.
        let mut pair_counts: HashMap<(usize, usize), usize> = HashMap::new();

        for entries in self.index.values() {
            // Skip N-grams that appear in only one document -- no pair to form.
            if entries.len() < 2 {
                continue;
            }

            // For each pair of documents sharing this N-gram, increment the overlap count.
            for i in 0..entries.len() {
                for j in (i + 1)..entries.len() {
                    let a = entries[i].doc_id.min(entries[j].doc_id);
                    let b = entries[i].doc_id.max(entries[j].doc_id);
                    *pair_counts.entry((a, b)).or_insert(0) += 1;
                }
            }
        }

        // Filter pairs that meet the minimum overlap threshold.
        pair_counts
            .into_iter()
            .filter(|&(_, count)| count >= min_overlap)
            .map(|((a, b), count)| (a, b, count))
            .collect()
    }

    /// Find candidates using BinaryFuseFilter pre-screening before full overlap.
    ///
    /// For each document pair, first queries the target document's binary fuse
    /// filter to estimate whether the source document shares enough N-grams.
    /// Only pairs that pass the filter threshold proceed to exact overlap
    /// counting via the inverted index.
    ///
    /// This eliminates false candidates at ~22 ns per filter query, which is
    /// significantly cheaper than counting exact overlap through the inverted index.
    ///
    /// `doc_ngrams` maps `doc_id -> &[u64]` (the raw N-gram hashes for each document).
    /// `min_overlap` is the minimum number of shared distinct N-gram hashes required.
    pub fn find_candidates_prescreened(
        &self,
        doc_ngrams: &HashMap<usize, Vec<u64>>,
        min_overlap: usize,
    ) -> Vec<(usize, usize, usize)> {
        // Collect all document IDs that have filters
        let doc_ids: Vec<usize> = doc_ngrams.keys().copied().collect();

        let mut pair_results: Vec<(usize, usize, usize)> = Vec::new();

        for i in 0..doc_ids.len() {
            for j in (i + 1)..doc_ids.len() {
                let id_a = doc_ids[i].min(doc_ids[j]);
                let id_b = doc_ids[i].max(doc_ids[j]);

                // Pre-screen: query filter of doc_b with ngrams of doc_a
                let pass_prescreen = if let Some(filter_b) = self.filters.get(&id_b) {
                    if let Some(ngrams_a) = doc_ngrams.get(&id_a) {
                        filter_b.estimate_overlap(ngrams_a) >= min_overlap
                    } else {
                        false
                    }
                } else {
                    // No filter available; fall through to exact check
                    true
                };

                if !pass_prescreen {
                    continue;
                }

                // Exact overlap: count shared distinct N-gram hashes via the inverted index
                if let (Some(ngrams_a), Some(ngrams_b)) =
                    (doc_ngrams.get(&id_a), doc_ngrams.get(&id_b))
                {
                    let set_a: std::collections::HashSet<u64> = ngrams_a.iter().copied().collect();
                    let overlap = ngrams_b
                        .iter()
                        .collect::<std::collections::HashSet<_>>()
                        .iter()
                        .filter(|h| set_a.contains(h))
                        .count();

                    if overlap >= min_overlap {
                        pair_results.push((id_a, id_b, overlap));
                    }
                }
            }
        }

        pair_results
    }

    /// Convert a token sequence to N-gram hashes using xxh3_64.
    ///
    /// Produces `(tokens.len() - n + 1)` hashes, one for each sliding window of
    /// `n` consecutive tokens. Returns an empty vector if there are fewer than `n` tokens.
    pub fn ngrams_from_tokens(tokens: &[&str], n: usize) -> Vec<u64> {
        if n == 0 || tokens.len() < n {
            return Vec::new();
        }

        tokens
            .windows(n)
            .map(|window| {
                let combined = window.join(" ");
                xxh3_64(combined.as_bytes())
            })
            .collect()
    }
}

impl Default for NgramIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ngrams_from_tokens_basic() {
        let tokens = vec!["fn", "foo", "(", "x", ")", "{", "return", "x", "}"];
        let ngrams = NgramIndex::ngrams_from_tokens(&tokens, 3);
        // 9 tokens, window size 3 => 7 N-grams
        assert_eq!(ngrams.len(), 7);
    }

    #[test]
    fn test_ngrams_from_tokens_too_few() {
        let tokens = vec!["fn", "foo"];
        let ngrams = NgramIndex::ngrams_from_tokens(&tokens, 3);
        assert!(ngrams.is_empty());
    }

    #[test]
    fn test_ngrams_from_tokens_zero_n() {
        let tokens = vec!["fn", "foo", "bar"];
        let ngrams = NgramIndex::ngrams_from_tokens(&tokens, 0);
        assert!(ngrams.is_empty());
    }

    #[test]
    fn test_ngrams_deterministic() {
        let tokens = vec!["if", "x", ">", "0", "return", "x"];
        let a = NgramIndex::ngrams_from_tokens(&tokens, 3);
        let b = NgramIndex::ngrams_from_tokens(&tokens, 3);
        assert_eq!(a, b);
    }

    #[test]
    fn test_add_document_and_num_docs() {
        let mut index = NgramIndex::new();
        let ngrams = NgramIndex::ngrams_from_tokens(&["a", "b", "c", "d"], 2);
        index.add_document(0, &ngrams);
        assert_eq!(index.num_docs(), 1);
        index.add_document(1, &ngrams);
        assert_eq!(index.num_docs(), 2);
    }

    #[test]
    fn test_similar_documents_found_as_candidates() {
        let mut index = NgramIndex::new();

        // Two very similar token sequences (share most N-grams).
        let tokens_a = vec![
            "fn", "compute", "(", "x", ")", "{", "return", "x", "+", "1", "}",
        ];
        let tokens_b = vec![
            "fn",
            "calculate",
            "(",
            "x",
            ")",
            "{",
            "return",
            "x",
            "+",
            "2",
            "}",
        ];

        let ngrams_a = NgramIndex::ngrams_from_tokens(&tokens_a, 3);
        let ngrams_b = NgramIndex::ngrams_from_tokens(&tokens_b, 3);

        index.add_document(0, &ngrams_a);
        index.add_document(1, &ngrams_b);

        // These share several 3-grams (e.g., "( x )", "x ) {", ") { return", "{ return x",
        // "return x +").
        let candidates = index.find_candidates(2);
        assert!(
            !candidates.is_empty(),
            "Similar documents should be candidate pairs"
        );

        let (a, b, overlap) = &candidates[0];
        assert_eq!(*a, 0);
        assert_eq!(*b, 1);
        assert!(
            *overlap >= 2,
            "Expected at least 2 shared N-grams, got {overlap}"
        );
    }

    #[test]
    fn test_dissimilar_documents_not_candidates() {
        let mut index = NgramIndex::new();

        // Completely different token sequences.
        let tokens_a = vec![
            "fn", "compute", "(", "x", ")", "{", "return", "x", "+", "1", "}",
        ];
        let tokens_b = vec![
            "class",
            "Widget",
            "extends",
            "Base",
            "implements",
            "Drawable",
        ];

        let ngrams_a = NgramIndex::ngrams_from_tokens(&tokens_a, 3);
        let ngrams_b = NgramIndex::ngrams_from_tokens(&tokens_b, 3);

        index.add_document(0, &ngrams_a);
        index.add_document(1, &ngrams_b);

        // With min_overlap=2, completely different code should not appear.
        let candidates = index.find_candidates(2);
        assert!(
            candidates.is_empty(),
            "Dissimilar documents should not be candidates, got {candidates:?}"
        );
    }

    #[test]
    fn test_empty_index_no_candidates() {
        let index = NgramIndex::new();
        let candidates = index.find_candidates(1);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_single_document_no_candidates() {
        let mut index = NgramIndex::new();
        let ngrams = NgramIndex::ngrams_from_tokens(&["a", "b", "c", "d"], 2);
        index.add_document(0, &ngrams);
        let candidates = index.find_candidates(1);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_multiple_documents_partial_overlap() {
        let mut index = NgramIndex::new();

        // Doc 0 and Doc 1 share some N-grams. Doc 2 is different.
        let tokens_0 = vec!["let", "x", "=", "foo", "(", "a", ")"];
        let tokens_1 = vec!["let", "y", "=", "foo", "(", "b", ")"];
        let tokens_2 = vec!["import", "os", "import", "sys", "print", "hello"];

        let ng0 = NgramIndex::ngrams_from_tokens(&tokens_0, 3);
        let ng1 = NgramIndex::ngrams_from_tokens(&tokens_1, 3);
        let ng2 = NgramIndex::ngrams_from_tokens(&tokens_2, 3);

        index.add_document(0, &ng0);
        index.add_document(1, &ng1);
        index.add_document(2, &ng2);

        let candidates = index.find_candidates(1);

        // (0, 1) should appear; (0, 2) and (1, 2) should not.
        let has_01 = candidates.iter().any(|&(a, b, _)| a == 0 && b == 1);
        let has_02 = candidates.iter().any(|&(a, b, _)| a == 0 && b == 2);
        let has_12 = candidates.iter().any(|&(a, b, _)| a == 1 && b == 2);

        assert!(has_01, "Docs 0 and 1 should be candidates");
        assert!(!has_02, "Docs 0 and 2 should not be candidates");
        assert!(!has_12, "Docs 1 and 2 should not be candidates");
    }

    #[test]
    fn test_min_overlap_filtering() {
        let mut index = NgramIndex::new();

        // Identical documents -- maximum overlap.
        let tokens = vec!["a", "b", "c", "d", "e", "f"];
        let ngrams = NgramIndex::ngrams_from_tokens(&tokens, 3);

        index.add_document(0, &ngrams);
        index.add_document(1, &ngrams);

        let candidates_low = index.find_candidates(1);
        assert!(!candidates_low.is_empty());

        // With a very high threshold, even identical docs may not meet it if the
        // threshold exceeds the total number of distinct N-grams.
        let candidates_high = index.find_candidates(100);
        assert!(
            candidates_high.is_empty(),
            "Threshold exceeds actual N-gram count"
        );
    }

    #[test]
    fn test_duplicate_ngrams_within_document() {
        let mut index = NgramIndex::new();

        // Repeated pattern: "a b a b a b" has repeating 2-grams.
        let tokens = vec!["a", "b", "a", "b", "a", "b"];
        let ngrams = NgramIndex::ngrams_from_tokens(&tokens, 2);

        index.add_document(0, &ngrams);
        // Should not panic or produce incorrect state.
        assert_eq!(index.num_docs(), 1);
    }

    // ------------------------------------------------------------------
    // BinaryFuseFilter pre-screening tests
    // ------------------------------------------------------------------

    #[test]
    fn test_document_filter_build() {
        let ngrams = NgramIndex::ngrams_from_tokens(&["fn", "foo", "(", "x", ")", "{", "}"], 3);
        let filter = DocumentFilter::build(0, &ngrams);
        assert!(
            filter.is_some(),
            "Should build filter from non-empty N-grams"
        );
        let f = filter.unwrap();
        assert_eq!(f.doc_id, 0);
        assert!(f.ngram_count > 0);
    }

    #[test]
    fn test_document_filter_contains_own_ngrams() {
        let ngrams = NgramIndex::ngrams_from_tokens(&["fn", "foo", "(", "x", ")", "{", "}"], 3);
        let filter = DocumentFilter::build(0, &ngrams).unwrap();

        // The filter should contain all N-grams used to build it
        for &hash in &ngrams {
            assert!(
                filter.contains(&hash),
                "Filter should contain its own N-gram hash"
            );
        }
    }

    #[test]
    fn test_document_filter_empty_ngrams() {
        let filter = DocumentFilter::build(0, &[]);
        assert!(filter.is_none(), "Empty N-grams should return None");
    }

    #[test]
    fn test_document_filter_estimate_overlap() {
        let tokens_a = vec![
            "fn", "compute", "(", "x", ")", "{", "return", "x", "+", "1", "}",
        ];
        let tokens_b = vec![
            "fn",
            "calculate",
            "(",
            "x",
            ")",
            "{",
            "return",
            "x",
            "+",
            "2",
            "}",
        ];

        let ngrams_a = NgramIndex::ngrams_from_tokens(&tokens_a, 3);
        let ngrams_b = NgramIndex::ngrams_from_tokens(&tokens_b, 3);

        let filter_a = DocumentFilter::build(0, &ngrams_a).unwrap();
        let estimated = filter_a.estimate_overlap(&ngrams_b);

        // These share several 3-grams; the estimate should be >= the true overlap
        assert!(
            estimated >= 2,
            "Similar documents should have estimated overlap >= 2, got {estimated}"
        );
    }

    #[test]
    fn test_document_filter_no_overlap_dissimilar() {
        let tokens_a = vec![
            "fn", "compute", "(", "x", ")", "{", "return", "x", "+", "1", "}",
        ];
        let tokens_b = vec![
            "class",
            "Widget",
            "extends",
            "Base",
            "implements",
            "Drawable",
        ];

        let ngrams_a = NgramIndex::ngrams_from_tokens(&tokens_a, 3);
        let ngrams_b = NgramIndex::ngrams_from_tokens(&tokens_b, 3);

        let filter_a = DocumentFilter::build(0, &ngrams_a).unwrap();
        let estimated = filter_a.estimate_overlap(&ngrams_b);

        // Completely different content -- overlap should be 0 or very small (FP only)
        assert!(
            estimated <= 1,
            "Dissimilar documents should have near-zero estimated overlap, got {estimated}"
        );
    }

    #[test]
    fn test_find_candidates_prescreened_similar() {
        let mut index = NgramIndex::new();

        let tokens_a = vec![
            "fn", "compute", "(", "x", ")", "{", "return", "x", "+", "1", "}",
        ];
        let tokens_b = vec![
            "fn",
            "calculate",
            "(",
            "x",
            ")",
            "{",
            "return",
            "x",
            "+",
            "2",
            "}",
        ];

        let ngrams_a = NgramIndex::ngrams_from_tokens(&tokens_a, 3);
        let ngrams_b = NgramIndex::ngrams_from_tokens(&tokens_b, 3);

        index.add_document(0, &ngrams_a);
        index.add_document(1, &ngrams_b);

        let mut doc_ngrams = HashMap::new();
        doc_ngrams.insert(0, ngrams_a);
        doc_ngrams.insert(1, ngrams_b);

        let candidates = index.find_candidates_prescreened(&doc_ngrams, 2);
        assert!(
            !candidates.is_empty(),
            "Pre-screened search should find similar documents"
        );
        let (a, b, overlap) = &candidates[0];
        assert_eq!(*a, 0);
        assert_eq!(*b, 1);
        assert!(*overlap >= 2);
    }

    #[test]
    fn test_find_candidates_prescreened_dissimilar() {
        let mut index = NgramIndex::new();

        let tokens_a = vec![
            "fn", "compute", "(", "x", ")", "{", "return", "x", "+", "1", "}",
        ];
        let tokens_b = vec![
            "class",
            "Widget",
            "extends",
            "Base",
            "implements",
            "Drawable",
        ];

        let ngrams_a = NgramIndex::ngrams_from_tokens(&tokens_a, 3);
        let ngrams_b = NgramIndex::ngrams_from_tokens(&tokens_b, 3);

        index.add_document(0, &ngrams_a);
        index.add_document(1, &ngrams_b);

        let mut doc_ngrams = HashMap::new();
        doc_ngrams.insert(0, ngrams_a);
        doc_ngrams.insert(1, ngrams_b);

        let candidates = index.find_candidates_prescreened(&doc_ngrams, 2);
        assert!(
            candidates.is_empty(),
            "Pre-screened search should reject dissimilar documents"
        );
    }

    #[test]
    fn test_prescreened_matches_inverted_index_results() {
        // Verify that prescreened results are a superset of (or equal to) inverted-index results
        let mut index = NgramIndex::new();

        let tokens_0 = vec!["let", "x", "=", "foo", "(", "a", ")"];
        let tokens_1 = vec!["let", "y", "=", "foo", "(", "b", ")"];
        let tokens_2 = vec!["import", "os", "import", "sys", "print", "hello"];

        let ng0 = NgramIndex::ngrams_from_tokens(&tokens_0, 3);
        let ng1 = NgramIndex::ngrams_from_tokens(&tokens_1, 3);
        let ng2 = NgramIndex::ngrams_from_tokens(&tokens_2, 3);

        index.add_document(0, &ng0);
        index.add_document(1, &ng1);
        index.add_document(2, &ng2);

        let inverted_candidates = index.find_candidates(1);

        let mut doc_ngrams = HashMap::new();
        doc_ngrams.insert(0, ng0);
        doc_ngrams.insert(1, ng1);
        doc_ngrams.insert(2, ng2);

        let prescreened_candidates = index.find_candidates_prescreened(&doc_ngrams, 1);

        // Every pair found by the inverted index should also be found by prescreened
        for &(a, b, _) in &inverted_candidates {
            let found = prescreened_candidates
                .iter()
                .any(|&(pa, pb, _)| pa == a && pb == b);
            assert!(
                found,
                "Inverted-index candidate ({a}, {b}) missing from prescreened results"
            );
        }
    }

    #[test]
    fn test_add_document_builds_filter() {
        let mut index = NgramIndex::new();
        let ngrams = NgramIndex::ngrams_from_tokens(&["a", "b", "c", "d", "e"], 3);
        index.add_document(42, &ngrams);

        let filter = index.get_filter(42);
        assert!(filter.is_some(), "add_document should build a filter");
        assert_eq!(filter.unwrap().doc_id, 42);
    }
}
