//! JSON output formatter.

use crate::core::{Finding, Reporter};

/// Formats findings as JSON.
pub struct JsonFormatter;

impl Reporter for JsonFormatter {
    fn report(&self, findings: &[Finding]) -> crate::core::Result<String> {
        serde_json::to_string_pretty(findings)
            .map_err(|e| crate::core::Error::analysis(format!("JSON serialization error: {e}")))
    }

    fn format_name(&self) -> &str {
        "json"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Severity, SourceLocation};

    #[test]
    fn test_json_output() {
        let findings = vec![Finding::new(
            "TEST001",
            "Test Finding",
            Severity::Medium,
            SourceLocation::new("test.py".to_string(), 1, 1, 0, 10),
        )];

        let output = JsonFormatter.report(&findings).unwrap();
        assert!(output.contains("TEST001"));
        assert!(output.contains("test.py"));
    }
}
