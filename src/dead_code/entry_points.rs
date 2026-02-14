//! Entry point detection heuristics.

use std::collections::HashSet;
use std::path::Path;

use crate::core::{CodeNode, NodeKind, Visibility};
use crate::graph::CodeGraph;
use petgraph::graph::NodeIndex;
use regex::Regex;

/// Detects production and test entry points in a CodeGraph.
pub struct EntryPointDetector<'a> {
    graph: &'a CodeGraph,
    rules: crate::config::ResolvedEntryPointRules,
    serde_targets: HashSet<String>,  // Pre-built set of function names referenced by serde attrs
}

impl<'a> EntryPointDetector<'a> {
    /// Create a detector with default hardcoded rules (backward compatible).
    pub fn new(graph: &'a CodeGraph) -> Self {
        let serde_targets = Self::build_serde_target_index(graph);
        Self {
            graph,
            rules: crate::config::ResolvedEntryPointRules::with_defaults(),
            serde_targets,
        }
    }

    /// Create a detector with custom entry point rules.
    pub fn with_rules(graph: &'a CodeGraph, rules: crate::config::ResolvedEntryPointRules) -> Self {
        let serde_targets = Self::build_serde_target_index(graph);
        Self { graph, rules, serde_targets }
    }

    /// Build a set of function names referenced by serde attributes (O(n) preprocessing).
    /// This eliminates the O(n²) nested loop in is_framework_entry().
    fn build_serde_target_index(graph: &'a CodeGraph) -> HashSet<String> {
        let mut targets = HashSet::new();
        for (_, node) in graph.nodes() {
            for attr in &node.attributes {
                if attr.starts_with("serde_default:")
                    || attr.starts_with("serde_serialize_with:")
                    || attr.starts_with("serde_deserialize_with:")
                {
                    if let Some(fn_name) = attr.split(':').nth(1) {
                        targets.insert(fn_name.to_string());
                    }
                }
            }
        }
        targets
    }

    /// Detect production entry points.
    pub fn detect_production_entry_points(&self) -> HashSet<NodeIndex> {
        let mut entries = HashSet::new();

        // Start with graph-level entry points
        entries.extend(self.graph.entry_points());

        // Add heuristic-based entry points
        for (idx, node) in self.graph.nodes() {
            if self.is_production_entry(node) {
                entries.insert(idx);
            }
        }

        entries
    }

    /// Detect test entry points.
    pub fn detect_test_entry_points(&self) -> HashSet<NodeIndex> {
        let mut entries = HashSet::new();

        entries.extend(self.graph.test_entry_points());

        for (idx, node) in self.graph.nodes() {
            if self.is_test_entry(node) {
                entries.insert(idx);
            }
        }

        entries
    }

    fn is_production_entry(&self, node: &CodeNode) -> bool {
        self.is_main_function(node)
            || self.is_module_entry(node)
            || self.is_exported_entry(node)
            || self.is_framework_entry(node)
    }

    fn is_main_function(&self, node: &CodeNode) -> bool {
        matches!(
            node.name.as_str(),
            "main"
                | "__main__"
                | "Main"
                | "app"
                | "run"
                | "start"
                | "init"
                | "handler"
                | "lambda_handler"
        )
    }

    /// Recognize synthetic `<module:...>` nodes as entry points.
    /// These are created by the parser for top-level code in dynamic languages.
    fn is_module_entry(&self, node: &CodeNode) -> bool {
        node.name.starts_with("<module:")
    }

    fn is_exported_entry(&self, node: &CodeNode) -> bool {
        // Only treat public functions as entry points in languages with explicit
        // visibility systems (Rust, Java, C#, Go, Kotlin, Scala). In Python/JS/Ruby/PHP/etc,
        // all top-level functions are "public" by default, so treating them all as
        // entry points would make dead code detection useless.
        // Note: TS/JS has `export` but we intentionally keep it out — exported-but-unused
        // functions are legitimate dead code findings. Instead we fix import resolution
        // so callers connect properly.
        let has_explicit_visibility = matches!(
            node.language,
            crate::core::Language::Rust
                | crate::core::Language::Java
                | crate::core::Language::CSharp
                | crate::core::Language::Go
                | crate::core::Language::Kotlin
                | crate::core::Language::Scala
        );

        has_explicit_visibility
            && node.visibility == Visibility::Public
            && matches!(
                node.kind,
                NodeKind::Function | NodeKind::Method | NodeKind::AsyncFunction
            )
    }

    fn is_framework_entry(&self, node: &CodeNode) -> bool {
        // Check attributes against resolved rules
        let has_framework_attr = node.attributes.iter().any(|attr| {
            self.rules.matches_attribute(attr)
                // Also handle dataclass variants
                || attr.starts_with("dataclass(")
                || attr.starts_with("attr.s")
                || attr.starts_with("attr.attrs")
        });

        if has_framework_attr {
            return true;
        }

        // Check if this function is referenced by a serde attribute on another node
        // OPTIMIZATION: Use pre-built serde_targets set instead of O(n²) nested loop
        if self.serde_targets.contains(&node.name) {
            return true;
        }

        // Check user-configured function names
        if self.rules.matches_function(&node.name) {
            return true;
        }

        // Check function name patterns for framework entry points
        let name = &node.name;
        // FastAPI / Flask (Python)
        name.starts_with("app.get")
            || name.starts_with("app.post")
            || name.starts_with("app.put")
            || name.starts_with("app.delete")
            || name.starts_with("app.route")
            // Express (Node.js)
            || name.starts_with("router.get")
            || name.starts_with("router.post")
            || name.starts_with("router.put")
            || name.starts_with("router.delete")
            // Go Gin/Echo
            || name.starts_with("r.GET")
            || name.starts_with("r.POST")
            || name.starts_with("e.GET")
            || name.starts_with("e.POST")
    }

    fn is_test_entry(&self, node: &CodeNode) -> bool {
        if node.is_test {
            return true;
        }

        // Functions defined in test files are test infrastructure —
        // they're test entries regardless of naming convention.
        if Self::is_test_file(&node.location.file) {
            return true;
        }

        let name = &node.name;
        name.starts_with("test_")
            || name.starts_with("Test")
            || name.ends_with("_test")
            || name.starts_with("it_")
            || name.starts_with("should_")
            || name.starts_with("spec_")
    }

    /// Check if a file path indicates a test file.
    /// Generic: uses directory segments and filename stem patterns,
    /// not hardcoded per-extension lists.
    pub fn is_test_file(path: &str) -> bool {
        let normalized = path.replace('\\', "/").to_lowercase();

        // Directory-level: any path segment that IS a test directory
        for seg in normalized.split('/') {
            if matches!(
                seg,
                "tests"
                    | "test"
                    | "__tests__"
                    | "spec"
                    | "__mocks__"
                    | "mocks"
                    | "test-utils"
                    | "testutils"
                    | "testing"
            ) {
                return true;
            }
        }

        // File-level: strip final extension, check stem for test/spec markers
        if let Some(filename) = normalized.rsplit('/').next() {
            if filename.starts_with("test_") {
                return true;
            }
            if let Some(stem) = filename.rsplit_once('.').map(|(s, _)| s) {
                if stem.ends_with("_test")
                    || stem.ends_with("_tests")
                    || stem.ends_with(".test")
                    || stem.ends_with(".spec")
                    || stem.ends_with("_spec")
                {
                    return true;
                }
                // setupTests.ts, setup-test.js, etc.
                if stem.starts_with("setup") && stem.contains("test") {
                    return true;
                }
            }
        }

        false
    }
}

/// Detect entry points referenced by configuration files (Dockerfile, docker-compose, package.json).
///
/// Walks the project for config files and extracts entry script paths, then resolves
/// them to graph nodes. This catches standalone services (ECS containers, CLI tools, etc.)
/// that appear dead because no code calls them directly.
pub fn detect_config_entry_points(root: &Path, graph: &CodeGraph) -> HashSet<NodeIndex> {
    let mut entries = HashSet::new();
    let mut entry_files: HashSet<String> = HashSet::new();

    // Walk project directories for config files
    let walker = ignore::WalkBuilder::new(root)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .hidden(true)
        .max_depth(Some(10))
        .build();

    // Match all quoted strings in ENTRYPOINT/CMD lines
    let re_entrypoint_line = Regex::new(r#"(?i)^(?:ENTRYPOINT|CMD)\s+(.+)$"#).ok();
    let re_quoted_arg = Regex::new(r#""([^"]+)""#).ok();
    let re_docker_build_context =
        Regex::new(r#"(?:build:\s*(?:context:\s*)?|build:\s*\n\s*context:\s*)([^\s\n]+)"#).ok();

    for entry in walker.flatten() {
        let path = entry.path();
        let file_name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };

        match file_name.as_str() {
            "Dockerfile" => {
                if let Ok(content) = std::fs::read_to_string(path) {
                    extract_dockerfile_entries(
                        &content,
                        path,
                        &re_entrypoint_line,
                        &re_quoted_arg,
                        &mut entry_files,
                    );
                }
            }
            name if name.starts_with("docker-compose")
                && (name.ends_with(".yml") || name.ends_with(".yaml")) =>
            {
                if let Ok(content) = std::fs::read_to_string(path) {
                    extract_docker_compose_entries(
                        &content,
                        path,
                        &re_docker_build_context,
                        &mut entry_files,
                    );
                }
            }
            "package.json" => {
                if let Ok(content) = std::fs::read_to_string(path) {
                    extract_package_json_entries(&content, path, &mut entry_files);
                }
            }
            "cdk.json" => {
                if let Ok(content) = std::fs::read_to_string(path) {
                    extract_cdk_json_entries(&content, path, &mut entry_files);
                }
            }
            _ => {}
        }
    }

    // Resolve entry files to graph nodes.
    // Any function in a config-referenced entry file is a potential entry point
    // (exported functions, module entries, and main-like functions). This avoids
    // hardcoding specific function names — Dockerfiles and package.json files
    // can reference any entry pattern (custom handlers, factories, etc.).
    for entry_file in &entry_files {
        for (idx, node) in graph.nodes() {
            let node_file = &node.location.file;
            if node_file.ends_with(entry_file) || entry_file.ends_with(node_file) {
                // Module-level code is always an entry
                if node.name.starts_with("<module:") {
                    entries.insert(idx);
                    continue;
                }
                // Exported functions in entry files are entry points
                if node.visibility == Visibility::Public {
                    entries.insert(idx);
                }
            }
        }
    }

    entries
}

fn extract_dockerfile_entries(
    content: &str,
    dockerfile_path: &Path,
    re_entrypoint_line: &Option<Regex>,
    re_quoted_arg: &Option<Regex>,
    entry_files: &mut HashSet<String>,
) {
    let dir = dockerfile_path.parent().unwrap_or(Path::new("."));

    let skip_commands = [
        "node",
        "python",
        "python3",
        "java",
        "npm",
        "yarn",
        "pnpm",
        "sh",
        "bash",
        "/bin/sh",
        "/bin/bash",
        "/usr/bin/python",
        "/usr/bin/python3",
        "/usr/local/bin/node",
        "/usr/local/bin/python",
    ];

    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(ref re_line) = re_entrypoint_line {
            if let Some(caps) = re_line.captures(trimmed) {
                let args_str = caps.get(1).map(|m| m.as_str()).unwrap_or("");

                // Extract all arguments (quoted or unquoted)
                let args: Vec<String> = if let Some(ref re_q) = re_quoted_arg {
                    re_q.captures_iter(args_str)
                        .filter_map(|c| c.get(1).map(|m| m.as_str().to_string()))
                        .collect()
                } else {
                    args_str.split_whitespace().map(|s| s.to_string()).collect()
                };

                for arg in &args {
                    if skip_commands.contains(&arg.as_str()) {
                        continue;
                    }
                    // Resolve relative to Dockerfile directory
                    let resolved = dir.join(arg);
                    entry_files.insert(resolved.to_string_lossy().to_string());
                    // Also store just the script name for matching
                    entry_files.insert(arg.clone());
                }
            }
        }
    }
}

fn extract_docker_compose_entries(
    content: &str,
    compose_path: &Path,
    re_build_context: &Option<Regex>,
    entry_files: &mut HashSet<String>,
) {
    let dir = compose_path.parent().unwrap_or(Path::new("."));

    if let Some(ref re) = re_build_context {
        for caps in re.captures_iter(content) {
            if let Some(m) = caps.get(1) {
                let context = m.as_str().trim_matches('\"').trim_matches('\'');
                // Resolve relative to compose file directory
                let context_dir = dir.join(context);
                // Look for common entry points in the build context directory
                for entry_name in &[
                    "main.py",
                    "app.py",
                    "index.js",
                    "index.ts",
                    "main.go",
                    "main.rs",
                    "server.js",
                    "server.ts",
                ] {
                    let entry_path = context_dir.join(entry_name);
                    if entry_path.exists() {
                        entry_files.insert(entry_path.to_string_lossy().to_string());
                    }
                }
                // Also check for package.json to find the main entry
                let pkg_json = context_dir.join("package.json");
                if pkg_json.exists() {
                    if let Ok(pkg_content) = std::fs::read_to_string(&pkg_json) {
                        extract_package_json_entries(&pkg_content, &pkg_json, entry_files);
                    }
                }
            }
        }
    }
}

fn extract_package_json_entries(content: &str, pkg_path: &Path, entry_files: &mut HashSet<String>) {
    let dir = pkg_path.parent().unwrap_or(Path::new("."));

    // Parse JSON manually (serde_json is available)
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
        // "main" field
        if let Some(main) = json.get("main").and_then(|v| v.as_str()) {
            let resolved = dir.join(main);
            entry_files.insert(resolved.to_string_lossy().to_string());
            entry_files.insert(main.to_string());
        }

        // "bin" field (can be string or object)
        if let Some(bin) = json.get("bin") {
            match bin {
                serde_json::Value::String(s) => {
                    let resolved = dir.join(s.as_str());
                    entry_files.insert(resolved.to_string_lossy().to_string());
                    entry_files.insert(s.clone());
                }
                serde_json::Value::Object(map) => {
                    for (_, v) in map {
                        if let Some(s) = v.as_str() {
                            let resolved = dir.join(s);
                            entry_files.insert(resolved.to_string_lossy().to_string());
                            entry_files.insert(s.to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        // "scripts.start" field
        if let Some(scripts) = json.get("scripts").and_then(|v| v.as_object()) {
            if let Some(start) = scripts.get("start").and_then(|v| v.as_str()) {
                // Extract the file argument from "node src/index.js" etc.
                let parts: Vec<&str> = start.split_whitespace().collect();
                for part in &parts {
                    if part.ends_with(".js")
                        || part.ends_with(".ts")
                        || part.ends_with(".py")
                        || part.ends_with(".mjs")
                    {
                        let resolved = dir.join(part);
                        entry_files.insert(resolved.to_string_lossy().to_string());
                        entry_files.insert(part.to_string());
                    }
                }
            }
        }
    }
}

fn extract_cdk_json_entries(content: &str, cdk_path: &Path, entry_files: &mut HashSet<String>) {
    let dir = cdk_path.parent().unwrap_or(Path::new("."));

    if let Ok(json) = serde_json::from_str::<serde_json::Value>(content) {
        // "app" field contains the CDK app command, e.g. "npx ts-node bin/app.ts"
        if let Some(app) = json.get("app").and_then(|v| v.as_str()) {
            let parts: Vec<&str> = app.split_whitespace().collect();
            for part in &parts {
                if part.ends_with(".ts")
                    || part.ends_with(".js")
                    || part.ends_with(".py")
                    || part.ends_with(".mjs")
                {
                    let resolved = dir.join(part);
                    entry_files.insert(resolved.to_string_lossy().to_string());
                    entry_files.insert(part.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Language, SourceLocation};

    fn make_node(name: &str, kind: NodeKind, vis: Visibility) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            kind,
            SourceLocation::new("test.py".to_string(), 1, 10, 0, 0),
            Language::Python,
            vis,
        )
    }

    #[test]
    fn test_detect_main_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node("main", NodeKind::Function, Visibility::Public));
        graph.add_node(make_node("helper", NodeKind::Function, Visibility::Private));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();

        assert!(!entries.is_empty());
    }

    #[test]
    fn test_detect_test_entries() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node(
            "test_foo",
            NodeKind::Function,
            Visibility::Public,
        ));
        graph.add_node(make_node("helper", NodeKind::Function, Visibility::Private));

        let detector = EntryPointDetector::new(&graph);
        let test_entries = detector.detect_test_entry_points();

        assert!(!test_entries.is_empty());
    }

    fn make_node_with_attrs(
        name: &str,
        kind: NodeKind,
        vis: Visibility,
        attrs: Vec<&str>,
    ) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            kind,
            SourceLocation::new("test.py".to_string(), 1, 10, 0, 0),
            Language::Python,
            vis,
        )
        .with_attributes(attrs.into_iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn test_spring_bean_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node_with_attrs(
            "myService",
            NodeKind::Function,
            Visibility::Public,
            vec!["Bean"],
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "Spring @Bean should be detected as entry point"
        );
    }

    #[test]
    fn test_spring_controller_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node_with_attrs(
            "UserController",
            NodeKind::Class,
            Visibility::Public,
            vec!["RestController"],
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "Spring @RestController should be detected as entry point"
        );
    }

    #[test]
    fn test_spring_scheduled_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node_with_attrs(
            "runCleanup",
            NodeKind::Function,
            Visibility::Public,
            vec!["Scheduled"],
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "Spring @Scheduled should be detected as entry point"
        );
    }

    #[test]
    fn test_aspnet_http_verbs_are_framework_entries() {
        for attr in &[
            "HttpGet",
            "HttpPost",
            "HttpPut",
            "HttpDelete",
            "ApiController",
        ] {
            let mut graph = CodeGraph::new();
            graph.add_node(make_node_with_attrs(
                "action",
                NodeKind::Function,
                Visibility::Public,
                vec![attr],
            ));

            let detector = EntryPointDetector::new(&graph);
            let entries = detector.detect_production_entry_points();
            assert!(
                !entries.is_empty(),
                "ASP.NET attribute '{}' should be detected as entry point",
                attr
            );
        }
    }

    #[test]
    fn test_fastapi_name_pattern_is_framework_entry() {
        for name in &["app.get", "app.post", "app.put", "app.delete", "app.route"] {
            let mut graph = CodeGraph::new();
            graph.add_node(make_node(name, NodeKind::Function, Visibility::Public));

            let detector = EntryPointDetector::new(&graph);
            let entries = detector.detect_production_entry_points();
            assert!(
                !entries.is_empty(),
                "FastAPI/Flask name pattern '{}' should be detected as entry point",
                name
            );
        }
    }

    #[test]
    fn test_express_name_pattern_is_framework_entry() {
        for name in &["router.get", "router.post", "router.put", "router.delete"] {
            let mut graph = CodeGraph::new();
            graph.add_node(make_node(name, NodeKind::Function, Visibility::Public));

            let detector = EntryPointDetector::new(&graph);
            let entries = detector.detect_production_entry_points();
            assert!(
                !entries.is_empty(),
                "Express name pattern '{}' should be detected as entry point",
                name
            );
        }
    }

    #[test]
    fn test_go_gin_echo_name_pattern_is_framework_entry() {
        for name in &["r.GET", "r.POST", "e.GET", "e.POST"] {
            let mut graph = CodeGraph::new();
            graph.add_node(make_node(name, NodeKind::Function, Visibility::Public));

            let detector = EntryPointDetector::new(&graph);
            let entries = detector.detect_production_entry_points();
            assert!(
                !entries.is_empty(),
                "Go Gin/Echo name pattern '{}' should be detected as entry point",
                name
            );
        }
    }

    #[test]
    fn test_request_mapping_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node_with_attrs(
            "handleRequest",
            NodeKind::Function,
            Visibility::Public,
            vec!["RequestMapping"],
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "Spring @RequestMapping should be detected as entry point"
        );
    }

    #[test]
    fn test_post_construct_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node_with_attrs(
            "initialize",
            NodeKind::Function,
            Visibility::Public,
            vec!["PostConstruct"],
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "Spring @PostConstruct should be detected as entry point"
        );
    }

    #[test]
    fn test_serde_default_is_framework_entry() {
        let mut graph = CodeGraph::new();
        // A struct node with serde_default attribute referencing "default_page_size"
        graph.add_node(make_node_with_attrs(
            "Config",
            NodeKind::Struct,
            Visibility::Public,
            vec!["serde_default:default_page_size"],
        ));
        // The function referenced by serde
        graph.add_node(CodeNode::new(
            "default_page_size".to_string(),
            NodeKind::Function,
            SourceLocation::new("config.rs".to_string(), 20, 25, 0, 0),
            Language::Rust,
            Visibility::Private,
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        let entry_names: Vec<String> = entries
            .iter()
            .filter_map(|idx| graph.get_node(*idx).map(|n| n.name.clone()))
            .collect();
        assert!(
            entry_names.contains(&"default_page_size".to_string()),
            "Serde-referenced function should be detected as entry point, got: {:?}",
            entry_names
        );
    }

    #[test]
    fn test_serde_serialize_with_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node_with_attrs(
            "serialize_date",
            NodeKind::Function,
            Visibility::Private,
            vec!["serde_serialize_with:serialize_date"],
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "serde_serialize_with function should be detected as entry point"
        );
    }

    #[test]
    fn test_impl_from_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(
            CodeNode::new(
                "from".to_string(),
                NodeKind::Function,
                SourceLocation::new("types.rs".to_string(), 1, 10, 0, 0),
                Language::Rust,
                Visibility::Public,
            )
            .with_attributes(vec!["impl_trait:From".to_string()]),
        );

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "impl From function should be detected as entry point"
        );
    }

    #[test]
    fn test_impl_display_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(
            CodeNode::new(
                "fmt".to_string(),
                NodeKind::Function,
                SourceLocation::new("types.rs".to_string(), 1, 10, 0, 0),
                Language::Rust,
                Visibility::Public,
            )
            .with_attributes(vec!["impl_trait:Display".to_string()]),
        );

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "impl Display function should be detected as entry point"
        );
    }

    #[test]
    fn test_impl_default_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(
            CodeNode::new(
                "default".to_string(),
                NodeKind::Function,
                SourceLocation::new("types.rs".to_string(), 1, 10, 0, 0),
                Language::Rust,
                Visibility::Public,
            )
            .with_attributes(vec!["impl_trait:Default".to_string()]),
        );

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "impl Default function should be detected as entry point"
        );
    }

    #[test]
    fn test_plain_impl_method_not_framework_entry() {
        let mut graph = CodeGraph::new();
        // A method in a plain impl (no trait) should NOT be a framework entry
        graph.add_node(CodeNode::new(
            "helper".to_string(),
            NodeKind::Function,
            SourceLocation::new("types.rs".to_string(), 1, 10, 0, 0),
            Language::Rust,
            Visibility::Private,
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            entries.is_empty(),
            "Plain impl method should not be framework entry"
        );
    }

    #[test]
    fn test_plain_function_not_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node(
            "helper_function",
            NodeKind::Function,
            Visibility::Private,
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            entries.is_empty(),
            "A plain private function should not be detected as entry point"
        );
    }

    #[test]
    fn test_impl_custom_trait_is_framework_entry() {
        // This test verifies the wildcard `impl_trait:*` matching,
        // which previously only matched 18 hardcoded traits.
        let mut graph = CodeGraph::new();
        graph.add_node(
            CodeNode::new(
                "validate".to_string(),
                NodeKind::Function,
                SourceLocation::new("types.rs".to_string(), 1, 10, 0, 0),
                Language::Rust,
                Visibility::Public,
            )
            .with_attributes(vec!["impl_trait:Validate".to_string()]),
        );

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "impl Validate (custom trait) should be detected as entry point"
        );
    }

    #[test]
    fn test_impl_arbitrary_trait_is_framework_entry() {
        // Any trait impl should be treated as entry point, not just std ones
        for trait_name in &[
            "MyAppHandler",
            "CustomSerializer",
            "ProtobufMessage",
            "Validator",
        ] {
            let mut graph = CodeGraph::new();
            graph.add_node(
                CodeNode::new(
                    "method".to_string(),
                    NodeKind::Function,
                    SourceLocation::new("types.rs".to_string(), 1, 10, 0, 0),
                    Language::Rust,
                    Visibility::Public,
                )
                .with_attributes(vec![format!("impl_trait:{}", trait_name)]),
            );

            let detector = EntryPointDetector::new(&graph);
            let entries = detector.detect_production_entry_points();
            assert!(
                !entries.is_empty(),
                "impl {} should be detected as entry point",
                trait_name
            );
        }
    }

    #[test]
    fn test_extends_attribute_is_framework_entry() {
        // Methods in classes that extend a parent should be treated as entry points
        let mut graph = CodeGraph::new();
        graph.add_node(
            CodeNode::new(
                "speak".to_string(),
                NodeKind::Function,
                SourceLocation::new("dog.py".to_string(), 1, 10, 0, 0),
                Language::Python,
                Visibility::Public,
            )
            .with_attributes(vec!["extends:Animal".to_string()]),
        );

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "Method with extends:Animal attribute should be detected as entry point"
        );
    }

    #[test]
    fn test_detect_config_entry_points_dockerfile() {
        use std::io::Write;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();

        // Create a Dockerfile
        let dockerfile_path = dir.path().join("Dockerfile");
        let mut f = std::fs::File::create(&dockerfile_path).unwrap();
        writeln!(f, "FROM python:3.9").unwrap();
        writeln!(f, "COPY . /app").unwrap();
        writeln!(f, "CMD [\"python\", \"main.py\"]").unwrap();

        // Create a graph with a module node for main.py
        let mut graph = CodeGraph::new();
        graph.add_node(CodeNode::new(
            "<module:main.py>".to_string(),
            NodeKind::Function,
            SourceLocation::new("main.py".to_string(), 1, 10, 0, 0),
            Language::Python,
            Visibility::Public,
        ));
        graph.add_node(CodeNode::new(
            "main".to_string(),
            NodeKind::Function,
            SourceLocation::new("main.py".to_string(), 5, 10, 0, 0),
            Language::Python,
            Visibility::Public,
        ));

        let config_entries = detect_config_entry_points(dir.path(), &graph);
        // Should find main.py entries via the Dockerfile CMD
        let entry_names: Vec<String> = config_entries
            .iter()
            .filter_map(|idx| graph.get_node(*idx).map(|n| n.name.clone()))
            .collect();
        assert!(
            entry_names
                .iter()
                .any(|n| n == "main" || n.contains("main.py")),
            "Dockerfile CMD should detect main.py as entry, got: {:?}",
            entry_names
        );
    }

    #[test]
    fn test_detect_config_entry_points_package_json() {
        use std::io::Write;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();

        // Create package.json
        let pkg_path = dir.path().join("package.json");
        let mut f = std::fs::File::create(&pkg_path).unwrap();
        writeln!(
            f,
            r#"{{"main": "src/index.js", "scripts": {{"start": "node src/server.js"}}}}"#
        )
        .unwrap();

        // Create a graph with nodes for those files
        let mut graph = CodeGraph::new();
        graph.add_node(CodeNode::new(
            "<module:src/index.js>".to_string(),
            NodeKind::Function,
            SourceLocation::new("src/index.js".to_string(), 1, 10, 0, 0),
            Language::JavaScript,
            Visibility::Public,
        ));
        graph.add_node(CodeNode::new(
            "<module:src/server.js>".to_string(),
            NodeKind::Function,
            SourceLocation::new("src/server.js".to_string(), 1, 10, 0, 0),
            Language::JavaScript,
            Visibility::Public,
        ));

        let config_entries = detect_config_entry_points(dir.path(), &graph);
        let entry_names: Vec<String> = config_entries
            .iter()
            .filter_map(|idx| graph.get_node(*idx).map(|n| n.name.clone()))
            .collect();
        assert!(
            !config_entries.is_empty(),
            "package.json should detect entry points, got: {:?}",
            entry_names
        );
    }

    #[test]
    fn test_python_dataclass_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node_with_attrs(
            "User",
            NodeKind::Class,
            Visibility::Public,
            vec!["dataclass"],
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "Python @dataclass should be detected as entry point"
        );
    }

    #[test]
    fn test_java_lombok_annotations_are_framework_entries() {
        for attr in &[
            "Data",
            "Getter",
            "Setter",
            "Builder",
            "NoArgsConstructor",
            "AllArgsConstructor",
            "Value",
        ] {
            let mut graph = CodeGraph::new();
            graph.add_node(make_node_with_attrs(
                "UserDto",
                NodeKind::Class,
                Visibility::Public,
                vec![attr],
            ));

            let detector = EntryPointDetector::new(&graph);
            let entries = detector.detect_production_entry_points();
            assert!(
                !entries.is_empty(),
                "Java Lombok @{} should be detected as entry point",
                attr
            );
        }
    }

    #[test]
    fn test_jpa_entity_is_framework_entry() {
        for attr in &["Entity", "Table", "MappedSuperclass", "Embeddable"] {
            let mut graph = CodeGraph::new();
            graph.add_node(make_node_with_attrs(
                "UserEntity",
                NodeKind::Class,
                Visibility::Public,
                vec![attr],
            ));

            let detector = EntryPointDetector::new(&graph);
            let entries = detector.detect_production_entry_points();
            assert!(
                !entries.is_empty(),
                "JPA @{} should be detected as entry point",
                attr
            );
        }
    }

    #[test]
    fn test_csharp_serialization_attrs_are_framework_entries() {
        for attr in &[
            "Serializable",
            "DataContract",
            "DataMember",
            "JsonConverter",
            "ProtoContract",
        ] {
            let mut graph = CodeGraph::new();
            graph.add_node(make_node_with_attrs(
                "UserModel",
                NodeKind::Class,
                Visibility::Public,
                vec![attr],
            ));

            let detector = EntryPointDetector::new(&graph);
            let entries = detector.detect_production_entry_points();
            assert!(
                !entries.is_empty(),
                "C# [{}] should be detected as entry point",
                attr
            );
        }
    }

    #[test]
    fn test_kotlin_parcelize_is_framework_entry() {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node_with_attrs(
            "UserParcel",
            NodeKind::Class,
            Visibility::Public,
            vec!["Parcelize"],
        ));

        let detector = EntryPointDetector::new(&graph);
        let entries = detector.detect_production_entry_points();
        assert!(
            !entries.is_empty(),
            "Kotlin @Parcelize should be detected as entry point"
        );
    }
}
