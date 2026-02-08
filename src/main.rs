#![forbid(unsafe_code)]

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let cli_subcommands = [
        "scan",
        "dead-code",
        "clones",
        "rules",
        "update",
        "mcp",
        "--help",
        "-h",
        "--version",
        "-V",
    ];

    // Background update check (non-blocking, once per day)
    // Skip for --help, --version, update (redundant), and MCP mode (machine-oriented)
    let skip_update_check = args.len() <= 1
        || matches!(
            args[1].as_str(),
            "--help" | "-h" | "--version" | "-V" | "update" | "mcp"
        );
    if !skip_update_check && std::env::var("FOSSIL_NO_UPDATE_CHECK").is_err() {
        std::thread::spawn(fossil_mcp::update::check_for_update_background);
    }

    if args.len() > 1 && cli_subcommands.contains(&args[1].as_str()) {
        if args[1] == "mcp" {
            fossil_mcp::mcp::McpServer::new().run().unwrap();
        } else {
            fossil_mcp::cli::run();
        }
    } else {
        // Default: MCP mode (stdin/stdout JSON-RPC)
        fossil_mcp::mcp::McpServer::new().run().unwrap();
    }
}
