//! Merkle AST hashing for Type 1-2 clone detection.
//!
//! Bottom-up hash on tree-sitter AST:
//! `hash(node) = H(kind || hash(child_0) || hash(child_1) || ...)`
//!
//! For Type 2: normalize identifiers to `$IDENT`, literals to `$LIT`.

use dashmap::DashMap;
use xxhash_rust::xxh3::xxh3_64;

use super::types::{CloneGroup, CloneInstance, CloneType};

/// Location of a hashed subtree.
#[derive(Debug, Clone)]
pub struct HashedLocation {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub function_name: Option<String>,
}

/// Merkle-based clone detector.
///
/// Computes bottom-up hashes of tree-sitter AST subtrees.
/// Identical hashes → Type 1 clones. Normalized hashes → Type 2.
pub struct MerkleDetector {
    /// Minimum subtree size (in nodes) to consider.
    pub min_nodes: usize,
    /// Minimum lines to consider.
    pub min_lines: usize,
    /// Type 1: exact hashes → locations
    exact_index: DashMap<u64, Vec<HashedLocation>>,
    /// Type 2: normalized hashes → locations
    normalized_index: DashMap<u64, Vec<HashedLocation>>,
}

impl MerkleDetector {
    pub fn new(min_nodes: usize, min_lines: usize) -> Self {
        Self {
            min_nodes,
            min_lines,
            exact_index: DashMap::new(),
            normalized_index: DashMap::new(),
        }
    }

    /// Index a tree-sitter tree for clone detection.
    pub fn index_tree(&self, root: tree_sitter::Node<'_>, source: &str, file_path: &str) {
        self.hash_subtree(root, source, file_path, false);
        self.hash_subtree(root, source, file_path, true);
    }

    /// Compute the Merkle hash of a subtree.
    fn hash_subtree(
        &self,
        node: tree_sitter::Node<'_>,
        source: &str,
        file_path: &str,
        normalize: bool,
    ) -> u64 {
        let mut hasher_input = Vec::new();

        // Hash the node kind
        let kind = node.kind();
        hasher_input.extend_from_slice(kind.as_bytes());
        hasher_input.push(0xFF); // separator

        // Hash children recursively
        let mut cursor = node.walk();
        let mut child_count = 0u32;
        for child in node.children(&mut cursor) {
            let child_hash = self.hash_subtree(child, source, file_path, normalize);
            hasher_input.extend_from_slice(&child_hash.to_le_bytes());
            child_count += 1;
        }

        // For leaf nodes, hash the text
        if child_count == 0 {
            let text = &source[node.byte_range()];
            if normalize {
                // Normalize: replace identifiers and literals
                let normalized = match kind {
                    "identifier" | "type_identifier" | "field_identifier" => "$IDENT",
                    "string" | "string_literal" | "raw_string_literal" | "template_string"
                    | "string_content" => "$STR",
                    "integer"
                    | "integer_literal"
                    | "float"
                    | "float_literal"
                    | "number"
                    | "decimal_integer_literal" => "$NUM",
                    "true" | "false" | "boolean" => "$BOOL",
                    _ => text,
                };
                hasher_input.extend_from_slice(normalized.as_bytes());
            } else {
                hasher_input.extend_from_slice(text.as_bytes());
            }
        }

        let hash = xxh3_64(&hasher_input);

        // Only index subtrees that are large enough
        let line_count = node
            .end_position()
            .row
            .saturating_sub(node.start_position().row)
            + 1;
        if child_count >= self.min_nodes as u32 && line_count >= self.min_lines {
            let location = HashedLocation {
                file: file_path.to_string(),
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                start_byte: node.start_byte(),
                end_byte: node.end_byte(),
                function_name: None,
            };

            let index = if normalize {
                &self.normalized_index
            } else {
                &self.exact_index
            };
            index.entry(hash).or_default().push(location);
        }

        hash
    }

    /// Extract clone groups from the index.
    pub fn extract_clones(&self) -> Vec<CloneGroup> {
        let mut groups = Vec::new();

        // Type 1: exact clones
        for entry in self.exact_index.iter() {
            if entry.value().len() >= 2 {
                let instances: Vec<CloneInstance> = entry
                    .value()
                    .iter()
                    .map(|loc| {
                        let mut inst =
                            CloneInstance::new(loc.file.clone(), loc.start_line, loc.end_line);
                        inst.start_byte = loc.start_byte;
                        inst.end_byte = loc.end_byte;
                        inst.function_name = loc.function_name.clone();
                        inst
                    })
                    .collect();

                groups.push(CloneGroup::new(CloneType::Type1, instances).with_hash(*entry.key()));
            }
        }

        // Type 2: normalized clones (only if not already found as Type 1)
        let exact_hashes: std::collections::HashSet<u64> = self
            .exact_index
            .iter()
            .filter(|e| e.value().len() >= 2)
            .map(|e| *e.key())
            .collect();

        for entry in self.normalized_index.iter() {
            if entry.value().len() >= 2 && !exact_hashes.contains(entry.key()) {
                let instances: Vec<CloneInstance> = entry
                    .value()
                    .iter()
                    .map(|loc| {
                        let mut inst =
                            CloneInstance::new(loc.file.clone(), loc.start_line, loc.end_line);
                        inst.start_byte = loc.start_byte;
                        inst.end_byte = loc.end_byte;
                        inst
                    })
                    .collect();

                groups.push(CloneGroup::new(CloneType::Type2, instances).with_hash(*entry.key()));
            }
        }

        groups
    }

    /// Clear the index.
    pub fn clear(&self) {
        self.exact_index.clear();
        self.normalized_index.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merkle_hashing_basics() {
        let detector = MerkleDetector::new(3, 2);
        assert_eq!(detector.min_nodes, 3);
        assert_eq!(detector.min_lines, 2);
    }
}
