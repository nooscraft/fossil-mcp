//! Clone detection command.

use std::path::Path;

use crate::core::Language;
use crate::clones::detector::{CloneConfig, CloneDetector};

pub fn run(
    path: &Path,
    min_lines: usize,
    similarity: f64,
    types: &str,
    language: Option<&str>,
    format: &str,
    quiet: bool,
) -> Result<String, crate::core::Error> {
    if !quiet {
        eprintln!("Detecting clones in: {}", path.display());
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

    let type_list: Vec<&str> = types.split(',').map(|t| t.trim()).collect();

    let config = CloneConfig {
        min_lines,
        min_nodes: 5,
        similarity_threshold: similarity,
        detect_type1: type_list.contains(&"type1"),
        detect_type2: type_list.contains(&"type2"),
        detect_type3: type_list.contains(&"type3"),
        detect_cross_language: true,
    };

    let detector = CloneDetector::new(config);
    let mut result = detector.detect(path)?;

    // Filter by language if specified
    if let Some(langs) = allowed_languages {
        result.groups.retain_mut(|group| {
            // Keep the clone group if at least one instance matches the language filter
            group.instances.retain(|inst| {
                if let Some(file_lang) = Language::from_file_path(&inst.file) {
                    langs.contains(&file_lang)
                } else {
                    false
                }
            });
            !group.instances.is_empty()
        });

        // Recalculate duplicate line count
        result.total_duplicated_lines = result
            .groups
            .iter()
            .flat_map(|g| g.instances.iter().map(|i| i.end_line - i.start_line + 1))
            .sum();
    }

    if !quiet {
        eprintln!(
            "Analyzed {} files: found {} clone group(s), {} duplicated lines",
            result.files_analyzed,
            result.groups.len(),
            result.total_duplicated_lines,
        );
    }

    // Format output based on format
    match format {
        "json" => {
            let output = serde_json::to_string_pretty(&result.groups)
                .map_err(|e| crate::core::Error::analysis(format!("JSON error: {e}")))?;
            Ok(output)
        }
        _ => {
            if result.groups.is_empty() {
                return Ok("No clones detected.\n".to_string());
            }

            let mut output = String::new();
            output.push_str(&format!(
                "Found {} clone group(s) ({} duplicated lines):\n\n",
                result.groups.len(),
                result.total_duplicated_lines,
            ));

            for (i, group) in result.groups.iter().enumerate() {
                output.push_str(&format!(
                    "{}. {:?} clone (similarity: {:.0}%)\n",
                    i + 1,
                    group.clone_type,
                    group.similarity * 100.0,
                ));
                for inst in &group.instances {
                    output.push_str(&format!(
                        "   {} lines {}-{}\n",
                        inst.file, inst.start_line, inst.end_line,
                    ));
                }
                output.push('\n');
            }

            Ok(output)
        }
    }
}
