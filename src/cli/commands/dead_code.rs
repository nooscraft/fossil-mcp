//! Dead code detection command.

use std::path::Path;

use crate::core::Language;
use crate::dead_code::detector::{Detector, DetectorConfig};

use super::{dead_code_to_findings, format_findings, parse_confidence};

pub fn run(
    path: &Path,
    include_tests: bool,
    min_confidence: &str,
    min_lines: usize,
    language: Option<&str>,
    format: &str,
    quiet: bool,
) -> Result<String, crate::core::Error> {
    if !quiet {
        eprintln!("Analyzing dead code in: {}", path.display());
    }

    // Parse and validate language filter
    let allowed_languages = if let Some(lang_str) = language {
        let (langs, invalid) = Language::parse_list(lang_str);
        if !invalid.is_empty() {
            return Err(crate::core::Error::analysis(format!(
                "Invalid language(s): {}. Valid options: {}",
                invalid.join(", "),
                Language::all()
                    .iter()
                    .map(|l| l.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        Some(langs)
    } else {
        None
    };

    // Load project config for entry point rules
    let fossil_config = crate::config::FossilConfig::discover(path);
    let rules = crate::config::ResolvedEntryPointRules::from_config(
        &fossil_config.entry_points,
        Some(path),
    );

    let config = DetectorConfig {
        include_tests,
        min_confidence: parse_confidence(min_confidence),
        min_lines,
        exclude_patterns: Vec::new(),
        detect_dead_stores: true,
        use_rta: true,
        use_sdg: false,
        entry_point_rules: Some(rules),
    };

    let detector = Detector::new(config);
    let result = detector.detect(path)?;

    if !quiet {
        eprintln!(
            "Analyzed {} nodes: {} reachable, {} unreachable ({} entry points, {} test entry points)",
            result.total_nodes,
            result.reachable_nodes,
            result.unreachable_nodes,
            result.entry_points,
            result.test_entry_points,
        );
    }

    let mut findings = dead_code_to_findings(&result.findings);

    // Filter by language if specified
    if let Some(langs) = allowed_languages {
        findings.retain(|f| {
            if let Some(file_lang) = Language::from_file_path(&f.location.file) {
                langs.contains(&file_lang)
            } else {
                false
            }
        });
    }

    if !quiet && findings.is_empty() {
        eprintln!("No dead code found.");
    }

    format_findings(&findings, format)
}
