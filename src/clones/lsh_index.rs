//! Locality-Sensitive Hashing (LSH) index for sub-linear candidate retrieval.
//!
//! Uses the banding technique on MinHash signatures to find candidate pairs
//! without O(n^2) pairwise comparison. Given `b` bands of `r` rows each,
//! two signatures with Jaccard similarity `s` collide in at least one band
//! with probability `1 - (1 - s^r)^b`.

use std::collections::{HashMap, HashSet};

use xxhash_rust::xxh3::xxh3_64;

/// LSH index using banding technique for MinHash signatures.
///
/// Items are inserted with their hash signatures (typically MinHash values or
/// shingle hashes). Querying returns all items that hash to the same bucket
/// in at least one band, providing candidate pairs for verification.
pub struct LshIndex {
    /// Number of bands.
    bands: usize,
    /// Number of rows (hash values) per band.
    rows_per_band: usize,
    /// Buckets per band: band_index -> (band_hash -> list of item indices).
    buckets: Vec<HashMap<u64, Vec<usize>>>,
    /// Total number of items inserted.
    num_items: usize,
}

impl LshIndex {
    /// Create a new LSH index with the given parameters.
    ///
    /// # Arguments
    /// * `bands` - Number of hash bands.
    /// * `rows_per_band` - Number of hash values per band.
    ///
    /// The total signature length consumed is `bands * rows_per_band`.
    pub fn new(bands: usize, rows_per_band: usize) -> Self {
        Self {
            bands,
            rows_per_band,
            buckets: (0..bands).map(|_| HashMap::new()).collect(),
            num_items: 0,
        }
    }

    /// Create an LSH index with automatically selected parameters for the given
    /// number of hash values and similarity threshold.
    ///
    /// Chooses `(bands, rows)` such that the S-curve inflection point
    /// `(1/b)^(1/r)` is closest to `threshold`.
    pub fn with_threshold(num_hashes: usize, threshold: f64) -> Self {
        let (bands, rows) = select_lsh_params(num_hashes, threshold);
        Self::new(bands, rows)
    }

    /// Insert an item's hash signature into the index.
    ///
    /// `hash_values` should be the MinHash signature values (or any fixed-length
    /// hash vector). The index partitions the values into `bands` groups of
    /// `rows_per_band` consecutive values and hashes each group to a bucket.
    pub fn insert(&mut self, idx: usize, hash_values: &[u64]) {
        for band in 0..self.bands {
            let start = band * self.rows_per_band;
            let end = (start + self.rows_per_band).min(hash_values.len());
            if start >= hash_values.len() {
                break;
            }

            let band_hash = hash_band(&hash_values[start..end], band);
            self.buckets[band].entry(band_hash).or_default().push(idx);
        }
        self.num_items += 1;
    }

    /// Bulk-insert multiple items into the index.
    ///
    /// Equivalent to calling `insert` for each item, but avoids repeated
    /// method-call overhead.
    pub fn insert_bulk(&mut self, items: &[Vec<u64>]) {
        for (idx, hash_values) in items.iter().enumerate() {
            self.insert(idx, hash_values);
        }
    }

    /// Query for candidates similar to the given signature.
    ///
    /// Returns sorted, deduplicated indices of items that hash to the same
    /// bucket in at least one band.
    pub fn query(&self, hash_values: &[u64]) -> Vec<usize> {
        let mut candidates = HashSet::new();

        for band in 0..self.bands {
            let start = band * self.rows_per_band;
            let end = (start + self.rows_per_band).min(hash_values.len());
            if start >= hash_values.len() {
                break;
            }

            let band_hash = hash_band(&hash_values[start..end], band);
            if let Some(items) = self.buckets[band].get(&band_hash) {
                for &item_idx in items {
                    candidates.insert(item_idx);
                }
            }
        }

        let mut result: Vec<usize> = candidates.into_iter().collect();
        result.sort_unstable();
        result
    }

    /// Find all candidate pairs across the entire index.
    ///
    /// Returns sorted, deduplicated pairs `(i, j)` where `i < j` that share
    /// at least one bucket in any band.
    pub fn candidate_pairs(&self) -> Vec<(usize, usize)> {
        let mut pairs = HashSet::new();

        for band_buckets in &self.buckets {
            for members in band_buckets.values() {
                if members.len() < 2 {
                    continue;
                }
                for i in 0..members.len() {
                    for j in (i + 1)..members.len() {
                        let a = members[i].min(members[j]);
                        let b = members[i].max(members[j]);
                        pairs.insert((a, b));
                    }
                }
            }
        }

        let mut result: Vec<(usize, usize)> = pairs.into_iter().collect();
        result.sort_unstable();
        result
    }

    /// Number of items in the index.
    pub fn len(&self) -> usize {
        self.num_items
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.num_items == 0
    }

    /// Number of bands.
    pub fn bands(&self) -> usize {
        self.bands
    }

    /// Number of rows per band.
    pub fn rows_per_band(&self) -> usize {
        self.rows_per_band
    }

    /// Estimate the probability that two items with the given Jaccard similarity
    /// will be found as candidates.
    ///
    /// P(candidate) = 1 - (1 - s^r)^b
    pub fn collision_probability(&self, similarity: f64) -> f64 {
        let r = self.rows_per_band as f64;
        let b = self.bands as f64;
        1.0 - (1.0 - similarity.powf(r)).powf(b)
    }
}

/// Select optimal LSH parameters for the given number of hash values and threshold.
///
/// Finds `(bands, rows_per_band)` such that `bands * rows_per_band <= num_permutations`
/// and the S-curve inflection point `(1/b)^(1/r)` is closest to `threshold`.
pub fn select_lsh_params(num_permutations: usize, threshold: f64) -> (usize, usize) {
    let mut best_bands = 1;
    let mut best_rows = num_permutations;
    let mut best_error = f64::MAX;

    let max_r = num_permutations.min(20);
    for r in 1..=max_r {
        let b = num_permutations / r;
        if b == 0 {
            continue;
        }
        // Inflection point of the S-curve: (1/b)^(1/r)
        let inflection = (1.0 / b as f64).powf(1.0 / r as f64);
        let error = (inflection - threshold).abs();
        if error < best_error {
            best_error = error;
            best_bands = b;
            best_rows = r;
        }
    }

    (best_bands, best_rows)
}

/// Hash a band of values into a single bucket hash.
///
/// Includes the band index as a seed so that the same values in different
/// bands produce different bucket hashes.
fn hash_band(values: &[u64], band_idx: usize) -> u64 {
    let mut data = Vec::with_capacity(values.len() * 8 + 8);
    data.extend_from_slice(&(band_idx as u64).to_le_bytes());
    for &v in values {
        data.extend_from_slice(&v.to_le_bytes());
    }
    xxh3_64(&data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_signatures_always_found() {
        let sig: Vec<u64> = (0..128).collect();
        let mut index = LshIndex::with_threshold(128, 0.5);
        index.insert(0, &sig);

        let candidates = index.query(&sig);
        assert!(
            candidates.contains(&0),
            "Identical signature must always be found as candidate"
        );
    }

    #[test]
    fn test_very_different_signatures_rarely_found() {
        let num_hashes = 128;
        let mut index = LshIndex::with_threshold(num_hashes, 0.8);

        // Insert many random-ish but deterministic signatures
        let sigs: Vec<Vec<u64>> = (0..100)
            .map(|i| {
                (0..num_hashes as u64)
                    .map(|j| xxh3_64(&[i * 1000 + j].map(|v| v.to_le_bytes()).concat()))
                    .collect()
            })
            .collect();

        for (i, sig) in sigs.iter().enumerate() {
            index.insert(i, sig);
        }

        // Count how many false candidates are returned for signature 0
        let candidates = index.query(&sigs[0]);
        // At threshold 0.8, truly dissimilar sigs should rarely collide.
        // With 100 items, we expect very few false positives.
        // The candidate list should be much smaller than the full set.
        assert!(
            candidates.len() < 50,
            "Expected few false positives at high threshold, got {} candidates out of 100",
            candidates.len()
        );
    }

    #[test]
    fn test_parameter_selection_reasonable() {
        let (bands, rows) = select_lsh_params(128, 0.5);
        assert!(bands > 0, "bands must be positive");
        assert!(rows > 0, "rows must be positive");
        assert!(
            bands * rows <= 128,
            "bands * rows must not exceed num_permutations"
        );

        // Check inflection point is near threshold
        let inflection = (1.0 / bands as f64).powf(1.0 / rows as f64);
        assert!(
            (inflection - 0.5).abs() < 0.15,
            "Inflection point {inflection} should be near threshold 0.5"
        );
    }

    #[test]
    fn test_parameter_selection_various_thresholds() {
        for threshold in [0.3, 0.5, 0.7, 0.9] {
            let (bands, rows) = select_lsh_params(128, threshold);
            assert!(bands > 0);
            assert!(rows > 0);
            assert!(bands * rows <= 128);
            let inflection = (1.0 / bands as f64).powf(1.0 / rows as f64);
            assert!(
                (inflection - threshold).abs() < 0.2,
                "threshold={threshold}, inflection={inflection}, bands={bands}, rows={rows}"
            );
        }
    }

    #[test]
    fn test_empty_index() {
        let index = LshIndex::new(16, 8);
        assert!(index.is_empty());
        assert_eq!(index.len(), 0);

        let candidates = index.query(&[1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_empty_signature() {
        let mut index = LshIndex::new(4, 2);
        index.insert(0, &[]);
        assert_eq!(index.len(), 1);

        let candidates = index.query(&[]);
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_query_returns_sorted_deduplicated() {
        let sig: Vec<u64> = (0..16).collect();
        let mut index = LshIndex::new(4, 4);
        index.insert(0, &sig);
        index.insert(1, &sig); // identical
        index.insert(2, &sig); // identical

        let candidates = index.query(&sig);
        // Verify sorted
        for w in candidates.windows(2) {
            assert!(w[0] <= w[1], "Results must be sorted");
        }
        // Verify deduplicated
        let unique: HashSet<usize> = candidates.iter().copied().collect();
        assert_eq!(
            unique.len(),
            candidates.len(),
            "Results must be deduplicated"
        );
    }

    #[test]
    fn test_candidate_pairs() {
        let sig_a: Vec<u64> = (0..16).collect();
        let sig_b: Vec<u64> = (0..16).collect(); // identical to a
        let sig_c: Vec<u64> = (1000..1016).collect(); // different

        let mut index = LshIndex::new(4, 4);
        index.insert(0, &sig_a);
        index.insert(1, &sig_b);
        index.insert(2, &sig_c);

        let pairs = index.candidate_pairs();
        // a and b are identical, should be a candidate pair
        assert!(
            pairs.contains(&(0, 1)),
            "Identical signatures should be candidate pair"
        );
        // a/b and c are very different, should NOT be candidate pair
        assert!(
            !pairs.contains(&(0, 2)),
            "Very different signatures should not be candidate pair"
        );
        assert!(
            !pairs.contains(&(1, 2)),
            "Very different signatures should not be candidate pair"
        );
    }

    #[test]
    fn test_collision_probability() {
        let index = LshIndex::new(16, 8);

        // Identical items: probability should be 1.0
        let prob_identical = index.collision_probability(1.0);
        assert!(
            (prob_identical - 1.0).abs() < f64::EPSILON,
            "P(collision | s=1.0) should be 1.0, got {prob_identical}"
        );

        // Completely different: probability should be ~0.0
        let prob_zero = index.collision_probability(0.0);
        assert!(
            prob_zero.abs() < f64::EPSILON,
            "P(collision | s=0.0) should be ~0.0, got {prob_zero}"
        );

        // Mid-range: probability should be between 0 and 1
        let prob_mid = index.collision_probability(0.5);
        assert!(
            (0.0..=1.0).contains(&prob_mid),
            "P(collision | s=0.5) should be in [0,1], got {prob_mid}"
        );
    }

    #[test]
    fn test_bulk_insert() {
        let items: Vec<Vec<u64>> = (0..10).map(|i| vec![i; 16]).collect();
        let mut index = LshIndex::new(4, 4);
        index.insert_bulk(&items);
        assert_eq!(index.len(), 10);
    }

    #[test]
    fn test_short_signature_handled() {
        // Signature shorter than bands * rows_per_band
        let sig: Vec<u64> = vec![42, 43];
        let mut index = LshIndex::new(16, 8);
        index.insert(0, &sig);

        let candidates = index.query(&sig);
        assert!(
            candidates.contains(&0),
            "Short signature should still be findable"
        );
    }
}
