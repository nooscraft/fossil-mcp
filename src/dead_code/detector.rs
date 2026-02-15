//! Main dead code detector — orchestrates entry point detection, reachability, and classification.

use std::collections::HashSet;
use std::path::Path;

use crate::analysis::Pipeline;
use crate::core::{
    Confidence, FossilType, LineOffsetTable, NodeKind, ParsedFile, RemovalImpact, Severity,
};
use crate::graph::CodeGraph;
use crate::parsers::ParserRegistry;
use petgraph::graph::NodeIndex;

use super::classifier::{DeadCodeClassifier, DeadCodeFinding};
use super::entry_points::EntryPointDetector;

// SDG-based inter-procedural slicing support
use crate::graph::sdg::{InterproceduralSliceCriterion, SystemDependenceGraph};

// RTA for more precise virtual call resolution
use crate::graph::class_hierarchy::ClassHierarchy;
use crate::graph::rta::RapidTypeAnalysis;

/// Configuration for the dead code detector.
#[derive(Debug, Clone)]
pub struct DetectorConfig {
    pub include_tests: bool,
    pub min_confidence: crate::core::Confidence,
    pub min_lines: usize,
    pub exclude_patterns: Vec<String>,
    /// Enable def-use chain analysis for dead store detection.
    pub detect_dead_stores: bool,
    /// Use Rapid Type Analysis (RTA) for more precise virtual call resolution
    /// during reachability computation. When enabled, only types that are
    /// actually instantiated in the reachable program are considered as
    /// potential virtual call targets, reducing false negatives in dead code
    /// detection for object-oriented codebases.
    pub use_rta: bool,
    /// Use System Dependence Graph (SDG) for more precise dead code narrowing.
    /// When enabled, functions covered by the SDG are intersected with SDG
    /// liveness data, reducing false positives.
    pub use_sdg: bool,
    /// Custom entry point rules from config. If None, uses hardcoded defaults.
    pub entry_point_rules: Option<crate::config::ResolvedEntryPointRules>,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            include_tests: true,
            min_confidence: crate::core::Confidence::Low,
            min_lines: 0,
            exclude_patterns: Vec::new(),
            detect_dead_stores: true,
            use_rta: false,
            use_sdg: false,
            entry_point_rules: None,
        }
    }
}

/// Result of dead code detection.
#[derive(Debug)]
pub struct DetectionResult {
    pub findings: Vec<DeadCodeFinding>,
    pub total_nodes: usize,
    pub reachable_nodes: usize,
    pub unreachable_nodes: usize,
    pub entry_points: usize,
    pub test_entry_points: usize,
}

/// Main dead code detector.
pub struct Detector {
    config: DetectorConfig,
}

impl Detector {
    pub fn new(config: DetectorConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self {
            config: DetectorConfig::default(),
        }
    }

    /// Run dead code detection on a directory.
    pub fn detect(&self, root: &Path) -> Result<DetectionResult, crate::core::Error> {
        let pipeline = Pipeline::with_defaults();
        let pipeline_result = pipeline.run(root)?;

        let result = self.detect_with_parsed_files(&pipeline_result.graph, &pipeline_result.parsed_files)?;
        Ok(result)
    }

    /// Run dead code detection on a pre-built CodeGraph.
    pub fn detect_in_graph(
        &self,
        graph: &CodeGraph,
    ) -> Result<DetectionResult, crate::core::Error> {
        self.detect_with_parsed_files(graph, &[])
    }

    /// Run dead code detection on a pre-built CodeGraph with optional parsed files
    /// for def-use chain analysis.
    pub fn detect_with_parsed_files(
        &self,
        graph: &CodeGraph,
        parsed_files: &[ParsedFile],
    ) -> Result<DetectionResult, crate::core::Error> {

        // Detect entry points using config rules if provided
        let entry_detector = if let Some(ref rules) = self.config.entry_point_rules {
            EntryPointDetector::with_rules(graph, rules.clone())
        } else {
            EntryPointDetector::new(graph)
        };
        let mut production_entries = entry_detector.detect_production_entry_points();
        let test_entries = entry_detector.detect_test_entry_points();

        // Detect config-based entry points (Dockerfile, docker-compose, package.json)
        // Only if we can infer a root path from the parsed files
        if let Some(root_path) = Self::infer_root_path(parsed_files) {
            let config_entries = crate::dead_code::entry_points::detect_config_entry_points(
                Path::new(&root_path),
                graph,
            );
            production_entries.extend(config_entries);
        }

        // Compute reachability
        let rta_mode = if self.config.use_rta { "with RTA" } else { "BFS" };
        let production_reachable = if self.config.use_rta {
            Self::compute_reachable_with_rta(graph, &production_entries)
        } else {
            graph.compute_reachable(&production_entries)
        };
        let test_reachable = if self.config.include_tests {
            if self.config.use_rta {
                Self::compute_reachable_with_rta(graph, &test_entries)
            } else {
                graph.compute_reachable(&test_entries)
            }
        } else {
            HashSet::new()
        };

        // Def-use chain dead store detection
        let dead_store_findings = if self.config.detect_dead_stores && !parsed_files.is_empty() {
            Self::detect_dead_stores(parsed_files)
        } else {
            Vec::new()
        };
        if !dead_store_findings.is_empty() {
        }

        // Classify dead code
        let classifier = DeadCodeClassifier::new(graph);
        let mut findings = classifier.classify(&production_reachable, &test_reachable);

        // Merge dead store findings into main findings
        findings.extend(dead_store_findings);

        // Apply filters
        findings.retain(|f| f.confidence >= self.config.min_confidence);
        if self.config.min_lines > 0 {
            findings.retain(|f| f.lines_of_code >= self.config.min_lines);
        }

        // Exclude shell scripts — dead function detection is unreliable for
        // shell languages where functions are invoked via sourcing, eval, or CLI.
        findings.retain(|f| !Self::is_shell_file(&f.file));

        // When include_tests is false, exclude test-identified nodes and
        // findings from test directories/files. Without this, test utility
        // functions (not tests themselves) still appear as dead.
        if !self.config.include_tests {
            findings.retain(|f| !test_entries.contains(&f.node_index));
            findings.retain(|f| !Self::is_test_file(&f.file));
        }

        // Remove tautological "test-only" findings: test functions being
        // "only reachable from test code" is expected behavior, not a defect.
        // This applies whether the test is in a dedicated test file OR in an
        // inline `#[cfg(test)] mod tests {}` block within a regular source file.
        findings.retain(|f| f.fossil_type != FossilType::TestOnlyCode);

        let all_reachable: HashSet<NodeIndex> = production_reachable
            .union(&test_reachable)
            .copied()
            .collect();

        Ok(DetectionResult {
            findings,
            total_nodes: graph.node_count(),
            reachable_nodes: all_reachable.len(),
            unreachable_nodes: graph.node_count() - all_reachable.len(),
            entry_points: production_entries.len(),
            test_entry_points: test_entries.len(),
        })
    }

    /// Run dead code detection using SDG-based inter-procedural slicing.
    ///
    /// This is more precise than graph reachability alone: it uses the
    /// System Dependence Graph to compute inter-procedural backward slices
    /// from every exit block across all functions. A node is considered dead
    /// if it does not appear in any such backward slice.
    ///
    /// When the SDG does not cover a particular function (e.g., because no
    /// CFG/PDG was built for it), the method falls back to the standard
    /// graph-reachability-based detection for that function.
    pub fn detect_with_sdg(
        &self,
        graph: &CodeGraph,
        sdg: &SystemDependenceGraph,
    ) -> Result<DetectionResult, crate::core::Error> {
        // Collect all SDG nodes that are "live" by backward slicing from
        // every exit block in every function's CFG.
        let mut sdg_live_blocks: HashSet<crate::graph::SdgNode> = HashSet::new();

        for (&func_idx, cfg) in &sdg.function_cfgs {
            // Find exit blocks.
            let exit_blocks: Vec<crate::graph::CfgNodeId> = cfg
                .blocks()
                .filter(|(_, bb)| bb.is_exit)
                .map(|(&id, _)| id)
                .collect();

            for exit_id in exit_blocks {
                let criterion = InterproceduralSliceCriterion {
                    func: func_idx,
                    block: exit_id,
                    variable: None,
                };
                let slice = sdg.interprocedural_backward_slice(&criterion);
                sdg_live_blocks.extend(slice.nodes);
            }

            // Entry blocks are always considered live.
            if let Some(entry_id) = cfg.entry() {
                sdg_live_blocks.insert(crate::graph::SdgNode {
                    func: func_idx,
                    block: entry_id,
                });
            }
        }

        // Map SDG live functions to the CodeGraph NodeIndex space.
        let sdg_live_functions: HashSet<NodeIndex> =
            sdg_live_blocks.iter().map(|n| n.func).collect();

        // Fall back to standard reachability for the overall CodeGraph.
        let entry_detector = EntryPointDetector::new(graph);
        let production_entries = entry_detector.detect_production_entry_points();
        let test_entries = entry_detector.detect_test_entry_points();

        let production_reachable = graph.compute_reachable(&production_entries);

        // Refine reachability using SDG liveness: if a function is covered
        // by the SDG but not in sdg_live_functions, remove it from reachable.
        let sdg_covered: HashSet<NodeIndex> = sdg.function_cfgs.keys().copied().collect();
        let production_reachable: HashSet<NodeIndex> = production_reachable
            .into_iter()
            .filter(|idx| {
                if sdg_covered.contains(idx) {
                    sdg_live_functions.contains(idx)
                } else {
                    true // Not covered by SDG, keep as reachable
                }
            })
            .collect();

        let test_reachable = if self.config.include_tests {
            graph.compute_reachable(&test_entries)
        } else {
            HashSet::new()
        };

        // Classify using graph-level reachability.
        let classifier = DeadCodeClassifier::new(graph);
        let mut findings = classifier.classify(&production_reachable, &test_reachable);

        // Apply filters.
        findings.retain(|f| f.confidence >= self.config.min_confidence);
        if self.config.min_lines > 0 {
            findings.retain(|f| f.lines_of_code >= self.config.min_lines);
        }
        findings.retain(|f| !Self::is_shell_file(&f.file));
        if !self.config.include_tests {
            findings.retain(|f| !test_entries.contains(&f.node_index));
            findings.retain(|f| !Self::is_test_file(&f.file));
        }
        // Remove tautological "test-only" findings (same as primary detect path)
        findings.retain(|f| f.fossil_type != FossilType::TestOnlyCode);

        let all_reachable: HashSet<NodeIndex> = production_reachable
            .union(&test_reachable)
            .copied()
            .collect();

        Ok(DetectionResult {
            findings,
            total_nodes: graph.node_count(),
            reachable_nodes: all_reachable.len(),
            unreachable_nodes: graph.node_count() - all_reachable.len(),
            entry_points: production_entries.len(),
            test_entry_points: test_entries.len(),
        })
    }

    /// Compute reachable nodes using RTA for more precise virtual call resolution.
    ///
    /// Builds a class hierarchy from the code graph's nodes, runs Rapid Type
    /// Analysis starting from the given entry points, and returns the set of
    /// reachable methods discovered by RTA. Falls back to standard BFS
    /// reachability if no class hierarchy information is available in the graph.
    fn compute_reachable_with_rta(
        graph: &CodeGraph,
        entry_points: &HashSet<NodeIndex>,
    ) -> HashSet<NodeIndex> {
        // Collect all CodeNodes to build the class hierarchy.
        let nodes: Vec<crate::core::CodeNode> =
            graph.nodes().map(|(_, node)| node.clone()).collect();

        let hierarchy = ClassHierarchy::build_from_nodes(&nodes);

        // If no types were found, RTA adds no value over BFS.
        if hierarchy.types.is_empty() {
            return graph.compute_reachable(entry_points);
        }

        let rta = RapidTypeAnalysis::analyze(graph, &hierarchy, entry_points);
        rta.reachable_methods
    }

    /// Infer the project root path from parsed files.
    /// Uses the common prefix of all file paths, or falls back to the first file's parent.
    fn infer_root_path(parsed_files: &[ParsedFile]) -> Option<String> {
        if parsed_files.is_empty() {
            return None;
        }
        // Use the first file's path to find a common root
        let first_path = Path::new(&parsed_files[0].path);
        if first_path.is_absolute() {
            // Find common ancestor of all paths
            let mut common: &Path = first_path;
            for pf in parsed_files.iter().skip(1) {
                let p = Path::new(&pf.path);
                // Walk up until we find a common prefix
                while !p.starts_with(common) {
                    common = match common.parent() {
                        Some(parent) => parent,
                        None => return Some("/".to_string()),
                    };
                }
            }
            Some(common.to_string_lossy().to_string())
        } else {
            // Relative paths - try current directory or parent of first file
            first_path
                .parent()
                .map(|p| p.to_string_lossy().to_string())
                .or_else(|| Some(".".to_string()))
        }
    }

    /// Check if a file path looks like it belongs to a test directory or is a test file.
    fn is_test_file(path: &str) -> bool {
        crate::dead_code::entry_points::EntryPointDetector::is_test_file(path)
    }

    /// Check if a file is a shell script (unreliable for dead code analysis).
    fn is_shell_file(path: &str) -> bool {
        let normalized = path.replace('\\', "/");
        if let Some(file_name) = normalized.rsplit('/').next() {
            file_name.ends_with(".sh") || file_name.ends_with(".bash")
        } else {
            false
        }
    }

    /// Detect dead stores using def-use chain analysis.
    ///
    /// For each parsed file, attempts to re-parse the source with tree-sitter,
    /// walks the AST to find function bodies, builds a simple 2-block CFG
    /// (entry -> exit) per function, extracts defs/uses via `var_extractor`,
    /// and runs `DataFlowGraph::find_dead_stores()`.
    fn detect_dead_stores(parsed_files: &[ParsedFile]) -> Vec<DeadCodeFinding> {
        let registry = match ParserRegistry::with_defaults() {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        let mut findings = Vec::new();

        for pf in parsed_files {
            let ext = std::path::Path::new(&pf.path)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");
            let parser = match registry
                .get_parser_for_extension(ext)
                .or_else(|| registry.get_parser(pf.language))
            {
                Some(p) => p,
                None => continue,
            };

            let tree = match parser.parse(&pf.source) {
                Ok(t) => t,
                Err(_) => continue,
            };

            let line_table = LineOffsetTable::new(&pf.source);

            // Walk the AST root to find function definitions
            let root = tree.root();
            Self::find_functions_and_analyze(
                root.as_ref(),
                &pf.path,
                &line_table,
                pf.language,
                &mut findings,
            );
        }

        findings
    }

    /// Recursively walk a tree node to find function definitions and analyze them
    /// for dead stores.
    fn find_functions_and_analyze(
        root: &dyn crate::core::TreeNode,
        file_path: &str,
        line_table: &LineOffsetTable,
        language: crate::core::Language,
        findings: &mut Vec<DeadCodeFinding>,
    ) {
        let kind = root.node_type();
        let is_function = matches!(
            kind,
            "function_definition"
                | "function_declaration"
                | "method_definition"
                | "method_declaration"
                | "function_item"
        );

        if is_function {
            Self::analyze_function_body(root, file_path, line_table, language, findings);
        }

        // Recurse into children
        for child in root.children() {
            Self::find_functions_and_analyze(
                child.as_ref(),
                file_path,
                line_table,
                language,
                findings,
            );
        }
    }

    /// Analyze a function body for dead stores using byte-position ordering.
    ///
    /// Collects variable definitions and uses from the function's AST subtree.
    /// A definition is flagged as dead if no use of the same variable name
    /// exists at a strictly later byte position in the source code.
    fn analyze_function_body(
        func_node: &dyn crate::core::TreeNode,
        file_path: &str,
        line_table: &LineOffsetTable,
        _language: crate::core::Language,
        findings: &mut Vec<DeadCodeFinding>,
    ) {
        let func_name = Self::extract_function_name(func_node);

        // Collect defs and uses from the entire function subtree.
        // Defs: (name, start_byte, end_byte, in_conditional)
        let mut all_defs: Vec<(String, usize, usize, bool)> = Vec::new();
        // Uses: (name, start_byte)
        let mut all_uses: Vec<(String, usize)> = Vec::new();

        Self::collect_defs(func_node, &mut all_defs, false);
        Self::collect_uses(func_node, &mut all_uses, _language);

        // Collect variable names referenced inside nested function bodies
        // (closures, callbacks). These are "captured" variables whose stores
        // should not be flagged as dead — they may be read across invocations.
        let mut captured_vars: HashSet<String> = HashSet::new();
        Self::collect_captured_vars(func_node, &mut captured_vars);

        // Collect locally-declared variable names (let/const/var/parameters).
        // Assignments to variables NOT in this set are outer-scope (module-level)
        // variables whose writes may be observable by other functions.
        let mut local_declarations: HashSet<String> = HashSet::new();
        Self::collect_local_declarations(func_node, &mut local_declarations);

        // Collect variables that appear in loop conditions/iterators.
        // These are read on every iteration via the loop back-edge, so
        // assignments inside the loop body are NOT dead stores.
        let mut loop_control_vars: HashSet<String> = HashSet::new();
        Self::collect_loop_control_vars(func_node, &mut loop_control_vars);

        if all_defs.is_empty() {
            return;
        }

        // OPTIMIZATION: Pre-build index of uses by variable name (O(u))
        // instead of scanning all_uses linearly for each def (O(d × u))
        let mut uses_by_name: std::collections::HashMap<&str, Vec<usize>> =
            std::collections::HashMap::new();
        for (use_name, use_pos) in &all_uses {
            uses_by_name
                .entry(use_name.as_str())
                .or_default()
                .push(*use_pos);
        }
        // Sort positions for binary search (O(u log u))
        for positions in uses_by_name.values_mut() {
            positions.sort_unstable();
        }

        // A def is dead if no use of the same variable name exists at a strictly
        // later byte position. This is a conservative approximation (low FP, some FN).
        for (def_name, start_byte, end_byte, _in_cond) in &all_defs {
            // Skip variables captured by nested closures — they may be read
            // on subsequent invocations.
            if captured_vars.contains(def_name) {
                continue;
            }
            // Skip variables not declared in this function — they're outer-scope
            // (module-level) variables whose assignments may be read by other
            // functions or subsequent calls to this function.
            if !local_declarations.contains(def_name) {
                continue;
            }
            // Convention: _-prefixed variables are intentionally unused
            // (Rust, TypeScript, JavaScript, Python)
            if def_name.starts_with('_') {
                continue;
            }
            // Skip variables used in loop conditions — the loop back-edge
            // means the condition re-reads the variable on the next iteration.
            if loop_control_vars.contains(def_name) {
                continue;
            }

            // OPTIMIZATION: O(log u) binary search on indexed positions instead of O(u) linear scan
            let has_later_use = uses_by_name
                .get(def_name.as_str())
                .map(|positions| {
                    // Binary search for first position > start_byte
                    match positions.binary_search(&start_byte) {
                        Ok(idx) => idx + 1 < positions.len(),  // Found exact match, check if later use exists
                        Err(idx) => idx < positions.len(),      // Not found, idx is insertion point
                    }
                })
                .unwrap_or(false);

            if !has_later_use {
                let line_start = line_table.byte_to_line1(*start_byte);
                let line_end = line_table.byte_to_line1(*end_byte);

                findings.push(DeadCodeFinding {
                    node_index: NodeIndex::new(0),
                    name: def_name.clone(),
                    full_name: format!("{}::{}", func_name, def_name),
                    kind: NodeKind::Variable,
                    fossil_type: FossilType::UnusedVariable,
                    confidence: Confidence::Certain,
                    severity: Severity::Medium,
                    removal_impact: RemovalImpact::Safe,
                    reason: format!(
                        "variable `{}` is assigned but never read in `{}`",
                        def_name, func_name
                    ),
                    file: file_path.to_string(),
                    line_start,
                    line_end,
                    lines_of_code: 1,
                });
            }
        }

        // Second pass: detect overwrites-before-read.
        // Group defs by variable name, sorted by position. For consecutive
        // defs of the same variable, if there's no use between them, the
        // first def is dead (overwritten before being read). This catches
        // assignments in loops that overwrite without accumulating.
        // (start_byte, end_byte, in_conditional)
        let mut defs_by_name: std::collections::HashMap<&str, Vec<(usize, usize, bool)>> =
            std::collections::HashMap::new();
        for (name, start, end, in_cond) in &all_defs {
            if captured_vars.contains(name) {
                continue;
            }
            if !local_declarations.contains(name) {
                continue;
            }
            if name.starts_with('_') {
                continue;
            }
            if loop_control_vars.contains(name) {
                continue;
            }
            defs_by_name
                .entry(name.as_str())
                .or_default()
                .push((*start, *end, *in_cond));
        }

        // Track already-reported positions to avoid duplicating findings from the first pass
        let already_reported: HashSet<usize> = findings
            .iter()
            .filter(|f| f.file == file_path)
            .map(|f| f.line_start)
            .collect();

        for (name, positions) in &defs_by_name {
            if positions.len() < 2 {
                continue;
            }
            let mut sorted = positions.clone();
            sorted.sort();
            for window in sorted.windows(2) {
                let (first_start, first_end, _first_in_cond) = window[0];
                let (second_start, _, second_in_cond) = window[1];
                let has_use_between = all_uses.iter().any(|(use_name, use_pos)| {
                    use_name == *name && *use_pos > first_start && *use_pos < second_start
                });
                if !has_use_between {
                    // Skip when the second def is inside any conditional branch.
                    // This covers two patterns:
                    // 1. Default→branch: `let x = default; if (cond) { x = val; }`
                    //    The first def is a fallback — not dead.
                    // 2. Branch→branch: `if (a) { x = 1; } else if (b) { x = 2; }`
                    //    Sibling branches are mutually exclusive — not overwrites.
                    // Conservative: may miss some true overwrites inside conditionals,
                    // but eliminates an entire class of false positives.
                    if second_in_cond {
                        continue;
                    }

                    // Guard against `x = f(x)` false positives: when the
                    // second def's RHS reads the variable, the first def IS
                    // used.  In tree-sitter the LHS identifier has a byte
                    // position *before* the RHS identifier even though the
                    // read happens first at runtime.  We detect this by
                    // looking for a use of the same variable at a position
                    // strictly after the second def's LHS on the same
                    // source line (covers single-line `x = f(x)`,
                    // `x = x + 1`, etc.).
                    let second_line = line_table.byte_to_line1(second_start);
                    let second_def_reads_var = all_uses.iter().any(|(use_name, use_pos)| {
                        use_name == *name
                            && *use_pos > second_start
                            && line_table.byte_to_line1(*use_pos) == second_line
                    });
                    if second_def_reads_var {
                        continue;
                    }

                    let line_start = line_table.byte_to_line1(first_start);
                    let line_end = line_table.byte_to_line1(first_end);
                    // Skip if already reported by the first pass
                    if already_reported.contains(&line_start) {
                        continue;
                    }
                    findings.push(DeadCodeFinding {
                        node_index: NodeIndex::new(0),
                        name: name.to_string(),
                        full_name: format!("{}::{}", func_name, name),
                        kind: NodeKind::Variable,
                        fossil_type: FossilType::UnusedVariable,
                        confidence: Confidence::High,
                        severity: Severity::Medium,
                        removal_impact: RemovalImpact::Safe,
                        reason: format!(
                            "variable `{}` is overwritten before being read in `{}`",
                            name, func_name
                        ),
                        file: file_path.to_string(),
                        line_start,
                        line_end,
                        lines_of_code: 1,
                    });
                }
            }
        }
    }

    /// Extract a function name from a tree node by looking for an identifier child.
    fn extract_function_name(func_node: &dyn crate::core::TreeNode) -> String {
        for child in func_node.children() {
            if child.node_type() == "identifier" || child.node_type() == "name" {
                return child.text().to_string();
            }
        }
        "<anonymous>".to_string()
    }

    /// Collect variable definitions from a tree node (recursive).
    ///
    /// Handles assignment expressions, variable declarators, and language-specific
    /// declaration forms. Skips type annotation nodes and language keywords to
    /// extract actual variable names.
    ///
    /// Each def is stored as `(name, start_byte, end_byte, in_conditional)`.
    /// The `in_conditional` flag tracks whether the def is inside a conditional
    /// branch (if/else/switch/match/try-catch), used to suppress false positives
    /// for the "default value then conditional overwrite" pattern.
    fn collect_defs(
        node: &dyn crate::core::TreeNode,
        defs: &mut Vec<(String, usize, usize, bool)>,
        in_conditional: bool,
    ) {
        let kind = node.node_type();

        // Skip type annotation/reference nodes entirely — never extract defs from these
        if matches!(
            kind,
            "type_identifier"
                | "primitive_type"
                | "type_annotation"
                | "generic_type"
                | "pointer_type"
                | "reference_type"
                | "array_type"
                | "scoped_type_identifier"
                | "predefined_type"
        ) {
            return;
        }

        // Detect conditional branch nodes — defs inside these are conditional
        let is_conditional_node = matches!(
            kind,
            "if_statement"
                | "if_expression"
                | "else_clause"
                | "elif_clause"
                | "else"
                | "switch_statement"
                | "switch_expression"
                | "match_expression"
                | "match_arm"
                | "case_clause"
                | "switch_case"
                | "switch_body"
                | "ternary_expression"
                | "conditional_expression"
                | "try_statement"
                | "catch_clause"
                | "except_clause"
        );
        let child_conditional = in_conditional || is_conditional_node;

        // Variable declarator (JS/TS/Java/C#): LHS identifier is the def.
        // For destructuring patterns (object_pattern, array_pattern) on the LHS,
        // skip — we don't track individual destructured bindings as defs, and
        // crucially we must NOT mistake the RHS identifier for a def.
        if kind == "variable_declarator" || kind == "init_declarator" {
            let children = node.children();
            if let Some(first) = children.first() {
                let ft = first.node_type();
                if ft == "identifier" || ft == "name" {
                    let text = first.text();
                    if Self::is_var_name(text) {
                        defs.push((
                            text.to_string(),
                            first.start_byte(),
                            first.end_byte(),
                            in_conditional,
                        ));
                    }
                }
                // If the LHS is a pattern (object_pattern, array_pattern),
                // don't extract any defs — the RHS is a read, not a write.
            }
            return;
        }

        // Assignment (Python/Ruby/JS): first child is the target
        if kind == "assignment" || kind == "assignment_expression" || kind == "augmented_assignment"
        {
            if let Some(first) = node.children().first() {
                let ft = first.node_type();
                if ft == "identifier" || ft == "name" {
                    let text = first.text();
                    if Self::is_var_name(text) {
                        defs.push((
                            text.to_string(),
                            first.start_byte(),
                            first.end_byte(),
                            in_conditional,
                        ));
                    }
                }
            }
            return;
        }

        // Short var declaration (Go): x := expr
        if kind == "short_var_declaration" {
            if let Some(first) = node.children().first() {
                if first.node_type() == "expression_list" {
                    for gc in first.children() {
                        if gc.node_type() == "identifier" {
                            let text = gc.text();
                            if Self::is_var_name(text) {
                                defs.push((
                                    text.to_string(),
                                    gc.start_byte(),
                                    gc.end_byte(),
                                    in_conditional,
                                ));
                            }
                        }
                    }
                } else if first.node_type() == "identifier" {
                    let text = first.text();
                    if Self::is_var_name(text) {
                        defs.push((
                            text.to_string(),
                            first.start_byte(),
                            first.end_byte(),
                            in_conditional,
                        ));
                    }
                }
            }
            return;
        }

        // Let declaration (Rust): let x = expr;
        if kind == "let_declaration" {
            for child in node.children() {
                let ct = child.node_type();
                if ct == "identifier" {
                    let text = child.text();
                    if text != "let" && text != "mut" && Self::is_var_name(text) {
                        defs.push((
                            text.to_string(),
                            child.start_byte(),
                            child.end_byte(),
                            in_conditional,
                        ));
                        return;
                    }
                }
            }
            return;
        }

        // For if-statements with always-true conditions, skip the else branch
        // because those defs are dead code (unreachable), not dead stores.
        if matches!(kind, "if_statement" | "if_expression") {
            let children = node.children();
            let is_always_true = Self::is_always_true_condition(node);
            for child in &children {
                if Self::is_nested_function(child.node_type()) {
                    continue;
                }
                // Skip the else clause of always-true conditionals
                if is_always_true && matches!(child.node_type(), "else_clause" | "else") {
                    continue;
                }
                Self::collect_defs(child.as_ref(), defs, child_conditional);
            }
            return;
        }

        // Recurse into children for container nodes (expression_statement, etc.)
        // Skip nested function definitions — they form separate scopes.
        for child in node.children() {
            if Self::is_nested_function(child.node_type()) {
                continue;
            }
            Self::collect_defs(child.as_ref(), defs, child_conditional);
        }
    }

    /// Collect variable uses (identifier reads) from a tree node (recursive).
    ///
    /// Collects all identifiers except those inside type annotations and those
    /// that match language keywords. Uses are stored with their byte position
    /// in the source code for ordering comparisons.
    ///
    /// Also extracts variable captures from format strings (e.g. Rust
    /// `format!("{var}")`, C# `$"{var}"`, Kotlin `"${var}"`) where
    /// tree-sitter does not decompose the interpolation into identifier nodes.
    fn collect_uses(
        node: &dyn crate::core::TreeNode,
        uses: &mut Vec<(String, usize)>,
        language: crate::core::Language,
    ) {
        let kind = node.node_type();

        // Special handling for macro invocations - recurse into token trees (#22)
        // Variables inside assert!(), println!(), etc. are used and should not be flagged as unused
        if kind == "macro_invocation" {
            for child in node.children() {
                if child.node_type() == "token_tree" {
                    Self::collect_uses(child.as_ref(), uses, language);
                }
            }
            return;
        }

        // Skip type annotation/reference nodes
        if matches!(
            kind,
            "type_identifier"
                | "primitive_type"
                | "type_annotation"
                | "generic_type"
                | "pointer_type"
                | "reference_type"
                | "array_type"
                | "scoped_type_identifier"
                | "predefined_type"
        ) {
            return;
        }

        if kind == "identifier" || kind == "name" || kind == "shorthand_property_identifier" {
            let text = node.text();
            if Self::is_var_name(text) {
                uses.push((text.to_string(), node.start_byte()));
            }
        }

        // Extract variable captures from format/interpolated string literals.
        // Rust: format!("{var}"), println!("{var}"), write!("{var:?}")
        // C#:   $"text {var} text"
        // Kotlin: "text ${var} text" or "text $var text"
        if matches!(
            kind,
            "string_literal" | "string_content" | "raw_string_literal"
        ) {
            let text = node.text();
            let start = node.start_byte();
            Self::extract_format_captures(text, start, language, uses);
        }

        for child in node.children() {
            if Self::is_nested_function(child.node_type()) {
                continue;
            }
            Self::collect_uses(child.as_ref(), uses, language);
        }
    }

    /// Extract variable names captured inside format/interpolated string literals.
    ///
    /// Handles patterns like `{var_name}`, `{var_name:format}`, `{var_name:#?}`,
    /// `$var`, `${var}` depending on the language.
    fn extract_format_captures(
        text: &str,
        base_byte: usize,
        language: crate::core::Language,
        uses: &mut Vec<(String, usize)>,
    ) {
        use crate::core::Language;

        match language {
            // Rust: {ident} or {ident:format_spec} inside format strings.
            // Skip escaped braces {{ and positional args {0}.
            Language::Rust => {
                let bytes = text.as_bytes();
                let len = bytes.len();
                let mut i = 0;
                while i < len {
                    if bytes[i] == b'{' {
                        // Skip escaped braces {{
                        if i + 1 < len && bytes[i + 1] == b'{' {
                            i += 2;
                            continue;
                        }
                        // Find the closing brace
                        if let Some(close) = text[i + 1..].find('}') {
                            let inner = &text[i + 1..i + 1 + close];
                            // Strip format spec: {var:spec} → var
                            let ident = inner.split(':').next().unwrap_or("");
                            // Must be a valid identifier (not empty, not a number)
                            if !ident.is_empty()
                                && !ident.starts_with(|c: char| c.is_ascii_digit())
                                && ident.chars().all(|c| c.is_alphanumeric() || c == '_')
                                && Self::is_var_name(ident)
                            {
                                uses.push((ident.to_string(), base_byte + i + 1));
                            }
                            i += close + 2;
                        } else {
                            i += 1;
                        }
                    } else if bytes[i] == b'}' && i + 1 < len && bytes[i + 1] == b'}' {
                        // Skip escaped }}
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            }
            // C#: {ident} inside $"..." interpolated strings
            Language::CSharp => {
                let bytes = text.as_bytes();
                let len = bytes.len();
                let mut i = 0;
                while i < len {
                    if bytes[i] == b'{' {
                        if i + 1 < len && bytes[i + 1] == b'{' {
                            i += 2;
                            continue;
                        }
                        if let Some(close) = text[i + 1..].find('}') {
                            let inner = &text[i + 1..i + 1 + close];
                            let ident = inner.split(':').next().unwrap_or("");
                            let ident = ident.split(',').next().unwrap_or(""); // alignment spec
                            if !ident.is_empty()
                                && !ident.starts_with(|c: char| c.is_ascii_digit())
                                && ident.chars().all(|c| c.is_alphanumeric() || c == '_')
                                && Self::is_var_name(ident)
                            {
                                uses.push((ident.to_string(), base_byte + i + 1));
                            }
                            i += close + 2;
                        } else {
                            i += 1;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            // Kotlin: $ident or ${expr} inside string literals
            Language::Kotlin => {
                let bytes = text.as_bytes();
                let len = bytes.len();
                let mut i = 0;
                while i < len {
                    if bytes[i] == b'$' && i + 1 < len {
                        if bytes[i + 1] == b'{' {
                            // ${expr} — extract simple identifier
                            if let Some(close) = text[i + 2..].find('}') {
                                let inner = &text[i + 2..i + 2 + close];
                                if !inner.is_empty()
                                    && inner.chars().all(|c| c.is_alphanumeric() || c == '_')
                                    && Self::is_var_name(inner)
                                {
                                    uses.push((inner.to_string(), base_byte + i + 2));
                                }
                                i += close + 3;
                            } else {
                                i += 1;
                            }
                        } else {
                            // $ident — read identifier chars
                            let start = i + 1;
                            let mut end = start;
                            while end < len
                                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
                            {
                                end += 1;
                            }
                            if end > start {
                                let ident = &text[start..end];
                                if Self::is_var_name(ident) {
                                    uses.push((ident.to_string(), base_byte + start));
                                }
                            }
                            i = end;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            _ => {}
        }
    }

    /// Check if a string looks like a valid variable name (not a keyword or type).
    fn is_var_name(name: &str) -> bool {
        if name.is_empty() {
            return false;
        }
        // Skip the blank identifier `_` used in Go/Python to intentionally discard values
        if name == "_" {
            return false;
        }
        let first = name.chars().next().unwrap();
        if !first.is_alphabetic() && first != '_' {
            return false;
        }
        // Language keywords and built-in type names
        !matches!(
            name,
            "let"
                | "var"
                | "const"
                | "mut"
                | "return"
                | "if"
                | "else"
                | "elif"
                | "for"
                | "while"
                | "do"
                | "break"
                | "continue"
                | "switch"
                | "case"
                | "fn"
                | "func"
                | "def"
                | "class"
                | "struct"
                | "enum"
                | "trait"
                | "interface"
                | "impl"
                | "import"
                | "from"
                | "export"
                | "module"
                | "package"
                | "use"
                | "pub"
                | "private"
                | "protected"
                | "public"
                | "static"
                | "final"
                | "abstract"
                | "override"
                | "virtual"
                | "true"
                | "false"
                | "none"
                | "None"
                | "null"
                | "nil"
                | "undefined"
                | "self"
                | "this"
                | "super"
                | "new"
                | "delete"
                | "typeof"
                | "instanceof"
                | "in"
                | "is"
                | "as"
                | "and"
                | "or"
                | "not"
                | "try"
                | "catch"
                | "finally"
                | "throw"
                | "raise"
                | "except"
                | "with"
                | "async"
                | "await"
                | "yield"
                | "pass"
                | "lambda"
                | "type"
                | "where"
                | "match"
                | "go"
                | "defer"
                | "select"
                | "chan"
                | "range"
                | "map"
                | "make"
                | "append"
                | "void"
                | "int"
                | "float"
                | "double"
                | "bool"
                | "char"
                | "byte"
                | "short"
                | "long"
                | "boolean"
                | "string"
        )
    }

    /// Check if a node type represents a function definition.
    /// Used to identify closure boundaries for variable capture analysis.
    fn is_nested_function(kind: &str) -> bool {
        matches!(
            kind,
            "function_definition"
                | "function_declaration"
                | "function_expression"
                | "function"
                | "method_definition"
                | "method_declaration"
                | "function_item"
                | "arrow_function"
                | "lambda"
                | "lambda_expression"
                | "closure_expression"
        )
    }

    /// Check if an `if_statement` / `if_expression` has an always-true condition.
    /// Recognizes `if True:`, `if (true)`, `if (1)`, etc.
    fn is_always_true_condition(node: &dyn crate::core::TreeNode) -> bool {
        let children = node.children();
        // The condition is typically the second child (after "if" keyword).
        // Look through children for literal true values.
        for child in &children {
            let ct = child.node_type();
            // Skip the keyword itself and block/body nodes
            if matches!(
                ct,
                "if" | "block"
                    | "then"
                    | "else_clause"
                    | "else"
                    | "comment"
                    | "{"
                    | "}"
                    | "("
                    | ")"
                    | ":"
            ) {
                continue;
            }
            // Check for always-true literals
            if ct == "true" || ct == "True" {
                return true;
            }
            // Parenthesized expression: check inside
            if ct == "parenthesized_expression" {
                for gc in child.children() {
                    let gct = gc.node_type();
                    if gct == "true" || gct == "True" {
                        return true;
                    }
                    if gct == "integer" && gc.text() != "0" {
                        return true;
                    }
                }
            }
            // Direct integer literal (e.g., Python `if 1:`)
            if ct == "integer" && child.text() != "0" {
                return true;
            }
        }
        false
    }

    /// Collect variable names that are DECLARED (not just assigned) inside a
    /// function body. This includes `let`/`const`/`var` declarations and function
    /// parameters, but NOT bare assignments like `x = value`.
    ///
    /// Used to distinguish local variables from outer-scope (module-level)
    /// variables. Assignments to outer-scope variables should not be flagged
    /// as dead stores because they may be read by other functions.
    fn collect_local_declarations(node: &dyn crate::core::TreeNode, decls: &mut HashSet<String>) {
        let kind = node.node_type();

        // variable_declarator (JS/TS/Java/C#), init_declarator (C/C++)
        if kind == "variable_declarator" || kind == "init_declarator" {
            if let Some(first) = node.children().first() {
                let ft = first.node_type();
                if ft == "identifier" || ft == "name" {
                    let text = first.text();
                    if Self::is_var_name(text) {
                        decls.insert(text.to_string());
                    }
                }
            }
            return;
        }

        // let_declaration (Rust)
        if kind == "let_declaration" {
            for child in node.children() {
                if child.node_type() == "identifier" {
                    let text = child.text();
                    if text != "let" && text != "mut" && Self::is_var_name(text) {
                        decls.insert(text.to_string());
                        return;
                    }
                }
            }
            return;
        }

        // assignment (Python/Ruby): in these languages, assignment inside a
        // function creates a local variable. Include in local declarations.
        // Note: `assignment_expression` (JS/TS) is intentionally NOT included
        // because it assigns to an existing (possibly outer-scope) variable.
        if kind == "assignment" {
            if let Some(first) = node.children().first() {
                let ft = first.node_type();
                if ft == "identifier" || ft == "name" {
                    let text = first.text();
                    if Self::is_var_name(text) {
                        decls.insert(text.to_string());
                    }
                }
            }
            return;
        }

        // short_var_declaration (Go)
        if kind == "short_var_declaration" {
            if let Some(first) = node.children().first() {
                if first.node_type() == "expression_list" {
                    for gc in first.children() {
                        if gc.node_type() == "identifier" {
                            let text = gc.text();
                            if Self::is_var_name(text) {
                                decls.insert(text.to_string());
                            }
                        }
                    }
                } else if first.node_type() == "identifier" {
                    let text = first.text();
                    if Self::is_var_name(text) {
                        decls.insert(text.to_string());
                    }
                }
            }
            return;
        }

        // Function parameters — these are local to the function
        if matches!(
            kind,
            "formal_parameters"
                | "parameters"
                | "parameter_list"
                | "formal_parameter"
                | "required_parameter"
                | "optional_parameter"
                | "parameter"
                | "simple_parameter"
                | "typed_parameter"
                | "typed_default_parameter"
                | "default_parameter"
        ) {
            for child in node.children() {
                let ct = child.node_type();
                if ct == "identifier" || ct == "name" {
                    let text = child.text();
                    if Self::is_var_name(text) {
                        decls.insert(text.to_string());
                    }
                }
                // Recurse into parameter sub-nodes
                Self::collect_local_declarations(child.as_ref(), decls);
            }
            return;
        }

        // Recurse into children — skip nested functions (separate scope)
        for child in node.children() {
            if Self::is_nested_function(child.node_type()) {
                continue;
            }
            Self::collect_local_declarations(child.as_ref(), decls);
        }
    }

    /// Collect variable names referenced inside nested function bodies.
    /// These are "captured" variables whose stores should not be flagged as dead
    /// because they may be read across multiple invocations of the closure.
    fn collect_captured_vars(node: &dyn crate::core::TreeNode, captured: &mut HashSet<String>) {
        for child in node.children() {
            if Self::is_nested_function(child.node_type()) {
                // Found a nested function — collect all identifiers from it
                Self::collect_all_identifiers(child.as_ref(), captured);
            } else {
                // Not a function — recurse looking for nested functions
                Self::collect_captured_vars(child.as_ref(), captured);
            }
        }
    }

    /// Recursively collect all identifier names from a subtree.
    fn collect_all_identifiers(node: &dyn crate::core::TreeNode, names: &mut HashSet<String>) {
        let kind = node.node_type();
        if kind == "identifier" || kind == "name" {
            let text = node.text();
            if Self::is_var_name(text) {
                names.insert(text.to_string());
            }
        }
        for child in node.children() {
            Self::collect_all_identifiers(child.as_ref(), names);
        }
    }

    /// Collect variable names that participate in loop iteration patterns.
    ///
    /// Two patterns are recognized:
    ///
    /// 1. **Loop conditions**: In `while cond { ... }`, variables in `cond`
    ///    are re-read every iteration via the loop back-edge, so assignments
    ///    inside the body are NOT dead stores.
    ///
    /// 2. **Loop body state**: Variables that are both defined AND used inside
    ///    a loop body — e.g. accumulators (`best = min(best, x)`), toggling
    ///    state (`in_string = !in_string`), iterators (`sibling = sib.next()`).
    ///    The loop back-edge means the use at the start of the next iteration
    ///    reads the def from the previous iteration, even though in byte-position
    ///    ordering the use comes before the def.
    fn collect_loop_control_vars(node: &dyn crate::core::TreeNode, vars: &mut HashSet<String>) {
        let kind = node.node_type();

        let is_loop = matches!(
            kind,
            "while_expression"
                | "while_statement"
                | "while_let_expression"
                | "for_expression"
                | "for_statement"
                | "for_in_statement"
                | "for_each_statement"
                | "loop_expression"
        );

        if is_loop {
            // Pattern 1: Extract identifiers from loop condition/iterator
            // (everything except the body block)
            let children: Vec<_> = node.children();
            for child in &children {
                let ck = child.node_type();
                if matches!(
                    ck,
                    "block"
                        | "statement_block"
                        | "{"
                        | "}"
                        | "while"
                        | "for"
                        | "let"
                        | "in"
                        | "("
                        | ")"
                ) {
                    continue;
                }
                Self::collect_all_identifiers(child.as_ref(), vars);
            }

            // Pattern 2: Variables both defined and used inside the loop body.
            // Find the body block and collect defs/uses from it.
            for child in node.children() {
                let ck = child.node_type();
                if matches!(ck, "block" | "statement_block") {
                    let mut body_defs: Vec<(String, usize, usize, bool)> = Vec::new();
                    let mut body_uses: Vec<(String, usize)> = Vec::new();
                    Self::collect_defs(child.as_ref(), &mut body_defs, false);
                    // Collect uses without format string extraction (use Unknown language
                    // to skip format parsing — we just need identifier nodes here)
                    Self::collect_uses_identifiers_only(child.as_ref(), &mut body_uses);

                    let def_names: HashSet<&str> =
                        body_defs.iter().map(|(n, _, _, _)| n.as_str()).collect();
                    for (use_name, _) in &body_uses {
                        if def_names.contains(use_name.as_str()) {
                            vars.insert(use_name.clone());
                        }
                    }
                }
            }
        }

        // Recurse into children (but NOT into nested functions)
        for child in node.children() {
            if Self::is_nested_function(child.node_type()) {
                continue;
            }
            Self::collect_loop_control_vars(child.as_ref(), vars);
        }
    }

    /// Collect only identifier nodes (no format string extraction).
    /// Used by `collect_loop_control_vars` to find variable uses in loop bodies.
    fn collect_uses_identifiers_only(
        node: &dyn crate::core::TreeNode,
        uses: &mut Vec<(String, usize)>,
    ) {
        let kind = node.node_type();
        if matches!(
            kind,
            "type_identifier"
                | "primitive_type"
                | "type_annotation"
                | "generic_type"
                | "pointer_type"
                | "reference_type"
                | "array_type"
                | "scoped_type_identifier"
                | "predefined_type"
        ) {
            return;
        }
        if kind == "identifier" || kind == "name" || kind == "shorthand_property_identifier" {
            let text = node.text();
            if Self::is_var_name(text) {
                uses.push((text.to_string(), node.start_byte()));
            }
        }
        for child in node.children() {
            if Self::is_nested_function(child.node_type()) {
                continue;
            }
            Self::collect_uses_identifiers_only(child.as_ref(), uses);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CallEdge, CodeNode, Language, NodeKind, SourceLocation, Visibility};

    fn make_node(name: &str, kind: NodeKind, vis: Visibility) -> CodeNode {
        CodeNode::new(
            name.to_string(),
            kind,
            SourceLocation::new("test.py".to_string(), 1, 10, 0, 0),
            Language::Python,
            vis,
        )
        .with_lines_of_code(10)
    }

    #[test]
    fn test_detector_finds_dead_code() {
        let mut graph = CodeGraph::new();
        let main = make_node("main", NodeKind::Function, Visibility::Public);
        let main_id = main.id;
        let helper = make_node("helper", NodeKind::Function, Visibility::Private);
        let helper_id = helper.id;
        let dead = make_node("dead_fn", NodeKind::Function, Visibility::Private);

        let main_idx = graph.add_node(main);
        graph.add_node(helper);
        graph.add_node(dead);

        graph
            .add_edge(CallEdge::certain(main_id, helper_id))
            .unwrap();
        graph.add_entry_point(main_idx);

        let detector = Detector::with_defaults();
        let result = detector.detect_in_graph(&graph).unwrap();

        assert_eq!(result.total_nodes, 3);
        assert!(result.findings.iter().any(|f| f.name == "dead_fn"));
    }

    #[test]
    fn test_detector_with_confidence_filter() {
        let mut graph = CodeGraph::new();
        let main = make_node("main", NodeKind::Function, Visibility::Public);
        let dead_pub = make_node("dead_public", NodeKind::Function, Visibility::Public);
        let dead_priv = make_node("dead_private", NodeKind::Function, Visibility::Private);

        let main_idx = graph.add_node(main);
        graph.add_node(dead_pub);
        graph.add_node(dead_priv);
        graph.add_entry_point(main_idx);

        let config = DetectorConfig {
            min_confidence: crate::core::Confidence::Certain,
            ..Default::default()
        };
        let detector = Detector::new(config);
        let result = detector.detect_in_graph(&graph).unwrap();

        // Only private dead code should be Certain
        assert!(result
            .findings
            .iter()
            .all(|f| f.confidence >= crate::core::Confidence::Certain));
    }

    // ---- Dead store detection tests ----

    #[test]
    fn test_detect_dead_stores_config_defaults_to_true() {
        let config = DetectorConfig::default();
        assert!(
            config.detect_dead_stores,
            "detect_dead_stores should default to true"
        );
    }

    #[test]
    fn test_is_var_name_rejects_keywords() {
        assert!(!Detector::is_var_name("let"));
        assert!(!Detector::is_var_name("return"));
        assert!(!Detector::is_var_name("const"));
        assert!(!Detector::is_var_name("int"));
        assert!(!Detector::is_var_name("void"));
        assert!(!Detector::is_var_name(""));
        assert!(!Detector::is_var_name("123"));
    }

    #[test]
    fn test_is_var_name_accepts_valid_names() {
        assert!(Detector::is_var_name("x"));
        assert!(Detector::is_var_name("sql"));
        assert!(Detector::is_var_name("conn"));
        assert!(Detector::is_var_name("_private"));
        assert!(Detector::is_var_name("myVar"));
        assert!(Detector::is_var_name("data_list"));
    }

    #[test]
    fn test_dead_stores_python_source() {
        // x is assigned but never used -> dead store
        // y is assigned and used in print -> not dead
        let source = "def foo():\n    x = 5\n    y = 10\n    print(y)\n";
        let parsed = crate::core::ParsedFile::new(
            "test.py".to_string(),
            Language::Python,
            source.to_string(),
        );
        let findings = Detector::detect_dead_stores(&[parsed]);
        let dead_names: Vec<&str> = findings.iter().map(|f| f.name.as_str()).collect();
        assert!(
            dead_names.contains(&"x"),
            "Expected 'x' to be dead, got: {:?}",
            dead_names
        );
        assert!(
            !dead_names.contains(&"y"),
            "Expected 'y' to NOT be dead, got: {:?}",
            dead_names
        );
    }

    #[test]
    fn test_dead_store_finding_has_correct_type() {
        // Verify that dead store findings are tagged with UnusedVariable type
        let source =
            "def example_fn():\n    unused_var = 1\n    used_var = 2\n    print(used_var)\n";
        let parsed = crate::core::ParsedFile::new(
            "test.py".to_string(),
            Language::Python,
            source.to_string(),
        );
        let findings = Detector::detect_dead_stores(&[parsed]);

        // unused_var should be detected
        let unused = findings.iter().find(|f| f.name == "unused_var");
        assert!(unused.is_some(), "Expected unused_var to be detected");
        let f = unused.unwrap();
        assert_eq!(f.fossil_type, FossilType::UnusedVariable);
        assert_eq!(f.confidence, Confidence::Certain);
        assert_eq!(f.kind, NodeKind::Variable);
    }

    #[test]
    fn test_dead_stores_default_then_conditional_overwrite() {
        // Default value pattern: first def is NOT dead because it's the
        // fallback when no branch matches
        let source = "function foo(err) {\n  let msg = 'default';\n  if (err) {\n    msg = 'error';\n  }\n  console.log(msg);\n}\n";
        let parsed = crate::core::ParsedFile::new(
            "test.ts".to_string(),
            Language::TypeScript,
            source.to_string(),
        );
        let findings = Detector::detect_dead_stores(&[parsed]);
        let dead_names: Vec<&str> = findings.iter().map(|f| f.name.as_str()).collect();
        assert!(
            !dead_names.contains(&"msg"),
            "msg should NOT be flagged — first def is a default value, got: {:?}",
            dead_names
        );
    }

    #[test]
    fn test_dead_stores_destructuring_rhs_is_use() {
        // `const { x, y } = options` — `options` on the RHS is a read.
        // It should NOT be flagged as "assigned but never read".
        let source = "function foo() {\n  const options = getOptions();\n  const { x, y } = options;\n  return x + y;\n}\n";
        let parsed = crate::core::ParsedFile::new(
            "test.ts".to_string(),
            Language::TypeScript,
            source.to_string(),
        );
        let findings = Detector::detect_dead_stores(&[parsed]);
        assert!(
            !findings.iter().any(|f| f.name == "options"),
            "options should NOT be flagged — it's read by destructuring, got: {:?}",
            findings.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_dead_stores_sibling_branches_not_flagged() {
        // Sibling if-else branches are mutually exclusive — no branch should
        // be flagged as overwriting another.
        let source = "function foo(x) {\n  let msg = 'default';\n  if (x === 1) {\n    msg = 'one';\n  } else if (x === 2) {\n    msg = 'two';\n  } else {\n    msg = 'other';\n  }\n  console.log(msg);\n}\n";
        let parsed = crate::core::ParsedFile::new(
            "test.ts".to_string(),
            Language::TypeScript,
            source.to_string(),
        );
        let findings = Detector::detect_dead_stores(&[parsed]);
        assert!(
            findings.is_empty(),
            "Expected no findings for mutually exclusive branches, got: {:?}",
            findings.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_dead_stores_rust_underscore_prefix_skipped() {
        // Rust convention: _-prefixed variables are intentionally unused
        let source =
            "fn foo() {\n    let _email = validate();\n    let used = compute();\n    println!(\"{}\", used);\n}\n";
        let parsed =
            crate::core::ParsedFile::new("test.rs".to_string(), Language::Rust, source.to_string());
        let findings = Detector::detect_dead_stores(&[parsed]);
        let dead_names: Vec<&str> = findings.iter().map(|f| f.name.as_str()).collect();
        assert!(
            !dead_names.contains(&"_email"),
            "_email should NOT be flagged (Rust convention), got: {:?}",
            dead_names
        );
    }

    #[test]
    fn test_dead_stores_module_scope_variable_not_flagged() {
        // Module-scope variable assigned inside a function should NOT be flagged.
        // The assignment is observable by other functions or subsequent calls.
        // Pattern: `let isShuttingDown = false; function shutdown() { isShuttingDown = true; }`
        // Here `isShuttingDown` is declared at module scope, not inside `shutdown`.
        let source = "let isShuttingDown = false;\nfunction handleShutdown() {\n  if (isShuttingDown) { return; }\n  isShuttingDown = true;\n}\n";
        let parsed = crate::core::ParsedFile::new(
            "test.ts".to_string(),
            Language::TypeScript,
            source.to_string(),
        );
        let findings = Detector::detect_dead_stores(&[parsed]);
        assert!(
            !findings.iter().any(|f| f.name == "isShuttingDown"),
            "isShuttingDown should NOT be flagged — it's a module-scope variable, got: {:?}",
            findings.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_dead_stores_ts_underscore_prefix_skipped() {
        // TypeScript/JavaScript convention: _-prefixed variables are intentionally unused
        let source = "function foo() {\n  const _unused = getResult();\n  const used = compute();\n  console.log(used);\n}\n";
        let parsed = crate::core::ParsedFile::new(
            "test.ts".to_string(),
            Language::TypeScript,
            source.to_string(),
        );
        let findings = Detector::detect_dead_stores(&[parsed]);
        let dead_names: Vec<&str> = findings.iter().map(|f| f.name.as_str()).collect();
        assert!(
            !dead_names.contains(&"_unused"),
            "_unused should NOT be flagged (TS/JS convention), got: {:?}",
            dead_names
        );
    }

    #[test]
    fn test_dead_stores_shorthand_property_is_use() {
        // ES6 shorthand property { x } should count as a read of x
        let source = "function foo() {\n  const x = 5;\n  const y = 10;\n  return { x, y };\n}\n";
        let parsed = crate::core::ParsedFile::new(
            "test.ts".to_string(),
            Language::TypeScript,
            source.to_string(),
        );
        let findings = Detector::detect_dead_stores(&[parsed]);
        let dead_names: Vec<&str> = findings.iter().map(|f| f.name.as_str()).collect();
        assert!(
            !dead_names.contains(&"x"),
            "x should NOT be flagged — used in shorthand property, got: {:?}",
            dead_names
        );
        assert!(
            !dead_names.contains(&"y"),
            "y should NOT be flagged — used in shorthand property, got: {:?}",
            dead_names
        );
    }

    #[test]
    fn test_diagnostic_auth_rb_graph() {
        use crate::graph::GraphBuilder;

        let auth_rb_path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/auth.rb");
        let source = std::fs::read_to_string(auth_rb_path).unwrap();

        let builder = GraphBuilder::new().unwrap();
        let graph = builder
            .build_file_graph(&source, "auth.rb", Language::Ruby)
            .unwrap();

        // Verify the fix: no spurious module -> function edges
        let unreachable_names: Vec<String> = {
            let entry_detector = crate::dead_code::entry_points::EntryPointDetector::new(&graph);
            let production_entries = entry_detector.detect_production_entry_points();
            let reachable = graph.compute_reachable(&production_entries);
            graph
                .nodes()
                .filter(|(idx, _)| !reachable.contains(idx))
                .map(|(_, node)| node.name.clone())
                .collect()
        };
        // All 19 functions + DeprecatedSession class should be unreachable
        // (only <module:auth.rb> is the entry point, no edges to definitions)
        assert!(
            unreachable_names.len() >= 15,
            "Expected at least 15 unreachable functions in auth.rb, got {}: {:?}",
            unreachable_names.len(),
            unreachable_names
        );
    }

    #[test]
    fn test_java_dead_private_methods_detected() {
        use crate::graph::GraphBuilder;

        let java_path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/UserService.java");
        let source = std::fs::read_to_string(java_path).unwrap();

        let builder = GraphBuilder::new().unwrap();
        let graph = builder
            .build_file_graph(&source, "UserService.java", Language::Java)
            .unwrap();

        let entry_detector = crate::dead_code::entry_points::EntryPointDetector::new(&graph);
        let production_entries = entry_detector.detect_production_entry_points();
        let reachable = graph.compute_reachable(&production_entries);

        let unreachable_names: Vec<String> = graph
            .nodes()
            .filter(|(idx, _)| !reachable.contains(idx))
            .map(|(_, node)| node.name.clone())
            .collect();

        // formatUserName and formatDisplayName are private dead methods
        assert!(
            unreachable_names.iter().any(|n| n == "formatUserName"),
            "Expected formatUserName to be unreachable, unreachable: {:?}",
            unreachable_names
        );
        assert!(
            unreachable_names.iter().any(|n| n == "formatDisplayName"),
            "Expected formatDisplayName to be unreachable, unreachable: {:?}",
            unreachable_names
        );
        // DeprecatedAuthProvider inner class also unreachable
        assert!(
            unreachable_names
                .iter()
                .any(|n| n == "DeprecatedAuthProvider"),
            "Expected DeprecatedAuthProvider to be unreachable, unreachable: {:?}",
            unreachable_names
        );
    }
}
