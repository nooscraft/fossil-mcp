use crate::cli::C;
use serde::Deserialize;

#[derive(Deserialize)]
struct WeeklyData {
    week: String,
    theme: String,
    projects: Vec<ProjectRanking>,
    key_insights: Vec<Insight>,
}

#[derive(Deserialize)]
struct ProjectRanking {
    rank: u32,
    name: String,
    github_stars: String,
    metrics: Metrics,
    slop_score: SlopScore,
    tier: String,
    key_finding: String,
}

#[derive(Deserialize)]
struct Metrics {
    dead_code: MetricDetail,
    clones: MetricDetail,
    scaffolding: ScaffoldingDetail,
}

#[derive(Deserialize)]
struct MetricDetail {
    count: u32,
}

#[derive(Deserialize)]
struct ScaffoldingDetail {
    total: u32,
}

#[derive(Deserialize)]
struct SlopScore {
    total: u32,
}

#[derive(Deserialize)]
struct Insight {
    title: String,
    finding: String,
}

pub fn run(detailed: bool) -> Result<String, crate::core::Error> {
    let c = C::new();
    let mut output = String::new();

    let json_str = super::weekly_cache::load_weekly_json().ok_or_else(|| {
        crate::core::Error::config(
            "Failed to fetch weekly data. Check your internet connection.".to_string(),
        )
    })?;

    let data: WeeklyData = serde_json::from_str(&json_str)
        .map_err(|e| crate::core::Error::config(format!("Failed to parse weekly data: {}", e)))?;

    // Build header
    output.push('\n');
    output.push_str(&format!(
        "  {}  {}\n",
        c.bold("WEEKLY ANALYSIS"),
        c.cyan(&data.week)
    ));
    let header_line = "═".repeat(70);
    output.push_str(&format!("  {}\n", c.dim(&header_line)));
    output.push_str(&format!("  {}: {}\n\n", c.dim("Theme"), &data.theme));

    output.push_str(&format!(
        "  {}\n\n",
        c.bold("TOP 10 TRENDING PROJECTS BY SLOP SCORE")
    ));

    // Format each project
    for project in &data.projects {
        let (tier_color, tier_emoji) = match project.tier.as_str() {
            "critical" => ("red", "🔴"),
            "high" => ("yellow", "🟠"),
            "medium" => ("yellow", "🟡"),
            _ => ("green", "🟢"),
        };

        let score_str = format!("{}", project.slop_score.total);
        let colored_score = match tier_color {
            "red" => c.red(&score_str),
            "yellow" => c.yellow(&score_str),
            _ => c.green(&score_str),
        };

        output.push_str(&format!(
            "  {}  {:<20} {}  {}: {}  {} {}\n",
            c.yellow(format!("#{}", project.rank).as_str()),
            c.cyan(&project.name),
            c.dim(format!("{} ★", project.github_stars).as_str()),
            c.dim("Slop"),
            colored_score,
            tier_emoji,
            project.tier.to_uppercase()
        ));

        output.push_str(&format!(
            "      {}  {}  {}  {}  {}\n",
            c.dim(format!("Dead: {}", project.metrics.dead_code.count).as_str()),
            c.dim(format!("Clones: {}", project.metrics.clones.count).as_str()),
            c.dim(format!("Scaffolding: {}", project.metrics.scaffolding.total).as_str()),
            "",
            ""
        ));

        // Wrap key_finding at 75 chars
        let mut current_line = String::new();
        for word in project.key_finding.split_whitespace() {
            if current_line.len() + word.len() + 1 > 75 && !current_line.is_empty() {
                output.push_str(&format!("      {} {}\n", c.dim("→"), c.dim(&current_line)));
                current_line.clear();
            }
            if !current_line.is_empty() {
                current_line.push(' ');
            }
            current_line.push_str(word);
        }
        if !current_line.is_empty() {
            output.push_str(&format!(
                "      {} {}\n\n",
                c.dim("→"),
                c.dim(&current_line)
            ));
        }

        if detailed {
            output.push_str(&format!(
                "      {}  Dead Code: {} functions (call_graph_reachability, high confidence)\n",
                c.dim("├─"),
                project.metrics.dead_code.count
            ));
            output.push_str(&format!(
                "      {}  Clones: {} instances (MinHash + LSH, high confidence)\n",
                c.dim("├─"),
                project.metrics.clones.count
            ));
            output.push_str(&format!(
                "      {}  Scaffolding: {} markers (pattern-based, very_high confidence)\n\n",
                c.dim("└─"),
                project.metrics.scaffolding.total
            ));
        }
    }

    // Key insights section
    output.push_str(&format!("  {}\n", c.bold("KEY INSIGHTS")));
    let insights_line = "─".repeat(70);
    output.push_str(&format!("  {}\n", c.dim(&insights_line)));

    for (i, insight) in data.key_insights.iter().enumerate() {
        output.push_str(&format!("  {} {}\n", c.green("•"), c.bold(&insight.title)));

        // Wrap finding text at 75 chars
        let mut current_line = String::new();
        for word in insight.finding.split_whitespace() {
            if current_line.len() + word.len() + 1 > 75 && !current_line.is_empty() {
                output.push_str(&format!("    {}\n", c.dim(&current_line)));
                current_line.clear();
            }
            if !current_line.is_empty() {
                current_line.push(' ');
            }
            current_line.push_str(word);
        }
        if !current_line.is_empty() {
            output.push_str(&format!("    {}\n", c.dim(&current_line)));
        }

        if i < data.key_insights.len() - 1 {
            output.push('\n');
        }
    }

    output.push('\n');
    if !detailed {
        output.push_str(&format!(
            "  {}\n",
            c.dim("Run 'fossil-mcp weekly --detailed' for full breakdown")
        ));
    }
    output.push('\n');

    Ok(output)
}
