//! CI/CD check command: fail builds on configurable thresholds.

use std::path::Path;

use serde_json::json;

use crate::ci::{CiRunner, DiffFilter};
use crate::config::FossilConfig;
use crate::core::Error;

/// Run the CI check command.
///
/// Returns a formatted output string. The exit code is determined by the result
/// (0 = pass, 1 = threshold exceeded), which is set via the exit handler in main().
#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &Path,
    diff: Option<&str>,
    max_dead_code: Option<usize>,
    max_clones: Option<usize>,
    max_scaffolding: Option<usize>,
    min_confidence: Option<&str>,
    fail_on_scaffolding: bool,
    config: &FossilConfig,
    format: &str,
    quiet: bool,
) -> Result<String, Error> {
    if !quiet {
        eprintln!("Running Fossil CI check...");
    }

    // Build CI config by merging config file + CLI overrides
    let mut ci_config = config.ci.clone();

    // CLI args override config file
    if let Some(val) = max_dead_code {
        ci_config.max_dead_code = Some(val);
    }
    if let Some(val) = max_clones {
        ci_config.max_clones = Some(val);
    }
    if let Some(val) = max_scaffolding {
        ci_config.max_scaffolding = Some(val);
    }
    if let Some(val) = min_confidence {
        ci_config.min_confidence = Some(val.to_string());
    }
    if fail_on_scaffolding {
        ci_config.fail_on_scaffolding = Some(true);
    }

    // Create optional diff filter
    let diff_filter = if let Some(base_branch) = diff {
        if !quiet {
            eprintln!("Using diff-aware mode (base: {})", base_branch);
        }
        Some(DiffFilter::new(base_branch, path)?)
    } else {
        None
    };

    // Run CI check
    let runner = CiRunner::new(ci_config, config.clone(), diff_filter);
    let result = runner.run(path)?;

    if !quiet {
        eprintln!(
            "Check complete: {} dead code, {} clones, {} scaffolding",
            result.dead_code_count, result.clone_count, result.scaffolding_count
        );
        if !result.violations.is_empty() {
            eprintln!("Found {} threshold violations", result.violations.len());
        }
    }

    // Format output based on format flag
    let output = match format {
        "json" => serde_json::to_string_pretty(&result)
            .map_err(|e| Error::analysis(format!("JSON error: {e}")))?,
        "sarif" => {
            // Create a minimal SARIF structure with findings
            let sarif_json = json!({
                "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
                "version": "2.1.0",
                "runs": [{
                    "tool": {
                        "driver": {
                            "name": "fossil",
                            "version": env!("CARGO_PKG_VERSION"),
                            "informationUri": "https://github.com/yfedoseev/fossil-mcp"
                        }
                    },
                    "results": result.findings.iter().map(|f| json!({
                        "ruleId": f.rule_id,
                        "message": { "text": f.description },
                        "level": match f.severity {
                            crate::core::Severity::Critical | crate::core::Severity::High => "error",
                            crate::core::Severity::Medium => "warning",
                            _ => "note"
                        },
                        "locations": [{
                            "physicalLocation": {
                                "artifactLocation": { "uri": f.location.file },
                                "region": {
                                    "startLine": f.location.line_start,
                                    "endLine": f.location.line_end
                                }
                            }
                        }]
                    })).collect::<Vec<_>>()
                }]
            });

            let sarif_str = serde_json::to_string_pretty(&sarif_json)
                .map_err(|e| Error::analysis(format!("SARIF error: {e}")))?;

            // Add invocations to mark check result
            crate::ci::report::add_sarif_invocations(&sarif_str, result.passed)
                .map_err(|e| Error::analysis(format!("SARIF invocations error: {e}")))?
        }
        _ => crate::ci::format_summary(&result, crate::cli::use_colors()),
    };

    // If thresholds were exceeded, return error which causes exit code 1
    // The output will still be printed by main() since we return it in the error path
    if !result.passed {
        // Still return the output, but as an error to trigger exit code 1
        // We need to encode this specially so main can print it
        eprintln!("{}", output);
        return Err(Error::analysis(format!(
            "CI check failed: {} violations found",
            result.violations.len()
        )));
    }

    // Check passed
    if !quiet {
        eprintln!("✓ All thresholds passed");
    }

    Ok(output)
}
