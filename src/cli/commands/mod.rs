pub mod check;
pub mod clones;
pub mod dead_code;
pub mod rules;
pub mod scan;
pub mod update;

use crate::core::{Finding, Severity, SourceLocation};
use crate::output::{create_formatter, OutputFormat};

/// Parse output format string into OutputFormat enum.
pub fn parse_format(format: &str) -> Result<OutputFormat, crate::core::Error> {
    match format.to_lowercase().as_str() {
        "text" => Ok(OutputFormat::Text),
        "json" => Ok(OutputFormat::Json),
        "sarif" => Ok(OutputFormat::Sarif),
        other => Err(crate::core::Error::config(format!(
            "Unknown output format: {other}. Supported: text, json, sarif"
        ))),
    }
}

/// Format findings using the specified output format.
pub fn format_findings(findings: &[Finding], format: &str) -> Result<String, crate::core::Error> {
    let fmt = parse_format(format)?;
    let formatter = create_formatter(fmt);
    formatter.report(findings)
}

/// Parse a severity string.
#[allow(dead_code)]
pub fn parse_severity(s: &str) -> Severity {
    match s.to_lowercase().as_str() {
        "critical" => Severity::Critical,
        "high" => Severity::High,
        "medium" => Severity::Medium,
        "low" => Severity::Low,
        _ => Severity::Info,
    }
}

/// Parse a confidence string.
pub fn parse_confidence(s: &str) -> crate::core::Confidence {
    match s.to_lowercase().as_str() {
        "certain" => crate::core::Confidence::Certain,
        "high" => crate::core::Confidence::High,
        "medium" => crate::core::Confidence::Medium,
        _ => crate::core::Confidence::Low,
    }
}

/// Convert dead code findings to crate::core::Finding for output.
pub fn dead_code_to_findings(
    findings: &[crate::dead_code::classifier::DeadCodeFinding],
) -> Vec<Finding> {
    findings
        .iter()
        .map(|f| {
            let location = SourceLocation::new(f.file.clone(), f.line_start, f.line_end, 0, 0);
            Finding::new(
                format!("DEAD-{}", f.fossil_type),
                f.name.clone(),
                f.severity,
                location,
            )
            .with_description(f.reason.clone())
            .with_confidence(f.confidence)
        })
        .collect()
}
