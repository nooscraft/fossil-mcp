//! Shared analysis infrastructure for Fossil.
//!
//! Provides:
//! - `FileScanner` — discovers source files respecting .gitignore
//! - `Pipeline` — orchestrates parsing and graph building with parallel execution
//! - `Aggregator` — merges per-file graphs into a project-level call graph
//! - `AnalysisCache` — in-memory caching of parse results

pub mod aggregator;
pub mod cache;
pub mod hot_functions;
pub mod incremental;
pub mod persistent_cache;
pub mod pipeline;
pub mod scanner;
pub mod sieve_cache;

pub use aggregator::Aggregator;
pub use cache::{AnalysisCache, TwoLevelCache};
pub use hot_functions::HotFunctionTracker;
pub use incremental::IncrementalAnalyzer;
pub use persistent_cache::PersistentCache;
pub use pipeline::{Pipeline, PipelineResult};
pub use scanner::FileScanner;
pub use sieve_cache::SieveCache;
