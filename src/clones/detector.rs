//! Unified clone detector combining Merkle hashing, MinHash+LSH, and SimHash.

use std::path::Path;

use super::clustering::{cluster_clone_groups, CloneClass};
use super::cross_language::CrossLanguageDetector;
use super::minhash::{FunctionSignature, MinHashDetector};
use super::simhash::SimHashFingerprinter;
use super::types::CloneGroup;

/// Configuration for clone detection.
#[derive(Debug, Clone)]
pub struct CloneConfig {
    /// Minimum lines for a clone to be reported.
    pub min_lines: usize,
    /// Minimum AST nodes for Merkle detection.
    pub min_nodes: usize,
    /// Similarity threshold for Type 3 (0.0-1.0).
    pub similarity_threshold: f64,
    /// Enable Type 1 (exact) detection.
    pub detect_type1: bool,
    /// Enable Type 2 (renamed) detection.
    pub detect_type2: bool,
    /// Enable Type 3 (near-miss) detection.
    pub detect_type3: bool,
    /// Enable cross-language clone detection.
    pub detect_cross_language: bool,
}

impl Default for CloneConfig {
    fn default() -> Self {
        Self {
            min_lines: 6,
            min_nodes: 5,
            similarity_threshold: 0.8,
            detect_type1: true,
            detect_type2: true,
            detect_type3: true,
            detect_cross_language: true,
        }
    }
}

/// Result of clone detection.
#[derive(Debug)]
pub struct CloneResult {
    pub groups: Vec<CloneGroup>,
    pub files_analyzed: usize,
    pub total_duplicated_lines: usize,
}

/// Unified clone detector.
pub struct CloneDetector {
    config: CloneConfig,
}

impl CloneDetector {
    pub fn new(config: CloneConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self {
            config: CloneConfig::default(),
        }
    }

    /// Detect clones in a list of source files using MinHash (Type 3).
    /// Runs both file-level and function-level detection.
    pub fn detect_in_sources(
        &self,
        files: &[(String, String)], // (path, source)
    ) -> CloneResult {
        let mut all_groups = Vec::new();
        let minhash = MinHashDetector::new(128, 3, self.config.similarity_threshold);

        // Function-level clone detection (primary)
        // Extract individual functions from each file and compare them
        if self.config.detect_type3 {
            let mut func_signatures: Vec<FunctionSignature> = Vec::new();

            for (path, source) in files {
                let functions = extract_functions_from_source(source);
                for (name, start_line, end_line, body) in functions {
                    let line_count = end_line - start_line + 1;
                    // Strictly enforce min_lines: skip any function below the threshold
                    if line_count < self.config.min_lines {
                        continue;
                    }
                    let shingle_hashes = minhash.compute_shingles(&body);
                    if shingle_hashes.is_empty() {
                        continue;
                    }
                    let sketch = minhash.build_sketch(&shingle_hashes);
                    func_signatures.push(FunctionSignature {
                        file: path.clone(),
                        name,
                        start_line,
                        end_line,
                        sketch,
                        shingle_hashes,
                    });
                }
            }

            if func_signatures.len() >= 2 {
                let func_groups = minhash.detect_clones(&func_signatures);
                all_groups.extend(func_groups);
            }
        }

        // File-level clone detection (supplementary)
        // Uses SimHash for fast O(1) pre-screening, then MinHash for verification.
        if self.config.detect_type3 && files.len() >= 2 {
            // SimHash fingerprinting — O(N) to compute, O(1) per comparison
            let simhash_threshold =
                similarity_to_hamming_distance(self.config.similarity_threshold);
            let simhasher = SimHashFingerprinter::new(simhash_threshold);
            let fingerprints = simhasher.fingerprint_files(files);
            let candidates = simhasher.find_candidates(&fingerprints);

            if !candidates.is_empty() {
                // Only compute MinHash for SimHash candidate pairs
                // Build shingles/sketches lazily (only for files in candidate pairs)
                let mut needed: std::collections::HashSet<usize> = std::collections::HashSet::new();
                for &(i, j) in &candidates {
                    needed.insert(i);
                    needed.insert(j);
                }

                let file_sketches: std::collections::HashMap<usize, FunctionSignature> = needed
                    .into_iter()
                    .map(|idx| {
                        let (path, source) = &files[idx];
                        let shingle_hashes = minhash.compute_shingles(source);
                        let sketch = minhash.build_sketch(&shingle_hashes);
                        (
                            idx,
                            FunctionSignature {
                                file: path.clone(),
                                name: format!("[file] {path}"),
                                start_line: 1,
                                end_line: source.lines().count(),
                                sketch,
                                shingle_hashes,
                            },
                        )
                    })
                    .collect();

                // Verify candidates with MinHash Jaccard
                for (i, j) in candidates {
                    if let (Some(sig_a), Some(sig_b)) =
                        (file_sketches.get(&i), file_sketches.get(&j))
                    {
                        // Enforce min_lines for file-level clones
                        let lines_a = sig_a.end_line.saturating_sub(sig_a.start_line) + 1;
                        let lines_b = sig_b.end_line.saturating_sub(sig_b.start_line) + 1;
                        if lines_a < self.config.min_lines || lines_b < self.config.min_lines {
                            continue;
                        }
                        let similarity =
                            MinHashDetector::jaccard_similarity(&sig_a.sketch, &sig_b.sketch);
                        if similarity >= self.config.similarity_threshold {
                            let instance_a = crate::clones::types::CloneInstance {
                                file: sig_a.file.clone(),
                                start_line: sig_a.start_line,
                                end_line: sig_a.end_line,
                                start_byte: 0,
                                end_byte: 0,
                                function_name: Some(sig_a.name.clone()),
                            };
                            let instance_b = crate::clones::types::CloneInstance {
                                file: sig_b.file.clone(),
                                start_line: sig_b.start_line,
                                end_line: sig_b.end_line,
                                start_byte: 0,
                                end_byte: 0,
                                function_name: Some(sig_b.name.clone()),
                            };
                            all_groups.push(
                                CloneGroup::new(
                                    crate::clones::types::CloneType::Type3,
                                    vec![instance_a, instance_b],
                                )
                                .with_similarity(similarity),
                            );
                        }
                    }
                }
            }
        }

        // Cross-language clone detection
        // Uses IR token normalization to find clones across different programming languages.
        // Extracts language-agnostic IR tokens from source text and compares via MinHash.
        if self.config.detect_cross_language && files.len() >= 2 {
            let cross_lang = CrossLanguageDetector::new(self.config.similarity_threshold);
            let mut signatures = Vec::new();

            // Check that we have files from at least two different languages
            let mut languages_seen = std::collections::HashSet::new();
            for (path, _) in files {
                if let Some(lang) = crate::core::Language::from_path(std::path::Path::new(path)) {
                    languages_seen.insert(lang);
                }
            }

            if languages_seen.len() >= 2 {
                for (path, source) in files {
                    let language =
                        match crate::core::Language::from_path(std::path::Path::new(path)) {
                            Some(lang) => lang,
                            None => continue,
                        };

                    // Extract functions and build cross-language signatures using
                    // text-based IR token extraction (no tree-sitter node required).
                    let functions = extract_functions_from_source(source);
                    for (name, start_line, end_line, _body) in &functions {
                        let line_count = end_line - start_line + 1;
                        if line_count < self.config.min_lines {
                            continue;
                        }

                        let ir_tokens = crate::clones::ir_tokenizer::extract_ir_tokens_from_source(
                            source,
                            *start_line,
                            *end_line,
                        );
                        // Require more IR tokens for cross-language detection to avoid
                        // trivial matches between small boilerplate functions.
                        if ir_tokens.len() < 8 {
                            continue;
                        }

                        let shingles =
                            crate::clones::ir_tokenizer::ir_tokens_to_shingles(&ir_tokens, 4);
                        if shingles.is_empty() {
                            continue;
                        }

                        let minhash_det = crate::clones::minhash::MinHashDetector::new(
                            128,
                            3,
                            self.config.similarity_threshold,
                        );
                        let sketch = minhash_det.build_sketch(&shingles);

                        signatures.push(crate::clones::cross_language::CrossLanguageSignature {
                            file: path.clone(),
                            name: name.clone(),
                            language,
                            start_line: *start_line,
                            end_line: *end_line,
                            ir_tokens,
                            sketch,
                        });
                    }
                }
            }

            if signatures.len() >= 2 {
                let cross_groups = cross_lang.detect_clones(&signatures);
                all_groups.extend(cross_groups);
            }
        }

        // Filter trivial micro-clones: groups where ALL instances are ≤5 lines
        // and have single-statement bodies (getters, setters, error returns).
        all_groups.retain(|g| !is_trivial_clone_group(g, files));

        // Filter trait/interface implementation clones: methods that are
        // structurally similar because a trait/interface imposes the pattern,
        // not because of copy-paste. Detected via AST context (e.g. Rust
        // `impl Trait for Type`, Java `@Override`, Python dunder methods).
        all_groups.retain(|g| !is_trait_impl_clone_group(g, files));

        // Filter test-only clones: when ALL instances in a group are in test
        // context, the duplication is intentional. Uses file path heuristics
        // AND source context (e.g. #[cfg(test)] modules) — not function name prefixes.
        all_groups.retain(|g| is_any_instance_in_non_test_context(g, files));

        let total_duplicated_lines: usize = all_groups.iter().map(|g| g.duplicated_lines()).sum();

        CloneResult {
            groups: all_groups,
            files_analyzed: files.len(),
            total_duplicated_lines,
        }
    }

    /// Detect clones and cluster related groups into `CloneClass` objects.
    ///
    /// This runs the full detection pipeline and then merges overlapping
    /// clone groups into higher-level clusters using Union-Find.
    pub fn detect_and_cluster(&self, files: &[(String, String)]) -> (CloneResult, Vec<CloneClass>) {
        let result = self.detect_in_sources(files);
        let classes = cluster_clone_groups(&result.groups);
        (result, classes)
    }

    /// Detect clones in a directory.
    pub fn detect(&self, root: &Path) -> Result<CloneResult, crate::core::Error> {
        let scanner = crate::analysis::FileScanner::new();
        let source_files = scanner.scan(root)?;

        let files: Vec<(String, String)> = source_files
            .iter()
            .filter_map(|f| {
                let source = std::fs::read_to_string(&f.path).ok()?;
                Some((f.path.to_string_lossy().to_string(), source))
            })
            .collect();

        Ok(self.detect_in_sources(&files))
    }
}

/// Convert a similarity threshold (0.0-1.0) to a SimHash Hamming distance threshold (0-64).
///
/// SimHash similarity = (64 - hamming_distance) / 64
/// So hamming_distance = 64 * (1 - similarity)
///
/// We add a small margin (looser threshold) since SimHash is a pre-filter
/// and false positives are eliminated by MinHash verification.
fn similarity_to_hamming_distance(similarity_threshold: f64) -> u32 {
    // Add 10% margin for pre-filtering (SimHash is approximate)
    let loose_threshold = (similarity_threshold - 0.1).max(0.0);
    let distance = (64.0 * (1.0 - loose_threshold)).ceil() as u32;
    distance.min(64)
}

/// Extract function bodies from source code using simple text-based heuristics.
/// Returns (name, start_line, end_line, body_text) for each function.
/// Works across Python, JavaScript, Java, Go, Ruby, PHP, Rust, C# etc.
fn extract_functions_from_source(source: &str) -> Vec<(String, usize, usize, String)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut functions = Vec::new();

    // Regex patterns for function definitions across languages
    let patterns = [
        // Python: def name(...)
        regex::Regex::new(r"^\s*(?:async\s+)?def\s+(\w+)\s*\(").unwrap(),
        // JavaScript/TypeScript: function name(...)
        regex::Regex::new(r"^\s*(?:export\s+)?(?:async\s+)?function\s+(\w+)\s*\(").unwrap(),
        // Java/C#: <modifiers> <return_type> name(...)
        regex::Regex::new(
            r"^\s*(?:public|private|protected|static|final|abstract|\s)*\w+\s+(\w+)\s*\(",
        )
        .unwrap(),
        // Go with receiver: func (r *Type) methodName(...)
        regex::Regex::new(r"^\s*func\s+\([^)]+\)\s+(\w+)\s*\(").unwrap(),
        // Go standalone: func name(...)
        regex::Regex::new(r"^\s*func\s+(\w+)\s*\(").unwrap(),
        // Ruby: def name
        regex::Regex::new(r"^\s*def\s+(\w+)").unwrap(),
        // Rust: fn name(...)
        regex::Regex::new(r"^\s*(?:pub\s+)?(?:async\s+)?fn\s+(\w+)\s*[\(<]").unwrap(),
        // PHP: function name(...)
        regex::Regex::new(r"^\s*(?:public|private|protected|static|\s)*function\s+(\w+)\s*\(")
            .unwrap(),
    ];

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let mut matched_name = None;

        for pattern in &patterns {
            if let Some(caps) = pattern.captures(line) {
                if let Some(name) = caps.get(1) {
                    matched_name = Some(name.as_str().to_string());
                    break;
                }
            }
        }

        if let Some(name) = matched_name {
            let start_line = i + 1; // 1-indexed
            let end_line = find_function_end(&lines, i);
            let body: String = lines[i..end_line].join("\n");
            functions.push((name, start_line, end_line, body));
            i = end_line; // Skip past this function
        } else {
            i += 1;
        }
    }

    functions
}

/// Find the end of a function starting at `start_idx` using indentation/brace heuristics.
fn find_function_end(lines: &[&str], start_idx: usize) -> usize {
    let start_line = lines[start_idx];
    let start_indent = start_line.len() - start_line.trim_start().len();

    // Check if this is a brace-delimited language (Java, JS, Go, C#, Rust, PHP)
    let has_brace = lines[start_idx..].iter().take(3).any(|l| l.contains('{'));

    if has_brace {
        // Brace matching
        let mut depth = 0;
        for (offset, line) in lines[start_idx..].iter().enumerate() {
            for ch in line.chars() {
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                    if depth == 0 {
                        return start_idx + offset + 1;
                    }
                }
            }
        }
        lines.len()
    } else {
        // No braces — determine if it's Ruby-style (end keyword) or Python-style (indentation)
        // Ruby: `def name` followed by `end` — no colon at end of def line
        // Python: `def name(...):` — has colon at end
        let trimmed_start = start_line.trim();
        let is_ruby_end_style = trimmed_start.starts_with("def ")
            && !trimmed_start.ends_with(':')
            && !trimmed_start.contains('{');

        if is_ruby_end_style {
            // Ruby: look for matching `end` keyword
            for (idx, line) in lines.iter().enumerate().skip(start_idx + 1) {
                let trimmed = line.trim();
                if trimmed == "end" {
                    let indent = line.len() - line.trim_start().len();
                    if indent <= start_indent {
                        return idx + 1;
                    }
                }
            }
            lines.len()
        } else {
            // Python-style: indentation-based scoping
            // The function body has deeper indentation than the def line
            for (idx, line) in lines.iter().enumerate().skip(start_idx + 1) {
                let trimmed = line.trim();

                // Skip empty lines and comments
                if trimmed.is_empty() || trimmed.starts_with('#') {
                    continue;
                }

                let indent = line.len() - line.trim_start().len();
                // Line at same or lower indentation = function ended
                if indent <= start_indent && !trimmed.starts_with('@') {
                    return idx;
                }
            }
            lines.len()
        }
    }
}

/// Check if a clone group is trivial boilerplate (e.g., getters, setters, error returns).
///
/// A group is trivial if ALL instances have ≤5 lines and single-statement bodies
/// (no control flow keywords like if/for/while/match/switch).
fn is_trivial_clone_group(group: &CloneGroup, files: &[(String, String)]) -> bool {
    group.instances.iter().all(|inst| {
        let body_lines = inst.end_line.saturating_sub(inst.start_line) + 1;
        if body_lines > 5 {
            return false;
        }
        // Try to get source text for this instance
        let source = files
            .iter()
            .find(|(path, _)| *path == inst.file)
            .map(|(_, s)| s.as_str());
        if let Some(source) = source {
            let lines: Vec<&str> = source.lines().collect();
            // Extract the body lines (skip first/last for signature/closing brace)
            let start = inst.start_line.saturating_sub(1); // 0-indexed
            let end = inst.end_line.min(lines.len());
            let body: String = lines[start..end]
                .iter()
                .map(|l| l.trim())
                .filter(|l| {
                    !l.is_empty()
                        && !l.starts_with('{')
                        && !l.starts_with('}')
                        && !l.starts_with("end")
                })
                .collect::<Vec<_>>()
                .join(" ");
            // If the body has control flow, it's not trivial
            let has_control_flow = body.contains("if ")
                || body.contains("for ")
                || body.contains("while ")
                || body.contains("match ")
                || body.contains("switch ")
                || body.contains("loop ")
                // Python
                || body.contains("elif ")
                || body.contains("except ")
                || body.contains("except:")
                || body.contains("with ")
                || body.contains("yield ")
                // Ruby
                || body.contains("unless ")
                || body.contains("until ")
                || body.contains("case ")
                || body.contains("when ")
                || body.contains("rescue ")
                || body.contains("rescue:")
                || body.contains("ensure ")
                || body.contains("ensure:")
                // General (Java/C#/JS/Go)
                || body.contains("try ")
                || body.contains("try{")
                || body.contains("catch ")
                || body.contains("catch(")
                || body.contains("else ")
                || body.contains("else{")
                || body.contains("else:")
                || body.contains("select ")
                || body.contains("select{");
            !has_control_flow
        } else {
            // Can't get source, assume trivial based on line count alone
            true
        }
    })
}

/// Check if a clone group consists entirely of trait/interface implementation
/// methods. These are structurally similar because the trait contract imposes
/// the pattern — not because of copy-paste duplication.
///
/// Detection strategy (language-aware, no hardcoded name blocklists):
/// - **Rust**: Scan backwards from the function for `impl ... for ...` (trait impl block)
/// - **Java/Kotlin**: Look for `@Override` annotation above the method
/// - **Python**: Dunder methods (`__str__`, `__repr__`, etc.) are protocol-imposed
///
/// A group is only filtered if ALL instances are trait impls with the same method name.
fn is_trait_impl_clone_group(group: &CloneGroup, files: &[(String, String)]) -> bool {
    if group.instances.len() < 2 {
        return false;
    }

    // All instances must share the same function name
    let first_name = group.instances[0].function_name.as_deref().unwrap_or("");
    if first_name.is_empty() {
        return false;
    }
    let same_name = group
        .instances
        .iter()
        .all(|inst| inst.function_name.as_deref().unwrap_or("") == first_name);
    if !same_name {
        return false;
    }

    // Check each instance for trait impl context
    group.instances.iter().all(|inst| {
        let source = files
            .iter()
            .find(|(path, _)| *path == inst.file)
            .map(|(_, s)| s.as_str());
        let Some(source) = source else { return false };

        is_trait_impl_function(source, inst.start_line, first_name)
    })
}

/// Determine if a function at `start_line` (1-indexed) is inside a
/// trait/interface implementation block by examining surrounding source context.
fn is_trait_impl_function(source: &str, start_line: usize, fn_name: &str) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    let fn_idx = start_line.saturating_sub(1); // 0-indexed

    // Python: dunder methods are protocol-imposed
    if fn_name.starts_with("__") && fn_name.ends_with("__") && fn_name.len() > 4 {
        return true;
    }

    // Java/Kotlin: @Override annotation on the line(s) directly above
    for look_back in 1..=3 {
        if fn_idx >= look_back {
            let above = lines[fn_idx - look_back].trim();
            if above == "@Override" {
                return true;
            }
            // Stop scanning past non-annotation, non-blank lines
            if !above.is_empty() && !above.starts_with('@') && !above.starts_with("//") {
                break;
            }
        }
    }

    // Rust: scan backwards for the enclosing `impl ... for ...` block.
    // Check each line for the trait impl pattern before tracking braces,
    // because the `impl X for Y {` line itself contains the opening `{`.
    let mut brace_depth: i32 = 0;
    for i in (0..fn_idx).rev() {
        let trimmed = lines[i].trim();
        // Rust trait impl: `impl TraitName for TypeName {`
        if trimmed.starts_with("impl ") && trimmed.contains(" for ") {
            return true;
        }
        // Hit another function def or module boundary — stop
        if trimmed.starts_with("fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("mod ")
        {
            break;
        }
        // Count braces in reverse to track scope
        for ch in lines[i].chars().rev() {
            if ch == '}' {
                brace_depth += 1;
            }
            if ch == '{' {
                brace_depth -= 1;
            }
        }
        // We've exited the enclosing block — stop
        if brace_depth < 0 {
            break;
        }
    }

    false
}

/// Returns true if at least one instance in the group is in non-test context.
/// When ALL instances are test code, the group is intentional test duplication
/// and should be filtered. Uses both file-path heuristics AND source context
/// (e.g. Rust `#[cfg(test)]` modules, `#[test]` attributes, Python/JS test
/// framework decorators).
fn is_any_instance_in_non_test_context(group: &CloneGroup, files: &[(String, String)]) -> bool {
    group.instances.iter().any(|inst| {
        if is_test_file(&inst.file) {
            return false; // definitely test
        }
        // Check source context for inline test modules
        let source = files
            .iter()
            .find(|(path, _)| *path == inst.file)
            .map(|(_, s)| s.as_str());
        let Some(source) = source else { return true }; // can't determine, assume non-test

        !is_in_test_context(source, inst.start_line)
    })
}

/// Check if a function at `start_line` (1-indexed) is inside a test context
/// by examining source code above it.
fn is_in_test_context(source: &str, start_line: usize) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    let fn_idx = start_line.saturating_sub(1); // 0-indexed

    // Check for test attributes/decorators directly above the function
    for look_back in 1..=5 {
        if fn_idx < look_back {
            break;
        }
        let above = lines[fn_idx - look_back].trim();
        // Any attribute/decorator containing "test" — covers:
        // Rust: #[test], #[tokio::test], #[actix_rt::test], etc.
        // Python: @pytest.mark.*, @unittest.*, etc.
        // Java/Kotlin: @Test, @ParameterizedTest, etc.
        if (above.starts_with('#') || above.starts_with('@'))
            && above.to_lowercase().contains("test")
        {
            return true;
        }
        // Stop at non-attribute, non-blank, non-comment lines
        if !above.is_empty()
            && !above.starts_with('#')
            && !above.starts_with('@')
            && !above.starts_with("//")
            && !above.starts_with("///")
            && !above.starts_with("*")
        {
            break;
        }
    }

    // Rust: check if inside a #[cfg(test)] module by scanning backwards
    // for `mod tests` preceded by `#[cfg(test)]`
    let mut brace_depth: i32 = 0;
    for i in (0..fn_idx).rev() {
        let trimmed = lines[i].trim();
        // Found a test module
        if (trimmed.starts_with("mod tests")
            || trimmed.starts_with("mod test")
            || trimmed.starts_with("pub mod tests"))
            && brace_depth == 0
        {
            // Check the line(s) above for #[cfg(test)]
            for j in 1..=3 {
                if i >= j {
                    let attr = lines[i - j].trim();
                    if attr == "#[cfg(test)]" {
                        return true;
                    }
                    if !attr.is_empty() && !attr.starts_with('#') && !attr.starts_with("//") {
                        break;
                    }
                }
            }
        }
        // Track braces
        for ch in lines[i].chars().rev() {
            if ch == '}' {
                brace_depth += 1;
            }
            if ch == '{' {
                brace_depth -= 1;
            }
        }
        // Exited the enclosing scope
        if brace_depth < 0 {
            break;
        }
    }

    false
}

/// Check if a file path indicates a test file using path heuristics.
fn is_test_file(path: &str) -> bool {
    let path_lower = path.to_lowercase();
    let segments: Vec<&str> = path_lower.split('/').collect();

    // Directory-level: /tests/, /test/, /__tests__/, /spec/
    for seg in &segments {
        if *seg == "tests" || *seg == "test" || *seg == "__tests__" || *seg == "spec" {
            return true;
        }
    }

    // File-level: check if filename stem contains test/spec markers.
    // Covers: test_foo.py, foo_test.go, foo_tests.rs, foo.test.ts, foo.spec.jsx, etc.
    if let Some(filename) = segments.last() {
        if filename.starts_with("test_") {
            return true;
        }
        // Strip the final extension, then check the stem for test/spec patterns
        if let Some(stem) = filename.rsplit_once('.').map(|(s, _)| s) {
            if stem.ends_with("_test")
                || stem.ends_with("_tests")
                || stem.ends_with(".test")
                || stem.ends_with(".spec")
                || stem.ends_with("_spec")
            {
                return true;
            }
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clone_detector_config() {
        let config = CloneConfig::default();
        assert_eq!(config.min_lines, 6);
        assert!(config.detect_type1);
        assert!(config.detect_type2);
        assert!(config.detect_type3);
    }

    #[test]
    fn test_detect_similar_files() {
        let source_a = "def foo(x):\n    y = x + 1\n    z = y * 2\n    return z\n\ndef bar(a):\n    b = a + 1\n    c = b * 2\n    return c\n";
        let source_b = "def foo(x):\n    y = x + 1\n    z = y * 2\n    return z\n\ndef baz(a):\n    b = a + 1\n    c = b * 2\n    return c\n";

        let detector = CloneDetector::with_defaults();
        let result = detector.detect_in_sources(&[
            ("a.py".to_string(), source_a.to_string()),
            ("b.py".to_string(), source_b.to_string()),
        ]);

        assert_eq!(result.files_analyzed, 2);
    }

    #[test]
    fn test_extract_functions_from_source() {
        let source = "def foo(x):\n    y = x + 1\n    z = y * 2\n    return z\n\ndef bar(a):\n    b = a + 1\n    c = b * 2\n    return c\n";
        let functions = extract_functions_from_source(source);
        let names: Vec<&str> = functions.iter().map(|(n, _, _, _)| n.as_str()).collect();
        assert!(names.contains(&"foo"));
        assert!(names.contains(&"bar"));
    }

    #[test]
    fn test_function_level_clones_within_file() {
        // Two very similar functions within the same file should be detected as clones
        let source = r#"
def format_bytes(size):
    for unit in ['B', 'KB', 'MB', 'GB']:
        if size < 1024:
            return f"{size:.1f} {unit}"
        size /= 1024
    return f"{size:.1f} TB"

def format_file_size(bytes_count):
    for unit in ['B', 'KB', 'MB', 'GB']:
        if bytes_count < 1024:
            return f"{bytes_count:.1f} {unit}"
        bytes_count /= 1024
    return f"{bytes_count:.1f} TB"
"#;

        let config = CloneConfig {
            similarity_threshold: 0.4,
            ..CloneConfig::default()
        };
        let detector = CloneDetector::new(config);
        let result = detector.detect_in_sources(&[("utils.py".to_string(), source.to_string())]);

        // Should find function-level clones within a single file
        assert!(
            !result.groups.is_empty(),
            "Should detect function-level clones within a single file"
        );
    }

    #[test]
    fn test_function_level_clones_across_files() {
        // Larger functions with more shared structure to ensure reliable detection
        let source_a = r#"
function formatFileSize(bytes) {
    if (bytes === 0) return '0 B';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const factor = 1024;
    let idx = 0;
    let size = bytes;
    while (size >= factor && idx < units.length - 1) {
        size /= factor;
        idx++;
    }
    return size.toFixed(1) + ' ' + units[idx];
}
"#;
        let source_b = r#"
function formatDataSize(numBytes) {
    if (numBytes === 0) return '0 B';
    const suffixes = ['B', 'KB', 'MB', 'GB', 'TB'];
    const base = 1024;
    let index = 0;
    let value = numBytes;
    while (value >= base && index < suffixes.length - 1) {
        value /= base;
        index++;
    }
    return value.toFixed(1) + ' ' + suffixes[index];
}
"#;

        let config = CloneConfig {
            similarity_threshold: 0.3,
            ..CloneConfig::default()
        };
        let detector = CloneDetector::new(config);
        let result = detector.detect_in_sources(&[
            ("a.js".to_string(), source_a.to_string()),
            ("b.js".to_string(), source_b.to_string()),
        ]);

        assert!(
            !result.groups.is_empty(),
            "Should detect function-level clones across files"
        );
    }

    #[test]
    fn test_clone_config_has_cross_language_field() {
        let config = CloneConfig::default();
        assert!(
            config.detect_cross_language,
            "cross-language detection should be enabled by default"
        );
    }

    #[test]
    fn test_cross_language_disabled_skips_phase3() {
        let source_a = "def foo(x):\n    y = x + 1\n    z = y * 2\n    return z\n";
        let source_b =
            "function foo(x) {\n    let y = x + 1;\n    let z = y * 2;\n    return z;\n}\n";

        let config = CloneConfig {
            detect_cross_language: false,
            detect_type3: false,
            ..CloneConfig::default()
        };
        let detector = CloneDetector::new(config);
        let result = detector.detect_in_sources(&[
            ("a.py".to_string(), source_a.to_string()),
            ("b.js".to_string(), source_b.to_string()),
        ]);

        // With both type3 and cross-language disabled, no clones should be found
        assert!(
            result.groups.is_empty(),
            "Disabling cross-language should skip cross-language detection"
        );
    }

    #[test]
    fn test_detect_and_cluster_returns_classes() {
        let source_a = "def foo(x):\n    y = x + 1\n    z = y * 2\n    return z\n\ndef bar(a):\n    b = a + 1\n    c = b * 2\n    return c\n";
        let source_b = "def foo(x):\n    y = x + 1\n    z = y * 2\n    return z\n\ndef baz(a):\n    b = a + 1\n    c = b * 2\n    return c\n";

        let config = CloneConfig {
            similarity_threshold: 0.4,
            detect_cross_language: false, // Keep test fast
            ..CloneConfig::default()
        };
        let detector = CloneDetector::new(config);
        let (result, classes) = detector.detect_and_cluster(&[
            ("a.py".to_string(), source_a.to_string()),
            ("b.py".to_string(), source_b.to_string()),
        ]);

        assert_eq!(result.files_analyzed, 2);
        // Classes should be <= groups (clustering merges overlapping groups)
        assert!(classes.len() <= result.groups.len() || result.groups.is_empty());
    }
}
