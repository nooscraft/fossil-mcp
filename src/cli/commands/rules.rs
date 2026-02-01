//! Rule management commands.

use std::path::Path;

use crate::rules::{RuleDatabase, RuleLoader};

/// List all available rules.
pub fn list() -> Result<String, crate::core::Error> {
    let db = RuleDatabase::with_defaults();
    let rules = db.all_rules();

    let mut output = format!("Available rules ({} total):\n\n", rules.len());

    for rule in rules {
        let langs: String = rule
            .languages
            .iter()
            .map(|l| l.to_string())
            .collect::<Vec<_>>()
            .join(", ");

        output.push_str(&format!(
            "  {:<20} [{}] {}\n",
            rule.id, rule.severity, rule.name,
        ));
        if !langs.is_empty() {
            output.push_str(&format!("  {:<20} Languages: {}\n", "", langs));
        }
        output.push_str(&format!("  {:<20} {}\n\n", "", rule.description));
    }

    Ok(output)
}

/// Validate rule files in a directory.
pub fn validate(path: &Path) -> Result<String, crate::core::Error> {
    let mut valid = 0;
    let mut invalid = 0;
    let mut output = String::new();

    let entries = std::fs::read_dir(path)
        .map_err(|e| crate::core::Error::analysis(format!("Cannot read rules dir: {e}")))?;

    for entry in entries.flatten() {
        let file_path = entry.path();
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let result = match ext {
            "yml" | "yaml" => RuleLoader::load_yaml_file(&file_path),
            "json" => RuleLoader::load_json_file(&file_path),
            _ => continue,
        };

        match result {
            Ok(rules) => {
                output.push_str(&format!(
                    "  OK: {} ({} rules)\n",
                    file_path.display(),
                    rules.len(),
                ));
                valid += rules.len();
            }
            Err(e) => {
                output.push_str(&format!("  FAIL: {} — {}\n", file_path.display(), e,));
                invalid += 1;
            }
        }
    }

    output.insert_str(
        0,
        &format!("Rule validation: {} valid, {} invalid\n\n", valid, invalid,),
    );

    Ok(output)
}
