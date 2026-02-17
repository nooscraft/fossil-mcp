#![forbid(unsafe_code)]

use std::io::IsTerminal;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    // Background update check (non-blocking, once per day)
    // Skip for --help, --version, --quiet, update (redundant), MCP mode, and no-args piped stdin
    let is_interactive_repl = args.len() <= 1 && std::io::stdin().is_terminal();
    let skip_update_check = (!is_interactive_repl && args.len() <= 1)
        || args.iter().skip(1).any(|arg| {
            matches!(
                arg.as_str(),
                "--help" | "-h" | "--version" | "-V" | "--quiet" | "-q" | "update" | "mcp"
            )
        });
    if !skip_update_check && std::env::var("FOSSIL_NO_UPDATE_CHECK").is_err() {
        std::thread::spawn(fossil_mcp::update::check_for_update_background);
        std::thread::spawn(fossil_mcp::cli::commands::weekly_cache::prefetch_weekly_data);
    }

    if args.len() > 1 && args[1] == "mcp" {
        // Explicit MCP mode
        fossil_mcp::mcp::McpServer::new().run().unwrap();
    } else if args.len() <= 1 && !std::io::stdin().is_terminal() {
        // No args + piped stdin = MCP mode (launched by AI tools)
        fossil_mcp::mcp::McpServer::new().run().unwrap();
    } else if args.len() <= 1 {
        // No args + terminal → same as `fossil-mcp scan .`
        fossil_mcp::cli::run_scan_default();
    } else {
        // CLI mode with subcommand
        fossil_mcp::cli::run();
    }
}
