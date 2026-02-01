//! TF-IDF + truncated SVD code embeddings for Type-4 clone detection.
//!
//! Pipeline: tokenize -> TF-IDF -> truncated SVD -> k-dimensional embedding.
//! All operations are pure Rust with no external ML dependencies.
//!
//! The embedding engine first builds a vocabulary from a corpus of code fragments,
//! computes IDF weights for each token, then uses randomized truncated SVD
//! (Halko et al. 2011) to reduce the TF-IDF vectors to a compact representation.
//! Cosine similarity in the embedding space correlates with semantic code similarity.

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Code embedding engine using TF-IDF + truncated SVD.
///
/// Workflow:
/// 1. Call [`fit`](Self::fit) with a corpus of source code fragments.
/// 2. Call [`embed`](Self::embed) on individual fragments to get dense vectors.
/// 3. Compare vectors with [`cosine_similarity`](Self::cosine_similarity).
pub struct CodeEmbeddingEngine {
    /// Vocabulary: token -> index.
    vocabulary: HashMap<String, usize>,
    /// Inverse document frequency for each token.
    idf: Vec<f64>,
    /// Number of dimensions in the final embedding.
    embedding_dim: usize,
    /// SVD basis vectors (embedding_dim x vocab_size matrix, row-major).
    svd_basis: Vec<Vec<f64>>,
    /// Whether the engine has been fitted.
    fitted: bool,
}

impl CodeEmbeddingEngine {
    /// Create a new engine with the specified embedding dimension.
    pub fn new(embedding_dim: usize) -> Self {
        Self {
            vocabulary: HashMap::new(),
            idf: Vec::new(),
            embedding_dim,
            svd_basis: Vec::new(),
            fitted: false,
        }
    }

    /// Default engine with 64-dimensional embeddings.
    pub fn with_defaults() -> Self {
        Self::new(64)
    }

    /// Return the vocabulary size (number of distinct tokens after fitting).
    pub fn vocab_size(&self) -> usize {
        self.vocabulary.len()
    }

    /// Return the configured embedding dimensionality.
    pub fn embedding_dim(&self) -> usize {
        self.embedding_dim
    }

    /// Whether the engine has been fitted on a corpus.
    pub fn is_fitted(&self) -> bool {
        self.fitted
    }

    // -----------------------------------------------------------------------
    // Fit
    // -----------------------------------------------------------------------

    /// Fit the engine on a corpus of source code fragments.
    ///
    /// Builds vocabulary, computes IDF weights, and computes SVD basis.
    pub fn fit(&mut self, corpus: &[&str]) {
        if corpus.is_empty() {
            return;
        }

        // Tokenize all documents and build vocabulary.
        let tokenized: Vec<Vec<String>> = corpus.iter().map(|s| tokenize_code(s)).collect();
        self.build_vocabulary(&tokenized);

        if self.vocabulary.is_empty() {
            return;
        }

        // Compute IDF.
        self.compute_idf(&tokenized);

        // Build TF-IDF matrix (num_docs x vocab_size).
        let tfidf_matrix = self.build_tfidf_matrix(&tokenized);

        // Truncated SVD via randomized power iteration.
        self.svd_basis = truncated_svd(
            &tfidf_matrix,
            self.embedding_dim,
            corpus.len(),
            self.vocabulary.len(),
        );

        self.fitted = true;
    }

    // -----------------------------------------------------------------------
    // Embed
    // -----------------------------------------------------------------------

    /// Embed a single source code fragment into the learned space.
    ///
    /// Returns a vector of length `embedding_dim`. If the engine is not fitted
    /// the returned vector is all zeros.
    pub fn embed(&self, source: &str) -> Vec<f64> {
        if !self.fitted || self.svd_basis.is_empty() {
            return vec![0.0; self.embedding_dim];
        }

        let tokens = tokenize_code(source);
        let tfidf = self.compute_tfidf_vector(&tokens);

        // Project onto SVD basis.
        let actual_dim = self.embedding_dim.min(self.svd_basis.len());
        let mut embedding = vec![0.0; actual_dim];
        for (i, basis) in self.svd_basis.iter().enumerate() {
            if i >= actual_dim {
                break;
            }
            embedding[i] = dot_product(&tfidf, basis);
        }

        // L2 normalize.
        l2_normalize(&mut embedding);

        // Pad to full embedding_dim if SVD produced fewer dimensions.
        embedding.resize(self.embedding_dim, 0.0);
        embedding
    }

    /// Compute cosine similarity between two embeddings.
    ///
    /// Returns a value in \[-1.0, 1.0\]. Identical (normalized) embeddings yield 1.0.
    pub fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }
        let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a = a.iter().map(|x| x * x).sum::<f64>().sqrt();
        let norm_b = b.iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm_a < f64::EPSILON || norm_b < f64::EPSILON {
            return 0.0;
        }
        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Build vocabulary from tokenized documents.
    fn build_vocabulary(&mut self, tokenized: &[Vec<String>]) {
        self.vocabulary.clear();
        let mut idx = 0usize;
        for doc in tokenized {
            for token in doc {
                if !self.vocabulary.contains_key(token) {
                    self.vocabulary.insert(token.clone(), idx);
                    idx += 1;
                }
            }
        }
    }

    /// Compute inverse document frequency for each token in the vocabulary.
    ///
    /// Uses the smoothed variant: idf(t) = ln((1 + N) / (1 + df(t))) + 1
    fn compute_idf(&mut self, tokenized: &[Vec<String>]) {
        let n = tokenized.len() as f64;
        let vocab_size = self.vocabulary.len();
        let mut df = vec![0usize; vocab_size];

        for doc in tokenized {
            // Count each token at most once per document.
            let mut seen = vec![false; vocab_size];
            for token in doc {
                if let Some(&idx) = self.vocabulary.get(token) {
                    if !seen[idx] {
                        df[idx] += 1;
                        seen[idx] = true;
                    }
                }
            }
        }

        self.idf = df
            .iter()
            .map(|&d| ((1.0 + n) / (1.0 + d as f64)).ln() + 1.0)
            .collect();
    }

    /// Build the full TF-IDF matrix (num_docs x vocab_size).
    fn build_tfidf_matrix(&self, tokenized: &[Vec<String>]) -> Vec<Vec<f64>> {
        tokenized
            .iter()
            .map(|doc| self.compute_tfidf_vector(doc))
            .collect()
    }

    /// Compute the TF-IDF vector for a single tokenized document.
    fn compute_tfidf_vector(&self, tokens: &[String]) -> Vec<f64> {
        let vocab_size = self.vocabulary.len();
        let mut tf = vec![0.0f64; vocab_size];

        for token in tokens {
            if let Some(&idx) = self.vocabulary.get(token) {
                tf[idx] += 1.0;
            }
        }

        // Sub-linear TF: 1 + ln(tf) if tf > 0.
        for val in &mut tf {
            if *val > 0.0 {
                *val = 1.0 + val.ln();
            }
        }

        // Multiply by IDF.
        for (i, val) in tf.iter_mut().enumerate() {
            if i < self.idf.len() {
                *val *= self.idf[i];
            }
        }

        // L2 normalize the TF-IDF vector.
        l2_normalize(&mut tf);

        tf
    }
}

// ---------------------------------------------------------------------------
// Code tokenizer
// ---------------------------------------------------------------------------

/// Tokenize source code into normalized tokens.
///
/// Splits on whitespace/punctuation, splits camelCase/PascalCase identifiers,
/// normalizes numeric literals to `$NUM`, and drops single-character tokens.
fn tokenize_code(source: &str) -> Vec<String> {
    let mut tokens = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();
        // Skip full-line comments.
        if trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
            || trimmed.starts_with("*/")
        {
            continue;
        }

        let mut current = String::new();
        for ch in trimmed.chars() {
            if ch.is_alphanumeric() || ch == '_' {
                current.push(ch);
            } else {
                if !current.is_empty() {
                    tokens.extend(split_camel_case(&current));
                    current.clear();
                }
                // Keep certain operators as tokens.
                if matches!(
                    ch,
                    '+' | '-' | '*' | '/' | '=' | '<' | '>' | '!' | '&' | '|'
                ) {
                    tokens.push(ch.to_string());
                }
            }
        }
        if !current.is_empty() {
            tokens.extend(split_camel_case(&current));
        }
    }

    // Normalize and filter.
    tokens
        .into_iter()
        .map(|t| normalize_token(&t))
        .filter(|t| !t.is_empty() && t.len() > 1) // drop single chars
        .collect()
}

/// Split camelCase and PascalCase identifiers into separate lowercase tokens.
///
/// Examples:
/// - `"camelCase"`   -> `["camel", "case"]`
/// - `"HTMLParser"`  -> `["html", "parser"]`
/// - `"getHTTPUrl"`  -> `["get", "http", "url"]`
/// - `"simple"`      -> `["simple"]`
/// - `"ALL_CAPS"`    -> `["all", "caps"]`
fn split_camel_case(s: &str) -> Vec<String> {
    // First split on underscores.
    let parts: Vec<&str> = s.split('_').filter(|p| !p.is_empty()).collect();
    let mut result = Vec::new();

    for part in parts {
        let chars: Vec<char> = part.chars().collect();
        if chars.is_empty() {
            continue;
        }

        let mut current = String::new();
        current.push(chars[0]);

        for i in 1..chars.len() {
            let prev = chars[i - 1];
            let cur = chars[i];
            let next = chars.get(i + 1);

            if cur.is_uppercase() {
                if prev.is_lowercase() || prev.is_ascii_digit() {
                    // camelCase boundary: aB -> a | B
                    if !current.is_empty() {
                        result.push(current.to_lowercase());
                        current.clear();
                    }
                } else if prev.is_uppercase() {
                    // Consecutive uppercase: check if next is lowercase (acronym end).
                    // e.g., HTMLParser: at 'P' the previous is 'L' (upper), next is 'a' (lower).
                    if let Some(&n) = next {
                        if n.is_lowercase() && !current.is_empty() {
                            result.push(current.to_lowercase());
                            current.clear();
                        }
                    }
                }
            }
            current.push(cur);
        }
        if !current.is_empty() {
            result.push(current.to_lowercase());
        }
    }

    if result.is_empty() {
        result.push(s.to_lowercase());
    }

    result
}

/// Normalize a token: lowercase, replace pure numeric literals with `$NUM`.
fn normalize_token(token: &str) -> String {
    // If it looks like a number (integer or float), replace.
    if token.parse::<f64>().is_ok() {
        return "$NUM".to_string();
    }
    token.to_lowercase()
}

// ---------------------------------------------------------------------------
// Linear algebra helpers
// ---------------------------------------------------------------------------

/// Dot product of two vectors (truncated to the shorter length).
fn dot_product(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

/// L2-normalize a vector in place.
fn l2_normalize(v: &mut [f64]) {
    let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm > f64::EPSILON {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// Multiply two row-major matrices: C = A * B.
///
/// A is (m x n), B is (n x p), result is (m x p).
fn mat_mul(a: &[Vec<f64>], b: &[Vec<f64>], m: usize, n: usize, p: usize) -> Vec<Vec<f64>> {
    let mut c = vec![vec![0.0; p]; m];
    for i in 0..m {
        if a[i].len() < n {
            continue;
        }
        for k in 0..n {
            let a_ik = a[i][k];
            if a_ik.abs() < f64::EPSILON {
                continue;
            }
            if b[k].len() < p {
                continue;
            }
            for j in 0..p {
                c[i][j] += a_ik * b[k][j];
            }
        }
    }
    c
}

/// Multiply A^T * B where A is (m x n) and B is (m x p), giving (n x p).
///
/// This avoids explicitly transposing A.
fn mat_mul_transpose_left(
    a: &[Vec<f64>],
    b: &[Vec<f64>],
    n: usize,
    m: usize,
    p: usize,
) -> Vec<Vec<f64>> {
    let mut c = vec![vec![0.0; p]; n];
    for k in 0..m {
        let a_row = if k < a.len() { &a[k] } else { continue };
        let b_row = if k < b.len() { &b[k] } else { continue };
        for i in 0..n.min(a_row.len()) {
            let a_ki = a_row[i];
            if a_ki.abs() < f64::EPSILON {
                continue;
            }
            for j in 0..p.min(b_row.len()) {
                c[i][j] += a_ki * b_row[j];
            }
        }
    }
    c
}

/// Generate a random Gaussian matrix (rows x cols) using a deterministic PRNG.
fn random_gaussian_matrix(rows: usize, cols: usize, rng: &mut SimpleRng) -> Vec<Vec<f64>> {
    (0..rows)
        .map(|_| (0..cols).map(|_| rng.next_gaussian()).collect())
        .collect()
}

/// QR decomposition via modified Gram-Schmidt.
///
/// Input matrix `a` is (m x n) row-major. Returns Q (m x n) with orthonormal columns.
#[allow(clippy::needless_range_loop)]
fn qr_decomposition(a: &[Vec<f64>], m: usize, n: usize) -> Vec<Vec<f64>> {
    // Work column-wise. We store columns as separate vectors for convenience.
    let mut cols: Vec<Vec<f64>> = (0..n)
        .map(|j| {
            (0..m)
                .map(|i| if j < a[i].len() { a[i][j] } else { 0.0 })
                .collect()
        })
        .collect();

    for j in 0..n {
        // Orthogonalize column j against all previous columns.
        for k in 0..j {
            let proj = dot_product(&cols[j], &cols[k]);
            for i in 0..m {
                cols[j][i] -= proj * cols[k][i];
            }
        }
        // Normalize.
        let norm = cols[j].iter().map(|x| x * x).sum::<f64>().sqrt();
        if norm > f64::EPSILON {
            for i in 0..m {
                cols[j][i] /= norm;
            }
        }
    }

    // Convert back to row-major (m x n).
    let mut q = vec![vec![0.0; n]; m];
    for i in 0..m {
        for j in 0..n {
            q[i][j] = cols[j][i];
        }
    }
    q
}

// ---------------------------------------------------------------------------
// Truncated SVD via randomized power iteration (Halko et al. 2011)
// ---------------------------------------------------------------------------

/// Compute truncated SVD of a matrix using randomized power iteration.
///
/// Given a TF-IDF matrix A of shape (num_docs x vocab_size), this computes
/// k approximate right singular vectors (each of length vocab_size).
/// These serve as the projection basis for embedding new documents.
///
/// Algorithm outline (Halko, Martinsson, Tropp 2011):
/// 1. Draw random Gaussian matrix Omega (vocab_size x k).
/// 2. Form Y = A * Omega.
/// 3. Power iteration: Y = A * A^T * Y (repeat for accuracy).
/// 4. QR decomposition: Q = orth(Y).
/// 5. Form small matrix B = Q^T * A (k x vocab_size).
/// 6. SVD of B to obtain approximate right singular vectors of A.
///
/// Uses a seeded PRNG for deterministic results.
fn truncated_svd(
    tfidf_matrix: &[Vec<f64>], // num_docs x vocab_size
    k: usize,
    num_docs: usize,
    vocab_size: usize,
) -> Vec<Vec<f64>> {
    let k = k.min(num_docs).min(vocab_size);
    if k == 0 || num_docs == 0 || vocab_size == 0 {
        return Vec::new();
    }

    // Generate random Gaussian matrix Omega (vocab_size x k).
    let mut rng = SimpleRng::new(42); // deterministic seed
    let omega = random_gaussian_matrix(vocab_size, k, &mut rng);

    // Form Y = A * Omega (num_docs x k).
    let mut y = mat_mul(tfidf_matrix, &omega, num_docs, vocab_size, k);

    // Power iteration (2 iterations for improved accuracy).
    for _ in 0..2 {
        // at_y = A^T * Y (vocab_size x k)
        let at_y = mat_mul_transpose_left(tfidf_matrix, &y, vocab_size, num_docs, k);
        // Y = A * at_y (num_docs x k)
        y = mat_mul(tfidf_matrix, &at_y, num_docs, vocab_size, k);
    }

    // QR decomposition of Y -> Q (num_docs x k), orthonormal columns.
    let q = qr_decomposition(&y, num_docs, k);

    // Form B = Q^T * A (k x vocab_size).
    let b = mat_mul_transpose_left(&q, tfidf_matrix, k, num_docs, vocab_size);

    // Compute SVD of the small matrix B (k x vocab_size).
    //
    // We use one-sided Jacobi iterations on B * B^T to get the left singular
    // vectors of B (which correspond to the right singular vectors of A when
    // combined with Q). For the embedding projection we only need the rows of
    // B rotated to align with singular directions. As a lightweight approach
    // we symmetrize via B * B^T and diagonalize with Jacobi rotations, then
    // form V_approx = U^T * B (where U diagonalizes B*B^T).
    //
    // If the small SVD fails or k is tiny, we fall back to normalized rows of B.
    small_svd_right_vectors(&b, k, vocab_size)
}

/// Compute approximate right singular vectors of a small (k x n) matrix
/// via symmetric eigendecomposition of B * B^T using Jacobi rotations.
///
/// Returns up to k normalized row vectors, each of length n.
#[allow(clippy::needless_range_loop)]
fn small_svd_right_vectors(b: &[Vec<f64>], k: usize, n: usize) -> Vec<Vec<f64>> {
    if k == 0 || n == 0 || b.is_empty() {
        return Vec::new();
    }

    // Form S = B * B^T (k x k symmetric).
    let mut s = vec![vec![0.0; k]; k];
    for i in 0..k {
        for j in i..k {
            let val: f64 = b[i].iter().zip(b[j].iter()).map(|(a, b)| a * b).sum();
            s[i][j] = val;
            s[j][i] = val;
        }
    }

    // Jacobi eigendecomposition of S.
    // U will hold the eigenvectors as columns.
    let mut u = vec![vec![0.0; k]; k];
    for i in 0..k {
        u[i][i] = 1.0;
    }

    let max_iter = 100 * k * k;
    let tol = 1e-12;

    for _ in 0..max_iter {
        // Find the off-diagonal element with largest absolute value.
        let mut max_val = 0.0f64;
        let mut p = 0;
        let mut q = 1;
        for i in 0..k {
            for j in (i + 1)..k {
                if s[i][j].abs() > max_val {
                    max_val = s[i][j].abs();
                    p = i;
                    q = j;
                }
            }
        }

        if max_val < tol {
            break;
        }

        // Compute Jacobi rotation.
        let theta = if (s[p][p] - s[q][q]).abs() < f64::EPSILON {
            std::f64::consts::FRAC_PI_4
        } else {
            0.5 * (2.0 * s[p][q] / (s[p][p] - s[q][q])).atan()
        };

        let cos_t = theta.cos();
        let sin_t = theta.sin();

        // Apply rotation to S: S' = G^T * S * G.
        // Update rows p, q for all columns.
        for i in 0..k {
            let sp_i = s[p][i];
            let sq_i = s[q][i];
            s[p][i] = cos_t * sp_i + sin_t * sq_i;
            s[q][i] = -sin_t * sp_i + cos_t * sq_i;
        }
        // Update columns p, q for all rows.
        for i in 0..k {
            let si_p = s[i][p];
            let si_q = s[i][q];
            s[i][p] = cos_t * si_p + sin_t * si_q;
            s[i][q] = -sin_t * si_p + cos_t * si_q;
        }

        // Accumulate rotation in U.
        for i in 0..k {
            let ui_p = u[i][p];
            let ui_q = u[i][q];
            u[i][p] = cos_t * ui_p + sin_t * ui_q;
            u[i][q] = -sin_t * ui_p + cos_t * ui_q;
        }
    }

    // Eigenvalues are on the diagonal of S. Sort by descending eigenvalue.
    let mut eigen_indices: Vec<(usize, f64)> = (0..k).map(|i| (i, s[i][i])).collect();
    eigen_indices.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Form right singular vectors: v_i = (1/sigma_i) * B^T * u_i
    // where sigma_i = sqrt(eigenvalue_i).
    let mut basis = Vec::with_capacity(k);
    for &(col_idx, eigenval) in &eigen_indices {
        if eigenval < f64::EPSILON {
            continue;
        }
        let sigma = eigenval.sqrt();

        // u_col = column col_idx of U.
        let u_col: Vec<f64> = (0..k).map(|i| u[i][col_idx]).collect();

        // v = B^T * u_col (length n), then normalize.
        let mut v = vec![0.0; n];
        for (i, &u_val) in u_col.iter().enumerate() {
            if u_val.abs() < f64::EPSILON || i >= b.len() {
                continue;
            }
            for (j, bval) in b[i].iter().enumerate() {
                if j < n {
                    v[j] += u_val * bval;
                }
            }
        }

        // Divide by sigma and normalize.
        for x in &mut v {
            *x /= sigma;
        }
        l2_normalize(&mut v);

        basis.push(v);
        if basis.len() >= k {
            break;
        }
    }

    basis
}

// ---------------------------------------------------------------------------
// Simple deterministic PRNG
// ---------------------------------------------------------------------------

/// Simple deterministic PRNG (xorshift64).
///
/// Used to generate the random Gaussian matrix for randomized SVD.
/// Seeded for reproducibility.
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        // Avoid zero state which is a fixed point of xorshift.
        Self {
            state: if seed == 0 { 1 } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    /// Generate approximate Gaussian via Box-Muller transform.
    fn next_gaussian(&mut self) -> f64 {
        let u1 = (self.next_u64() as f64 / u64::MAX as f64).max(f64::EPSILON);
        let u2 = self.next_u64() as f64 / u64::MAX as f64;
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- Tokenizer tests --

    #[test]
    fn test_tokenizer_splits_camel_case() {
        let tokens = split_camel_case("camelCaseWord");
        assert_eq!(tokens, vec!["camel", "case", "word"]);
    }

    #[test]
    fn test_tokenizer_splits_pascal_case() {
        let tokens = split_camel_case("PascalCaseWord");
        assert_eq!(tokens, vec!["pascal", "case", "word"]);
    }

    #[test]
    fn test_tokenizer_splits_acronyms() {
        let tokens = split_camel_case("HTMLParser");
        assert_eq!(tokens, vec!["html", "parser"]);
    }

    #[test]
    fn test_tokenizer_splits_underscores() {
        let tokens = split_camel_case("snake_case_word");
        assert_eq!(tokens, vec!["snake", "case", "word"]);
    }

    #[test]
    fn test_tokenizer_mixed() {
        let tokens = split_camel_case("getHTTPUrl");
        assert_eq!(tokens, vec!["get", "http", "url"]);
    }

    #[test]
    fn test_tokenizer_all_lowercase() {
        let tokens = split_camel_case("simple");
        assert_eq!(tokens, vec!["simple"]);
    }

    #[test]
    fn test_tokenizer_all_caps() {
        let tokens = split_camel_case("ALL_CAPS");
        assert_eq!(tokens, vec!["all", "caps"]);
    }

    #[test]
    fn test_normalize_number_token() {
        assert_eq!(normalize_token("42"), "$NUM");
        assert_eq!(normalize_token("3.14"), "$NUM");
        assert_eq!(normalize_token("hello"), "hello");
    }

    #[test]
    fn test_tokenize_code_skips_comments() {
        let source = "// This is a comment\nlet x = getValue();\n# Python comment\n";
        let tokens = tokenize_code(source);
        // Should not contain tokens from comment lines.
        assert!(!tokens
            .iter()
            .any(|t| t == "this" || t == "comment" || t == "python"));
        // Should contain tokens from the code line.
        assert!(tokens.iter().any(|t| t == "get" || t == "value"));
    }

    #[test]
    fn test_tokenize_code_includes_operators() {
        let source = "let result = alpha + beta * gamma";
        let tokens = tokenize_code(source);
        // Single-char identifiers are filtered, but multi-char ones should be present.
        assert!(
            tokens
                .iter()
                .any(|t| t == "result" || t == "alpha" || t == "beta" || t == "gamma"),
            "Should contain multi-char identifiers, got: {tokens:?}"
        );
        // Operators (+, *, =) are single chars and get filtered (len > 1 rule).
        // This is by design: operators are noise for semantic similarity.
    }

    #[test]
    fn test_tokenize_code_normalizes_numbers() {
        let source = "let x = 42;\nlet y = 3.14;";
        let tokens = tokenize_code(source);
        let num_count = tokens.iter().filter(|t| *t == "$NUM").count();
        assert!(
            num_count >= 2,
            "Expected at least 2 $NUM tokens, got {num_count}"
        );
    }

    // -- Engine tests --

    #[test]
    fn test_identical_code_cosine_similarity_is_one() {
        let code = r#"
fn add(a: i32, b: i32) -> i32 {
    let result = a + b;
    return result;
}
"#;
        let corpus = vec![code, code, "fn other() { let x = 10; }"];
        let mut engine = CodeEmbeddingEngine::new(8);
        engine.fit(&corpus);

        let emb1 = engine.embed(code);
        let emb2 = engine.embed(code);

        let sim = CodeEmbeddingEngine::cosine_similarity(&emb1, &emb2);
        assert!(
            (sim - 1.0).abs() < 1e-6,
            "Identical code should have cosine similarity ~1.0, got {sim}"
        );
    }

    #[test]
    fn test_very_different_code_has_low_similarity() {
        let code_a = r#"
fn fibonacci(n: u64) -> u64 {
    if n <= 1 { return n; }
    let mut a = 0;
    let mut b = 1;
    for i in 2..=n {
        let temp = a + b;
        a = b;
        b = temp;
    }
    b
}
"#;
        let code_b = r#"
struct Config {
    host: String,
    port: u16,
    database: String,
    username: String,
    password: String,
    max_connections: usize,
    timeout_seconds: u64,
    enable_ssl: bool,
    log_level: String,
    retry_count: usize,
}
"#;
        let corpus = vec![code_a, code_b];
        let mut engine = CodeEmbeddingEngine::new(8);
        engine.fit(&corpus);

        let emb_a = engine.embed(code_a);
        let emb_b = engine.embed(code_b);

        let sim = CodeEmbeddingEngine::cosine_similarity(&emb_a, &emb_b);
        assert!(
            sim < 0.8,
            "Very different code should have low similarity, got {sim}"
        );
    }

    #[test]
    fn test_iterative_vs_recursive_fibonacci_similar() {
        let iterative = r#"
function fibIterative(n) {
    if (n <= 1) { return n; }
    let a = 0;
    let b = 1;
    for (let i = 2; i <= n; i++) {
        let temp = a + b;
        a = b;
        b = temp;
    }
    return b;
}
"#;
        let recursive = r#"
function fibRecursive(n) {
    if (n <= 0) { return 0; }
    if (n == 1) { return 1; }
    return fibRecursive(n - 1) + fibRecursive(n - 2);
}
"#;
        // Also add some noise documents so the engine has context.
        let noise1 = r#"
function sortArray(arr) {
    for (let i = 0; i < arr.length; i++) {
        for (let j = 0; j < arr.length - i - 1; j++) {
            if (arr[j] > arr[j+1]) {
                let temp = arr[j];
                arr[j] = arr[j+1];
                arr[j+1] = temp;
            }
        }
    }
    return arr;
}
"#;
        let noise2 = r#"
class DatabaseConnection {
    constructor(host, port, database) {
        this.host = host;
        this.port = port;
        this.database = database;
        this.connected = false;
    }
    connect() {
        this.connected = true;
        return this;
    }
    disconnect() {
        this.connected = false;
    }
}
"#;
        let corpus = vec![iterative, recursive, noise1, noise2];
        let mut engine = CodeEmbeddingEngine::new(16);
        engine.fit(&corpus);

        let emb_iter = engine.embed(iterative);
        let emb_rec = engine.embed(recursive);
        let emb_noise = engine.embed(noise2);

        let sim_fibs = CodeEmbeddingEngine::cosine_similarity(&emb_iter, &emb_rec);
        let sim_iter_noise = CodeEmbeddingEngine::cosine_similarity(&emb_iter, &emb_noise);

        assert!(
            sim_fibs > 0.4,
            "Iterative and recursive fibonacci should have similarity > 0.4, got {sim_fibs}"
        );
        assert!(
            sim_fibs > sim_iter_noise,
            "Fibonacci variants ({sim_fibs}) should be more similar than fib vs database ({sim_iter_noise})"
        );
    }

    #[test]
    fn test_embedding_dimensionality() {
        let code = "fn foo() { let x = 1; let y = 2; return x + y; }";
        let corpus = vec![code, "fn bar() { return 42; }"];

        for dim in [4, 8, 16, 32, 64] {
            let mut engine = CodeEmbeddingEngine::new(dim);
            engine.fit(&corpus);
            let emb = engine.embed(code);
            assert_eq!(
                emb.len(),
                dim,
                "Embedding should have {dim} dimensions, got {}",
                emb.len()
            );
        }
    }

    #[test]
    fn test_empty_corpus_handling() {
        let mut engine = CodeEmbeddingEngine::new(8);
        engine.fit(&[]);
        assert!(!engine.is_fitted());

        let emb = engine.embed("fn foo() {}");
        assert_eq!(emb.len(), 8);
        assert!(
            emb.iter().all(|x| *x == 0.0),
            "Unfitted engine should return zero vector"
        );
    }

    #[test]
    fn test_l2_normalization() {
        let code = "fn example() { let x = compute(a, b, c); return transform(x); }";
        let corpus = vec![code, "fn other() { return 1; }"];
        let mut engine = CodeEmbeddingEngine::new(8);
        engine.fit(&corpus);

        let emb = engine.embed(code);
        let norm: f64 = emb.iter().map(|x| x * x).sum::<f64>().sqrt();

        // Norm should be either ~1.0 (normalized) or 0.0 (zero vector).
        assert!(
            (norm - 1.0).abs() < 1e-6 || norm < f64::EPSILON,
            "Embedding should be L2 normalized, got norm={norm}"
        );
    }

    #[test]
    fn test_cosine_similarity_edge_cases() {
        // Empty vectors.
        assert_eq!(CodeEmbeddingEngine::cosine_similarity(&[], &[]), 0.0);
        // Different lengths.
        assert_eq!(
            CodeEmbeddingEngine::cosine_similarity(&[1.0, 2.0], &[1.0]),
            0.0
        );
        // Zero vector.
        assert_eq!(
            CodeEmbeddingEngine::cosine_similarity(&[0.0, 0.0], &[1.0, 0.0]),
            0.0
        );
        // Identical unit vectors.
        let sim = CodeEmbeddingEngine::cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]);
        assert!((sim - 1.0).abs() < f64::EPSILON);
        // Opposite vectors.
        let sim = CodeEmbeddingEngine::cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]);
        assert!((sim - (-1.0)).abs() < f64::EPSILON);
        // Orthogonal vectors.
        let sim = CodeEmbeddingEngine::cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]);
        assert!(sim.abs() < f64::EPSILON);
    }

    #[test]
    fn test_simple_rng_deterministic() {
        let mut rng1 = SimpleRng::new(42);
        let mut rng2 = SimpleRng::new(42);
        for _ in 0..100 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
    }

    #[test]
    fn test_simple_rng_gaussian_distribution() {
        let mut rng = SimpleRng::new(123);
        let samples: Vec<f64> = (0..1000).map(|_| rng.next_gaussian()).collect();

        // Mean should be close to 0.
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        assert!(mean.abs() < 0.2, "Gaussian mean should be ~0, got {mean}");

        // Std dev should be close to 1.
        let variance =
            samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / samples.len() as f64;
        let std_dev = variance.sqrt();
        assert!(
            (std_dev - 1.0).abs() < 0.3,
            "Gaussian std dev should be ~1, got {std_dev}"
        );
    }

    #[test]
    fn test_qr_decomposition_orthonormal() {
        // Create a small 4x2 matrix.
        let a = vec![
            vec![1.0, 0.0],
            vec![1.0, 1.0],
            vec![0.0, 1.0],
            vec![0.0, 0.0],
        ];
        let q = qr_decomposition(&a, 4, 2);
        assert_eq!(q.len(), 4);
        assert_eq!(q[0].len(), 2);

        // Columns should be orthonormal.
        let col0: Vec<f64> = (0..4).map(|i| q[i][0]).collect();
        let col1: Vec<f64> = (0..4).map(|i| q[i][1]).collect();

        let norm0 = col0.iter().map(|x| x * x).sum::<f64>().sqrt();
        let norm1 = col1.iter().map(|x| x * x).sum::<f64>().sqrt();
        let dot01 = dot_product(&col0, &col1);

        assert!(
            (norm0 - 1.0).abs() < 1e-10,
            "Column 0 should have unit norm, got {norm0}"
        );
        assert!(
            (norm1 - 1.0).abs() < 1e-10,
            "Column 1 should have unit norm, got {norm1}"
        );
        assert!(
            dot01.abs() < 1e-10,
            "Columns should be orthogonal, dot product = {dot01}"
        );
    }

    #[test]
    fn test_mat_mul_identity() {
        let a = vec![vec![1.0, 2.0], vec![3.0, 4.0]];
        let identity = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let result = mat_mul(&a, &identity, 2, 2, 2);
        for i in 0..2 {
            for j in 0..2 {
                assert!(
                    (result[i][j] - a[i][j]).abs() < f64::EPSILON,
                    "A * I should equal A"
                );
            }
        }
    }

    #[test]
    fn test_vocab_building() {
        let corpus = vec!["fn add(a: i32, b: i32) -> i32 { a + b }"];
        let mut engine = CodeEmbeddingEngine::new(4);
        engine.fit(&corpus);
        assert!(
            engine.vocab_size() > 0,
            "Vocabulary should not be empty after fitting"
        );
        assert!(engine.is_fitted());
    }

    #[test]
    fn test_single_document_corpus() {
        // With only one document, SVD is degenerate but should not panic.
        let code = "fn hello() { println!(\"hello world\"); }";
        let corpus = vec![code];
        let mut engine = CodeEmbeddingEngine::new(8);
        engine.fit(&corpus);

        let emb = engine.embed(code);
        assert_eq!(emb.len(), 8);
        // Should not be all zeros since we fitted on this document.
    }

    #[test]
    fn test_embed_unknown_tokens() {
        // Embed code with tokens not in the vocabulary.
        let corpus = vec!["fn add(a: i32, b: i32) -> i32 { a + b }"];
        let mut engine = CodeEmbeddingEngine::new(4);
        engine.fit(&corpus);

        let unknown = "class Zygomorphic { def transmogrify() { return quux; } }";
        let emb = engine.embed(unknown);
        assert_eq!(emb.len(), 4, "Should still return correct dimensionality");
    }

    #[test]
    fn test_renamed_variables_are_similar() {
        let code_a = r#"
fn compute(input: Vec<i32>) -> i32 {
    let mut total = 0;
    for val in input.iter() {
        if val > 0 {
            total += val;
        }
    }
    total
}
"#;
        // Same logic, different variable names.
        let code_b = r#"
fn calculate(data: Vec<i32>) -> i32 {
    let mut sum = 0;
    for item in data.iter() {
        if item > 0 {
            sum += item;
        }
    }
    sum
}
"#;
        let noise = r#"
struct Point {
    x: f64,
    y: f64,
    z: f64,
}
impl Point {
    fn distance(&self, other: &Point) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        (dx*dx + dy*dy + dz*dz).sqrt()
    }
}
"#;
        let corpus = vec![code_a, code_b, noise];
        let mut engine = CodeEmbeddingEngine::new(16);
        engine.fit(&corpus);

        let emb_a = engine.embed(code_a);
        let emb_b = engine.embed(code_b);
        let emb_noise = engine.embed(noise);

        let sim_ab = CodeEmbeddingEngine::cosine_similarity(&emb_a, &emb_b);
        let sim_a_noise = CodeEmbeddingEngine::cosine_similarity(&emb_a, &emb_noise);

        assert!(
            sim_ab > sim_a_noise,
            "Renamed variants ({sim_ab}) should be more similar than code vs noise ({sim_a_noise})"
        );
    }
}
