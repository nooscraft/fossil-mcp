# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.2] - 2025-02-09

### Added

- `cargo binstall fossil-mcp` support via `[package.metadata.binstall]` in Cargo.toml
- Per-target binary overrides for all 6 release platforms (Linux glibc/musl/aarch64, macOS Intel/Apple Silicon, Windows)
- `cargo-binstall` install section in README

### Changed

- Moved VS Code / Cursor one-click install badges from top of README into MCP Server Setup section for clearer install flow
- Fixed hardcoded `0.1.0` download URLs in README to use version-less `/releases/latest/download/` URLs

## [0.1.1] - 2025-02-07

### Added

- `fossil-mcp update` command for self-updating from GitHub Releases
- `fossil-mcp update --check` to check for updates without installing
- Automatic background update check on startup (once per day, non-blocking)
- `FOSSIL_NO_UPDATE_CHECK=1` environment variable to disable automatic update checks
- Version-less release asset URLs for stable install scripts
- This changelog

### Changed

- Install commands now use stable (version-less) download URLs

### Dependencies

- Added `self_update` for GitHub Release-based binary updates
- Updated `petgraph` 0.6 → 0.8
- Updated `tree-sitter-bash` 0.23 → 0.25
- Updated `tree-sitter-php` 0.23 → 0.24
- Updated `toml` 0.8 → 0.9
- Updated remaining tree-sitter parsers

## [0.1.0] - 2025-01-21

### Added

- Initial release
- Dead code detection across 15 languages
- Code clone detection (Type 1, 2, 3) with MinHash/SimHash
- AI scaffolding artifact detection
- MCP server for AI tool integration
- CLI with `scan`, `dead-code`, `clones`, `rules` subcommands
- SARIF, JSON, and text output formats
- Configuration via `.fossil.toml`

[0.1.2]: https://github.com/yfedoseev/fossil-mcp/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/yfedoseev/fossil-mcp/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/yfedoseev/fossil-mcp/releases/tag/v0.1.0
