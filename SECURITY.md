# Security Policy

## Supported Versions

We release patches for security vulnerabilities. Currently supported versions:

| Version | Supported          |
| ------- | ------------------ |
| 0.1.x   | :white_check_mark: |
| < 0.1   | :x:                |

## Reporting a Vulnerability

We take the security of Fossil MCP seriously. If you believe you have found a security vulnerability, please report it to us as described below.

### Where to Report

**Please do not report security vulnerabilities through public GitHub issues.**

Instead, please use GitHub's private vulnerability reporting:
- Go to https://github.com/yfedoseev/fossil-mcp/security/advisories/new

### What to Include

Please include the following information in your report:

* Type of issue (e.g. credential exposure, injection, etc.)
* Full paths of source file(s) related to the manifestation of the issue
* The location of the affected source code (tag/branch/commit or direct URL)
* Any special configuration required to reproduce the issue
* Step-by-step instructions to reproduce the issue
* Proof-of-concept or exploit code (if possible)
* Impact of the issue, including how an attacker might exploit it

### What to Expect

* We will acknowledge your email within 48 hours
* We will send a more detailed response within 7 days indicating the next steps
* We will keep you informed about progress towards a fix
* We may ask for additional information or guidance
* Once fixed, we will publicly disclose the vulnerability (crediting you if desired)

## Security Considerations

Fossil MCP is a static analysis tool that reads source code. This tool:

* **Read-only analysis**: Fossil never modifies your source files
* **No network access**: All analysis is local — no code is sent externally
* **No unsafe code**: Core library avoids unsafe Rust code
* **Input validation**: File paths and patterns are validated before processing
* **Dependency auditing**: Regular security audits via `cargo audit`

### Best Practices

When using Fossil MCP:

1. **Review SARIF output**: Verify findings before acting on them — static analysis can have false positives
2. **Update regularly**: Keep Fossil MCP updated with latest patches
3. **CI integration**: Run Fossil in CI pipelines to catch issues before merge

## Disclosure Policy

When we receive a security bug report, we will:

1. Confirm the problem and determine affected versions
2. Audit code to find similar problems
3. Prepare fixes for all supported versions
4. Release patches as soon as possible

We ask security researchers to:

* Give us reasonable time to respond before public disclosure
* Make a good faith effort to avoid privacy violations and service disruption
* Not access or modify other users' data

## Comments on this Policy

If you have suggestions on how this process could be improved, please submit a pull request.
