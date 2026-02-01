//! Plain text output formatter.

use crate::core::{Finding, Reporter};

/// Formats findings as human-readable text.
pub struct TextFormatter;

impl Reporter for TextFormatter {
    fn report(&self, findings: &[Finding]) -> crate::core::Result<String> {
        if findings.is_empty() {
            return Ok("No findings.\n".to_string());
        }

        let mut output = String::new();
        output.push_str(&format!("Found {} issue(s):\n\n", findings.len()));

        for (i, f) in findings.iter().enumerate() {
            output.push_str(&format!(
                "{}. [{}] {} ({})\n   {} at {}:{}\n",
                i + 1,
                f.severity,
                f.title,
                f.rule_id,
                f.description,
                f.location.file,
                f.location.line_start,
            ));
            if let Some(ref snippet) = f.code_snippet {
                output.push_str(&format!("   > {}\n", snippet));
            }
            output.push('\n');
        }

        Ok(output)
    }

    fn format_name(&self) -> &str {
        "text"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Severity, SourceLocation};

    #[test]
    fn test_text_output() {
        let findings = vec![Finding::new(
            "TEST001",
            "Test Finding",
            Severity::High,
            SourceLocation::new("test.py".to_string(), 5, 5, 0, 10),
        )
        .with_description("Something is wrong")];

        let output = TextFormatter.report(&findings).unwrap();
        assert!(output.contains("Found 1 issue(s)"));
        assert!(output.contains("[HIGH]"));
        assert!(output.contains("test.py:5"));
    }

    #[test]
    fn test_text_empty() {
        let output = TextFormatter.report(&[]).unwrap();
        assert!(output.contains("No findings"));
    }
}
