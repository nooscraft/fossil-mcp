//! CLI command: `fossil-mcp scaffolding [path]`

use std::collections::HashMap;
use std::path::Path;

use crate::cli::C;
use crate::core::{Finding, Language};

use super::{format_findings, scaffolding_json_to_findings};

pub fn run(
    path: &Path,
    language: Option<&str>,
    include_todos: bool,
    format: &str,
    quiet: bool,
) -> Result<String, crate::core::Error> {
    let c = C::new();

    if !quiet {
        eprintln!(
            "\n  {} Detecting scaffolding in {}",
            c.bold("FOSSIL"),
            c.white(&path.display().to_string()),
        );
        eprintln!(
            "  {}",
            c.dim("────────────────────────────────────────────────")
        );
    }

    let mut args = HashMap::new();
    args.insert(
        "path".to_string(),
        serde_json::Value::String(path.to_string_lossy().to_string()),
    );
    args.insert(
        "include_todos".to_string(),
        serde_json::Value::Bool(include_todos),
    );
    args.insert(
        "include_placeholders".to_string(),
        serde_json::Value::Bool(true),
    );
    args.insert(
        "include_phased_comments".to_string(),
        serde_json::Value::Bool(true),
    );
    args.insert(
        "include_temp_files".to_string(),
        serde_json::Value::Bool(true),
    );
    args.insert(
        "limit".to_string(),
        serde_json::Value::Number(serde_json::Number::from(10000)),
    );

    let result = crate::mcp::tools::scaffolding::execute_detect_scaffolding(&args)
        .map_err(crate::core::Error::analysis)?;

    let parsed = result
        .pointer("/content/0/text")
        .and_then(|v| v.as_str())
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .ok_or_else(|| crate::core::Error::analysis("Failed to parse scaffolding results"))?;

    let json_findings = parsed
        .get("findings")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut findings = scaffolding_json_to_findings(&json_findings);

    // Apply language filter
    if let Some(lang_str) = language {
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
        if !langs.is_empty() {
            findings.retain(|f| {
                Language::from_path(std::path::Path::new(&f.location.file))
                    .is_some_and(|l| langs.contains(&l))
            });
        }
    }

    if !quiet {
        eprintln!(
            "  {} {} scaffolding artifacts found",
            c.green("✓"),
            findings.len(),
        );
    }

    if format != "text" {
        return format_findings(&findings, format);
    }

    // Text output
    if findings.is_empty() {
        if !quiet {
            eprintln!("\n  {} No scaffolding artifacts found.", c.green("✓"));
        }
        return Ok(String::new());
    }

    // Group by category
    let mut by_category: HashMap<String, Vec<&Finding>> = HashMap::new();
    for f in &findings {
        let cat = f
            .rule_id
            .strip_prefix("SCAFFOLD-")
            .unwrap_or(&f.rule_id)
            .to_string();
        by_category.entry(cat).or_default().push(f);
    }

    let mut categories: Vec<(String, Vec<&Finding>)> = by_category.into_iter().collect();
    categories.sort_by(|a, b| b.1.len().cmp(&a.1.len()));

    eprintln!();
    for (cat, items) in &categories {
        let label = cat.replace('_', " ");
        eprintln!("  {} {} ({})", c.magenta("▐"), c.white(&label), items.len(),);
        for f in items.iter().take(20) {
            eprintln!(
                "    {} {}:{}  {}",
                c.dim("·"),
                c.dim(&f.location.file),
                f.location.line_start,
                c.white(&f.title),
            );
        }
        if items.len() > 20 {
            eprintln!("    {} ... and {} more", c.dim("·"), items.len() - 20);
        }
        eprintln!();
    }

    Ok(String::new())
}
