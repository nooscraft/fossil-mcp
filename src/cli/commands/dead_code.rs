//! Dead code detection command.

use std::path::Path;

use crate::analysis::{CodeGraphStats, Pipeline};
use crate::config::cache::{CacheConfig, CacheStore};
use crate::core::Language;
use crate::dead_code::detector::{Detector, DetectorConfig};

use super::{dead_code_to_findings, format_findings, parse_confidence};

#[allow(clippy::too_many_arguments)]
pub fn run(
    path: &Path,
    include_tests: bool,
    min_confidence: &str,
    min_lines: usize,
    language: Option<&str>,
    format: &str,
    quiet: bool,
    stats: bool,
    cache_dir: Option<&Path>,
    cache_stats: bool,
    diff: Option<&str>,
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

    let detector_config = DetectorConfig {
        include_tests,
        min_confidence: parse_confidence(min_confidence),
        min_lines,
        exclude_patterns: Vec::new(),
        detect_dead_stores: false, // Disabled by default: requires re-parsing all files (expensive)
        use_rta: true,
        use_sdg: false,
        entry_point_rules: Some(rules),
    };

    let detector = Detector::new(detector_config);

    // Run pipeline with differential analysis if --diff is specified, otherwise run full pipeline
    let pipeline_result = if let Some(base_branch) = diff {
        let pipeline = Pipeline::with_defaults();
        pipeline.run_with_diff(path, base_branch, cache_dir)?
    } else {
        let pipeline = Pipeline::with_defaults();
        pipeline.run(path)?
    };

    // Run detection on the built graph
    let result =
        detector.detect_with_parsed_files(&pipeline_result.graph, &pipeline_result.parsed_files)?;

    // Display cache statistics if requested
    if cache_stats {
        if let Some(dir) = cache_dir {
            let config = CacheConfig {
                enabled: true,
                cache_dir: Some(dir.to_string_lossy().to_string()),
                ttl_hours: 168,
            };
            let cache_store = CacheStore::new(&config)?;
            match cache_store.get_stats() {
                Ok(stats) => {
                    eprintln!(
                        "\nCache Statistics:\n  Files: {}\n  Hit Rate: {:.1}%\n  Size: {:.2} MB",
                        stats.total_files,
                        stats.hit_rate(),
                        stats.total_size_mb()
                    );
                }
                Err(e) => {
                    eprintln!("\nCache statistics error: {}", e);
                }
            }
        } else {
            eprintln!("\nCache statistics requested but --cache-dir not specified");
        }
    }

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

    // Compute and print graph statistics if requested (reuse existing pipeline_result)
    if stats {
        let graph_stats = CodeGraphStats::compute(&pipeline_result.graph);
        eprint!("{}", graph_stats.report());
    }

    // Format and output findings (can be expensive for SARIF on large result sets)
    let output = format_findings(&findings, format)?;
    Ok(output)
}
