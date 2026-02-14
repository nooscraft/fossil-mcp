//! Shared analysis infrastructure for Fossil.
//!
//! Provides:
//! - `FileScanner` — discovers source files respecting .gitignore
//! - `Pipeline` — orchestrates parsing and graph building with parallel execution
//! - `Aggregator` — merges per-file graphs into a project-level call graph
//! - `AnalysisCache` — in-memory caching of parse results

pub mod aggregator;
pub mod cache;
pub mod diff_analyzer;
pub mod hot_functions;
pub mod incremental;
pub mod persistent_cache;
pub mod pipeline;
pub mod scanner;
pub mod sieve_cache;
pub mod stats;
pub mod stress_test;

pub use aggregator::Aggregator;
pub use cache::{AnalysisCache, TwoLevelCache};
pub use diff_analyzer::{DiffInfo, DependentAnalyzer};
pub use hot_functions::HotFunctionTracker;
pub use incremental::IncrementalAnalyzer;
pub use persistent_cache::PersistentCache;
pub use pipeline::{Pipeline, PipelineResult, get_rss_mb};
pub use scanner::FileScanner;
pub use sieve_cache::SieveCache;
pub use stats::CodeGraphStats;
pub use stress_test::{StressTestRunner, StressTestResult};
