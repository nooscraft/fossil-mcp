//! MinHash + LSH for Type 3 (near-miss) clone detection.
//!
//! Uses `sketch_oxide::MinHash` (Broder 1997) for signature computation and
//! Jaccard similarity estimation. Adds LSH banding on top for scalable
//! candidate generation (avoids O(N²) pairwise comparisons).

use std::collections::HashMap;

use sketch_oxide::similarity::MinHash;
use sketch_oxide::Sketch;
use xxhash_rust::xxh3::xxh3_64;

use super::types::{CloneGroup, CloneInstance, CloneType};

/// Extract the MinHash signature values (one per permutation) from a sketch.
///
/// Uses `Sketch::serialize()` to access the private `hash_values` field.
/// Serialization format: `[num_perm:8][seeds:num_perm*8][values:num_perm*8]`
fn extract_minhash_values(sketch: &MinHash) -> Vec<u64> {
    let bytes = sketch.serialize();
    let num_perm =
        u64::from_le_bytes(bytes[0..8].try_into().expect("serialize has 8-byte header")) as usize;
    let values_start = 8 + num_perm * 8;
    (0..num_perm)
        .map(|i| {
            let offset = values_start + i * 8;
            u64::from_le_bytes(
                bytes[offset..offset + 8]
                    .try_into()
                    .expect("valid 8-byte slice"),
            )
        })
        .collect()
}

/// Select LSH bands and rows per band to match the similarity threshold.
///
/// The LSH probability curve has an inflection point near `(1/b)^(1/r)`.
/// We pick (b, r) where b*r ≤ n that best approximates the target threshold.
fn select_lsh_params(num_permutations: usize, threshold: f64) -> (usize, usize) {
    let mut best_bands = 16;
    let mut best_rows = num_permutations / 16;
    let mut best_error = f64::MAX;

    for r in 1..=20 {
        let b = num_permutations / r;
        if b == 0 {
            continue;
        }
        // Inflection point: (1/b)^(1/r)
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

/// Common language keywords that should NOT be normalized to `$ID`.
const KEYWORDS: &[&str] = &[
    // Python
    "def",
    "class",
    "if",
    "elif",
    "else",
    "for",
    "while",
    "return",
    "import",
    "from",
    "as",
    "try",
    "except",
    "finally",
    "with",
    "raise",
    "pass",
    "break",
    "continue",
    "and",
    "or",
    "not",
    "in",
    "is",
    "None",
    "True",
    "False",
    "self",
    "lambda",
    "yield",
    "global",
    // JavaScript / TypeScript
    "function",
    "var",
    "let",
    "const",
    "do",
    "switch",
    "case",
    "default",
    "throw",
    "catch",
    "new",
    "this",
    "typeof",
    "instanceof",
    "void",
    "delete",
    "true",
    "false",
    "null",
    "undefined",
    "async",
    "await",
    "export",
    "extends",
    "implements",
    "interface",
    "super",
    "static",
    // Java / C# / Go
    "public",
    "private",
    "protected",
    "final",
    "abstract",
    "synchronized",
    "package",
    "struct",
    "type",
    "func",
    "go",
    "defer",
    "chan",
    "select",
    "range",
    "map",
    // Ruby / PHP
    "require",
    "module",
    "end",
    "begin",
    "rescue",
    "ensure",
    "puts",
    "attr_accessor",
    "echo",
    "foreach",
    "array",
    "include",
    "namespace",
    "use",
    // Common across languages
    "int",
    "float",
    "string",
    "bool",
    "boolean",
    "char",
    "double",
    "long",
    "short",
    "byte",
];

/// Split source into tokens by whitespace AND structural punctuation.
///
/// Ensures tokens like `func(arg,` are split into `["func", "(", "arg", ","]`
/// so that identifier normalization works correctly.
/// String literals (including those with internal spaces like Ruby interpolation)
/// are kept as single tokens.
fn tokenize_source(source: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut string_delim = '"';

    for c in source.chars() {
        if in_string {
            current.push(c);
            if c == string_delim {
                in_string = false;
                // End of string — emit the entire string as one token
                tokens.push(std::mem::take(&mut current));
            }
        } else if c == '"' || c == '\'' {
            // Flush any accumulated non-string token
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            current.push(c);
            in_string = true;
            string_delim = c;
        } else if "(){}[],;:".contains(c) {
            // Flush current token, emit punctuation as its own token
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            tokens.push(c.to_string());
        } else if c.is_whitespace() {
            // Flush current token on whitespace
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }

    // Flush any remaining token (e.g., unclosed string or trailing identifier)
    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

/// Normalize a token for clone detection.
/// Identifiers → `$ID`, numbers → `$NUM`, strings → `$STR`.
/// Keywords and operators are preserved.
fn normalize_token(token: &str) -> String {
    // Strip common punctuation at boundaries for matching
    let trimmed = token.trim_matches(|c: char| {
        c == '('
            || c == ')'
            || c == ','
            || c == ';'
            || c == ':'
            || c == '{'
            || c == '}'
            || c == '['
            || c == ']'
    });

    if trimmed.is_empty() {
        return token.to_string();
    }

    // String literals
    if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
        || (trimmed.starts_with("f\"") || trimmed.starts_with("f'"))
    {
        return "$STR".to_string();
    }

    // Numeric literals
    if trimmed.parse::<f64>().is_ok() {
        return "$NUM".to_string();
    }

    // Keywords — preserve as-is
    if KEYWORDS.contains(&trimmed) {
        return token.to_string();
    }

    // Operators and short symbols — preserve
    if trimmed.len() <= 2 && !trimmed.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return token.to_string();
    }

    // Identifiers (alphanumeric + underscore, starting with letter or _)
    if trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
        && trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
    {
        return "$ID".to_string();
    }

    token.to_string()
}

/// A function's MinHash signature for clone detection.
#[derive(Debug, Clone)]
pub struct FunctionSignature {
    pub file: String,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
    /// The sketch_oxide MinHash sketch for this function's shingle set.
    pub sketch: MinHash,
    /// Raw shingle hashes (kept for LSH banding).
    pub shingle_hashes: Vec<u64>,
}

/// MinHash + LSH based clone detector for Type 3 clones.
///
/// Uses `sketch_oxide::MinHash` (128 permutations by default) for signature
/// computation and Jaccard similarity estimation. LSH banding provides
/// sub-quadratic candidate generation for large codebases.
pub struct MinHashDetector {
    /// Number of hash permutations for MinHash signature.
    num_permutations: usize,
    /// Size of token shingles (k-grams).
    shingle_size: usize,
    /// Similarity threshold for reporting clones.
    similarity_threshold: f64,
    /// Number of LSH bands.
    num_bands: usize,
    /// Rows per band.
    rows_per_band: usize,
}

impl MinHashDetector {
    pub fn new(num_permutations: usize, shingle_size: usize, similarity_threshold: f64) -> Self {
        let (num_bands, rows_per_band) = select_lsh_params(num_permutations, similarity_threshold);

        Self {
            num_permutations,
            shingle_size,
            similarity_threshold,
            num_bands,
            rows_per_band,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(128, 3, 0.6)
    }

    /// Compute normalized token shingles from source text.
    ///
    /// Tokens are normalized to reduce sensitivity to renames:
    /// - Identifiers (snake_case, camelCase) → `$ID`
    /// - Numeric literals → `$NUM`
    /// - String literals → `$STR`
    /// - Keywords and operators are preserved as-is
    ///
    /// Returns a vector of xxh3 hashes of k-token shingles.
    pub fn compute_shingles(&self, source: &str) -> Vec<u64> {
        let tokens: Vec<String> = tokenize_source(source)
            .into_iter()
            .map(|t| normalize_token(&t))
            .collect();
        if tokens.len() < self.shingle_size {
            return Vec::new();
        }

        tokens
            .windows(self.shingle_size)
            .map(|window| {
                let combined: String = window.join(" ");
                xxh3_64(combined.as_bytes())
            })
            .collect()
    }

    /// Build a `sketch_oxide::MinHash` sketch from shingle hashes.
    ///
    /// Each shingle hash is fed as an item to the MinHash sketch, which
    /// maintains the minimum hash value across 128 permutations.
    pub fn build_sketch(&self, shingles: &[u64]) -> MinHash {
        let mut sketch = MinHash::new(self.num_permutations)
            .expect("num_permutations >= 16 guaranteed by constructor");

        for &shingle in shingles {
            sketch.update(&shingle);
        }

        sketch
    }

    /// Compute Jaccard similarity between two shingle sets using sketch_oxide.
    pub fn jaccard_similarity(sketch_a: &MinHash, sketch_b: &MinHash) -> f64 {
        sketch_a.jaccard_similarity(sketch_b).unwrap_or(0.0)
    }

    /// Compute exact Jaccard similarity between two raw shingle sets.
    pub fn exact_jaccard(a: &[u64], b: &[u64]) -> f64 {
        if a.is_empty() && b.is_empty() {
            return 1.0;
        }
        if a.is_empty() || b.is_empty() {
            return 0.0;
        }

        let set_a: std::collections::HashSet<u64> = a.iter().copied().collect();
        let set_b: std::collections::HashSet<u64> = b.iter().copied().collect();

        let intersection = set_a.intersection(&set_b).count();
        let union = set_a.union(&set_b).count();

        if union == 0 {
            0.0
        } else {
            intersection as f64 / union as f64
        }
    }

    /// Detect clones from a set of function signatures using LSH + sketch_oxide.
    ///
    /// Pipeline:
    /// 1. LSH banding over MinHash signature for candidate generation
    /// 2. Verify candidates with `sketch_oxide::MinHash::jaccard_similarity()`
    pub fn detect_clones(&self, signatures: &[FunctionSignature]) -> Vec<CloneGroup> {
        // LSH: band the MinHash signature (not raw shingles) for proper
        // probability guarantees. The MinHash signature has exactly
        // `num_permutations` values; dividing into `num_bands` bands of
        // `rows_per_band` rows gives an S-curve inflection point near the
        // target similarity threshold.
        let mut buckets: HashMap<(usize, u64), Vec<usize>> = HashMap::new();

        for (idx, sig) in signatures.iter().enumerate() {
            if sig.shingle_hashes.is_empty() {
                continue;
            }

            // Extract the actual MinHash signature values for LSH banding
            let minhash_sig = extract_minhash_values(&sig.sketch);

            for band in 0..self.num_bands {
                let start = band * self.rows_per_band;
                let end = (start + self.rows_per_band).min(minhash_sig.len());
                if start >= minhash_sig.len() {
                    break;
                }

                let band_slice = &minhash_sig[start..end];
                let band_hash = xxh3_64(
                    &band_slice
                        .iter()
                        .flat_map(|h| h.to_le_bytes())
                        .collect::<Vec<u8>>(),
                );
                buckets.entry((band, band_hash)).or_default().push(idx);
            }
        }

        // Collect candidate pairs from LSH buckets
        let mut candidate_pairs: std::collections::HashSet<(usize, usize)> =
            std::collections::HashSet::new();
        for members in buckets.values() {
            if members.len() >= 2 {
                for i in 0..members.len() {
                    for j in (i + 1)..members.len() {
                        let a = members[i].min(members[j]);
                        let b = members[i].max(members[j]);
                        candidate_pairs.insert((a, b));
                    }
                }
            }
        }

        // Verify candidates with sketch_oxide MinHash Jaccard estimation
        let mut groups: Vec<CloneGroup> = Vec::new();

        for (i, j) in candidate_pairs {
            let similarity = Self::jaccard_similarity(&signatures[i].sketch, &signatures[j].sketch);

            if similarity >= self.similarity_threshold {
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

        groups
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sketch_oxide::Sketch;

    #[test]
    fn test_compute_shingles() {
        let detector = MinHashDetector::with_defaults();
        let source = "def foo ( x ) : return x + 1";
        let shingles = detector.compute_shingles(source);
        assert!(!shingles.is_empty());
    }

    #[test]
    fn test_sketch_oxide_jaccard_identical() {
        let detector = MinHashDetector::with_defaults();
        let shingles = vec![1u64, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let sketch_a = detector.build_sketch(&shingles);
        let sketch_b = detector.build_sketch(&shingles);
        let similarity = MinHashDetector::jaccard_similarity(&sketch_a, &sketch_b);
        assert!(
            (similarity - 1.0).abs() < 0.05,
            "Identical sets should have ~1.0 Jaccard, got {similarity}"
        );
    }

    #[test]
    fn test_sketch_oxide_jaccard_disjoint() {
        let detector = MinHashDetector::with_defaults();
        let a: Vec<u64> = (0..100).collect();
        let b: Vec<u64> = (1000..1100).collect();
        let sketch_a = detector.build_sketch(&a);
        let sketch_b = detector.build_sketch(&b);
        let similarity = MinHashDetector::jaccard_similarity(&sketch_a, &sketch_b);
        assert!(
            similarity < 0.1,
            "Disjoint sets should have ~0.0 Jaccard, got {similarity}"
        );
    }

    #[test]
    fn test_sketch_oxide_jaccard_partial_overlap() {
        let detector = MinHashDetector::with_defaults();
        let a: Vec<u64> = (0..100).collect();
        let b: Vec<u64> = (50..150).collect();
        let sketch_a = detector.build_sketch(&a);
        let sketch_b = detector.build_sketch(&b);
        let similarity = MinHashDetector::jaccard_similarity(&sketch_a, &sketch_b);
        // Expected: 50/150 ≈ 0.33
        assert!(
            (similarity - 0.33).abs() < 0.15,
            "50/150 overlap should have ~0.33 Jaccard, got {similarity}"
        );
    }

    #[test]
    fn test_exact_jaccard() {
        let a = vec![1, 2, 3, 4, 5];
        let similarity = MinHashDetector::exact_jaccard(&a, &a);
        assert!((similarity - 1.0).abs() < f64::EPSILON);

        let b = vec![4, 5, 6];
        let similarity = MinHashDetector::exact_jaccard(&a, &b);
        // Intersection: {4, 5}, Union: {1, 2, 3, 4, 5, 6} → 2/6 = 0.333
        assert!((similarity - 2.0 / 6.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_build_sketch_empty() {
        let detector = MinHashDetector::with_defaults();
        let sketch = detector.build_sketch(&[]);
        assert!(sketch.is_empty());
    }

    #[test]
    fn test_ruby_string_interpolation_preserved_as_single_token() {
        // Ruby string interpolation like "$#{format('%.2f', amount)}"
        // must be kept as a single token despite internal spaces.
        let source = "def format_currency(amount, currency)\n  \"$#{format('%.2f', amount)}\"";
        let tokens = tokenize_source(source);

        // The interpolated string should be one token, not split on internal spaces
        let string_tokens: Vec<&str> = tokens
            .iter()
            .filter(|t| t.starts_with('"'))
            .map(|t| t.as_str())
            .collect();
        assert!(
            string_tokens.iter().all(|t| t.ends_with('"')),
            "All tokens starting with \\\" must also end with \\\": got {:?}",
            string_tokens
        );
    }

    #[test]
    fn test_format_currency_vs_format_money_detected() {
        // Regression: functions differing only in parameter names must be
        // detected as clones. The tokenizer must not split string literals
        // containing spaces (e.g., Ruby interpolation).
        let auth_path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/auth.rb");
        let source = std::fs::read_to_string(auth_path).expect("auth.rb must exist");
        let lines: Vec<&str> = source.lines().collect();

        let body_a = lines[77..89].join("\n");
        let body_b = lines[90..102].join("\n");

        let detector = MinHashDetector::with_defaults();
        let shingles_a = detector.compute_shingles(&body_a);
        let shingles_b = detector.compute_shingles(&body_b);

        let exact = MinHashDetector::exact_jaccard(&shingles_a, &shingles_b);
        assert!(
            exact > 0.9,
            "format_currency and format_money should have near-identical shingles, got {exact}"
        );

        let sketch_a = detector.build_sketch(&shingles_a);
        let sketch_b = detector.build_sketch(&shingles_b);
        let sigs = vec![
            FunctionSignature {
                file: "auth.rb".to_string(),
                name: "format_currency".to_string(),
                start_line: 78,
                end_line: 89,
                sketch: sketch_a,
                shingle_hashes: shingles_a,
            },
            FunctionSignature {
                file: "auth.rb".to_string(),
                name: "format_money".to_string(),
                start_line: 91,
                end_line: 102,
                sketch: sketch_b,
                shingle_hashes: shingles_b,
            },
        ];
        let groups = detector.detect_clones(&sigs);
        assert!(
            !groups.is_empty(),
            "format_currency and format_money should be detected as clones"
        );
    }
}
