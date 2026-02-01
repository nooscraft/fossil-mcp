#![forbid(unsafe_code)]

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let cli_subcommands = [
        "scan",
        "dead-code",
        "clones",
        "rules",
        "mcp",
        "--help",
        "-h",
        "--version",
        "-V",
    ];

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
