//! Analysis pipeline: orchestrates parsing, graph building, and analysis.
//!
//! Uses Rayon for parallel file processing with directory-grouped chunking
//! for improved cache locality.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use crate::core::{Language, ParsedFile, Timer};
use crate::graph::{CodeGraph, GraphBuilder};
use crate::parsers::ParserRegistry;
use rayon::prelude::*;
use tracing::{info, warn};

use super::scanner::{FileScanner, SourceFile};

/// Configuration for the analysis pipeline.
#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub max_file_size: u64,
    pub exclude_patterns: Vec<String>,
    pub include_tests: bool,
    pub parallel: bool,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            max_file_size: 1024 * 1024,
            exclude_patterns: Vec::new(),
            include_tests: true,
            parallel: true,
        }
    }
}

/// Result of running the pipeline.
#[derive(Debug)]
pub struct PipelineResult {
    pub graph: CodeGraph,
    pub parsed_files: Vec<ParsedFile>,
    pub errors: Vec<(String, String)>,
    pub files_scanned: usize,
    pub files_parsed: usize,
    pub total_lines: usize,
    pub duration_ms: u64,
}

/// Analysis pipeline: scan → parse → build graph.
pub struct Pipeline {
    config: PipelineConfig,
}

impl Pipeline {
    pub fn new(config: PipelineConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self {
            config: PipelineConfig::default(),
        }
    }

    /// Run the full pipeline on a directory.
    ///
    /// Files are grouped by parent directory for cache locality and distributed
    /// across Rayon threads in chunks of `num_files / (num_cpus * 2)`.
    pub fn run(&self, root: &Path) -> Result<PipelineResult, crate::core::Error> {
        let start = Instant::now();
        let timer = Timer::start("Pipeline");

        // Scan for source files
        let scan_timer = Timer::start_nested("File Scanning", "Pipeline");
        let scanner = FileScanner::new().with_max_file_size(self.config.max_file_size);
        let files = scanner.scan(root)?;
        let files_scanned = files.len();
        info!("Scanned {} source files", files_scanned);
        scan_timer.stop_with_info(format!("{} files", files_scanned));

        // Group files by directory for cache locality
        let files = group_by_directory(files);

        // Parse files (parallel, chunk-based)
        let parse_timer = Timer::start_nested("File Parsing", "Pipeline");
        let registry = ParserRegistry::with_defaults()?;
        let parse_results: Vec<Result<ParsedFile, (String, String)>> = if self.config.parallel {
            let chunk_size = compute_chunk_size(files_scanned);
            files
                .par_chunks(chunk_size)
                .flat_map(|chunk| {
                    chunk
                        .iter()
                        .map(|f| parse_file(f, &registry))
                        .collect::<Vec<_>>()
                })
                .collect()
        } else {
            files.iter().map(|f| parse_file(f, &registry)).collect()
        };

        let mut parsed_files = Vec::new();
        let mut errors = Vec::new();
        let mut total_lines = 0usize;

        for result in parse_results {
            match result {
                Ok(pf) => {
                    total_lines += pf.source.lines().count();
                    parsed_files.push(pf);
                }
                Err((path, err)) => {
                    warn!("Failed to parse {}: {}", path, err);
                    errors.push((path, err));
                }
            }
        }
        let files_parsed = parsed_files.len();
        info!("Parsed {} files ({} errors)", files_parsed, errors.len());
        parse_timer.stop_with_info(format!("{} files parsed, {} errors", files_parsed, errors.len()));

        // Build project graph
        let graph_timer = Timer::start_nested("Graph Building", "Pipeline");
        let builder = GraphBuilder::new()?;
        let graph = builder.build_project_graph(&parsed_files)?;
        info!(
            "Built graph: {} nodes, {} edges",
            graph.node_count(),
            graph.edge_count()
        );
        graph_timer.stop_with_info(format!("{} nodes, {} edges", graph.node_count(), graph.edge_count()));

        let duration_ms = start.elapsed().as_millis() as u64;
        timer.stop();

        Ok(PipelineResult {
            graph,
            parsed_files,
            errors,
            files_scanned,
            files_parsed,
            total_lines,
            duration_ms,
        })
    }

    /// Parse a single file and build its graph (for incremental use).
    pub fn parse_single_file(
        &self,
        file_path: &Path,
    ) -> Result<(ParsedFile, CodeGraph), crate::core::Error> {
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .ok_or_else(|| crate::core::Error::parse("No file extension"))?;
        let language = Language::from_extension(ext)
            .ok_or_else(|| crate::core::Error::parse(format!("Unsupported extension: {ext}")))?;

        let source = std::fs::read_to_string(file_path)
            .map_err(|e| crate::core::Error::analysis(format!("IO error: {e}")))?;

        let registry = ParserRegistry::with_defaults()?;
        let parser = registry
            .get_parser_for_extension(ext)
            .or_else(|| registry.get_parser(language))
            .ok_or_else(|| {
                crate::core::Error::parse(format!("No parser for {}", language.name()))
            })?;

        let parsed = parser.parse_file(file_path.to_str().unwrap_or("unknown"), &source)?;

        let builder = GraphBuilder::new()?;
        let graph = builder.build_from_parsed_file(&parsed)?;

        Ok((parsed, graph))
    }
}

/// Compute the chunk size for parallel processing.
///
/// Formula: `num_files / (num_cpus * 2)`, with a minimum of 1.
fn compute_chunk_size(num_files: usize) -> usize {
    let num_cpus = rayon::current_num_threads().max(1);
    let ideal = num_files / (num_cpus * 2);
    ideal.max(1)
}

/// Group source files by their parent directory for cache locality.
///
/// Files in the same directory are likely to share imports and similar
/// structure, so grouping them together improves OS page-cache and
/// disk-read performance during parallel parsing.
fn group_by_directory(files: Vec<SourceFile>) -> Vec<SourceFile> {
    // Bucket files by parent directory.
    let mut buckets: HashMap<Option<std::path::PathBuf>, Vec<SourceFile>> = HashMap::new();
    for file in files {
        let parent = file.path.parent().map(|p| p.to_path_buf());
        buckets.entry(parent).or_default().push(file);
    }

    // Flatten buckets back into a single vec (files from the same directory
    // are now contiguous).
    let mut grouped = Vec::new();
    for (_, mut bucket) in buckets {
        grouped.append(&mut bucket);
    }
    grouped
}

fn parse_file(
    file: &SourceFile,
    registry: &ParserRegistry,
) -> Result<ParsedFile, (String, String)> {
    let path_str = file.path.to_string_lossy().to_string();

    let source = std::fs::read_to_string(&file.path)
        .map_err(|e| (path_str.clone(), format!("IO error: {e}")))?;

    // Prefer extension-based lookup so language variants that share the same
    // Language enum (e.g. TypeScript vs TSX) pick the right grammar.
    let ext = file.path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let parser = registry
        .get_parser_for_extension(ext)
        .or_else(|| registry.get_parser(file.language))
        .ok_or_else(|| {
            (
                path_str.clone(),
                format!("No parser for {}", file.language.name()),
            )
        })?;

    parser
        .parse_file(&path_str, &source)
        .map_err(|e| (path_str, e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_pipeline_basic() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("main.py"),
            "def main():\n    helper()\n\ndef helper():\n    pass\n",
        )
        .unwrap();

        let pipeline = Pipeline::with_defaults();
        let result = pipeline.run(dir.path()).unwrap();

        assert_eq!(result.files_scanned, 1);
        assert_eq!(result.files_parsed, 1);
        assert!(result.graph.node_count() >= 2);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn test_pipeline_multi_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.py"), "def a():\n    pass\n").unwrap();
        fs::write(dir.path().join("b.py"), "def b():\n    pass\n").unwrap();

        let pipeline = Pipeline::with_defaults();
        let result = pipeline.run(dir.path()).unwrap();

        assert_eq!(result.files_parsed, 2);
        assert!(result.graph.node_count() >= 2);
    }

    #[test]
    fn test_compute_chunk_size_minimum_one() {
        // Even for a single file, chunk size should be at least 1
        assert!(compute_chunk_size(1) >= 1);
        assert!(compute_chunk_size(0) >= 1);
    }

    #[test]
    fn test_compute_chunk_size_scales() {
        // For a large number of files, chunk size should be > 1
        let size = compute_chunk_size(10_000);
        assert!(size >= 1);
        // It should be roughly num_files / (num_cpus * 2)
        let num_cpus = rayon::current_num_threads().max(1);
        let expected = 10_000 / (num_cpus * 2);
        assert_eq!(size, expected.max(1));
    }

    #[test]
    fn test_group_by_directory_preserves_files() {
        use crate::analysis::scanner::SourceFile;
        use crate::core::Language;
        use std::path::PathBuf;

        let files = vec![
            SourceFile {
                path: PathBuf::from("/project/src/a.py"),
                language: Language::Python,
                size_bytes: 100,
            },
            SourceFile {
                path: PathBuf::from("/project/lib/b.py"),
                language: Language::Python,
                size_bytes: 200,
            },
            SourceFile {
                path: PathBuf::from("/project/src/c.py"),
                language: Language::Python,
                size_bytes: 150,
            },
        ];

        let grouped = group_by_directory(files);

        // All three files should still be present
        assert_eq!(grouped.len(), 3);

        // Files from /project/src/ should be contiguous
        let src_indices: Vec<usize> = grouped
            .iter()
            .enumerate()
            .filter(|(_, f)| f.path.parent().unwrap().ends_with("src"))
            .map(|(i, _)| i)
            .collect();

        if src_indices.len() == 2 {
            // They should be adjacent
            assert_eq!(src_indices[1] - src_indices[0], 1);
        }
    }

    #[test]
    fn test_pipeline_with_subdirectories() {
        let dir = TempDir::new().unwrap();

        // Create files in subdirectories
        let sub1 = dir.path().join("pkg1");
        let sub2 = dir.path().join("pkg2");
        fs::create_dir_all(&sub1).unwrap();
        fs::create_dir_all(&sub2).unwrap();

        fs::write(sub1.join("mod_a.py"), "def a():\n    pass\n").unwrap();
        fs::write(sub1.join("mod_b.py"), "def b():\n    pass\n").unwrap();
        fs::write(sub2.join("mod_c.py"), "def c():\n    pass\n").unwrap();

        let pipeline = Pipeline::with_defaults();
        let result = pipeline.run(dir.path()).unwrap();

        assert_eq!(result.files_scanned, 3);
        assert_eq!(result.files_parsed, 3);
        assert!(result.graph.node_count() >= 3);
        assert!(result.errors.is_empty());
    }
}
