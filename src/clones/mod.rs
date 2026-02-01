//! Clone detection with Merkle AST hashing, MinHash+LSH, SimHash, and semantic analysis.
//!
//! Provides:
//! - `MerkleDetector` — Type 1-2 exact/renamed clone detection via AST hashing
//! - `MinHashDetector` — Type 3 gapped clone detection via MinHash+LSH
//! - `SimHashFingerprinter` — Fast file-level near-duplicate screening via SimHash
//! - `SemanticCloneDetector` — Type 4 semantic clone detection via feature analysis + tree edit distance
//! - `CloneDetector` — unified detector combining all approaches
//! - `LshIndex` — Locality-Sensitive Hashing index for sub-linear candidate retrieval
//! - `CloneBenchmark` — BigCloneBench evaluation harness

pub mod apted;
pub mod ast_tree;
pub mod benchmark;
pub mod block_clones;
pub mod clustering;
pub mod code_embeddings;
pub mod cross_language;
pub mod detector;
pub mod evolution;
pub mod fingerprint_store;
pub mod ir_tokenizer;
pub mod lsh_index;
pub mod merkle;
pub mod minhash;
pub mod ngram_index;
pub mod scalability;
pub mod semantic;
pub mod simhash;
pub mod sketch_features;
pub mod tree_edit_distance;
pub mod types;

pub use apted::{apted_distance, normalized_apted_distance};
pub use ast_tree::{ast_to_labeled_tree, parse_to_labeled_tree};
pub use benchmark::{BenchmarkResult, CloneBenchmark, CodeFragment};
pub use clustering::{cluster_clone_groups, CloneClass, UnionFind};
pub use code_embeddings::CodeEmbeddingEngine;
pub use cross_language::CrossLanguageDetector;
pub use detector::CloneDetector;
pub use ir_tokenizer::{
    extract_ir_tokens, extract_ir_tokens_from_source, ir_tokens_to_shingles, IRToken,
};
pub use lsh_index::LshIndex;
pub use semantic::{
    extract_semantic_features, feature_distance, SemanticCloneDetector, SemanticFeatures,
    SemanticFunction,
};
pub use sketch_features::{jaccard_similarity, ProjectSketchStats, SketchFeatures};
pub use tree_edit_distance::{
    normalized_tree_edit_distance, source_to_labeled_tree, tree_edit_distance, LabeledTree,
};
pub use types::{CloneGroup, CloneInstance, CloneType};
