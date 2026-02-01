# Contributing to Fossil MCP

Thank you for your interest in contributing to Fossil MCP! This document provides guidelines and instructions for contributing.

## Code of Conduct

This project adheres to the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code. Please report unacceptable behavior to the project maintainers.

## How Can I Contribute?

### Reporting Bugs

Before creating bug reports, please check existing issues to avoid duplicates.

When creating a bug report, include:

- **Clear title** describing the issue
- **Steps to reproduce** the behavior
- **Expected behavior** vs what actually happened
- **Environment details**:
  - Fossil MCP version (`fossil-mcp --version`)
  - Operating system
  - Language of the analyzed code
- **Code samples** or minimal reproduction
- **Error messages** (full output if available)

### Suggesting Features

Feature requests are welcome! Please include:

- **Clear description** of the feature
- **Use case** - why is this feature needed?
- **Alternative solutions** you've considered

### Pull Requests

1. **Fork** the repository
2. **Create a branch** from `main`:
   ```bash
   git checkout -b feature/your-feature-name
   ```
3. **Make your changes** following our style guidelines
4. **Add tests** for new functionality
5. **Run tests** to ensure nothing is broken:
   ```bash
   cargo test
   ```
6. **Commit** with a clear message (see commit guidelines below)
7. **Push** and create a Pull Request

## Development Setup

### Prerequisites

- Rust 1.75+

### Building from Source

```bash
# Clone the repository
git clone https://github.com/yfedoseev/fossil-mcp.git
cd fossil

# Build
cargo build

# Build release
cargo build --release
```

### Running Tests

```bash
# All tests
cargo test

# With output
cargo test -- --nocapture
```

### Code Quality

```bash
# Formatting
cargo fmt

# Linting
cargo clippy -- -D warnings

# Both
cargo fmt && cargo clippy -- -D warnings
```

## Style Guidelines

### Rust

- Follow standard Rust formatting (`cargo fmt`)
- Pass clippy checks (`cargo clippy -- -D warnings`)
- Use meaningful variable and function names
- Add doc comments for public APIs

### Git Commit Messages

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

**Types:**
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation only
- `style`: Code style (formatting, etc.)
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

**Examples:**
```
feat(dead-code): improve cross-file import resolution
fix(clones): handle min_lines filter for Type 3 clones
docs(readme): update installation instructions
```

## Project Structure

```
src/
├── main.rs           # Binary entry point (MCP default, CLI via subcommands)
├── core/             # Types, errors, traits
├── parsers/          # Tree-sitter parsers (15 languages) + extractor
├── graph/            # CodeGraph, import resolver, CFG, dataflow
├── analysis/         # Pipeline, scanner, incremental analysis
├── dead_code/        # Dead code detector, entry points, classifier
├── clones/           # Clone detector (MinHash + SimHash)
├── rules/            # Rule database
├── output/           # SARIF, text, JSON formatters
├── config/           # Configuration + framework presets
├── cli/              # CLI commands (scan, dead-code, clones, rules)
├── mcp/              # MCP server + tools
└── lsp/              # LSP stub
tests/
├── end_to_end.rs     # Dead code end-to-end tests
└── min_lines_filter.rs # Clone filter tests
```

## Adding Language Support

To add a new language:

1. Add the tree-sitter grammar dependency to `Cargo.toml`
2. Add a parser entry in `src/parsers/parser_macro.rs` using `define_parser!`
3. Update the `Language` enum in `src/core/types.rs`
4. Add extraction patterns in `src/parsers/extractor.rs` if the language has unique syntax
5. Add tests

## Pull Request Checklist

- [ ] Code follows the project's style guidelines
- [ ] Tests added/updated for changes
- [ ] All tests pass locally (`cargo test`)
- [ ] Clippy passes (`cargo clippy -- -D warnings`)
- [ ] Code is formatted (`cargo fmt`)
- [ ] Commit messages follow Conventional Commits
- [ ] PR description explains the changes and motivation

## Questions?

If you have questions, feel free to:

- Open a [GitHub Discussion](https://github.com/yfedoseev/fossil-mcp/discussions)
- Open an issue with the `question` label

## License

By contributing, you agree that your contributions will be licensed under the same [MIT OR Apache-2.0](LICENSE-MIT) license as the project.
