//! Cross-language clone detection using IR tokens and MinHash.

use crate::core::Language;
use sketch_oxide::similarity::MinHash;

use super::ir_tokenizer::{extract_ir_tokens, ir_tokens_to_shingles, IRToken};
use super::minhash::MinHashDetector;
use super::types::{CloneGroup, CloneInstance, CloneType};

/// A function signature for cross-language comparison.
#[derive(Debug, Clone)]
pub struct CrossLanguageSignature {
    pub file: String,
    pub name: String,
    pub language: Language,
    pub start_line: usize,
    pub end_line: usize,
    pub ir_tokens: Vec<IRToken>,
    pub sketch: MinHash,
}

/// Cross-language clone detector.
pub struct CrossLanguageDetector {
    /// Number of hash functions for MinHash.
    num_hashes: usize,
    /// Shingle size for IR token n-grams.
    shingle_size: usize,
    /// Similarity threshold.
    threshold: f64,
}

impl CrossLanguageDetector {
    pub fn new(threshold: f64) -> Self {
        Self {
            num_hashes: 128,
            shingle_size: 4,
            threshold,
        }
    }

    /// Build a signature from a tree-sitter node.
    #[allow(clippy::too_many_arguments)]
    pub fn build_signature(
        &self,
        node: tree_sitter::Node<'_>,
        source: &str,
        file: &str,
        name: &str,
        language: Language,
        start_line: usize,
        end_line: usize,
    ) -> Option<CrossLanguageSignature> {
        let ir_tokens = extract_ir_tokens(node, source);
        if ir_tokens.len() < self.shingle_size {
            return None;
        }

        let shingles = ir_tokens_to_shingles(&ir_tokens, self.shingle_size);
        if shingles.is_empty() {
            return None;
        }

        let minhash = MinHashDetector::new(self.num_hashes, 3, self.threshold);
        let sketch = minhash.build_sketch(&shingles);

        Some(CrossLanguageSignature {
            file: file.to_string(),
            name: name.to_string(),
            language,
            start_line,
            end_line,
            ir_tokens,
            sketch,
        })
    }

    /// Detect clones across a set of signatures from different languages.
    ///
    /// Uses LSH-based candidate generation for sub-quadratic performance.
    /// Falls back to pairwise comparison only for the candidate pairs
    /// identified by the LSH index.
    pub fn detect_clones(&self, signatures: &[CrossLanguageSignature]) -> Vec<CloneGroup> {
        if signatures.len() < 2 {
            return Vec::new();
        }

        // Build LSH-compatible hash values from IR token shingles
        let hash_values: Vec<Vec<u64>> = signatures
            .iter()
            .map(|sig| self.compute_lsh_hashes(&sig.ir_tokens))
            .collect();

        // Build LSH index for candidate generation
        let mut lsh =
            crate::clones::lsh_index::LshIndex::with_threshold(self.num_hashes, self.threshold);
        for (i, hv) in hash_values.iter().enumerate() {
            lsh.insert(i, hv);
        }

        let mut groups = Vec::new();
        let mut seen_pairs = std::collections::HashSet::new();

        // Query each signature against the index
        for i in 0..signatures.len() {
            let candidates = lsh.query(&hash_values[i]);
            for &j in &candidates {
                if j <= i {
                    continue;
                }
                // Only compare across different languages
                if signatures[i].language == signatures[j].language {
                    continue;
                }
                if seen_pairs.contains(&(i, j)) {
                    continue;
                }
                seen_pairs.insert((i, j));

                // Verify with actual MinHash similarity
                let similarity = MinHashDetector::jaccard_similarity(
                    &signatures[i].sketch,
                    &signatures[j].sketch,
                );

                if similarity >= self.threshold {
                    // Cap cross-language similarity at 0.99 — exact 1.0 across
                    // different languages is an artifact of coarse IR tokenization
                    // rather than true semantic equivalence.
                    let similarity = similarity.min(0.99);

                    let instance_a = CloneInstance {
                        file: signatures[i].file.clone(),
                        start_line: signatures[i].start_line,
                        end_line: signatures[i].end_line,
                        start_byte: 0,
                        end_byte: 0,
                        function_name: Some(signatures[i].name.clone()),
                    };
                    let instance_b = CloneInstance {
                        file: signatures[j].file.clone(),
                        start_line: signatures[j].start_line,
                        end_line: signatures[j].end_line,
                        start_byte: 0,
                        end_byte: 0,
                        function_name: Some(signatures[j].name.clone()),
                    };

                    groups.push(
                        CloneGroup::new(CloneType::Type3, vec![instance_a, instance_b])
                            .with_similarity(similarity),
                    );
                }
            }
        }

        groups
    }

    /// Compute LSH-compatible hash values from IR tokens.
    ///
    /// Generates `num_hashes` independent MinHash values by computing
    /// min(h(shingle, seed)) over all shingles for each seed. This produces
    /// a fixed-length signature suitable for LSH banding without needing
    /// access to sketch_oxide's internal MinHash values.
    fn compute_lsh_hashes(&self, ir_tokens: &[IRToken]) -> Vec<u64> {
        use xxhash_rust::xxh3::xxh3_64;

        let shingles = ir_tokens_to_shingles(ir_tokens, self.shingle_size);
        if shingles.is_empty() {
            return vec![0u64; self.num_hashes];
        }

        (0..self.num_hashes as u64)
            .map(|seed| {
                let mut min_hash = u64::MAX;
                for &shingle in &shingles {
                    let mut data = [0u8; 16];
                    data[..8].copy_from_slice(&shingle.to_le_bytes());
                    data[8..].copy_from_slice(&seed.to_le_bytes());
                    let h = xxh3_64(&data);
                    min_hash = min_hash.min(h);
                }
                min_hash
            })
            .collect()
    }
}

impl Default for CrossLanguageDetector {
    fn default() -> Self {
        Self::new(0.6)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detector_creation() {
        let detector = CrossLanguageDetector::new(0.7);
        assert_eq!(detector.threshold, 0.7);
    }

    #[test]
    fn test_empty_signatures() {
        let detector = CrossLanguageDetector::default();
        let groups = detector.detect_clones(&[]);
        assert!(groups.is_empty());
    }
}
