//! Clone detection types.

use serde::{Deserialize, Serialize};

/// Type of code clone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CloneType {
    /// Exact copy (identical tokens).
    Type1,
    /// Renamed (identical structure, different identifiers/literals).
    Type2,
    /// Near-miss (similar structure with gaps/modifications).
    Type3,
    /// Semantic (functionally similar but structurally different).
    Type4,
}

impl std::fmt::Display for CloneType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloneType::Type1 => write!(f, "Type-1 (exact)"),
            CloneType::Type2 => write!(f, "Type-2 (renamed)"),
            CloneType::Type3 => write!(f, "Type-3 (near-miss)"),
            CloneType::Type4 => write!(f, "Type-4 (semantic)"),
        }
    }
}

/// A single clone instance (location of cloned code).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneInstance {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub start_byte: usize,
    pub end_byte: usize,
    pub function_name: Option<String>,
}

impl CloneInstance {
    pub fn new(file: String, start_line: usize, end_line: usize) -> Self {
        Self {
            file,
            start_line,
            end_line,
            start_byte: 0,
            end_byte: 0,
            function_name: None,
        }
    }

    pub fn lines(&self) -> usize {
        self.end_line.saturating_sub(self.start_line) + 1
    }
}

/// A group of code clones that are similar to each other.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloneGroup {
    pub clone_type: CloneType,
    pub instances: Vec<CloneInstance>,
    pub similarity: f64,
    pub hash: Option<u64>,
}

impl CloneGroup {
    pub fn new(clone_type: CloneType, instances: Vec<CloneInstance>) -> Self {
        Self {
            clone_type,
            instances,
            similarity: 1.0,
            hash: None,
        }
    }

    pub fn with_similarity(mut self, similarity: f64) -> Self {
        self.similarity = similarity;
        self
    }

    pub fn with_hash(mut self, hash: u64) -> Self {
        self.hash = Some(hash);
        self
    }

    /// Number of instances in this group.
    pub fn size(&self) -> usize {
        self.instances.len()
    }

    /// Total duplicated lines across all instances (minus one original).
    pub fn duplicated_lines(&self) -> usize {
        if self.instances.is_empty() {
            return 0;
        }
        let per_instance = self.instances.first().map_or(0, |i| i.lines());
        per_instance * (self.instances.len() - 1)
    }
}
