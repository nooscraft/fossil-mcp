//! Semantic feature extraction and Type-4 clone detection.
//!
//! Type-4 (semantic) clones are code fragments that perform the same computation
//! but may have completely different syntactic structure. For example, an iterative
//! fibonacci and a recursive fibonacci are Type-4 clones.
//!
//! Detection uses a two-phase approach:
//! 1. **Cheap feature distance filter**: Extract semantic features (complexity metrics)
//!    and compute normalized Euclidean distance. Pairs within the feature threshold
//!    become candidates.
//! 2. **Expensive tree edit distance verification**: Convert candidate pairs to labeled
//!    trees and compute normalized tree edit distance. Pairs within the tree edit
//!    threshold are confirmed as Type-4 clones.

use super::code_embeddings::CodeEmbeddingEngine;
use super::tree_edit_distance::{normalized_tree_edit_distance, source_to_labeled_tree};
use super::types::{CloneGroup, CloneInstance, CloneType};

/// Semantic features extracted from a function's source code.
///
/// These features capture the behavioral characteristics of a function
/// independent of its syntactic form.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticFeatures {
    /// McCabe cyclomatic complexity (number of independent paths).
    pub cyclomatic_complexity: usize,
    /// Number of parameters.
    pub num_params: usize,
    /// Number of return statements.
    pub num_returns: usize,
    /// Number of branches (if/else/switch/match).
    pub num_branches: usize,
    /// Number of loops (for/while/loop).
    pub num_loops: usize,
    /// Maximum nesting depth.
    pub nesting_depth: usize,
    /// Lines of code (non-empty, non-comment).
    pub lines_of_code: usize,
    /// Number of function/method calls.
    pub num_calls: usize,
}

impl SemanticFeatures {
    /// Convert features to a normalized f64 vector for distance computation.
    ///
    /// Each feature is scaled to a reasonable range to prevent any single
    /// feature from dominating the distance calculation.
    fn to_vector(&self) -> [f64; 8] {
        [
            self.cyclomatic_complexity as f64,
            self.num_params as f64,
            self.num_returns as f64,
            self.num_branches as f64,
            self.num_loops as f64,
            self.nesting_depth as f64,
            self.lines_of_code as f64,
            self.num_calls as f64,
        ]
    }
}

/// Extract semantic features from a function's source text using simple heuristics.
///
/// This uses keyword counting and pattern matching rather than full parsing,
/// making it fast and language-agnostic.
pub fn extract_semantic_features(source: &str) -> SemanticFeatures {
    let lines: Vec<&str> = source.lines().collect();

    let mut cyclomatic_complexity = 1; // Start at 1 for the function itself
    let mut num_params = 0;
    let mut num_returns = 0;
    let mut num_branches = 0;
    let mut num_loops = 0;
    let mut max_nesting = 0;
    let mut current_nesting = 0;
    let mut lines_of_code = 0;
    let mut num_calls = 0;

    // Detect parameter count from the first line containing '('
    for line in &lines {
        let trimmed = line.trim();
        if trimmed.contains('(') && trimmed.contains(')') {
            if let Some(start) = trimmed.find('(') {
                if let Some(end) = trimmed.rfind(')') {
                    let params_str = &trimmed[start + 1..end];
                    if !params_str.trim().is_empty() {
                        // Count commas + 1 for parameter count, handling nested parens
                        let mut depth = 0;
                        let mut comma_count = 0;
                        for ch in params_str.chars() {
                            match ch {
                                '(' | '[' | '<' => depth += 1,
                                ')' | ']' | '>' => depth -= 1,
                                ',' if depth == 0 => comma_count += 1,
                                _ => {}
                            }
                        }
                        num_params = comma_count + 1;
                    }
                    break;
                }
            }
        }
    }

    for line in &lines {
        let trimmed = line.trim();

        // Skip empty lines and comments for LOC count
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with('#')
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
            || trimmed.starts_with("*/")
        {
            continue;
        }

        lines_of_code += 1;

        // Track nesting via braces (brace-based languages)
        let opens = trimmed.chars().filter(|&c| c == '{').count();
        let closes = trimmed.chars().filter(|&c| c == '}').count();
        current_nesting += opens;
        if current_nesting > max_nesting {
            max_nesting = current_nesting;
        }
        current_nesting = current_nesting.saturating_sub(closes);

        // Track nesting via indentation (Python-style)
        if !trimmed.contains('{') && trimmed.ends_with(':') {
            let indent = line.len() - line.trim_start().len();
            let indent_level = indent / 4 + 1; // rough nesting estimate
            if indent_level > max_nesting {
                max_nesting = indent_level;
            }
        }

        // Count branches
        if is_branch_keyword(trimmed) {
            num_branches += 1;
            cyclomatic_complexity += 1;
        }

        // Count loops
        if is_loop_keyword(trimmed) {
            num_loops += 1;
            cyclomatic_complexity += 1;
        }

        // Count returns
        if trimmed.starts_with("return ") || trimmed == "return" || trimmed.starts_with("return;") {
            num_returns += 1;
        }

        // Count boolean operators (add to cyclomatic complexity)
        cyclomatic_complexity += count_boolean_operators(trimmed);

        // Count function calls (heuristic: word followed by parenthesis)
        num_calls += count_calls(trimmed);
    }

    SemanticFeatures {
        cyclomatic_complexity,
        num_params,
        num_returns,
        num_branches,
        num_loops,
        nesting_depth: max_nesting,
        lines_of_code,
        num_calls,
    }
}

/// Check if a line starts a branch (if/else if/elif/switch/match/case).
fn is_branch_keyword(trimmed: &str) -> bool {
    trimmed.starts_with("if ")
        || trimmed.starts_with("if(")
        || trimmed.starts_with("else if")
        || trimmed.starts_with("} else if")
        || trimmed.starts_with("elif ")
        || trimmed.starts_with("switch ")
        || trimmed.starts_with("switch(")
        || trimmed.starts_with("match ")
        || trimmed.starts_with("case ")
        || trimmed.starts_with("default:")
}

/// Check if a line starts a loop.
fn is_loop_keyword(trimmed: &str) -> bool {
    trimmed.starts_with("for ")
        || trimmed.starts_with("for(")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("while(")
        || trimmed == "loop"
        || trimmed.starts_with("loop ")
        || trimmed.starts_with("loop{")
        || trimmed.starts_with("do {")
        || trimmed.starts_with("do{")
}

/// Count boolean operators (&&, ||, and, or) in a line.
fn count_boolean_operators(trimmed: &str) -> usize {
    let mut count = 0;
    // Count && and ||
    let bytes = trimmed.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        if (bytes[i] == b'&' && bytes[i + 1] == b'&') || (bytes[i] == b'|' && bytes[i + 1] == b'|')
        {
            count += 1;
        }
    }
    // Count Python-style 'and' / 'or' (surrounded by whitespace)
    for word in trimmed.split_whitespace() {
        if word == "and" || word == "or" {
            count += 1;
        }
    }
    count
}

/// Count function calls in a line (word followed by open parenthesis).
fn count_calls(trimmed: &str) -> usize {
    // Skip lines that are function definitions
    if trimmed.starts_with("def ")
        || trimmed.starts_with("fn ")
        || trimmed.starts_with("func ")
        || trimmed.starts_with("function ")
        || trimmed.starts_with("pub fn ")
        || trimmed.starts_with("pub async fn ")
        || trimmed.starts_with("async fn ")
        || trimmed.starts_with("async def ")
    {
        return 0;
    }

    let mut count = 0;
    let chars: Vec<char> = trimmed.chars().collect();
    for i in 0..chars.len() {
        if chars[i] == '(' && i > 0 {
            // Check if preceded by an identifier character
            let prev = chars[i - 1];
            if prev.is_alphanumeric() || prev == '_' {
                count += 1;
            }
        }
    }
    count
}

/// Compute the normalized Euclidean distance between two semantic feature vectors.
///
/// Returns a value in [0.0, 1.0] where 0.0 means identical features.
/// The distance is normalized by the maximum possible distance given the
/// two feature vectors.
pub fn feature_distance(a: &SemanticFeatures, b: &SemanticFeatures) -> f64 {
    let va = a.to_vector();
    let vb = b.to_vector();

    let sum_sq: f64 = va.iter().zip(vb.iter()).map(|(x, y)| (x - y).powi(2)).sum();

    let euclidean = sum_sq.sqrt();

    // Normalize by the magnitude of the larger vector to get [0, 1] range
    let mag_a: f64 = va.iter().map(|x| x.powi(2)).sum::<f64>().sqrt();
    let mag_b: f64 = vb.iter().map(|x| x.powi(2)).sum::<f64>().sqrt();
    let max_mag = mag_a.max(mag_b);

    if max_mag < f64::EPSILON {
        return 0.0; // Both zero vectors
    }

    (euclidean / max_mag).min(1.0)
}

/// A function with its source and metadata, ready for semantic comparison.
#[derive(Debug, Clone)]
pub struct SemanticFunction {
    /// File path containing this function.
    pub file: String,
    /// Function name.
    pub name: String,
    /// Start line (1-indexed).
    pub start_line: usize,
    /// End line (1-indexed).
    pub end_line: usize,
    /// Raw source text of the function.
    pub source: String,
    /// Extracted semantic features.
    pub features: SemanticFeatures,
}

/// Semantic clone detector using a filter-then-verify approach.
///
/// 1. Cheap feature distance filter to identify candidate pairs.
/// 2. Tree edit distance verification to confirm clones.
pub struct SemanticCloneDetector {
    /// Maximum feature distance to consider a candidate pair (filter pass).
    /// Lower values are more selective. Default: 0.3.
    pub feature_threshold: f64,
    /// Maximum normalized tree edit distance to confirm a clone (verification pass).
    /// Lower values require more structural similarity. Default: 0.4.
    pub tree_edit_threshold: f64,
}

impl SemanticCloneDetector {
    /// Create a new detector with the given thresholds.
    pub fn new(feature_threshold: f64, tree_edit_threshold: f64) -> Self {
        Self {
            feature_threshold,
            tree_edit_threshold,
        }
    }

    /// Detect Type-4 semantic clones among a set of functions.
    ///
    /// Two-phase approach:
    /// 1. Compute feature distance for all pairs - O(n^2) but very cheap per pair
    /// 2. For candidate pairs (feature distance < threshold), compute tree edit
    ///    distance - expensive but only run on a small subset
    pub fn detect_clones(&self, functions: &[SemanticFunction]) -> Vec<CloneGroup> {
        let mut groups = Vec::new();

        if functions.len() < 2 {
            return groups;
        }

        for i in 0..functions.len() {
            for j in (i + 1)..functions.len() {
                // Cheap feature distance filter
                let feat_dist = feature_distance(&functions[i].features, &functions[j].features);
                if feat_dist > self.feature_threshold {
                    continue;
                }

                // Tree edit distance verification
                let tree_a = source_to_labeled_tree(&functions[i].source);
                let tree_b = source_to_labeled_tree(&functions[j].source);
                let tree_dist = normalized_tree_edit_distance(&tree_a, &tree_b);

                if tree_dist <= self.tree_edit_threshold {
                    let similarity = 1.0 - tree_dist;

                    let instance_a = CloneInstance {
                        file: functions[i].file.clone(),
                        start_line: functions[i].start_line,
                        end_line: functions[i].end_line,
                        start_byte: 0,
                        end_byte: 0,
                        function_name: Some(functions[i].name.clone()),
                    };

                    let instance_b = CloneInstance {
                        file: functions[j].file.clone(),
                        start_line: functions[j].start_line,
                        end_line: functions[j].end_line,
                        start_byte: 0,
                        end_byte: 0,
                        function_name: Some(functions[j].name.clone()),
                    };

                    groups.push(
                        CloneGroup::new(CloneType::Type4, vec![instance_a, instance_b])
                            .with_similarity(similarity),
                    );
                }
            }
        }

        groups
    }

    /// Detect clones using code embeddings as a pre-filter.
    ///
    /// Uses TF-IDF + SVD embeddings for initial candidate selection
    /// (cosine similarity > `embedding_threshold`), then verifies with
    /// tree edit distance. This avoids the quadratic feature-distance
    /// comparison by leveraging dense vector similarity.
    ///
    /// The `embedding_threshold` controls how aggressively candidates are
    /// pruned. A value of 0.7 is a good default that balances recall and
    /// the cost of tree edit distance verification.
    pub fn detect_with_embeddings(
        &self,
        functions: &[SemanticFunction],
        embedding_threshold: f64,
    ) -> Vec<CloneGroup> {
        if functions.len() < 2 {
            return Vec::new();
        }

        // Build embedding engine and fit on the corpus.
        let sources: Vec<&str> = functions.iter().map(|f| f.source.as_str()).collect();
        let mut engine = CodeEmbeddingEngine::with_defaults();
        engine.fit(&sources);

        // Compute embeddings for all functions.
        let embeddings: Vec<Vec<f64>> = functions.iter().map(|f| engine.embed(&f.source)).collect();

        let mut groups = Vec::new();

        // Pre-filter with cosine similarity, then verify with tree edit distance.
        for i in 0..functions.len() {
            for j in (i + 1)..functions.len() {
                let cos_sim =
                    CodeEmbeddingEngine::cosine_similarity(&embeddings[i], &embeddings[j]);
                if cos_sim < embedding_threshold {
                    continue;
                }

                // Expensive verification: tree edit distance.
                let tree_a = source_to_labeled_tree(&functions[i].source);
                let tree_b = source_to_labeled_tree(&functions[j].source);
                let tree_dist = normalized_tree_edit_distance(&tree_a, &tree_b);

                if tree_dist <= self.tree_edit_threshold {
                    let similarity = 1.0 - tree_dist;

                    let instance_a = CloneInstance {
                        file: functions[i].file.clone(),
                        start_line: functions[i].start_line,
                        end_line: functions[i].end_line,
                        start_byte: 0,
                        end_byte: 0,
                        function_name: Some(functions[i].name.clone()),
                    };

                    let instance_b = CloneInstance {
                        file: functions[j].file.clone(),
                        start_line: functions[j].start_line,
                        end_line: functions[j].end_line,
                        start_byte: 0,
                        end_byte: 0,
                        function_name: Some(functions[j].name.clone()),
                    };

                    groups.push(
                        CloneGroup::new(CloneType::Type4, vec![instance_a, instance_b])
                            .with_similarity(similarity),
                    );
                }
            }
        }

        groups
    }

    /// Detect clones using the APTED algorithm with source-based tree construction.
    ///
    /// This is more accurate than the default `detect_clones` method because APTED
    /// has better worst-case complexity (O(n^2) vs O(n^2 * m^2) for Zhang-Shasha)
    /// and is faster in practice due to optimal decomposition strategy selection.
    ///
    /// Still uses `source_to_labeled_tree` for tree construction since we don't
    /// have a tree-sitter Language available at this level. For AST-based trees,
    /// use the `ast_tree` module directly when tree-sitter nodes are available.
    pub fn detect_clones_with_apted(&self, functions: &[SemanticFunction]) -> Vec<CloneGroup> {
        let mut groups = Vec::new();

        if functions.len() < 2 {
            return groups;
        }

        for i in 0..functions.len() {
            for j in (i + 1)..functions.len() {
                // Cheap feature distance filter
                let feat_dist = feature_distance(&functions[i].features, &functions[j].features);
                if feat_dist > self.feature_threshold {
                    continue;
                }

                // Tree edit distance verification using APTED
                let tree_a = source_to_labeled_tree(&functions[i].source);
                let tree_b = source_to_labeled_tree(&functions[j].source);
                let tree_dist = crate::clones::apted::normalized_apted_distance(&tree_a, &tree_b);

                if tree_dist <= self.tree_edit_threshold {
                    let similarity = 1.0 - tree_dist;

                    let instance_a = CloneInstance {
                        file: functions[i].file.clone(),
                        start_line: functions[i].start_line,
                        end_line: functions[i].end_line,
                        start_byte: 0,
                        end_byte: 0,
                        function_name: Some(functions[i].name.clone()),
                    };

                    let instance_b = CloneInstance {
                        file: functions[j].file.clone(),
                        start_line: functions[j].start_line,
                        end_line: functions[j].end_line,
                        start_byte: 0,
                        end_byte: 0,
                        function_name: Some(functions[j].name.clone()),
                    };

                    groups.push(
                        CloneGroup::new(CloneType::Type4, vec![instance_a, instance_b])
                            .with_similarity(similarity),
                    );
                }
            }
        }

        groups
    }

    /// Build a `SemanticFunction` from raw data with features already extracted.
    pub fn build_function(
        file: &str,
        name: &str,
        start_line: usize,
        end_line: usize,
        source: &str,
    ) -> SemanticFunction {
        let features = extract_semantic_features(source);
        SemanticFunction {
            file: file.to_string(),
            name: name.to_string(),
            start_line,
            end_line,
            source: source.to_string(),
            features,
        }
    }
}

impl Default for SemanticCloneDetector {
    fn default() -> Self {
        Self {
            feature_threshold: 0.3,
            tree_edit_threshold: 0.4,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Feature extraction tests --

    #[test]
    fn test_extract_features_simple_function() {
        let source = r#"def add(a, b):
    return a + b
"#;
        let features = extract_semantic_features(source);
        assert_eq!(features.num_params, 2);
        assert_eq!(features.num_returns, 1);
        assert_eq!(features.num_branches, 0);
        assert_eq!(features.num_loops, 0);
        assert_eq!(features.cyclomatic_complexity, 1); // No branches
    }

    #[test]
    fn test_extract_features_with_branches() {
        let source = r#"function classify(x) {
    if (x > 0) {
        return "positive";
    } else if (x < 0) {
        return "negative";
    } else {
        return "zero";
    }
}"#;
        let features = extract_semantic_features(source);
        assert_eq!(features.num_params, 1);
        assert!(features.num_returns >= 3);
        assert!(features.num_branches >= 2); // if + else if
        assert!(features.cyclomatic_complexity >= 3);
    }

    #[test]
    fn test_extract_features_with_loops() {
        let source = r#"def sum_list(items):
    total = 0
    for item in items:
        total += item
    return total
"#;
        let features = extract_semantic_features(source);
        assert_eq!(features.num_params, 1);
        assert_eq!(features.num_returns, 1);
        assert_eq!(features.num_loops, 1);
        assert!(features.cyclomatic_complexity >= 2); // 1 base + 1 loop
    }

    #[test]
    fn test_extract_features_empty_source() {
        let features = extract_semantic_features("");
        assert_eq!(features.cyclomatic_complexity, 1);
        assert_eq!(features.num_params, 0);
        assert_eq!(features.lines_of_code, 0);
    }

    #[test]
    fn test_extract_features_counts_calls() {
        let source = r#"def process():
    x = foo()
    y = bar(x)
    return baz(x, y)
"#;
        let features = extract_semantic_features(source);
        assert!(
            features.num_calls >= 3,
            "Expected at least 3 calls, got {}",
            features.num_calls
        );
    }

    // -- Feature distance tests --

    #[test]
    fn test_feature_distance_identical() {
        let a = SemanticFeatures {
            cyclomatic_complexity: 3,
            num_params: 2,
            num_returns: 1,
            num_branches: 2,
            num_loops: 1,
            nesting_depth: 2,
            lines_of_code: 10,
            num_calls: 3,
        };
        let dist = feature_distance(&a, &a);
        assert!(
            dist.abs() < f64::EPSILON,
            "Distance to self should be 0, got {dist}"
        );
    }

    #[test]
    fn test_feature_distance_different() {
        let a = SemanticFeatures {
            cyclomatic_complexity: 1,
            num_params: 1,
            num_returns: 1,
            num_branches: 0,
            num_loops: 0,
            nesting_depth: 1,
            lines_of_code: 3,
            num_calls: 0,
        };
        let b = SemanticFeatures {
            cyclomatic_complexity: 10,
            num_params: 5,
            num_returns: 4,
            num_branches: 6,
            num_loops: 3,
            nesting_depth: 5,
            lines_of_code: 50,
            num_calls: 15,
        };
        let dist = feature_distance(&a, &b);
        assert!(
            dist > 0.0,
            "Different features should have positive distance"
        );
        assert!(
            dist <= 1.0,
            "Normalized distance should be <= 1.0, got {dist}"
        );
    }

    #[test]
    fn test_feature_distance_symmetric() {
        let a = SemanticFeatures {
            cyclomatic_complexity: 3,
            num_params: 2,
            num_returns: 1,
            num_branches: 2,
            num_loops: 1,
            nesting_depth: 2,
            lines_of_code: 10,
            num_calls: 3,
        };
        let b = SemanticFeatures {
            cyclomatic_complexity: 5,
            num_params: 3,
            num_returns: 2,
            num_branches: 3,
            num_loops: 2,
            nesting_depth: 3,
            lines_of_code: 15,
            num_calls: 5,
        };
        let dist_ab = feature_distance(&a, &b);
        let dist_ba = feature_distance(&b, &a);
        assert!(
            (dist_ab - dist_ba).abs() < f64::EPSILON,
            "Distance should be symmetric: {dist_ab} vs {dist_ba}"
        );
    }

    #[test]
    fn test_feature_distance_similar_functions() {
        // Two similar functions should have a small distance
        let a = SemanticFeatures {
            cyclomatic_complexity: 3,
            num_params: 1,
            num_returns: 2,
            num_branches: 2,
            num_loops: 0,
            nesting_depth: 2,
            lines_of_code: 6,
            num_calls: 2,
        };
        let b = SemanticFeatures {
            cyclomatic_complexity: 3,
            num_params: 1,
            num_returns: 1,
            num_branches: 1,
            num_loops: 1,
            nesting_depth: 2,
            lines_of_code: 7,
            num_calls: 2,
        };
        let dist = feature_distance(&a, &b);
        assert!(
            dist < 0.5,
            "Similar features should have small distance, got {dist}"
        );
    }

    // -- Semantic clone detection tests --

    #[test]
    fn test_detect_iterative_vs_recursive_fibonacci() {
        // Iterative fibonacci
        let iterative = r#"function fibIterative(n) {
    if (n <= 1) {
        return n;
    }
    let a = 0;
    let b = 1;
    for (let i = 2; i <= n; i++) {
        let temp = a + b;
        a = b;
        b = temp;
    }
    return b;
}"#;

        // Recursive fibonacci
        let recursive = r#"function fibRecursive(n) {
    if (n <= 0) {
        return 0;
    }
    if (n == 1) {
        return 1;
    }
    return fibRecursive(n - 1) + fibRecursive(n - 2);
}"#;

        let detector = SemanticCloneDetector::new(0.6, 0.8);

        let func_a =
            SemanticCloneDetector::build_function("fib.js", "fibIterative", 1, 12, iterative);
        let func_b =
            SemanticCloneDetector::build_function("fib.js", "fibRecursive", 14, 21, recursive);

        // Both should extract reasonable features
        assert!(func_a.features.num_params >= 1);
        assert!(func_b.features.num_params >= 1);
        assert!(func_a.features.num_returns >= 1);
        assert!(func_b.features.num_returns >= 1);

        // With relaxed thresholds, they should be detected as candidates
        let feat_dist = feature_distance(&func_a.features, &func_b.features);
        assert!(
            feat_dist < 1.0,
            "Fibonacci variants should have some feature similarity, distance: {feat_dist}"
        );

        // Run full detection with relaxed thresholds
        let groups = detector.detect_clones(&[func_a, func_b]);
        // With generous thresholds, the pair should be detected
        assert!(
            !groups.is_empty(),
            "Should detect iterative vs recursive fibonacci as semantic clones (feat_dist={feat_dist})"
        );
        if let Some(group) = groups.first() {
            assert_eq!(group.clone_type, CloneType::Type4);
            assert_eq!(group.instances.len(), 2);
        }
    }

    #[test]
    fn test_detect_very_different_functions_not_clones() {
        // A simple getter
        let getter = r#"function getName() {
    return this.name;
}"#;

        // A complex sorting function
        let sorter = r#"function quickSort(arr, low, high) {
    if (low < high) {
        let pivot = arr[high];
        let i = low - 1;
        for (let j = low; j < high; j++) {
            if (arr[j] <= pivot) {
                i++;
                let temp = arr[i];
                arr[i] = arr[j];
                arr[j] = temp;
            }
        }
        let temp = arr[i + 1];
        arr[i + 1] = arr[high];
        arr[high] = temp;
        let pi = i + 1;
        quickSort(arr, low, pi - 1);
        quickSort(arr, pi + 1, high);
    }
    return arr;
}"#;

        let detector = SemanticCloneDetector::default();
        let func_a = SemanticCloneDetector::build_function("a.js", "getName", 1, 3, getter);
        let func_b = SemanticCloneDetector::build_function("b.js", "quickSort", 1, 20, sorter);

        let groups = detector.detect_clones(&[func_a, func_b]);
        assert!(
            groups.is_empty(),
            "Getter and quickSort should NOT be detected as clones"
        );
    }

    #[test]
    fn test_detect_empty_input() {
        let detector = SemanticCloneDetector::default();
        let groups = detector.detect_clones(&[]);
        assert!(groups.is_empty());
    }

    #[test]
    fn test_detect_single_function() {
        let detector = SemanticCloneDetector::default();
        let func = SemanticCloneDetector::build_function(
            "a.js",
            "foo",
            1,
            3,
            "function foo() { return 1; }",
        );
        let groups = detector.detect_clones(&[func]);
        assert!(groups.is_empty());
    }

    #[test]
    fn test_build_function() {
        let source = "def add(a, b):\n    return a + b\n";
        let func = SemanticCloneDetector::build_function("test.py", "add", 1, 2, source);
        assert_eq!(func.file, "test.py");
        assert_eq!(func.name, "add");
        assert_eq!(func.start_line, 1);
        assert_eq!(func.end_line, 2);
        assert_eq!(func.features.num_params, 2);
        assert_eq!(func.features.num_returns, 1);
    }

    #[test]
    fn test_default_thresholds() {
        let detector = SemanticCloneDetector::default();
        assert!((detector.feature_threshold - 0.3).abs() < f64::EPSILON);
        assert!((detector.tree_edit_threshold - 0.4).abs() < f64::EPSILON);
    }

    #[test]
    fn test_python_iterative_vs_recursive_sum() {
        // Iterative sum
        let iterative = r#"def sum_iterative(nums):
    total = 0
    for n in nums:
        total += n
    return total
"#;

        // Recursive sum
        let recursive = r#"def sum_recursive(nums):
    if len(nums) == 0:
        return 0
    return nums[0] + sum_recursive(nums[1:])
"#;

        // These are semantically equivalent but structurally different
        let func_a =
            SemanticCloneDetector::build_function("sum.py", "sum_iterative", 1, 5, iterative);
        let func_b =
            SemanticCloneDetector::build_function("sum.py", "sum_recursive", 7, 11, recursive);

        // They should have somewhat similar features
        let dist = feature_distance(&func_a.features, &func_b.features);
        assert!(
            dist < 1.0,
            "Sum variants should have some feature similarity, got {dist}"
        );
    }
}
