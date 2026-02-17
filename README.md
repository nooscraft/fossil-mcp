# Fossil MCP

**The code quality toolkit for the vibe coding era.**

Static analysis that finds the mess vibe coding leaves behind — dead code, duplicated logic, scaffolding artifacts, and disconnected functions — across 16 languages.

**[fossil-mcp.com](https://fossil-mcp.com)**

[![CI](https://github.com/yfedoseev/fossil-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/yfedoseev/fossil-mcp/actions/workflows/ci.yml)
[![Crates.io](https://img.shields.io/crates/v/fossil-mcp.svg)](https://crates.io/crates/fossil-mcp)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE-MIT)

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
- **16 languages.** One tool for polyglot codebases.

---

## Install

### Quick install (recommended)

**macOS / Linux:**
```bash
curl -fsSL fossil-mcp.com/install.sh | sh
```

**Windows (PowerShell):**
```powershell
irm fossil-mcp.com/install.ps1 | iex
```

Auto-detects your OS and architecture, downloads the latest binary, and adds it to your PATH.

### Manual download

Download the latest binary for your platform from [GitHub Releases](https://github.com/yfedoseev/fossil-mcp/releases/latest):

```bash
# macOS (Apple Silicon)
curl -L https://github.com/yfedoseev/fossil-mcp/releases/latest/download/fossil-mcp-macos-aarch64.tar.gz | tar xz
mv fossil-mcp /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/yfedoseev/fossil-mcp/releases/latest/download/fossil-mcp-macos-x86_64.tar.gz | tar xz
mv fossil-mcp /usr/local/bin/

# Linux (x86_64)
curl -L https://github.com/yfedoseev/fossil-mcp/releases/latest/download/fossil-mcp-linux-x86_64-musl.tar.gz | tar xz
mv fossil-mcp ~/.local/bin/
```

| Platform | Architecture | Archive |
|----------|-------------|---------|
| Linux | x86_64 (recommended) | `fossil-mcp-linux-x86_64-musl` |
| Linux | x86_64 (glibc) | `fossil-mcp-linux-x86_64` |
| Linux | ARM64 | `fossil-mcp-linux-aarch64` |
| macOS | Intel | `fossil-mcp-macos-x86_64` |
| macOS | Apple Silicon | `fossil-mcp-macos-aarch64` |
| Windows | x86_64 | `fossil-mcp-windows-x86_64` |

### cargo-binstall

If you have [cargo-binstall](https://github.com/cargo-bins/cargo-binstall), it downloads pre-built binaries instead of compiling from source:

```bash
cargo binstall fossil-mcp
```

### From crates.io

```bash
cargo install fossil-mcp
```

This downloads the source from crates.io and compiles it locally. Requires a Rust toolchain.

### From source

```bash
git clone https://github.com/yfedoseev/fossil-mcp.git
cd fossil-mcp
cargo build --release
```

The binary is at `./target/release/fossil-mcp`.

### Updating

```bash
fossil-mcp update
```

---

## MCP Server Setup

[![Install in VS Code](https://img.shields.io/badge/VS_Code-Install_Server-0098FF?style=flat-square&logo=visualstudiocode&logoColor=white)](https://insiders.vscode.dev/redirect/mcp/install?name=fossil&config=%7B%22command%22%3A%22fossil-mcp%22%7D)
[![Install in VS Code Insiders](https://img.shields.io/badge/VS_Code_Insiders-Install_Server-24bfa5?style=flat-square&logo=visualstudiocode&logoColor=white)](https://insiders.vscode.dev/redirect/mcp/install?name=fossil&config=%7B%22command%22%3A%22fossil-mcp%22%7D&quality=insiders)
[![Install in Cursor](https://cursor.com/deeplink/mcp-install-dark.svg)](https://cursor.com/en/install-mcp?name=fossil&config=eyJjb21tYW5kIjoiZm9zc2lsLW1jcCJ9)

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
| `scan_all` | Run all analyses (dead code + clones + scaffolding) on a project |
| `analyze_dead_code` | Detect unreachable code with configurable confidence |
| `detect_clones` | Find duplicated code (Type 1/2/3 clones) |
| `fossil_refresh` | Incremental re-analysis after file changes (fast) |
| `fossil_inspect` | Inspect call graph, data flow, control flow, or blast radius for any function |
| `fossil_trace` | Find call paths between two functions — understand how code connects |
| `fossil_explain_finding` | Get rich context about a specific finding |
| `fossil_detect_scaffolding` | Find AI scaffolding: phased comments, TODOs, placeholders, and temp files |

---

## CLI Usage

### Modes

Fossil has four modes of operation:

| Mode | How to invoke | What it does |
|------|--------------|-------------|
| **Interactive** | `fossil-mcp` (no args) | Runs full scan + opens interactive REPL to explore findings |
| **CLI** | `fossil-mcp <command>` | Run a specific analysis command |
| **MCP Server** | `fossil-mcp mcp` or piped stdin | JSON-RPC server for AI coding tools |
| **CI/CD** | `fossil-mcp check` | Fail builds when thresholds exceeded |

### Interactive Mode

Running `fossil-mcp` with no arguments (or `fossil-mcp scan .`) scans the current directory for dead code, clones, and scaffolding, shows a dashboard, then drops into an interactive REPL:

```
  FOSSIL Scanning .
  ────────────────────────────────────────────────
   ✓  1200 nodes analyzed, 42 unreachable
   ✓  380 files analyzed, 8 clone groups
   ✓  3 scaffolding artifacts

  ══════════════════════════════════════════════════
  RESULTS  53  findings across 28 files
  ══════════════════════════════════════════════════

  ▐ Dead Code     42   ██████████████░░
  ▐ Clones         8   ██░░░░░░░░░░░░░░    120 duplicated lines
  ▐ Scaffolding    3   █░░░░░░░░░░░░░░░

  fossil>
```

#### REPL Commands

All exploration commands support optional count and language filter: `command [N] [lang]`

```
fossil> dead 10                  # Top 10 dead code findings
fossil> dead 20 typescript       # Top 20 dead code in TypeScript
fossil> clones 5 rust            # Top 5 clone groups in Rust
fossil> scaffolding              # All scaffolding findings
fossil> scaffolding 10 python    # Top 10 scaffolding in Python
fossil> hotspots                 # Files with most findings
fossil> hotspots 10 go           # Top 10 hotspot files in Go
fossil> file auth.ts             # All findings in a specific file
fossil> langs                    # Language breakdown
fossil> export sarif             # Export full SARIF report
fossil> summary                  # Re-show dashboard
fossil> q                        # Quit
```

### Commands

#### `fossil-mcp scan [path]`

Run all analyses (dead code + clones + scaffolding) with interactive dashboard.

```bash
fossil-mcp scan .
fossil-mcp scan /path/to/project --format sarif -o results.sarif
fossil-mcp scan /path/to/project --format json
```

#### `fossil-mcp dead-code [path]`

Dead code detection only.

```bash
fossil-mcp dead-code .
fossil-mcp dead-code . --min-confidence high
fossil-mcp dead-code . --min-lines 10
fossil-mcp dead-code . --language rust,python
fossil-mcp dead-code . --diff main                  # Only changed files
fossil-mcp dead-code . --stats                      # Show graph statistics
fossil-mcp dead-code . --cache-dir .fossil-cache     # Persistent cache
fossil-mcp dead-code . --cache-stats                 # Cache hit rate
```

| Flag | Description |
|------|-------------|
| `--min-confidence <LEVEL>` | Filter by confidence: `low`, `medium`, `high`, `certain` |
| `--min-lines <N>` | Minimum lines of code for a finding |
| `--language <LANGS>` | Filter by language (comma-separated): `rust,python,go` |
| `--include-tests` | Include test-only code in results |
| `--diff <BRANCH>` | Only analyze files changed vs base branch |
| `--stats` | Print graph cardinality estimates (HyperLogLog) |
| `--cache-dir <PATH>` | Persistent cache directory for incremental analysis |
| `--cache-stats` | Print cache hit rate and memory usage |

#### `fossil-mcp clones [path]`

Clone (duplicated code) detection only.

```bash
fossil-mcp clones .
fossil-mcp clones . --min-lines 10
fossil-mcp clones . --similarity 0.9
fossil-mcp clones . --language typescript
fossil-mcp clones . --types type1,type2
```

| Flag | Description |
|------|-------------|
| `--min-lines <N>` | Minimum lines for a clone (default: 6) |
| `--similarity <F>` | Similarity threshold 0.0–1.0 for Type 3 clones (default: 0.8) |
| `--types <TYPES>` | Clone types to detect: `type1,type2,type3` (default: all) |
| `--language <LANGS>` | Filter by language (comma-separated) |

#### `fossil-mcp scaffolding [path]`

Detect AI-generated scaffolding artifacts.

```bash
fossil-mcp scaffolding .
fossil-mcp scaffolding . --language rust
fossil-mcp scaffolding . --include-todos
fossil-mcp scaffolding . --format json
```

| Flag | Description |
|------|-------------|
| `--language <LANGS>` | Filter by language (comma-separated) |
| `--include-todos` | Include TODO/FIXME/HACK markers (excluded by default) |

Detects: placeholder bodies (`pass`, `todo!()`, `unimplemented!()`), phased comments (`Phase 1`, `Step 2`), scaffolding identifiers (`scaffold_*`, `boilerplate_*`), debug prints, and temp files (`temp_*`, `backup_*`, `old_*`).

#### `fossil-mcp check [path]`

CI/CD mode — fails builds when thresholds are exceeded. See [CI/CD Integration](#cicd-integration) for full details.

```bash
fossil-mcp check
fossil-mcp check --max-dead-code 10 --max-clones 5
fossil-mcp check --diff origin/main
fossil-mcp check --diff origin/main --format sarif
fossil-mcp check --fail-on-scaffolding
```

| Flag | Description |
|------|-------------|
| `--max-dead-code <N>` | Maximum dead code findings allowed |
| `--max-clones <N>` | Maximum clone findings allowed |
| `--max-scaffolding <N>` | Maximum scaffolding findings allowed |
| `--min-confidence <LEVEL>` | Minimum confidence for counting findings |
| `--diff <BRANCH>` | Only check files changed vs base branch |
| `--fail-on-scaffolding` | Fail if any scaffolding artifacts found |

#### `fossil-mcp weekly`

Show the weekly AI slop rankings across open-source projects.

```bash
fossil-mcp weekly
fossil-mcp weekly --detailed
```

#### `fossil-mcp update`

Update fossil-mcp to the latest version.

```bash
fossil-mcp update
fossil-mcp update --check    # Check without installing
```

#### `fossil-mcp mcp`

Start the MCP server explicitly (normally auto-detected via piped stdin).

### Global Flags

These flags work with all commands:

| Flag | Description |
|------|-------------|
| `--format <FMT>` | Output format: `text`, `json`, `sarif` (default: text) |
| `-o, --output <FILE>` | Write output to file instead of stdout |
| `-q, --quiet` | Suppress all non-error output |
| `-v, --verbose` | Enable debug logging |
| `-c, --config <FILE>` | Path to config file |

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
| R | `.r`, `.R` |

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

## CI/CD Integration

Fossil includes a **`check`** command for CI/CD pipelines. It fails builds when code quality thresholds are exceeded, helping teams enforce code standards and prevent technical debt from accumulating.

### Basic Usage

```bash
# Check against configured thresholds
fossil-mcp check

# Override thresholds via CLI
fossil-mcp check --max-dead-code 10 --max-clones 5

# Diff-aware mode (only analyze changed files in PR)
fossil-mcp check --diff origin/main

# Generate SARIF for GitHub code scanning
fossil-mcp check --diff origin/main --format sarif

# Quiet mode (no diagnostic output)
fossil-mcp check --quiet
```

### Configuration

Add a `[ci]` section to `fossil.toml`:

```toml
[ci]
max_dead_code = 10           # Maximum dead code findings (0 = fail on any)
max_clones = 5               # Maximum clone findings
max_scaffolding = 3          # Maximum scaffolding findings
min_confidence = "medium"    # Minimum confidence (low|medium|high|certain)
fail_on_scaffolding = false  # Fail if any scaffolding found
```

### GitHub Actions Integration

Create `.github/workflows/fossil-check.yml`:

```yaml
name: Fossil CI Check

on:
  pull_request:
  push:
    branches: [main]

jobs:
  fossil:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Install Fossil
        run: curl -fsSL fossil-mcp.com/install.sh | sh

      - name: Run Fossil check
        run: |
          fossil-mcp check \
            --diff origin/${{ github.base_ref || 'main' }} \
            --format sarif \
            > fossil-results.sarif

      - name: Upload to GitHub Security
        if: always()
        uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: fossil-results.sarif
```

### How It Works

1. **Scans** the project using the same analysis engine as `scan`
2. **Optionally filters** to only changed files (via `--diff branch`)
3. **Evaluates** against configured thresholds
4. **Reports** findings as text, JSON, or SARIF
5. **Exits** with code 1 if thresholds exceeded (fails CI build)

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | All thresholds passed ✓ |
| 1 | Threshold exceeded (build fails) |
| 2 | Error (missing git, invalid config, etc.) |

For complete examples, see [examples/fossil.toml](examples/fossil.toml) and [examples/fossil-check.yml](examples/fossil-check.yml).

---

## How It Works

1. **Scan** — walks project files, respects `.gitignore`, skips vendored/generated code
2. **Parse** — builds tree-sitter ASTs for each source file (16 languages)
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
