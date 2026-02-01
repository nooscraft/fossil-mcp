//! Clone detection command.

use std::path::Path;

use crate::clones::detector::{CloneConfig, CloneDetector};

pub fn run(
    path: &Path,
    min_lines: usize,
    similarity: f64,
    types: &str,
    format: &str,
    quiet: bool,
) -> Result<String, crate::core::Error> {
    if !quiet {
        eprintln!("Detecting clones in: {}", path.display());
    }

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
    let result = detector.detect(path)?;

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
