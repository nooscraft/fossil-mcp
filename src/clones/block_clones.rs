//! Block-level clone detection using MinHash + LSH.
//!
//! Extracts logical code blocks (if/else, for/while loops, try/catch, match/switch arms,
//! function bodies) from source files using text-based heuristics, then detects
//! near-duplicate blocks via MinHash sketching and LSH candidate generation.
//!
//! This complements function-level detection by finding duplicated control-flow
//! fragments that are smaller than entire functions.

use std::collections::HashMap;
#[cfg(test)]
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_64;

use crate::graph::ControlFlowGraph;

use super::minhash::MinHashDetector;
use super::types::CloneType;

// ---------------------------------------------------------------------------
// Block-level types
// ---------------------------------------------------------------------------

/// A single extracted code block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBlock {
    /// File path this block belongs to.
    pub file: String,
    /// Enclosing function name, if determinable.
    pub function_name: Option<String>,
    /// 1-indexed start line.
    pub start_line: usize,
    /// 1-indexed end line (inclusive).
    pub end_line: usize,
    /// Raw source text of the block.
    pub content: String,
    /// xxh3 hash of the normalized content for fast exact-match screening.
    pub block_hash: u64,
}

impl CodeBlock {
    /// Number of source lines in this block.
    pub fn lines(&self) -> usize {
        self.end_line.saturating_sub(self.start_line) + 1
    }
}

/// A group of similar code blocks detected as clones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockCloneGroup {
    /// Clone type classification for this group.
    pub block_type: CloneType,
    /// The set of similar block instances.
    pub instances: Vec<CodeBlock>,
    /// Estimated Jaccard similarity between the blocks.
    pub similarity: f64,
}

// ---------------------------------------------------------------------------
// Block-start keyword patterns
// ---------------------------------------------------------------------------

/// Keywords that indicate the start of a logical code block across languages.
const BLOCK_START_KEYWORDS: &[&str] = &[
    "if", "else", "elif", "else if", "for", "while", "do", "try", "catch", "except", "finally",
    "match", "switch", "case", "def", "fn", "func", "function",
];

/// Check whether a trimmed line starts with a block keyword.
fn starts_with_block_keyword(trimmed: &str) -> bool {
    for &kw in BLOCK_START_KEYWORDS {
        if let Some(rest) = trimmed.strip_prefix(kw) {
            // Ensure the keyword is followed by a non-alphanumeric character
            // (whitespace, paren, colon, brace) or is at end of line.
            if rest.is_empty() || rest.starts_with(|c: char| !c.is_alphanumeric() && c != '_') {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// BlockCloneDetector
// ---------------------------------------------------------------------------

/// Detector for block-level code clones.
///
/// Extracts logical blocks (if/else, loops, try/catch, match arms, function
/// bodies) from source files and detects near-duplicate blocks using MinHash
/// sketching and LSH-based candidate generation.
pub struct BlockCloneDetector {
    /// Minimum number of lines for a block to be considered.
    min_block_lines: usize,
    /// Jaccard similarity threshold for reporting clone pairs.
    similarity_threshold: f64,
    /// Token shingle size for MinHash.
    shingle_size: usize,
}

impl BlockCloneDetector {
    /// Create a new detector with the given parameters.
    pub fn new(min_block_lines: usize, similarity_threshold: f64) -> Self {
        Self {
            min_block_lines,
            similarity_threshold,
            shingle_size: 3,
        }
    }

    /// Create a detector with sensible defaults (min 4 lines, 0.6 threshold).
    pub fn with_defaults() -> Self {
        Self::new(4, 0.6)
    }

    // -----------------------------------------------------------------------
    // Block extraction
    // -----------------------------------------------------------------------

    /// Extract logical code blocks from a single source file.
    ///
    /// Uses heuristic detection of block-start keywords combined with
    /// brace-matching or indentation-based scoping to determine block extent.
    pub fn extract_blocks(&self, source: &str, file_path: &str) -> Vec<CodeBlock> {
        let lines: Vec<&str> = source.lines().collect();
        let mut blocks = Vec::new();
        let mut i = 0;

        while i < lines.len() {
            let trimmed = lines[i].trim();

            if !trimmed.is_empty() && starts_with_block_keyword(trimmed) {
                let start_idx = i;
                let end_idx = self.find_block_end(&lines, i);

                let line_count = end_idx - start_idx;
                if line_count >= self.min_block_lines {
                    let content: String = lines[start_idx..end_idx].join("\n");
                    let normalized = normalize_block_content(&content);
                    let block_hash = xxh3_64(normalized.as_bytes());

                    blocks.push(CodeBlock {
                        file: file_path.to_string(),
                        function_name: self.find_enclosing_function(&lines, start_idx),
                        start_line: start_idx + 1, // 1-indexed
                        end_line: end_idx,         // 1-indexed (inclusive)
                        content,
                        block_hash,
                    });
                }

                // Advance past the block but only by one line so nested blocks
                // at deeper indentation levels are still discovered on the next
                // iteration.
                i += 1;
            } else {
                i += 1;
            }
        }

        blocks
    }

    /// Extract code blocks from a Control Flow Graph when available.
    ///
    /// Uses CFG basic block boundaries instead of text heuristics, producing
    /// more accurate block boundaries that align with actual control flow.
    /// Each basic block's statements are gathered by their `SourceSpan` byte
    /// ranges, converted to line ranges and source text.
    ///
    /// Falls back to text-based `extract_blocks()` when the CFG is `None`.
    pub fn extract_blocks_from_cfg(
        &self,
        source: &str,
        file_path: &str,
        cfg: Option<&ControlFlowGraph>,
    ) -> Vec<CodeBlock> {
        let cfg = match cfg {
            Some(c) => c,
            None => return self.extract_blocks(source, file_path),
        };

        let mut blocks = Vec::new();
        let lines: Vec<&str> = source.lines().collect();

        for (_node_id, basic_block) in cfg.blocks() {
            // Skip entry/exit sentinel blocks that have no statements
            if basic_block.is_entry || basic_block.is_exit || basic_block.statements.is_empty() {
                continue;
            }

            // Determine the line range covered by this basic block's statements.
            // SourceSpan is byte-based; convert to 1-indexed lines.
            let mut min_line = usize::MAX;
            let mut max_line = 0usize;

            for span in &basic_block.statements {
                let start_line = byte_offset_to_line(source, span.start);
                let end_line =
                    byte_offset_to_line(source, span.end.saturating_sub(1).max(span.start));
                min_line = min_line.min(start_line);
                max_line = max_line.max(end_line);
            }

            if min_line == usize::MAX || max_line == 0 {
                continue;
            }

            let line_count = max_line - min_line + 1;
            if line_count < self.min_block_lines {
                continue;
            }

            // Extract source text for the block (1-indexed lines -> 0-indexed array)
            let start_idx = min_line.saturating_sub(1);
            let end_idx = max_line.min(lines.len());
            let content: String = lines[start_idx..end_idx].join("\n");
            let normalized = normalize_block_content(&content);
            let block_hash = xxh3_64(normalized.as_bytes());

            blocks.push(CodeBlock {
                file: file_path.to_string(),
                function_name: Some(cfg.function_name.clone()),
                start_line: min_line,
                end_line: max_line,
                content,
                block_hash,
            });
        }

        blocks
    }

    /// Find the end of a block starting at `start_idx`.
    ///
    /// Uses brace matching if braces are found within the first few lines,
    /// otherwise falls back to indentation-based scoping (Python-style) or
    /// Ruby-style `end` keyword matching.
    fn find_block_end(&self, lines: &[&str], start_idx: usize) -> usize {
        let start_line = lines[start_idx];
        let start_indent = start_line.len() - start_line.trim_start().len();

        // Check if there is an opening brace within the first 3 lines of the block
        let has_brace = lines[start_idx..].iter().take(3).any(|l| l.contains('{'));

        if has_brace {
            // Brace-delimited block
            let mut depth = 0i32;
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
            // Unbalanced braces: return to end of file
            lines.len()
        } else {
            // Indentation-based (Python) or end-keyword (Ruby) scoping
            let trimmed_start = start_line.trim();
            let is_ruby_end_style = (trimmed_start.starts_with("def ")
                || trimmed_start.starts_with("do")
                || trimmed_start.starts_with("begin"))
                && !trimmed_start.ends_with(':')
                && !trimmed_start.contains('{');

            if is_ruby_end_style {
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
                // Python-style indentation scoping
                for (idx, line) in lines.iter().enumerate().skip(start_idx + 1) {
                    let trimmed = line.trim();

                    // Skip blank lines and comments
                    if trimmed.is_empty() || trimmed.starts_with('#') {
                        continue;
                    }

                    let indent = line.len() - line.trim_start().len();
                    if indent <= start_indent {
                        return idx;
                    }
                }
                lines.len()
            }
        }
    }

    /// Try to find the name of the enclosing function for a block at the given line.
    fn find_enclosing_function(&self, lines: &[&str], block_start: usize) -> Option<String> {
        // Walk backwards from block_start looking for a function definition
        // at a shallower indentation level.
        let block_indent = lines[block_start].len() - lines[block_start].trim_start().len();

        for i in (0..block_start).rev() {
            let line = lines[i];
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let indent = line.len() - line.trim_start().len();
            if indent < block_indent {
                // Check for function-definition patterns
                if let Some(name) = extract_function_name(trimmed) {
                    return Some(name);
                }
            }
        }
        None
    }

    // -----------------------------------------------------------------------
    // Clone detection
    // -----------------------------------------------------------------------

    /// Detect block-level clones across a set of source files.
    ///
    /// Pipeline:
    /// 1. Extract blocks from every file
    /// 2. Compute MinHash sketch for each block
    /// 3. LSH candidate generation via `MinHashDetector::detect_clones`
    /// 4. Verify candidates with Jaccard similarity
    pub fn detect_block_clones(&self, files: &[(String, String)]) -> Vec<BlockCloneGroup> {
        let minhash = MinHashDetector::new(128, self.shingle_size, self.similarity_threshold);

        // Extract all blocks
        let mut all_blocks: Vec<CodeBlock> = Vec::new();
        for (path, source) in files {
            let blocks = self.extract_blocks(source, path);
            all_blocks.extend(blocks);
        }

        if all_blocks.len() < 2 {
            return Vec::new();
        }

        // Build MinHash signatures for each block
        let mut signatures: Vec<crate::clones::minhash::FunctionSignature> =
            Vec::with_capacity(all_blocks.len());

        for block in &all_blocks {
            let shingle_hashes = minhash.compute_shingles(&block.content);
            if shingle_hashes.is_empty() {
                // Push a dummy with empty shingles; detect_clones will skip it
                signatures.push(crate::clones::minhash::FunctionSignature {
                    file: block.file.clone(),
                    name: format!(
                        "block@{}:{}-{}",
                        block.file, block.start_line, block.end_line
                    ),
                    start_line: block.start_line,
                    end_line: block.end_line,
                    sketch: minhash.build_sketch(&[]),
                    shingle_hashes: Vec::new(),
                });
                continue;
            }
            let sketch = minhash.build_sketch(&shingle_hashes);
            signatures.push(crate::clones::minhash::FunctionSignature {
                file: block.file.clone(),
                name: format!(
                    "block@{}:{}-{}",
                    block.file, block.start_line, block.end_line
                ),
                start_line: block.start_line,
                end_line: block.end_line,
                sketch,
                shingle_hashes,
            });
        }

        // LSH candidate generation + Jaccard verification
        let raw_groups = minhash.detect_clones(&signatures);

        // Convert CloneGroup results into BlockCloneGroup with full CodeBlock instances
        let mut block_groups: Vec<BlockCloneGroup> = Vec::new();

        // Build an index from (file, start_line) -> block for fast lookup
        let block_index: HashMap<(&str, usize), &CodeBlock> = all_blocks
            .iter()
            .map(|b| ((b.file.as_str(), b.start_line), b))
            .collect();

        for group in &raw_groups {
            let mut instances: Vec<CodeBlock> = Vec::new();
            for inst in &group.instances {
                if let Some(block) = block_index.get(&(inst.file.as_str(), inst.start_line)) {
                    instances.push((*block).clone());
                }
            }
            if instances.len() >= 2 {
                // Classify: exact hash match = Type1, otherwise Type3
                let all_same_hash = instances
                    .windows(2)
                    .all(|w| w[0].block_hash == w[1].block_hash);
                let block_type = if all_same_hash {
                    CloneType::Type1
                } else {
                    CloneType::Type3
                };

                block_groups.push(BlockCloneGroup {
                    block_type,
                    instances,
                    similarity: group.similarity,
                });
            }
        }

        block_groups
    }

    /// Merge adjacent clone groups where blocks in the same file are contiguous.
    ///
    /// When two clone groups contain blocks that are adjacent (one ends where the
    /// next begins, or they overlap) in the same file, they are merged into a
    /// single larger clone region. This reduces noise from block-level detection
    /// that fragments a single large duplicated region into many small groups.
    pub fn merge_adjacent_blocks(&self, groups: &mut Vec<BlockCloneGroup>) {
        if groups.len() < 2 {
            return;
        }

        let mut merged = true;
        while merged {
            merged = false;
            let mut i = 0;
            while i < groups.len() {
                let mut j = i + 1;
                while j < groups.len() {
                    if Self::groups_are_adjacent(&groups[i], &groups[j]) {
                        // Merge group j into group i
                        let group_j = groups.remove(j);
                        Self::merge_into(&mut groups[i], &group_j);
                        merged = true;
                        // Do not increment j; the next element slid into position j
                    } else {
                        j += 1;
                    }
                }
                i += 1;
            }
        }
    }

    /// Check whether two groups have adjacent or overlapping blocks in the same file.
    fn groups_are_adjacent(a: &BlockCloneGroup, b: &BlockCloneGroup) -> bool {
        for inst_a in &a.instances {
            for inst_b in &b.instances {
                if inst_a.file == inst_b.file {
                    // Adjacent: one ends where the other starts (within 2 lines gap)
                    let gap_threshold = 2;
                    let a_end = inst_a.end_line;
                    let b_start = inst_b.start_line;
                    let b_end = inst_b.end_line;
                    let a_start = inst_a.start_line;

                    if (a_end >= b_start && a_end <= b_end + gap_threshold)
                        || (b_end >= a_start && b_end <= a_end + gap_threshold)
                        || (b_start <= a_end + gap_threshold && b_start >= a_start)
                        || (a_start <= b_end + gap_threshold && a_start >= b_start)
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Merge the instances from `source` into `target`, expanding line ranges
    /// for blocks in the same file.
    fn merge_into(target: &mut BlockCloneGroup, source: &BlockCloneGroup) {
        for src_inst in &source.instances {
            let mut found = false;
            for tgt_inst in target.instances.iter_mut() {
                if tgt_inst.file == src_inst.file {
                    // Expand the range
                    let new_start = tgt_inst.start_line.min(src_inst.start_line);
                    let new_end = tgt_inst.end_line.max(src_inst.end_line);
                    // Rebuild content from the merged range description
                    tgt_inst.start_line = new_start;
                    tgt_inst.end_line = new_end;
                    // Recompute content and hash (append source content if needed)
                    tgt_inst.content = format!("{}\n{}", tgt_inst.content, src_inst.content);
                    tgt_inst.block_hash = xxh3_64(tgt_inst.content.as_bytes());
                    found = true;
                    break;
                }
            }
            if !found {
                target.instances.push(src_inst.clone());
            }
        }

        // Update similarity to the minimum (conservative)
        target.similarity = target.similarity.min(source.similarity);
    }
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

/// Convert a byte offset within `source` to a 1-indexed line number.
///
/// Returns 1 for byte 0, and increments for each `\n` encountered before `offset`.
fn byte_offset_to_line(source: &str, offset: usize) -> usize {
    let clamped = offset.min(source.len());
    source[..clamped].matches('\n').count() + 1
}

/// Normalize block content for hashing: collapse whitespace, lowercase.
fn normalize_block_content(content: &str) -> String {
    content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Try to extract a function name from a line that looks like a function definition.
fn extract_function_name(trimmed: &str) -> Option<String> {
    // Python: def name(
    // Rust: fn name(  /  pub fn name(  /  pub async fn name(
    // JS: function name(  /  async function name(
    // Go: func name(
    // Java/C#: <modifiers> <type> name(
    let patterns: &[(&str, &str)] = &[
        ("def ", "("),
        ("fn ", "("),
        ("fn ", "<"),
        ("func ", "("),
        ("function ", "("),
    ];

    for &(prefix, delimiter) in patterns {
        if let Some(rest) = find_after_keyword(trimmed, prefix) {
            if let Some(name_end) = rest.find(delimiter) {
                let name = rest[..name_end].trim();
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    return Some(name.to_string());
                }
            }
        }
    }

    None
}

/// Find the substring after a keyword prefix, handling modifiers like `pub`, `async`, etc.
fn find_after_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    if let Some(pos) = line.find(keyword) {
        Some(&line[pos + keyword.len()..])
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_blocks_python_if_for_while() {
        let source = r#"
def process(items):
    total = 0
    for item in items:
        if item > 0:
            total += item
            count += 1
            log(item)
        else:
            skip(item)
            log_skip(item)
            count_skip += 1
    while total > 100:
        total -= 10
        adjust(total)
        log(total)
        record(total)
    return total
"#;
        let detector = BlockCloneDetector::with_defaults();
        let blocks = detector.extract_blocks(source, "test.py");

        // Should find: def (function body), for, if, else, while
        // Filter by min_block_lines=4
        let block_starts: Vec<usize> = blocks.iter().map(|b| b.start_line).collect();

        // The `for` block starts on line 4 (1-indexed), `if` on line 5,
        // `while` on line 13. `def` on line 2.
        assert!(
            !blocks.is_empty(),
            "Should extract at least some blocks from Python source"
        );

        // Verify we got a for-loop block
        let has_for = blocks
            .iter()
            .any(|b| b.content.contains("for item in items"));
        assert!(
            has_for,
            "Should extract the for-loop block, got starts: {block_starts:?}"
        );

        // Verify we got an if block
        let has_if = blocks.iter().any(|b| {
            b.content.starts_with("if item > 0")
                || b.content.trim_start().starts_with("if item > 0")
        });
        assert!(has_if, "Should extract the if block");

        // Verify we got a while block
        let has_while = blocks
            .iter()
            .any(|b| b.content.contains("while total > 100"));
        assert!(has_while, "Should extract the while block");
    }

    #[test]
    fn test_extract_blocks_javascript_brace_delimited() {
        let source = r#"function validate(input) {
    if (input === null) {
        console.log("null input");
        throw new Error("null");
        return false;
    }
    for (let i = 0; i < input.length; i++) {
        if (input[i] < 0) {
            console.log("negative");
            input[i] = 0;
            count++;
            flag = true;
        }
    }
    try {
        process(input);
        save(input);
        log(input);
        notify();
    } catch (e) {
        handleError(e);
        log(e);
        cleanup();
        retry();
    }
    return true;
}"#;

        let detector = BlockCloneDetector::with_defaults();
        let blocks = detector.extract_blocks(source, "test.js");

        assert!(
            !blocks.is_empty(),
            "Should extract blocks from JavaScript source"
        );

        // Should find: function, if (null check), for loop, nested if, try, catch
        let has_function = blocks
            .iter()
            .any(|b| b.content.contains("function validate"));
        assert!(has_function, "Should extract the function block");

        let has_for = blocks.iter().any(|b| b.content.contains("for (let i"));
        assert!(has_for, "Should extract the for-loop block");

        let has_try = blocks.iter().any(|b| b.content.contains("try {"));
        assert!(has_try, "Should extract the try block");
    }

    #[test]
    fn test_detect_duplicate_if_blocks() {
        // Two files with nearly identical if-blocks
        let source_a = r#"
function checkA(val) {
    if (val > 0) {
        let result = val * 2;
        console.log(result);
        save(result);
        return result;
    }
}
"#;
        let source_b = r#"
function checkB(value) {
    if (value > 0) {
        let output = value * 2;
        console.log(output);
        save(output);
        return output;
    }
}
"#;

        let detector = BlockCloneDetector::new(4, 0.3);
        let groups = detector.detect_block_clones(&[
            ("a.js".to_string(), source_a.to_string()),
            ("b.js".to_string(), source_b.to_string()),
        ]);

        // The if-blocks are structurally identical (Type 2/3 clone)
        assert!(
            !groups.is_empty(),
            "Should detect duplicate if-blocks across files"
        );

        // At least one group should contain instances from both files
        let cross_file = groups.iter().any(|g| {
            let files: HashSet<&str> = g.instances.iter().map(|i| i.file.as_str()).collect();
            files.len() >= 2
        });
        assert!(
            cross_file,
            "Should have at least one cross-file clone group"
        );
    }

    #[test]
    fn test_merge_adjacent_blocks() {
        let detector = BlockCloneDetector::with_defaults();

        let block_a1 = CodeBlock {
            file: "a.py".to_string(),
            function_name: Some("foo".to_string()),
            start_line: 5,
            end_line: 10,
            content: "if x > 0:\n    do_a()\n    do_b()\n    do_c()\n    do_d()\n    do_e()"
                .to_string(),
            block_hash: 111,
        };
        let block_a2 = CodeBlock {
            file: "b.py".to_string(),
            function_name: Some("bar".to_string()),
            start_line: 5,
            end_line: 10,
            content: "if y > 0:\n    do_a()\n    do_b()\n    do_c()\n    do_d()\n    do_e()"
                .to_string(),
            block_hash: 222,
        };

        let block_b1 = CodeBlock {
            file: "a.py".to_string(),
            function_name: Some("foo".to_string()),
            start_line: 11,
            end_line: 16,
            content: "for i in range(10):\n    process(i)\n    log(i)\n    save(i)\n    check(i)\n    done(i)".to_string(),
            block_hash: 333,
        };
        let block_b2 = CodeBlock {
            file: "b.py".to_string(),
            function_name: Some("bar".to_string()),
            start_line: 11,
            end_line: 16,
            content: "for j in range(10):\n    process(j)\n    log(j)\n    save(j)\n    check(j)\n    done(j)".to_string(),
            block_hash: 444,
        };

        let mut groups = vec![
            BlockCloneGroup {
                block_type: CloneType::Type3,
                instances: vec![block_a1, block_a2],
                similarity: 0.8,
            },
            BlockCloneGroup {
                block_type: CloneType::Type3,
                instances: vec![block_b1, block_b2],
                similarity: 0.7,
            },
        ];

        detector.merge_adjacent_blocks(&mut groups);

        // The two groups should be merged because blocks in a.py (lines 5-10 and 11-16)
        // and in b.py (lines 5-10 and 11-16) are adjacent.
        assert_eq!(
            groups.len(),
            1,
            "Adjacent clone groups should be merged into one, got {}",
            groups.len()
        );

        // The merged group should span the combined line range
        let merged = &groups[0];
        for inst in &merged.instances {
            assert_eq!(inst.start_line, 5, "Merged start should be 5");
            assert_eq!(inst.end_line, 16, "Merged end should be 16");
        }

        // Similarity should be the minimum (conservative)
        assert!(
            (merged.similarity - 0.7).abs() < f64::EPSILON,
            "Merged similarity should be min(0.8, 0.7) = 0.7, got {}",
            merged.similarity
        );
    }

    #[test]
    fn test_blocks_below_min_lines_filtered() {
        let source = "if x:\n    pass\n";
        let detector = BlockCloneDetector::with_defaults(); // min=4
        let blocks = detector.extract_blocks(source, "test.py");
        assert!(
            blocks.is_empty(),
            "2-line if block should be filtered out (min is 4)"
        );
    }

    #[test]
    fn test_block_hash_deterministic() {
        let source = "for i in range(10):\n    process(i)\n    log(i)\n    save(i)\n    done(i)\n";
        let detector = BlockCloneDetector::with_defaults();
        let blocks_a = detector.extract_blocks(source, "test.py");
        let blocks_b = detector.extract_blocks(source, "test.py");

        assert!(!blocks_a.is_empty());
        assert_eq!(
            blocks_a[0].block_hash, blocks_b[0].block_hash,
            "Same content should produce same hash"
        );
    }

    #[test]
    fn test_enclosing_function_detected() {
        let source = r#"def outer():
    x = 1
    if x > 0:
        do_something()
        do_more()
        finish()
        cleanup()
"#;
        let detector = BlockCloneDetector::with_defaults();
        let blocks = detector.extract_blocks(source, "test.py");

        // Find the block whose content starts with the if-statement (not the enclosing def block)
        let if_block = blocks
            .iter()
            .find(|b| b.content.trim_start().starts_with("if x > 0"));
        assert!(if_block.is_some(), "Should find the if block");
        assert_eq!(
            if_block.unwrap().function_name.as_deref(),
            Some("outer"),
            "Should detect 'outer' as the enclosing function"
        );
    }

    #[test]
    fn test_no_clones_in_dissimilar_blocks() {
        let source_a = r#"
function alpha() {
    if (true) {
        doAlpha();
        alphaStep2();
        alphaStep3();
        alphaStep4();
    }
}
"#;
        let source_b = r#"
function beta() {
    for (let i = 0; i < 100; i++) {
        doBeta(i);
        betaStep2(i);
        betaStep3(i);
        betaStep4(i);
    }
}
"#;

        let detector = BlockCloneDetector::new(4, 0.8);
        let groups = detector.detect_block_clones(&[
            ("a.js".to_string(), source_a.to_string()),
            ("b.js".to_string(), source_b.to_string()),
        ]);

        // With a high threshold, structurally different blocks should not match
        let cross_file = groups.iter().any(|g| {
            let files: HashSet<&str> = g.instances.iter().map(|i| i.file.as_str()).collect();
            files.len() >= 2
        });
        assert!(
            !cross_file,
            "Dissimilar blocks should not be detected as clones at 0.8 threshold"
        );
    }

    // ------------------------------------------------------------------
    // CFG-based block extraction tests
    // ------------------------------------------------------------------

    #[test]
    fn test_extract_blocks_from_cfg_none_falls_back() {
        let source = r#"
def process(items):
    total = 0
    for item in items:
        if item > 0:
            total += item
            count += 1
            log(item)
        else:
            skip(item)
            log_skip(item)
            count_skip += 1
    while total > 100:
        total -= 10
        adjust(total)
        log(total)
        record(total)
    return total
"#;
        let detector = BlockCloneDetector::with_defaults();

        // Passing None for CFG should fall back to text-based extraction
        let blocks_text = detector.extract_blocks(source, "test.py");
        let blocks_cfg = detector.extract_blocks_from_cfg(source, "test.py", None);

        assert_eq!(
            blocks_text.len(),
            blocks_cfg.len(),
            "CFG=None should produce identical results to text-based extraction"
        );

        for (a, b) in blocks_text.iter().zip(blocks_cfg.iter()) {
            assert_eq!(a.start_line, b.start_line);
            assert_eq!(a.end_line, b.end_line);
            assert_eq!(a.block_hash, b.block_hash);
        }
    }

    #[test]
    fn test_extract_blocks_from_cfg_with_cfg() {
        use crate::core::SourceSpan;
        use crate::graph::ControlFlowGraph;

        // Build a simple CFG manually
        let mut cfg = ControlFlowGraph::new("test_func");
        let entry = cfg.create_entry();
        let block_a = cfg.create_block("body");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, block_a, crate::graph::CfgEdgeKind::FallThrough);
        cfg.add_edge(block_a, exit, crate::graph::CfgEdgeKind::Return);

        // The source has 6 lines total; block_a covers bytes for lines 2-6
        let source = "def test_func():\n    x = 1\n    y = 2\n    z = 3\n    w = 4\n    return x + y + z + w\n";

        // Line 2 starts at byte 17, line 6 ends at byte ~74
        // Add statements to block_a that cover lines 2-6
        cfg.add_statement(block_a, SourceSpan::new(17, 74));

        let detector = BlockCloneDetector::new(3, 0.6);
        let blocks = detector.extract_blocks_from_cfg(source, "test.py", Some(&cfg));

        // Should extract the block from the CFG (not entry/exit sentinels)
        assert!(
            !blocks.is_empty(),
            "Should extract at least one block from CFG"
        );

        // The block should reference the function name from the CFG
        let block = &blocks[0];
        assert_eq!(
            block.function_name.as_deref(),
            Some("test_func"),
            "CFG-extracted block should carry function name"
        );
    }

    #[test]
    fn test_extract_blocks_from_cfg_filters_small_blocks() {
        use crate::core::SourceSpan;
        use crate::graph::ControlFlowGraph;

        let mut cfg = ControlFlowGraph::new("small_func");
        let entry = cfg.create_entry();
        let block_a = cfg.create_block("tiny");
        let exit = cfg.create_exit();

        cfg.add_edge(entry, block_a, crate::graph::CfgEdgeKind::FallThrough);
        cfg.add_edge(block_a, exit, crate::graph::CfgEdgeKind::Return);

        // Only 2 lines of actual code -- below min_block_lines=4
        let source = "def small():\n    return 1\n";
        cfg.add_statement(block_a, SourceSpan::new(13, 26));

        let detector = BlockCloneDetector::with_defaults(); // min 4 lines
        let blocks = detector.extract_blocks_from_cfg(source, "test.py", Some(&cfg));

        assert!(
            blocks.is_empty(),
            "Blocks smaller than min_block_lines should be filtered out"
        );
    }

    #[test]
    fn test_byte_offset_to_line() {
        let source = "line1\nline2\nline3\n";
        assert_eq!(byte_offset_to_line(source, 0), 1); // start of line 1
        assert_eq!(byte_offset_to_line(source, 5), 1); // the '\n' after line1
        assert_eq!(byte_offset_to_line(source, 6), 2); // start of line 2
        assert_eq!(byte_offset_to_line(source, 12), 3); // start of line 3
    }
}
