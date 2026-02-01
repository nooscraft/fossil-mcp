//! Rule engine for Fossil security analysis.
//!
//! Provides YAML-based rule loading, validation, and a built-in rule database.

pub mod database;
pub mod loader;
pub mod semgrep_converter;

pub use database::RuleDatabase;
pub use loader::RuleLoader;
pub use semgrep_converter::SemgrepConverter;
