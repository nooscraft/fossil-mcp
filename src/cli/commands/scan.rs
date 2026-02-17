//! Combined scan command — runs all analyses with rich dashboard + interactive REPL.

use std::collections::HashMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use crate::cli::C;
use crate::config::FossilConfig;
use crate::core::{Confidence, Finding, Language, Severity, SourceLocation};

use super::{
    dead_code_to_findings, format_findings, parse_confidence, scaffolding_json_to_findings,
};

/// A progress reporter that prints status to stderr.
struct Progress {
    c: C,
}

impl Progress {
    fn new() -> Self {
        Self { c: C::new() }
    }
    fn step(&self, msg: &str) {
        eprintln!("  {} {}", self.c.cyan(">>>"), msg);
    }
    fn done(&self, msg: &str) {
        eprintln!("  {} {}", self.c.green(" ✓ "), msg);
    }
}

/// Build a severity bar: ████░░░░ (filled portion proportional to count/max).
pub(crate) fn severity_bar(count: usize, max: usize, width: usize) -> String {
    if max == 0 {
        return "░".repeat(width);
    }
    let filled = ((count as f64 / max as f64) * width as f64).ceil() as usize;
    let filled = filled.min(width);
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}

/// Detect languages present in findings by file extension.
fn detect_languages(findings: &[Finding]) -> Vec<(Language, usize)> {
    let mut by_lang: HashMap<Language, usize> = HashMap::new();
    for f in findings {
        if let Some(lang) = Language::from_path(std::path::Path::new(&f.location.file)) {
            *by_lang.entry(lang).or_default() += 1;
        }
    }
    let mut langs: Vec<(Language, usize)> = by_lang.into_iter().collect();
    langs.sort_by(|a, b| b.1.cmp(&a.1));
    langs
}

pub fn run(
    path: &Path,
    config: &FossilConfig,
    format: &str,
    quiet: bool,
) -> Result<String, crate::core::Error> {
    // For non-text formats, use the machine-readable pipeline (no dashboard)
    if format != "text" {
        return run_machine_output(path, config, format, quiet);
    }

    let c = C::new();
    let progress = Progress::new();

    if !quiet {
        eprintln!(
            "\n  {} Scanning {}",
            c.bold("FOSSIL"),
            c.white(&path.display().to_string()),
        );
        eprintln!(
            "  {}",
            c.dim("────────────────────────────────────────────────")
        );
    }

    let mut all_findings: Vec<Finding> = Vec::new();
    let mut dead_code_count = 0usize;
    let mut clone_count = 0usize;
    let mut nodes_analyzed = 0usize;
    let mut duplicated_lines = 0usize;

    // ── Dead code analysis ─────────────────────────────────────────
    if config.dead_code.enabled {
        if !quiet {
            progress.step("Analyzing dead code...");
        }

        let rules =
            crate::config::ResolvedEntryPointRules::from_config(&config.entry_points, Some(path));

        let dc_config = crate::dead_code::detector::DetectorConfig {
            include_tests: config.dead_code.include_tests,
            min_confidence: parse_confidence(&config.dead_code.min_confidence),
            min_lines: 0,
            exclude_patterns: config.dead_code.exclude.clone(),
            detect_dead_stores: true,
            use_rta: true,
            use_sdg: false,
            entry_point_rules: Some(rules),
        };

        let detector = crate::dead_code::Detector::new(dc_config);
        match detector.detect(path) {
            Ok(result) => {
                nodes_analyzed = result.total_nodes;
                let findings = dead_code_to_findings(&result.findings);
                dead_code_count = findings.len();
                if !quiet {
                    progress.done(&format!(
                        "{} nodes analyzed, {} unreachable",
                        result.total_nodes, result.unreachable_nodes,
                    ));
                }
                all_findings.extend(findings);
            }
            Err(e) => {
                if !quiet {
                    eprintln!("  {} Dead code analysis failed: {e}", c.red("✗"));
                }
            }
        }
    }

    // ── Clone detection ────────────────────────────────────────────
    if config.clones.enabled {
        if !quiet {
            progress.step("Detecting code clones...");
        }

        let clone_config = crate::clones::detector::CloneConfig {
            min_lines: config.clones.min_lines,
            min_nodes: 5,
            similarity_threshold: config.clones.similarity_threshold,
            detect_type1: config.clones.types.contains(&"type1".to_string()),
            detect_type2: config.clones.types.contains(&"type2".to_string()),
            detect_type3: config.clones.types.contains(&"type3".to_string()),
            detect_cross_language: true,
        };

        let detector = crate::clones::CloneDetector::new(clone_config);
        match detector.detect(path) {
            Ok(result) => {
                duplicated_lines = result.total_duplicated_lines;
                clone_count = result.groups.len();
                if !quiet {
                    progress.done(&format!(
                        "{} files analyzed, {} clone groups",
                        result.files_analyzed,
                        result.groups.len(),
                    ));
                }
                for group in &result.groups {
                    if group.instances.is_empty() {
                        continue;
                    }
                    let primary = &group.instances[0];
                    let location = SourceLocation::new(
                        primary.file.clone(),
                        primary.start_line,
                        primary.end_line,
                        0,
                        0,
                    );
                    let title = primary
                        .function_name
                        .as_deref()
                        .unwrap_or("Code clone")
                        .to_string();
                    let related: Vec<_> = group
                        .instances
                        .iter()
                        .skip(1)
                        .map(|inst| {
                            SourceLocation::new(
                                inst.file.clone(),
                                inst.start_line,
                                inst.end_line,
                                0,
                                0,
                            )
                        })
                        .collect();
                    let finding = Finding::new(
                        format!("CLONE-{:?}", group.clone_type),
                        title,
                        Severity::Low,
                        location,
                    )
                    .with_description(format!(
                        "Duplicated code ({:.0}% similarity, {} instances)",
                        group.similarity * 100.0,
                        group.instances.len(),
                    ))
                    .with_related_locations(related);
                    all_findings.push(finding);
                }
            }
            Err(e) => {
                if !quiet {
                    eprintln!("  {} Clone detection failed: {e}", c.red("✗"));
                }
            }
        }
    }

    // ── Scaffolding detection ──────────────────────────────────────
    let mut scaffolding_count = 0usize;
    {
        if !quiet {
            progress.step("Detecting scaffolding artifacts...");
        }

        let mut args = HashMap::new();
        args.insert(
            "path".to_string(),
            serde_json::Value::String(path.to_string_lossy().to_string()),
        );
        args.insert("include_todos".to_string(), serde_json::Value::Bool(false));
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

        match crate::mcp::tools::scaffolding::execute_detect_scaffolding(&args) {
            Ok(result) => {
                if let Some(parsed) = result
                    .pointer("/content/0/text")
                    .and_then(|v| v.as_str())
                    .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
                {
                    let json_findings = parsed
                        .get("findings")
                        .and_then(|v| v.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let scaffold_findings = scaffolding_json_to_findings(&json_findings);
                    scaffolding_count = scaffold_findings.len();
                    all_findings.extend(scaffold_findings);
                }
                if !quiet {
                    progress.done(&format!("{} scaffolding artifacts", scaffolding_count));
                }
            }
            Err(e) => {
                if !quiet {
                    eprintln!("  {} Scaffolding detection failed: {e}", c.red("✗"));
                }
            }
        }
    }

    if quiet {
        return format_findings(&all_findings, format);
    }

    // ── Dashboard ──────────────────────────────────────────────────
    print_dashboard(
        &c,
        &all_findings,
        dead_code_count,
        clone_count,
        duplicated_lines,
    );

    // ── Interactive REPL ───────────────────────────────────────────
    if std::io::stdin().is_terminal() {
        interactive_repl(
            &c,
            &all_findings,
            dead_code_count,
            clone_count,
            nodes_analyzed,
            path,
        );
    } else {
        // Not a TTY — print next steps and exit
        print_next_steps(&c, dead_code_count, nodes_analyzed, path);
    }

    Ok(String::new())
}

pub(crate) fn print_dashboard(
    c: &C,
    all_findings: &[Finding],
    dead_code_count: usize,
    clone_count: usize,
    duplicated_lines: usize,
) {
    let scaffolding_count = all_findings
        .iter()
        .filter(|f| f.rule_id.starts_with("SCAFFOLD"))
        .count();
    let total = all_findings.len();

    // Count by confidence
    let mut certain_count = 0usize;
    let mut high_count = 0usize;
    for f in all_findings {
        match f.confidence {
            Confidence::Certain => certain_count += 1,
            Confidence::High => high_count += 1,
            _ => {}
        }
    }

    // File hotspots
    let mut by_file: HashMap<&str, usize> = HashMap::new();
    for f in all_findings {
        *by_file.entry(&f.location.file).or_default() += 1;
    }

    // Language breakdown
    let languages = detect_languages(all_findings);

    let bar_max = total;
    let bar_w = 16;

    eprintln!();
    eprintln!(
        "  {}",
        c.bold("══════════════════════════════════════════════════")
    );
    eprintln!(
        "  {}  {}  findings across {} files",
        c.bold("RESULTS"),
        c.white(&total.to_string()),
        c.white(&by_file.len().to_string()),
    );
    eprintln!(
        "  {}",
        c.bold("══════════════════════════════════════════════════")
    );

    // Category breakdown — right-align counts before coloring
    eprintln!();
    let cat_w = [dead_code_count, clone_count, scaffolding_count]
        .iter()
        .map(|n| n.to_string().len())
        .max()
        .unwrap_or(1);
    if dead_code_count > 0 {
        let bar = severity_bar(dead_code_count, bar_max, bar_w);
        let n = format!("{:>w$}", dead_code_count, w = cat_w);
        eprintln!(
            "  {} Dead Code     {}   {}",
            c.yellow("▐"),
            c.white(&n),
            c.yellow(&bar),
        );
    }
    if clone_count > 0 {
        let bar = severity_bar(clone_count, bar_max, bar_w);
        let n = format!("{:>w$}", clone_count, w = cat_w);
        eprintln!(
            "  {} Clones        {}   {}    {}",
            c.cyan("▐"),
            c.white(&n),
            c.cyan(&bar),
            c.dim(&format!("{} duplicated lines", duplicated_lines)),
        );
    }
    if scaffolding_count > 0 {
        let bar = severity_bar(scaffolding_count, bar_max, bar_w);
        let n = format!("{:>w$}", scaffolding_count, w = cat_w);
        eprintln!(
            "  {} Scaffolding   {}   {}",
            c.magenta("▐"),
            c.white(&n),
            c.magenta(&bar),
        );
    }

    // Confidence — right-align all numbers to same width
    if certain_count > 0 || high_count > 0 {
        let other_count = total - certain_count - high_count;
        let cw = [certain_count, high_count, other_count]
            .iter()
            .map(|n| n.to_string().len())
            .max()
            .unwrap_or(1);
        let cs = format!("{:>w$}", certain_count, w = cw);
        let hs = format!("{:>w$}", high_count, w = cw);
        let os = format!("{:>w$}", other_count, w = cw);
        eprintln!();
        eprintln!(
            "  {} {} certain   {} high   {} other",
            c.dim("Confidence:"),
            c.white(&cs),
            c.white(&hs),
            c.dim(&os),
        );
    }

    // Per-language breakdown with colored composition bars
    if !languages.is_empty() {
        // Collect per-language dead code vs clone counts
        let mut lang_dead: HashMap<Language, usize> = HashMap::new();
        let mut lang_clone: HashMap<Language, usize> = HashMap::new();
        for f in all_findings {
            if let Some(lang) = Language::from_path(std::path::Path::new(&f.location.file)) {
                if f.rule_id.starts_with("CLONE") {
                    *lang_clone.entry(lang).or_default() += 1;
                } else if !f.rule_id.starts_with("SCAFFOLD") {
                    *lang_dead.entry(lang).or_default() += 1;
                }
            }
        }

        eprintln!();
        eprintln!("  {}", c.bold("LANGUAGES"));
        eprintln!(
            "  {}",
            c.dim("────────────────────────────────────────────────")
        );

        // Dynamic column widths
        let lang_max = languages[0].1;
        let bar_total = 20;
        let count_w = languages
            .iter()
            .map(|(_, n)| n.to_string().len())
            .max()
            .unwrap_or(1);
        let name_w = languages
            .iter()
            .map(|(l, _)| l.name().len())
            .max()
            .unwrap_or(1);
        let any_dead = languages.iter().any(|(l, _)| lang_dead.contains_key(l));
        let any_clone = languages.iter().any(|(l, _)| lang_clone.contains_key(l));

        for (lang, count) in &languages {
            let dead = *lang_dead.get(lang).unwrap_or(&0);
            let clone = *lang_clone.get(lang).unwrap_or(&0);

            // Build a stacked bar: dead code (yellow █) + clones (cyan █) + empty (░)
            let filled = if lang_max > 0 {
                (*count * bar_total) / lang_max
            } else {
                0
            };
            let dead_chars = if *count > 0 {
                (dead * filled) / *count
            } else {
                0
            };
            let clone_chars = filled - dead_chars;
            let empty_chars = bar_total - filled;

            let dead_bar: String = "█".repeat(dead_chars);
            let clone_bar: String = "█".repeat(clone_chars);
            let empty_bar: String = "░".repeat(empty_chars);

            let bar = format!(
                "{}{}{}",
                c.yellow(&dead_bar),
                c.cyan(&clone_bar),
                c.dim(&empty_bar),
            );

            // Fixed-width percentage columns (right-aligned numbers)
            let dead_pct = if *count > 0 { (dead * 100) / *count } else { 0 };
            let clone_pct = if *count > 0 {
                (clone * 100) / *count
            } else {
                0
            };
            let mut pct = String::new();
            if any_dead {
                if dead > 0 {
                    pct.push_str(&format!("{:>3}% dead", dead_pct));
                } else {
                    pct.push_str("         "); // 9 chars padding
                }
            }
            if any_clone {
                if any_dead {
                    pct.push_str("  ");
                }
                if clone > 0 {
                    pct.push_str(&format!("{:>3}% clones", clone_pct));
                }
            }

            // Pad plain text before coloring to avoid ANSI-breaking alignment
            let name_padded = format!("{:<w$}", lang.name(), w = name_w);
            eprintln!(
                "  {}  {:>cw$}  {}  {}",
                bar,
                count,
                c.white(&name_padded),
                c.dim(&pct),
                cw = count_w,
            );
        }
        eprintln!();
        eprintln!(
            "  {} {} dead code   {} clones   {} scaffolding",
            c.dim("Legend:"),
            c.yellow("██"),
            c.cyan("██"),
            c.magenta("██"),
        );
    }
}

fn print_next_steps(c: &C, dead_code_count: usize, nodes_analyzed: usize, path: &Path) {
    let path_str = path.display().to_string();
    eprintln!();
    eprintln!("  {}", c.bold("NEXT STEPS"));
    eprintln!(
        "  {}",
        c.dim("────────────────────────────────────────────────")
    );
    if dead_code_count > 20 {
        eprintln!("  {}  Focus on high-confidence findings first:", c.dim("▸"),);
        eprintln!(
            "    {}",
            c.cyan(&format!(
                "fossil-mcp dead-code {path_str} --min-confidence high"
            )),
        );
    }
    eprintln!(
        "  {}  Export full SARIF report for IDE integration:",
        c.dim("▸"),
    );
    eprintln!(
        "    {}",
        c.cyan(&format!(
            "fossil-mcp scan {path_str} --format sarif -o fossil-report.sarif"
        )),
    );
    if nodes_analyzed > 0 {
        eprintln!(
            "  {}  Filter small functions from dead code results:",
            c.dim("▸"),
        );
        eprintln!(
            "    {}",
            c.cyan(&format!("fossil-mcp dead-code {path_str} --min-lines 10")),
        );
    }
    eprintln!();
}

/// Parse `[N] [lang]` from REPL command args.
/// Examples: `dead 10`, `dead typescript`, `dead 10 typescript`, `dead 10 rust,python`
fn parse_explore_args(parts: &[&str]) -> (usize, Option<Vec<Language>>) {
    let mut n = 10usize;
    let mut lang_filter: Option<Vec<Language>> = None;

    for part in parts {
        if let Ok(num) = part.parse::<usize>() {
            n = num;
        } else {
            let (langs, _invalid) = Language::parse_list(part);
            if !langs.is_empty() {
                lang_filter = Some(langs);
            }
        }
    }

    (n, lang_filter)
}

/// Print REPL help with consistent column alignment.
fn print_repl_help(c: &C) {
    let cmds: &[(&str, &str)] = &[
        (
            "dead [N] [lang]",
            "Show top N dead code findings (default 10)",
        ),
        (
            "clones [N] [lang]",
            "Show top N clone findings (default 10)",
        ),
        (
            "scaffolding [N] [lang]",
            "Show top N scaffolding findings (default 10)",
        ),
        (
            "hotspots [N] [lang]",
            "Show top N files by finding count (default 5)",
        ),
        ("file <path>", "Show all findings in a specific file"),
        ("export sarif", "Export full SARIF report"),
        ("langs", "Show language breakdown"),
        ("summary", "Re-show dashboard"),
        ("q / exit", "Quit"),
    ];
    let w = cmds.iter().map(|(c, _)| c.len()).max().unwrap_or(20);
    eprintln!();
    for (cmd, desc) in cmds {
        eprintln!(
            "    {}  {}",
            c.cyan(&format!("{:<w$}", cmd, w = w)),
            c.dim(desc),
        );
    }
}

/// Interactive REPL — lets users explore findings without re-running analysis.
fn interactive_repl(
    c: &C,
    all_findings: &[Finding],
    dead_code_count: usize,
    clone_count: usize,
    _nodes_analyzed: usize,
    path: &Path,
) {
    // Partition findings by category, each sorted by confidence (desc) + severity (desc)
    let sort_fn = |a: &&Finding, b: &&Finding| {
        b.confidence
            .cmp(&a.confidence)
            .then_with(|| b.severity.cmp(&a.severity))
    };
    let mut dead: Vec<&Finding> = all_findings
        .iter()
        .filter(|f| !f.rule_id.starts_with("CLONE") && !f.rule_id.starts_with("SCAFFOLD"))
        .collect();
    dead.sort_by(sort_fn);
    let mut clones: Vec<&Finding> = all_findings
        .iter()
        .filter(|f| f.rule_id.starts_with("CLONE"))
        .collect();
    clones.sort_by(sort_fn);
    let mut scaffolding: Vec<&Finding> = all_findings
        .iter()
        .filter(|f| f.rule_id.starts_with("SCAFFOLD"))
        .collect();
    scaffolding.sort_by(sort_fn);

    // File hotspot data
    let mut by_file: HashMap<&str, usize> = HashMap::new();
    for f in all_findings {
        *by_file.entry(&f.location.file).or_default() += 1;
    }
    let mut hotspots: Vec<(&str, usize)> = by_file.iter().map(|(k, v)| (*k, *v)).collect();
    hotspots.sort_by(|a, b| b.1.cmp(&a.1));

    eprintln!();
    eprintln!(
        "  {}",
        c.bold("────────────────────────────────────────────────")
    );
    eprintln!(
        "  {} Type a command to explore results. {} to exit.",
        c.white("Interactive mode."),
        c.dim("q"),
    );
    print_repl_help(c);
    eprintln!(
        "  {}",
        c.bold("────────────────────────────────────────────────")
    );

    let stdin = std::io::stdin();
    let mut reader = stdin.lock();

    loop {
        // Print prompt
        eprint!("\n  {} ", c.green("fossil>"));
        std::io::stderr().flush().ok();

        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        let cmd = parts[0].to_lowercase();
        let arg1 = parts.get(1).copied().unwrap_or("");

        match cmd.as_str() {
            "q" | "quit" | "exit" => break,

            "dead" | "deadcode" | "dead-code" => {
                let (n, lang_filter) = parse_explore_args(&parts[1..]);
                let filtered: Vec<&&Finding> = if let Some(ref langs) = lang_filter {
                    dead.iter()
                        .filter(|f| {
                            Language::from_path(std::path::Path::new(&f.location.file))
                                .is_some_and(|l| langs.contains(&l))
                        })
                        .collect()
                } else {
                    dead.iter().collect()
                };
                let show = n.min(filtered.len());
                if filtered.is_empty() {
                    eprintln!(
                        "  No dead code findings{}.",
                        lang_filter
                            .as_ref()
                            .map(|l| format!(
                                " for {}",
                                l.iter().map(|x| x.name()).collect::<Vec<_>>().join(", ")
                            ))
                            .unwrap_or_default()
                    );
                    continue;
                }
                let label = lang_filter
                    .as_ref()
                    .map(|l| {
                        format!(
                            " [{}]",
                            l.iter().map(|x| x.name()).collect::<Vec<_>>().join(", ")
                        )
                    })
                    .unwrap_or_default();
                eprintln!();
                eprintln!(
                    "  {} {}",
                    c.bold("DEAD CODE"),
                    c.dim(&format!(
                        "(showing {} of {}{})",
                        show,
                        filtered.len(),
                        label
                    )),
                );
                eprintln!(
                    "  {}",
                    c.dim("────────────────────────────────────────────────")
                );
                for (i, f) in filtered.iter().take(show).enumerate() {
                    print_finding(c, i + 1, f);
                }
            }

            "clones" | "clone" => {
                let (n, lang_filter) = parse_explore_args(&parts[1..]);
                let filtered: Vec<&&Finding> = if let Some(ref langs) = lang_filter {
                    clones
                        .iter()
                        .filter(|f| {
                            Language::from_path(std::path::Path::new(&f.location.file))
                                .is_some_and(|l| langs.contains(&l))
                        })
                        .collect()
                } else {
                    clones.iter().collect()
                };
                let show = n.min(filtered.len());
                if filtered.is_empty() {
                    eprintln!(
                        "  No clone findings{}.",
                        lang_filter
                            .as_ref()
                            .map(|l| format!(
                                " for {}",
                                l.iter().map(|x| x.name()).collect::<Vec<_>>().join(", ")
                            ))
                            .unwrap_or_default()
                    );
                    continue;
                }
                let label = lang_filter
                    .as_ref()
                    .map(|l| {
                        format!(
                            " [{}]",
                            l.iter().map(|x| x.name()).collect::<Vec<_>>().join(", ")
                        )
                    })
                    .unwrap_or_default();
                eprintln!();
                eprintln!(
                    "  {} {}",
                    c.bold("CLONES"),
                    c.dim(&format!(
                        "(showing {} of {}{})",
                        show,
                        filtered.len(),
                        label
                    )),
                );
                eprintln!(
                    "  {}",
                    c.dim("────────────────────────────────────────────────")
                );
                for (i, f) in filtered.iter().take(show).enumerate() {
                    print_finding(c, i + 1, f);
                }
            }

            "scaffolding" | "scaffold" => {
                let (n, lang_filter) = parse_explore_args(&parts[1..]);
                let filtered: Vec<&&Finding> = if let Some(ref langs) = lang_filter {
                    scaffolding
                        .iter()
                        .filter(|f| {
                            Language::from_path(std::path::Path::new(&f.location.file))
                                .is_some_and(|l| langs.contains(&l))
                        })
                        .collect()
                } else {
                    scaffolding.iter().collect()
                };
                let show = n.min(filtered.len());
                if filtered.is_empty() {
                    eprintln!(
                        "  No scaffolding findings{}.",
                        lang_filter
                            .as_ref()
                            .map(|l| format!(
                                " for {}",
                                l.iter().map(|x| x.name()).collect::<Vec<_>>().join(", ")
                            ))
                            .unwrap_or_default()
                    );
                    continue;
                }
                let label = lang_filter
                    .as_ref()
                    .map(|l| {
                        format!(
                            " [{}]",
                            l.iter().map(|x| x.name()).collect::<Vec<_>>().join(", ")
                        )
                    })
                    .unwrap_or_default();
                eprintln!();
                eprintln!(
                    "  {} {}",
                    c.bold("SCAFFOLDING"),
                    c.dim(&format!(
                        "(showing {} of {}{})",
                        show,
                        filtered.len(),
                        label
                    )),
                );
                eprintln!(
                    "  {}",
                    c.dim("────────────────────────────────────────────────")
                );
                for (i, f) in filtered.iter().take(show).enumerate() {
                    print_finding(c, i + 1, f);
                }
            }

            "hotspots" | "hotspot" | "hot" => {
                let (n, lang_filter) = parse_explore_args(&parts[1..]);
                // Rebuild hotspots with optional lang filter
                let filtered_findings: Vec<&Finding> = if let Some(ref langs) = lang_filter {
                    all_findings
                        .iter()
                        .filter(|f| {
                            Language::from_path(std::path::Path::new(&f.location.file))
                                .is_some_and(|l| langs.contains(&l))
                        })
                        .collect()
                } else {
                    all_findings.iter().collect()
                };
                let mut hm: HashMap<&str, usize> = HashMap::new();
                for f in &filtered_findings {
                    *hm.entry(&f.location.file).or_default() += 1;
                }
                let mut hs: Vec<(&str, usize)> = hm.into_iter().collect();
                hs.sort_by(|a, b| b.1.cmp(&a.1));
                let show = n.min(hs.len());
                if hs.is_empty() {
                    eprintln!("  No files with findings.");
                    continue;
                }
                let max_count = hs[0].1;
                let hw = hs
                    .iter()
                    .take(show)
                    .map(|(_, n)| n.to_string().len())
                    .max()
                    .unwrap_or(1);
                let label = lang_filter
                    .as_ref()
                    .map(|l| {
                        format!(
                            " [{}]",
                            l.iter().map(|x| x.name()).collect::<Vec<_>>().join(", ")
                        )
                    })
                    .unwrap_or_default();
                eprintln!();
                eprintln!(
                    "  {} {}",
                    c.bold("HOTSPOTS"),
                    c.dim(&format!(
                        "(showing {} of {} files{})",
                        show,
                        hs.len(),
                        label
                    )),
                );
                eprintln!(
                    "  {}",
                    c.dim("────────────────────────────────────────────────")
                );
                for (file, count) in hs.iter().take(show) {
                    let bar = severity_bar(*count, max_count, 12);
                    eprintln!(
                        "  {} {:>w$} findings   {}",
                        c.yellow(&bar),
                        count,
                        c.dim(file),
                        w = hw,
                    );
                }
            }

            "file" | "show" => {
                if arg1.is_empty() {
                    eprintln!("  Usage: {} <path>", c.cyan("file"));
                    continue;
                }
                let matches: Vec<&Finding> = all_findings
                    .iter()
                    .filter(|f| f.location.file.contains(arg1))
                    .collect();
                if matches.is_empty() {
                    eprintln!("  No findings matching '{}'", arg1);
                    continue;
                }
                eprintln!();
                eprintln!("  {} {}", c.bold("FINDINGS IN"), c.white(arg1),);
                eprintln!(
                    "  {}",
                    c.dim("────────────────────────────────────────────────")
                );
                for (i, f) in matches.iter().enumerate() {
                    print_finding(c, i + 1, f);
                }
            }

            "langs" | "languages" | "lang" => {
                let languages = detect_languages(all_findings);
                if languages.is_empty() {
                    eprintln!("  No language data available.");
                    continue;
                }
                let lang_max = languages[0].1;
                let lw = languages
                    .iter()
                    .map(|(_, n)| n.to_string().len())
                    .max()
                    .unwrap_or(1);
                let nw = languages
                    .iter()
                    .map(|(l, _)| l.name().len())
                    .max()
                    .unwrap_or(1);
                eprintln!();
                eprintln!("  {}", c.bold("LANGUAGES"));
                eprintln!(
                    "  {}",
                    c.dim("────────────────────────────────────────────────")
                );
                for (lang, count) in &languages {
                    let bar = severity_bar(*count, lang_max, 10);
                    let name_padded = format!("{:<w$}", lang.name(), w = nw);
                    eprintln!(
                        "  {} {:>cw$}   {}  {}",
                        c.green(&bar),
                        count,
                        c.white(&name_padded),
                        c.dim(
                            &lang
                                .extensions()
                                .first()
                                .map(|e| format!(".{e}"))
                                .unwrap_or_default()
                        ),
                        cw = lw,
                    );
                }
            }

            "summary" | "dashboard" => {
                let dup_lines = all_findings
                    .iter()
                    .filter(|f| f.rule_id.starts_with("CLONE"))
                    .count(); // approximate
                print_dashboard(
                    c,
                    all_findings,
                    dead_code_count,
                    clone_count,
                    dup_lines, // not exact but close enough for re-display
                );
            }

            "export" => {
                let fmt = if arg1.is_empty() { "sarif" } else { arg1 };
                let filename = parts.get(2).copied().unwrap_or("fossil-report.sarif");
                match format_findings(all_findings, fmt) {
                    Ok(output) => match std::fs::write(filename, &output) {
                        Ok(_) => {
                            eprintln!(
                                "  {} Exported {} findings to {}",
                                c.green("✓"),
                                all_findings.len(),
                                c.white(filename),
                            );
                        }
                        Err(e) => eprintln!("  {} Failed to write: {e}", c.red("✗")),
                    },
                    Err(e) => eprintln!("  {} Export failed: {e}", c.red("✗")),
                }
            }

            "help" | "?" => {
                print_repl_help(c);
            }

            _ => {
                eprintln!(
                    "  Unknown command '{}'. Type {} for help.",
                    c.yellow(line),
                    c.cyan("help"),
                );
            }
        }
    }

    let _ = path; // suppress unused warning
}

pub(crate) fn print_finding(c: &C, idx: usize, f: &Finding) {
    let conf_tag = match f.confidence {
        Confidence::Certain => c.green("CERTAIN"),
        Confidence::High => c.yellow("HIGH   "),
        Confidence::Medium => c.dim("MEDIUM "),
        Confidence::Low => c.dim("LOW    "),
    };
    let kind = if f.rule_id.starts_with("CLONE") {
        c.cyan("clone")
    } else if f.rule_id.starts_with("SCAFFOLD") {
        c.magenta("scaffold")
    } else {
        c.yellow("dead")
    };
    eprintln!(
        "  {:>3}. [{}] {} {}",
        idx,
        conf_tag,
        c.white(&f.title),
        c.dim(&format!("({})", kind)),
    );
    if !f.description.is_empty() {
        eprintln!("       {}", c.dim(&f.description));
    }
    eprintln!(
        "       {} {}:{}",
        c.dim("at"),
        c.dim(&f.location.file),
        f.location.line_start,
    );
}

/// Machine-readable output (json, sarif) — no dashboard, just structured data.
fn run_machine_output(
    path: &Path,
    config: &FossilConfig,
    format: &str,
    quiet: bool,
) -> Result<String, crate::core::Error> {
    let mut all_findings = Vec::new();

    if config.dead_code.enabled {
        if !quiet {
            eprintln!("Running dead code analysis...");
        }

        let rules =
            crate::config::ResolvedEntryPointRules::from_config(&config.entry_points, Some(path));

        let dc_config = crate::dead_code::detector::DetectorConfig {
            include_tests: config.dead_code.include_tests,
            min_confidence: parse_confidence(&config.dead_code.min_confidence),
            min_lines: 0,
            exclude_patterns: config.dead_code.exclude.clone(),
            detect_dead_stores: true,
            use_rta: true,
            use_sdg: false,
            entry_point_rules: Some(rules),
        };

        let detector = crate::dead_code::Detector::new(dc_config);
        if let Ok(result) = detector.detect(path) {
            all_findings.extend(dead_code_to_findings(&result.findings));
        }
    }

    if config.clones.enabled {
        if !quiet {
            eprintln!("Running clone detection...");
        }

        let clone_config = crate::clones::detector::CloneConfig {
            min_lines: config.clones.min_lines,
            min_nodes: 5,
            similarity_threshold: config.clones.similarity_threshold,
            detect_type1: config.clones.types.contains(&"type1".to_string()),
            detect_type2: config.clones.types.contains(&"type2".to_string()),
            detect_type3: config.clones.types.contains(&"type3".to_string()),
            detect_cross_language: true,
        };

        let detector = crate::clones::CloneDetector::new(clone_config);
        if let Ok(result) = detector.detect(path) {
            for group in &result.groups {
                if group.instances.is_empty() {
                    continue;
                }
                let primary = &group.instances[0];
                let location = SourceLocation::new(
                    primary.file.clone(),
                    primary.start_line,
                    primary.end_line,
                    0,
                    0,
                );
                let title = primary
                    .function_name
                    .as_deref()
                    .unwrap_or("Code clone")
                    .to_string();
                let related: Vec<_> = group
                    .instances
                    .iter()
                    .skip(1)
                    .map(|inst| {
                        SourceLocation::new(inst.file.clone(), inst.start_line, inst.end_line, 0, 0)
                    })
                    .collect();
                let finding = Finding::new(
                    format!("CLONE-{:?}", group.clone_type),
                    title,
                    Severity::Low,
                    location,
                )
                .with_description(format!(
                    "Duplicated code ({:.0}% similarity, {} instances)",
                    group.similarity * 100.0,
                    group.instances.len(),
                ))
                .with_related_locations(related);
                all_findings.push(finding);
            }
        }
    }

    // Run scaffolding detection
    {
        let mut args = std::collections::HashMap::new();
        args.insert(
            "path".to_string(),
            serde_json::Value::String(path.to_string_lossy().to_string()),
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

        if let Ok(result) = crate::mcp::tools::scaffolding::execute_detect_scaffolding(&args) {
            if let Some(parsed) = result
                .pointer("/content/0/text")
                .and_then(|v| v.as_str())
                .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
            {
                let json_findings = parsed
                    .get("findings")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                all_findings.extend(scaffolding_json_to_findings(&json_findings));
            }
        }
    }

    format_findings(&all_findings, format)
}
