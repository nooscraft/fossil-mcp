# Fossil MCP

**The code quality toolkit for the vibe coding era.**

Static analysis that finds the mess vibe coding leaves behind — dead code, duplicated logic, scaffolding artifacts, and disconnected functions — across 15 languages.

**[fossil-mcp.com](https://fossil-mcp.com)**

[![CI](https://github.com/yfedoseev/fossil-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/yfedoseev/fossil-mcp/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/fossil-mcp.svg)](https://crates.io/crates/fossil-mcp)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

[![Install in VS Code](https://img.shields.io/badge/VS_Code-Install_Server-0098FF?style=flat-square&logo=visualstudiocode&logoColor=white)](https://insiders.vscode.dev/redirect/mcp/install?name=fossil&config=%7B%22command%22%3A%22fossil-mcp%22%7D)
[![Install in VS Code Insiders](https://img.shields.io/badge/VS_Code_Insiders-Install_Server-24bfa5?style=flat-square&logo=visualstudiocode&logoColor=white)](https://insiders.vscode.dev/redirect/mcp/install?name=fossil&config=%7B%22command%22%3A%22fossil-mcp%22%7D&quality=insiders)
[![Install in Cursor](https://cursor.com/deeplink/mcp-install-dark.svg)](https://cursor.com/en/install-mcp?name=fossil&config=eyJjb21tYW5kIjoiZm9zc2lsLW1jcCJ9)

---

## The Problem

AI-assisted coding is fine — you review the code, you understand the architecture, you stay in control. **Vibe coding is different.** You describe what you want, the AI writes it, and you ship it without reading every line. Tools like [Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Cursor](https://cursor.com), [GitHub Copilot](https://github.com/features/copilot), and [Windsurf](https://windsurf.com) make this workflow fast and productive. But over days and weeks, vibe-coded projects accumulate a specific class of problems that traditional linters don't catch:

**Dead code piles up fast.** When the AI refactors a function, it writes the new version but often forgets to remove the old one. You don't notice because you didn't read the diff line by line. Over multiple sessions, unused functions, unreachable branches, and orphaned utilities pile up — the codebase grows but nothing gets pruned. A [METR study](https://metr.org/) found developers spend significant time checking and debugging AI output. Dead code makes this exponentially harder.

**Duplication spreads silently.** Each AI session has limited context window. It generates a utility function that already exists elsewhere, or solves the same problem with a slightly different implementation three files over. You asked for a feature, it works, so you move on. Traditional duplicate detection focuses on copy-paste — vibe coding duplication is structural: similar logic, different names, scattered across modules.

**`// Phase 1`, `// TODO`, `// Step 2` — everywhere.** AI agents work in phases. They leave behind scaffolding markers that were meant to be temporary: `// Phase 1: Setup`, `// TODO: implement error handling`, placeholder function bodies with `pass` or `todo!()`, and phased naming like `process_data_v2`. In vibe coding, nobody goes back to clean these up. They become permanent fixtures.

**Functions exist that nothing calls.** This is the vibe coding signature. The AI writes a helper function, uses it, then in a later session rewrites the caller to use a different approach — but the helper stays. Without a call graph, neither you nor the AI can tell which functions are actually connected to the rest of the codebase. Current AI coding tools navigate code by text search, not by understanding how functions call each other.

**Temp files accumulate in the repo.** AI sessions create `temp_`, `backup_`, `old_`, `phase_1_` files and directories. In vibe coding, you don't audit your file tree after each session. These artifacts persist across commits.

## The Solution

Fossil MCP is a static analysis toolkit purpose-built for vibe-coded projects. It detects the artifacts that accumulate when AI writes most of the code — and it works both as a **CLI tool** for developers and as an **MCP server** that gives AI agents a code graph instead of just text search.

```
  ███████╗ ██████╗ ███████╗███████╗██╗██╗           ()    ()
  ██╔════╝██╔═══██╗██╔════╝██╔════╝██║██║            \    /
  █████╗  ██║   ██║███████╗███████╗██║██║             |  |
  ██╔══╝  ██║   ██║╚════██║╚════██║██║██║             |  |
  ██║     ╚██████╔╝███████║███████║██║███████╗       /    \
  ╚═╝      ╚═════╝ ╚══════╝╚══════╝╚═╝╚══════╝      ()    ()
  Dig up dead code. Unearth clones. Expose scaffolding.
```

### What Fossil Detects

| Analysis | What it finds | The vibe coding problem |
|----------|--------------|------------------------|
| **Dead Code** | Unreachable functions, unused exports, orphaned methods | AI rewrites a caller but forgets to delete the old helper — nobody notices |
| **Code Clones** | Type 1 (exact), Type 2 (renamed), Type 3 (structural) duplicates | Each AI session reinvents utilities that already exist elsewhere in the codebase |
| **Scaffolding** | `Phase N` / `Step N` comments, `TODO`/`FIXME` markers, placeholder bodies | AI works in phases and leaves temporary markers that never get cleaned up |
| **Temp Files** | `temp_*`, `backup_*`, `old_*`, `phase_*` files and directories | Session artifacts that persist because nobody audits the file tree |
| **Code Graph** | Trace paths between any two functions, blast radius analysis, call graph traversal | AI tools navigate by text search — Fossil gives them a graph to trace how functions connect and what breaks if you change one |

### What Makes Fossil Different

- **Purpose-built for vibe coding.** Not a general linter — specifically targets the mess that accumulates when AI writes most of the code and humans review less of it.
- **Graph, not grep.** AI coding tools navigate code by searching for text. Fossil builds a call graph and lets agents trace how functions connect, find blast radius before refactoring, and discover dead ends — without reading every file.
- **MCP-native.** Runs as an MCP server so AI agents can self-check their output during development.
- **Saves tokens, saves money.** Instead of an agent scanning files over and over to find issues, Fossil identifies dead code, clones, and scaffolding in one pass — fewer rounds of LLM inference, lower cost.
- **Built in Rust.** Single binary, no runtime dependencies. Scans thousands of files in seconds. Memory-safe by design.
- **Cross-file analysis.** Resolves imports, barrel re-exports, and class hierarchies to find dead code across module boundaries.
- **Framework-aware.** Auto-detects React, Next.js, Django, Spring, Axum, and more — won't flag lifecycle methods as dead code.
- **Zero configuration.** Works out of the box. Config file is optional.
- **15 languages.** One tool for polyglot codebases.

---

## Install

### From crates.io

```bash
cargo install fossil-mcp
```

This installs the `fossil-mcp` binary which serves as both the MCP server and CLI tool.

### From source

```bash
git clone https://github.com/yfedoseev/fossil-mcp.git
cd fossil
cargo build --release
```

The binary is at `./target/release/fossil-mcp`.

---

## MCP Server Setup

Fossil runs as an MCP server by default — just run `fossil-mcp` with no arguments. Connect it to your AI coding tool:

<details>
<summary><b>Claude Code</b></summary>

```bash
claude mcp add fossil fossil-mcp
```

</details>

<details>
<summary><b>OpenAI Codex</b></summary>

Add to your Codex MCP configuration:

```json
{
  "mcpServers": {
    "fossil": {
      "command": "fossil-mcp"
    }
  }
}
```

</details>

<details>
<summary><b>Cursor</b></summary>

Add to `~/.cursor/mcp.json`:

```json
{
  "mcpServers": {
    "fossil": {
      "command": "fossil-mcp"
    }
  }
}
```

Or click the Cursor install button above.

</details>

<details>
<summary><b>VS Code / VS Code Insiders</b></summary>

Add to `.vscode/mcp.json` in your workspace:

```json
{
  "mcp": {
    "servers": {
      "fossil": {
        "command": "fossil-mcp"
      }
    }
  }
}
```

Or click the VS Code install button above.

</details>

<details>
<summary><b>Windsurf</b></summary>

Add to `~/.codeium/windsurf/mcp_config.json`:

```json
{
  "mcpServers": {
    "fossil": {
      "command": "fossil-mcp"
    }
  }
}
```

</details>

<details>
<summary><b>Claude Desktop</b></summary>

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "fossil": {
      "command": "fossil-mcp"
    }
  }
}
```

</details>

### MCP Tools

Once connected, your AI agent has access to these tools:

| Tool | Description |
|------|-------------|
| `scan_all` | Run all analyses (dead code + clones) on a project |
| `analyze_dead_code` | Detect unreachable code with configurable confidence |
| `detect_clones` | Find duplicated code (Type 1/2/3 clones) |
| `fossil_refresh` | Incremental re-analysis after file changes (fast) |
| `fossil_inspect` | Inspect call graph, data flow, control flow, or blast radius for any function |
| `fossil_trace` | Find call paths between two functions — understand how code connects |
| `fossil_explain_finding` | Get rich context about a specific finding |
| `fossil_detect_scaffolding` | Find AI scaffolding: phased comments, TODOs, placeholders, and temp files |

---

## CLI Usage

```bash
# Run all analyses on a project
fossil-mcp scan /path/to/project

# Dead code detection only
fossil-mcp dead-code /path/to/project

# Clone detection only
fossil-mcp clones /path/to/project

# Filter by confidence
fossil-mcp dead-code /path/to/project --min-confidence high

# Filter small functions
fossil-mcp dead-code /path/to/project --min-lines 10

# Output as SARIF (for IDE integration)
fossil-mcp scan /path/to/project --format sarif -o results.sarif

# Output as JSON
fossil-mcp scan /path/to/project --format json

# Start MCP server explicitly
fossil-mcp mcp
```

The CLI provides an interactive dashboard with language breakdown, confidence summary, and file hotspots. When running in a terminal, you get a REPL to explore findings interactively:

```
fossil> dead 10        # Show top 10 dead code findings
fossil> clones 5       # Show top 5 clone groups
fossil> hotspots       # Show most affected files
fossil> file auth.ts   # Show findings in a specific file
fossil> langs          # Language breakdown
fossil> export sarif   # Export full SARIF report
```

---

## Supported Languages

| Language | Extensions |
|----------|-----------|
| Python | `.py` |
| JavaScript | `.js`, `.jsx`, `.mjs` |
| TypeScript | `.ts`, `.tsx` |
| Rust | `.rs` |
| Go | `.go` |
| Java | `.java` |
| C# | `.cs` |
| C/C++ | `.c`, `.h`, `.cpp`, `.cc`, `.cxx`, `.hpp` |
| Ruby | `.rb` |
| PHP | `.php` |
| Swift | `.swift` |
| Kotlin | `.kt` |
| Scala | `.scala` |
| Bash | `.sh`, `.bash` |
| Lua | `.lua` |

---

## Configuration (Optional)

Fossil works with zero configuration. All settings have sensible defaults. If you need to customize behavior, create a `fossil.toml` in your project root:

```toml
[dead_code]
min_confidence = "high"       # low, medium, high, certain
include_tests = false
exclude_patterns = ["generated/**", "vendor/**"]

[clones]
min_lines = 6
similarity_threshold = 0.8

[entry_points]
# Mark additional functions as entry points (won't be flagged as dead)
functions = ["custom_handler", "my_entry"]
# Additional entry point attributes/decorators
attributes = ["MyFramework::route"]
# Framework presets (auto-detected by default)
presets = ["axum", "react"]
auto_detect_presets = true
```

Config is auto-discovered from these filenames: `fossil.toml`, `.fossil.toml`, `fossil.yml`, `fossil.yaml`, `fossil.json`.

Environment variables override config file values:

| Variable | Effect |
|----------|--------|
| `FOSSIL_MIN_CONFIDENCE` | Minimum confidence for dead code findings |
| `FOSSIL_MIN_LINES` | Minimum lines for clone detection |
| `FOSSIL_SIMILARITY` | Similarity threshold for Type 3 clones |
| `FOSSIL_OUTPUT_FORMAT` | Output format (text, json, sarif) |

### Framework Presets

Presets are auto-detected from project dependencies. They tell Fossil which functions are framework entry points (lifecycle hooks, route handlers, etc.) so they aren't flagged as dead code:

| Preset | Detected by | Entry points recognized |
|--------|-------------|------------------------|
| `react` | `react` in deps | `componentDidMount`, `render`, `useEffect`, ... |
| `nextjs` | `next` in deps | `getServerSideProps`, `getStaticProps`, ... |
| `express` | `express` in deps | `router.*` patterns |
| `django` | `django` in deps | `get`, `post`, URL pattern handlers |
| `flask` | `flask` in deps | `app.route` patterns |
| `spring` | `spring-boot` in deps | `@Bean`, `@Controller`, `@Service`, ... |
| `axum` | `axum` in deps | `#[tokio::main]`, `#[debug_handler]` |
| `actix` | `actix-web` in deps | `#[actix_web::main]`, `#[get]`, `#[post]`, ... |
| `angular` | `@angular/core` in deps | `ngOnInit`, `ngOnDestroy`, ... |

---

## How It Works

1. **Scan** — walks project files, respects `.gitignore`, skips vendored/generated code
2. **Parse** — builds tree-sitter ASTs for each source file (15 languages)
3. **Extract** — pulls functions, calls, imports, attributes, and class hierarchy from ASTs
4. **Graph** — builds a cross-file `CodeGraph` with import resolution and barrel re-export support
5. **Analyze** — detects entry points (via heuristics + framework presets), runs reachability analysis, identifies dead code and clones
6. **Report** — outputs findings as text dashboard, JSON, or SARIF

---

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

- [Code of Conduct](CODE_OF_CONDUCT.md)
- [Security Policy](SECURITY.md)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
