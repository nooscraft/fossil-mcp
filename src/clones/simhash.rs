//! SimHash file-level fingerprinting for fast near-duplicate detection.
//!
//! Uses `sketch_oxide::SimHash` (Charikar 2002) for computing 64-bit
//! fingerprints of source files. Hamming distance between fingerprints
//! gives O(1) similarity estimation, enabling fast pre-screening before
//! more expensive MinHash verification.
//!
//! Pipeline: SimHash candidate generation → MinHash Jaccard verification.

use sketch_oxide::similarity::SimHash;

/// A file's SimHash fingerprint for fast similarity screening.
#[derive(Debug, Clone)]
pub struct FileFingerprint {
    pub file: String,
    pub fingerprint: u64,
    pub line_count: usize,
}

/// SimHash-based file fingerprinter.
///
/// Computes weighted SimHash fingerprints from normalized token features.
/// Structural tokens (keywords, operators) get higher weight than identifiers,
/// preserving code structure in the fingerprint.
pub struct SimHashFingerprinter {
    /// Maximum Hamming distance for considering files as candidates.
    /// Lower = stricter. 0-10 range typical. Default: 10 (~84% similarity).
    pub max_hamming_distance: u32,
}

impl SimHashFingerprinter {
    pub fn new(max_hamming_distance: u32) -> Self {
        Self {
            max_hamming_distance,
        }
    }

    /// Create with default settings (max Hamming distance = 10, ~84% similarity).
    pub fn with_defaults() -> Self {
        Self::new(10)
    }

    /// Compute a SimHash fingerprint for a source file.
    ///
    /// Tokens are normalized and weighted:
    /// - Keywords and operators: weight 3 (structural)
    /// - Normalized placeholders ($ID, $NUM, $STR): weight 1
    /// - Other tokens: weight 2
    pub fn fingerprint(&self, source: &str) -> SimHash {
        let mut sketch = SimHash::new();

        for token in source.split_whitespace() {
            let normalized = normalize_for_simhash(token);
            let weight = token_weight(&normalized);
            sketch.update_weighted(&normalized, weight);
        }

        sketch
    }

    /// Compute fingerprints for a set of files.
    pub fn fingerprint_files(&self, files: &[(String, String)]) -> Vec<FileFingerprint> {
        files
            .iter()
            .map(|(path, source)| {
                let mut sketch = self.fingerprint(source);
                FileFingerprint {
                    file: path.clone(),
                    fingerprint: sketch.fingerprint(),
                    line_count: source.lines().count(),
                }
            })
            .collect()
    }

    /// Find candidate file pairs using SimHash Hamming distance.
    ///
    /// Returns indices of file pairs whose Hamming distance is within the
    /// threshold. These candidates should be verified with MinHash for
    /// accurate Jaccard similarity.
    ///
    /// Complexity: O(N²) on fingerprints, but each comparison is O(1)
    /// (single `u64` XOR + popcount). For 1000 files, this is ~500K
    /// comparisons at ~1ns each = <1ms total.
    pub fn find_candidates(&self, fingerprints: &[FileFingerprint]) -> Vec<(usize, usize)> {
        let mut candidates = Vec::new();

        for i in 0..fingerprints.len() {
            for j in (i + 1)..fingerprints.len() {
                let distance = SimHash::hamming_distance_from_fingerprints(
                    fingerprints[i].fingerprint,
                    fingerprints[j].fingerprint,
                );
                if distance <= self.max_hamming_distance {
                    candidates.push((i, j));
                }
            }
        }

        candidates
    }

    /// Compute similarity between two pre-computed fingerprints.
    pub fn similarity(fp1: u64, fp2: u64) -> f64 {
        SimHash::similarity_from_fingerprints(fp1, fp2)
    }
}

/// Normalize a token for SimHash fingerprinting.
///
/// Similar to MinHash normalization but simpler — we want to preserve
/// structural patterns while abstracting away specific names.
fn normalize_for_simhash(token: &str) -> String {
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
    {
        return "$STR".to_string();
    }

    // Numeric literals
    if trimmed.parse::<f64>().is_ok() {
        return "$NUM".to_string();
    }

    // Keywords — preserve as-is
    if is_keyword(trimmed) {
        return trimmed.to_string();
    }

    // Identifiers
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

/// Assign weight to a normalized token for SimHash.
///
/// Structural tokens (keywords, operators) get higher weight to ensure
/// code structure dominates the fingerprint over variable names.
fn token_weight(normalized: &str) -> i64 {
    if is_keyword(normalized) {
        3 // High weight — structure-defining
    } else if normalized == "$ID" || normalized == "$NUM" || normalized == "$STR" {
        1 // Low weight — content that varies between clones
    } else {
        2 // Medium weight — operators, punctuation, etc.
    }
}

fn is_keyword(token: &str) -> bool {
    matches!(
        token,
        "def"
            | "class"
            | "if"
            | "elif"
            | "else"
            | "for"
            | "while"
            | "return"
            | "import"
            | "from"
            | "as"
            | "try"
            | "except"
            | "finally"
            | "with"
            | "raise"
            | "pass"
            | "break"
            | "continue"
            | "and"
            | "or"
            | "not"
            | "in"
            | "is"
            | "None"
            | "True"
            | "False"
            | "self"
            | "lambda"
            | "yield"
            | "function"
            | "var"
            | "let"
            | "const"
            | "do"
            | "switch"
            | "case"
            | "default"
            | "throw"
            | "catch"
            | "new"
            | "this"
            | "typeof"
            | "instanceof"
            | "void"
            | "delete"
            | "true"
            | "false"
            | "null"
            | "async"
            | "await"
            | "export"
            | "extends"
            | "implements"
            | "interface"
            | "public"
            | "private"
            | "protected"
            | "final"
            | "abstract"
            | "struct"
            | "type"
            | "func"
            | "go"
            | "defer"
            | "chan"
            | "select"
            | "range"
            | "require"
            | "module"
            | "end"
            | "begin"
            | "rescue"
            | "ensure"
            | "fn"
            | "pub"
            | "mut"
            | "impl"
            | "trait"
            | "enum"
            | "match"
            | "int"
            | "float"
            | "string"
            | "bool"
            | "boolean"
            | "char"
            | "double"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simhash_identical_files() {
        let fp = SimHashFingerprinter::with_defaults();
        let source = "def foo(x):\n    return x + 1\n\ndef bar(y):\n    return y * 2\n";
        let mut s1 = fp.fingerprint(source);
        let mut s2 = fp.fingerprint(source);
        let sim = s1.similarity(&mut s2);
        assert!(
            (sim - 1.0).abs() < f64::EPSILON,
            "Identical files should have similarity 1.0, got {sim}"
        );
    }

    #[test]
    fn test_simhash_similar_files() {
        let fp = SimHashFingerprinter::with_defaults();
        // Same structure, different identifiers
        let source_a = "def format_bytes(size):\n    for unit in ['B', 'KB', 'MB', 'GB']:\n        if size < 1024:\n            return f\"{size:.1f} {unit}\"\n        size /= 1024\n    return f\"{size:.1f} TB\"\n";
        let source_b = "def format_file_size(bytes_count):\n    for unit in ['B', 'KB', 'MB', 'GB']:\n        if bytes_count < 1024:\n            return f\"{bytes_count:.1f} {unit}\"\n        bytes_count /= 1024\n    return f\"{bytes_count:.1f} TB\"\n";

        let mut s1 = fp.fingerprint(source_a);
        let mut s2 = fp.fingerprint(source_b);
        let sim = s1.similarity(&mut s2);
        assert!(
            sim > 0.7,
            "Similar files should have high similarity, got {sim}"
        );
    }

    #[test]
    fn test_simhash_different_files() {
        let fp = SimHashFingerprinter::with_defaults();
        let source_a = "def foo(x):\n    return x + 1\n";
        let source_b = "class HttpServer:\n    def __init__(self, port):\n        self.port = port\n    def start(self):\n        self.listen()\n    def stop(self):\n        self.shutdown()\n";

        let mut s1 = fp.fingerprint(source_a);
        let mut s2 = fp.fingerprint(source_b);
        let sim = s1.similarity(&mut s2);
        assert!(
            sim < 0.9,
            "Different files should have lower similarity, got {sim}"
        );
    }

    #[test]
    fn test_find_candidates() {
        let fp = SimHashFingerprinter::new(15); // Lenient threshold
        let files = vec![
            ("a.py".to_string(), "def foo(x):\n    return x + 1\n".to_string()),
            ("b.py".to_string(), "def bar(y):\n    return y + 1\n".to_string()),
            ("c.py".to_string(), "class Server:\n    def __init__(self):\n        self.running = False\n    def start(self):\n        self.running = True\n        self.listen()\n    def stop(self):\n        self.running = False\n        self.shutdown()\n".to_string()),
        ];

        let fingerprints = fp.fingerprint_files(&files);
        assert_eq!(fingerprints.len(), 3);

        let candidates = fp.find_candidates(&fingerprints);
        // a.py and b.py should be candidates (very similar structure)
        let has_ab = candidates.iter().any(|&(i, j)| i == 0 && j == 1);
        assert!(
            has_ab,
            "a.py and b.py should be SimHash candidates, candidates: {candidates:?}"
        );
    }

    #[test]
    fn test_fingerprint_files() {
        let fp = SimHashFingerprinter::with_defaults();
        let files = vec![
            (
                "a.py".to_string(),
                "def foo(x):\n    return x + 1\n".to_string(),
            ),
            (
                "b.py".to_string(),
                "def bar(y):\n    return y * 2\n".to_string(),
            ),
        ];
        let fingerprints = fp.fingerprint_files(&files);
        assert_eq!(fingerprints.len(), 2);
        assert_eq!(fingerprints[0].file, "a.py");
        assert_eq!(fingerprints[1].file, "b.py");
        assert!(fingerprints[0].fingerprint != 0 || fingerprints[1].fingerprint != 0);
    }

    #[test]
    fn test_similarity_static() {
        let fp = SimHashFingerprinter::with_defaults();
        let source = "def foo(x):\n    return x + 1\n";
        let mut s = fp.fingerprint(source);
        let fingerprint = s.fingerprint();
        let sim = SimHashFingerprinter::similarity(fingerprint, fingerprint);
        assert!(
            (sim - 1.0).abs() < f64::EPSILON,
            "Same fingerprint should have similarity 1.0, got {sim}"
        );
    }
}
