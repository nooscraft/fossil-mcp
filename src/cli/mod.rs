//! Unified CLI for Fossil code analysis.
//!
//! Subcommands:
//! - `fossil-mcp dead-code [path]` — dead code detection
//! - `fossil-mcp clones [path]` — clone detection
//! - `fossil-mcp scan [path]` — all analyses combined
//! - `fossil-mcp rules list|validate` — rule management

pub mod commands;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

// ANSI color codes
pub(crate) const RED: &str = "\x1b[31m";
pub(crate) const GREEN: &str = "\x1b[32m";
pub(crate) const YELLOW: &str = "\x1b[33m";
pub(crate) const BLUE: &str = "\x1b[34m";
pub(crate) const MAGENTA: &str = "\x1b[35m";
pub(crate) const CYAN: &str = "\x1b[36m";
pub(crate) const WHITE: &str = "\x1b[97m";
pub(crate) const DIM: &str = "\x1b[2m";
pub(crate) const BOLD: &str = "\x1b[1m";
pub(crate) const RESET: &str = "\x1b[0m";

/// Returns whether stderr supports color output.
pub(crate) fn use_colors() -> bool {
    std::env::var("NO_COLOR").is_err()
        && std::env::var("CI").is_err()
        && std::io::stderr().is_terminal()
}

/// Color helper — wraps text in ANSI codes if colors are enabled.
#[allow(dead_code)]
pub(crate) struct C {
    pub enabled: bool,
}

#[allow(dead_code)]
impl C {
    pub fn new() -> Self {
        Self {
            enabled: use_colors(),
        }
    }
    pub fn red(&self, s: &str) -> String {
        if self.enabled {
            format!("{RED}{s}{RESET}")
        } else {
            s.to_string()
        }
    }
    pub fn green(&self, s: &str) -> String {
        if self.enabled {
            format!("{GREEN}{s}{RESET}")
        } else {
            s.to_string()
        }
    }
    pub fn yellow(&self, s: &str) -> String {
        if self.enabled {
            format!("{YELLOW}{s}{RESET}")
        } else {
            s.to_string()
        }
    }
    pub fn cyan(&self, s: &str) -> String {
        if self.enabled {
            format!("{CYAN}{s}{RESET}")
        } else {
            s.to_string()
        }
    }
    pub fn magenta(&self, s: &str) -> String {
        if self.enabled {
            format!("{MAGENTA}{s}{RESET}")
        } else {
            s.to_string()
        }
    }
    pub fn white(&self, s: &str) -> String {
        if self.enabled {
            format!("{WHITE}{s}{RESET}")
        } else {
            s.to_string()
        }
    }
    pub fn dim(&self, s: &str) -> String {
        if self.enabled {
            format!("{DIM}{s}{RESET}")
        } else {
            s.to_string()
        }
    }
    pub fn bold(&self, s: &str) -> String {
        if self.enabled {
            format!("{BOLD}{s}{RESET}")
        } else {
            s.to_string()
        }
    }
}

fn banner_string(use_colors: bool) -> String {
    let (g, c, bl, m, y, re, w, d, b, r) = if use_colors {
        (
            GREEN, CYAN, BLUE, MAGENTA, YELLOW, RED, WHITE, DIM, BOLD, RESET,
        )
    } else {
        ("", "", "", "", "", "", "", "", "", "")
    };

    let version = env!("CARGO_PKG_VERSION");

    // FOSSIL visible widths: lines 0-3 = 41 chars, lines 4-5 = 46 chars.
    // Pad short lines so all = 46, then 6-space gap before bone.
    let fossil = [
        format!("  {g}{b}███████╗{r} {c}{b}██████╗{r} {bl}{b}███████╗{r}{m}{b}███████╗{r}{y}{b}██╗{r}{re}{b}██╗{r}"),
        format!("  {g}{b}██{r}{g}╔════╝{r}{c}{b}██{r}{c}╔═══{r}{c}{b}██{r}{c}╗{r}{bl}{b}██{r}{bl}╔════╝{r}{m}{b}██{r}{m}╔════╝{r}{y}{b}██{r}{y}║{r}{re}{b}██{r}{re}║{r}"),
        format!("  {g}{b}█████╗{r}  {c}{b}██{r}{c}║   {r}{c}{b}██{r}{c}║{r}{bl}{b}███████╗{r}{m}{b}███████╗{r}{y}{b}██{r}{y}║{r}{re}{b}██{r}{re}║{r}"),
        format!("  {g}{b}██{r}{g}╔══╝{r}  {c}{b}██{r}{c}║   {r}{c}{b}██{r}{c}║{r}{bl}╚════{r}{bl}{b}██{r}{bl}║{r}{m}╚════{r}{m}{b}██{r}{m}║{r}{y}{b}██{r}{y}║{r}{re}{b}██{r}{re}║{r}"),
        format!("  {g}{b}██{r}{g}║{r}     {c}╚{r}{c}{b}██████{r}{c}╔╝{r}{bl}{b}███████{r}{bl}║{r}{m}{b}███████{r}{m}║{r}{y}{b}██{r}{y}║{r}{re}{b}███████{r}{re}╗{r}"),
        format!("  {d}╚═╝      ╚═════╝ ╚══════╝╚══════╝╚═╝╚══════╝{r}"),
    ];
    let pad = [5, 5, 5, 5, 0, 0]; // extra spaces to equalize to 46 visible
    let gap = 6;
    let bone = [
        format!("{d}()    (){r}"),
        format!("{d} \\    /{r}"),
        format!("{d}  |  |{r}"),
        format!("{d}  |  |{r}"),
        format!("{d} /    \\{r}"),
        format!("{d}()    (){r}"),
    ];

    let mut out = String::from("\n");
    for i in 0..6 {
        out.push_str(&fossil[i]);
        for _ in 0..(pad[i] + gap) {
            out.push(' ');
        }
        out.push_str(&bone[i]);
        out.push('\n');
    }
    out.push_str(&format!(
        "  {d}~~  ~~ ~  ~~  ~  ~~ ~  ~~  ~  ~~  ~  ~~  ~  ~~ ~  ~~  ~  ~~{r}\n"
    ));
    out.push_str(&format!(
        "{w}{b}  Dig up dead code. Unearth clones. Expose scaffolding.{r}\n"
    ));
    out.push_str(&format!(
        "{d}  ────────────────────────────────────────────────────────────────{r}\n"
    ));
    out.push_str(&format!("{d}  Version:{r} {y}{version}{r}    {d}Languages:{r} {w}16{r}    {d}Analyses:{r} {w}dead code · clones · scaffolding · temp files{r}\n"));
    out.push_str(&format!(
        "{d}  ────────────────────────────────────────────────────────────────{r}\n"
    ));
    out
}

/// Print the Fossil banner to stderr.
pub fn print_banner() {
    let colors = use_colors();
    eprint!("{}", banner_string(colors));
}

const HELP_BANNER: &str = r#"
  ███████╗ ██████╗ ███████╗███████╗██╗██╗           ()    ()
  ██╔════╝██╔═══██╗██╔════╝██╔════╝██║██║            \    /
  █████╗  ██║   ██║███████╗███████╗██║██║              |  |
  ██╔══╝  ██║   ██║╚════██║╚════██║██║██║              |  |
  ██║     ╚██████╔╝███████║███████║██║███████╗        /    \
  ╚═╝      ╚═════╝ ╚══════╝╚══════╝╚═╝╚══════╝      ()    ()
  ~~  ~~ ~  ~~  ~  ~~ ~  ~~  ~  ~~  ~  ~~  ~  ~~ ~  ~~  ~  ~~
  Dig up dead code. Unearth clones. Expose scaffolding.
"#;

/// Multi-language static analysis toolkit with MCP server.
/// Detects dead code, code clones, and AI scaffolding.
#[derive(Parser)]
#[command(name = "fossil-mcp", version, about, long_about = None)]
#[command(propagate_version = true)]
#[command(before_help = HELP_BANNER)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Output format: text, json, sarif
    #[arg(long, global = true, default_value = "text")]
    format: String,

    /// Output file (stdout if not specified)
    #[arg(short, long, global = true)]
    output: Option<PathBuf>,

    /// Suppress all non-error output
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Path to config file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    /// Detect dead (unreachable) code
    #[command(name = "dead-code")]
    DeadCode {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Include test-only code in results
        #[arg(long)]
        include_tests: bool,

        /// Minimum confidence level: low, medium, high, certain
        #[arg(long, default_value = "low")]
        min_confidence: String,

        /// Minimum lines of code for a finding to be reported
        #[arg(long, default_value = "0")]
        min_lines: usize,

        /// Filter by programming language(s): rust, python, typescript, etc.
        /// Use comma-separated list for multiple: rust,python,go
        #[arg(long)]
        language: Option<String>,

        /// Print graph statistics (cardinality estimates using HyperLogLog)
        #[arg(long)]
        stats: bool,

        /// Cache directory for persistent analysis results
        #[arg(long)]
        cache_dir: Option<PathBuf>,

        /// Print cache statistics (hit rate, memory usage)
        #[arg(long)]
        cache_stats: bool,

        /// Analyze only changed files (requires git repository)
        /// Provide base branch name (e.g., --diff main, --diff develop)
        #[arg(long, value_name = "BASE_BRANCH")]
        diff: Option<String>,
    },

    /// Detect code clones (duplicated code)
    Clones {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Minimum lines for a clone
        #[arg(long, default_value = "6")]
        min_lines: usize,

        /// Similarity threshold (0.0-1.0) for Type 3 clones
        #[arg(long, default_value = "0.8")]
        similarity: f64,

        /// Clone types to detect: type1, type2, type3 (comma-separated)
        #[arg(long, default_value = "type1,type2,type3")]
        types: String,

        /// Filter by programming language(s): rust, python, typescript, etc.
        /// Use comma-separated list for multiple: rust,python,go
        #[arg(long)]
        language: Option<String>,
    },

    /// Run all analyses (dead code + clones)
    Scan {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },

    /// CI/CD mode: fail builds on configurable thresholds
    #[command(name = "check")]
    Check {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Only check files changed vs base branch (enables diff-aware mode)
        /// Example: --diff main or --diff origin/main
        #[arg(long)]
        diff: Option<String>,

        /// Override max dead code threshold from config
        #[arg(long)]
        max_dead_code: Option<usize>,

        /// Override max clones threshold from config
        #[arg(long)]
        max_clones: Option<usize>,

        /// Override max scaffolding threshold from config
        #[arg(long)]
        max_scaffolding: Option<usize>,

        /// Minimum confidence level for counting findings (low, medium, high, certain)
        #[arg(long)]
        min_confidence: Option<String>,

        /// Fail if any scaffolding artifacts found
        #[arg(long)]
        fail_on_scaffolding: bool,
    },

    /// Detect AI scaffolding artifacts (placeholders, phased comments, temp files)
    Scaffolding {
        /// Path to analyze (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Filter by programming language(s): rust, python, typescript, etc.
        /// Use comma-separated list for multiple: rust,python,go
        #[arg(long)]
        language: Option<String>,

        /// Include TODO/FIXME markers in results
        #[arg(long)]
        include_todos: bool,
    },

    /// Manage security rules
    Rules {
        #[command(subcommand)]
        action: RulesAction,
    },

    /// Update fossil-mcp to the latest version
    Update {
        /// Check for updates without installing
        #[arg(long)]
        check: bool,
    },

    /// Show weekly AI slop rankings
    Weekly {
        /// Show detailed breakdown per project
        #[arg(long)]
        detailed: bool,
    },
}

#[derive(Subcommand)]
enum RulesAction {
    /// List all available rules
    List,
    /// Validate rule files
    Validate {
        /// Path to rules directory
        path: PathBuf,
    },
}

/// No-args terminal mode — same as `fossil-mcp scan .`
pub fn run_scan_default() {
    // Initialize tracing (errors only); ignore if already set
    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new("error"))
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();

    print_banner();

    let path = PathBuf::from(".");
    let config =
        crate::config::FossilConfig::discover(&path.canonicalize().unwrap_or(path.clone()));

    match commands::scan::run(&path, &config, "text", false) {
        Ok(_) => {}
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    }
}

pub fn run() {
    let cli = Cli::parse();

    if !cli.quiet {
        print_banner();
    }

    // Set up tracing — suppress by default for clean output, enable with --verbose
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("error")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();

    // Load config
    let mut config = if let Some(ref config_path) = cli.config {
        match crate::config::FossilConfig::load(config_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Error loading config: {e}");
                process::exit(1);
            }
        }
    } else {
        crate::config::FossilConfig::discover(&std::env::current_dir().unwrap_or_default())
    };
    config.apply_env_overrides();

    let result = match cli.command {
        Commands::DeadCode {
            path,
            include_tests,
            min_confidence,
            min_lines,
            language,
            stats,
            cache_dir,
            cache_stats,
            diff,
        } => commands::dead_code::run(
            &path,
            include_tests,
            &min_confidence,
            min_lines,
            language.as_deref(),
            &cli.format,
            cli.quiet,
            stats,
            cache_dir.as_deref(),
            cache_stats,
            diff.as_deref(),
        ),

        Commands::Clones {
            path,
            min_lines,
            similarity,
            types,
            language,
        } => commands::clones::run(
            &path,
            min_lines,
            similarity,
            &types,
            language.as_deref(),
            &cli.format,
            cli.quiet,
        ),

        Commands::Scan { path } => commands::scan::run(&path, &config, &cli.format, cli.quiet),

        Commands::Scaffolding {
            path,
            language,
            include_todos,
        } => commands::scaffolding::run(
            &path,
            language.as_deref(),
            include_todos,
            &cli.format,
            cli.quiet,
        ),

        Commands::Check {
            path,
            diff,
            max_dead_code,
            max_clones,
            max_scaffolding,
            min_confidence,
            fail_on_scaffolding,
        } => commands::check::run(
            &path,
            diff.as_deref(),
            max_dead_code,
            max_clones,
            max_scaffolding,
            min_confidence.as_deref(),
            fail_on_scaffolding,
            &config,
            &cli.format,
            cli.quiet,
        ),

        Commands::Rules { action } => match action {
            RulesAction::List => commands::rules::list(),
            RulesAction::Validate { path } => commands::rules::validate(&path),
        },

        Commands::Update { check } => commands::update::run(check),

        Commands::Weekly { detailed } => commands::weekly::run(detailed),
    };

    match result {
        Ok(output) => {
            if let Some(ref output_path) = cli.output {
                if let Err(e) = std::fs::write(output_path, &output) {
                    eprintln!("Error writing output: {e}");
                    process::exit(1);
                }
            } else if !output.is_empty() {
                print!("{output}");
            }
        }
        Err(e) => {
            eprintln!("Error: {e}");
            process::exit(1);
        }
    }
}
