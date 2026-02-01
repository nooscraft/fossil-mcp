//! Dead code detection with entry point analysis and reachability.
//!
//! Provides:
//! - `Detector` — orchestrates entry point detection, reachability, and classification
//! - `EntryPointDetector` — identifies production and test entry points
//! - `ReachabilityAnalyzer` — BFS from entry points
//! - `DeadCodeClassifier` — classifies and scores dead code findings

pub mod classifier;
pub mod detector;
pub mod entry_points;
pub mod feature_flags;

pub use classifier::DeadCodeClassifier;
pub use detector::Detector;
pub use entry_points::EntryPointDetector;
pub use feature_flags::FeatureFlagDetector;
