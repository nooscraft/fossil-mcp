//! Unified output formatters: SARIF 2.1.0, JSON, text.

pub mod json;
pub mod sarif;
pub mod text;

pub use json::JsonFormatter;
pub use sarif::SarifFormatter;
pub use text::TextFormatter;

use crate::core::Reporter;

/// Output format selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Sarif,
    Json,
    Text,
}

/// Create a formatter for the given format.
pub fn create_formatter(format: OutputFormat) -> Box<dyn Reporter> {
    match format {
        OutputFormat::Sarif => Box::new(SarifFormatter::new()),
        OutputFormat::Json => Box::new(JsonFormatter),
        OutputFormat::Text => Box::new(TextFormatter),
    }
}
