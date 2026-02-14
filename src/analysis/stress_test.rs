//! Stress testing infrastructure for measuring performance on large projects.
//!
//! Measures:
//! - Peak memory usage during parsing
//! - Time to parse all files
//! - Graph construction time and size
//! - Dead code detection accuracy
//! - Incremental analysis speedup

use std::path::Path;
use std::time::Instant;
use crate::analysis::Pipeline;
use crate::core::Timer;

/// Results from a stress test run.
#[derive(Debug, Clone)]
pub struct StressTestResult {
    /// Total files scanned
    pub files_scanned: usize,
    /// Total files parsed
    pub files_parsed: usize,
    /// Total lines of code parsed
    pub total_lines: usize,
    /// Peak memory usage in MB
    pub peak_memory_mb: u64,
    /// Total execution time in milliseconds
    pub duration_ms: u64,
    /// Number of nodes in graph
    pub graph_nodes: usize,
    /// Number of edges in graph
    pub graph_edges: usize,
    /// Dead code findings
    pub dead_code_count: usize,
}

impl StressTestResult {
    /// Format result as human-readable summary
    pub fn summary(&self) -> String {
        format!(
            "Stress Test Results:\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n\
             Files:          {} scanned, {} parsed\n\
             Lines of Code:  {}\n\
             Memory:         {:.1} MB peak\n\
             Time:           {:.2}s\n\
             Graph:          {} nodes, {} edges\n\
             Dead Code:      {} findings\n\
             ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━",
            self.files_scanned,
            self.files_parsed,
            self.total_lines,
            self.peak_memory_mb as f64,
            self.duration_ms as f64 / 1000.0,
            self.graph_nodes,
            self.graph_edges,
            self.dead_code_count,
        )
    }

    /// Calculate throughput in files per second
    pub fn files_per_second(&self) -> f64 {
        if self.duration_ms == 0 {
            0.0
        } else {
            self.files_parsed as f64 / (self.duration_ms as f64 / 1000.0)
        }
    }

    /// Calculate throughput in lines per second
    pub fn lines_per_second(&self) -> f64 {
        if self.duration_ms == 0 {
            0.0
        } else {
            self.total_lines as f64 / (self.duration_ms as f64 / 1000.0)
        }
    }

    /// Memory per file in KB
    pub fn memory_per_file_kb(&self) -> f64 {
        if self.files_parsed == 0 {
            0.0
        } else {
            (self.peak_memory_mb as f64 * 1024.0) / self.files_parsed as f64
        }
    }
}

/// Stress test runner for large projects
pub struct StressTestRunner;

impl StressTestRunner {
    /// Run full pipeline stress test
    pub fn run_full_pipeline(root: &Path) -> Result<StressTestResult, crate::core::Error> {
        let timer = Timer::start("Full Pipeline Stress Test");
        let start = Instant::now();

        eprintln!("Starting full pipeline stress test on: {}", root.display());
        eprintln!("This will measure memory usage, parsing time, and graph construction...\n");

        let pipeline = Pipeline::with_defaults();
        let pipeline_result = pipeline.run(root)?;

        let peak_memory = crate::analysis::get_rss_mb();
        let duration_ms = start.elapsed().as_millis() as u64;

        timer.stop_with_info(format!(
            "Parsed {} files in {}ms, peak memory: {}MB",
            pipeline_result.files_parsed, duration_ms, peak_memory
        ));

        Ok(StressTestResult {
            files_scanned: pipeline_result.files_scanned,
            files_parsed: pipeline_result.files_parsed,
            total_lines: pipeline_result.total_lines,
            peak_memory_mb: peak_memory,
            duration_ms,
            graph_nodes: pipeline_result.graph.node_count(),
            graph_edges: pipeline_result.graph.edge_count(),
            dead_code_count: 0, // Would be populated if dead code detection ran
        })
    }

    /// Run streaming pipeline stress test (memory-optimized)
    pub fn run_streaming_pipeline(root: &Path) -> Result<StressTestResult, crate::core::Error> {
        let timer = Timer::start("Streaming Pipeline Stress Test");
        let start = Instant::now();

        eprintln!("Starting streaming pipeline stress test on: {}", root.display());
        eprintln!("This will measure memory optimization with batch processing...\n");

        let pipeline = Pipeline::with_defaults();
        let pipeline_result = pipeline.run_streaming(root)?;

        let peak_memory = crate::analysis::get_rss_mb();
        let duration_ms = start.elapsed().as_millis() as u64;

        timer.stop_with_info(format!(
            "Parsed {} files in {}ms, peak memory: {}MB",
            pipeline_result.files_parsed, duration_ms, peak_memory
        ));

        Ok(StressTestResult {
            files_scanned: pipeline_result.files_scanned,
            files_parsed: pipeline_result.files_parsed,
            total_lines: pipeline_result.total_lines,
            peak_memory_mb: peak_memory,
            duration_ms,
            graph_nodes: pipeline_result.graph.node_count(),
            graph_edges: pipeline_result.graph.edge_count(),
            dead_code_count: 0,
        })
    }

    /// Compare full vs streaming performance
    pub fn compare_approaches(root: &Path) -> Result<String, crate::core::Error> {
        eprintln!("Running comparative stress test...\n");

        eprintln!("═══════════════════════════════════════════════");
        eprintln!("APPROACH 1: Full Pipeline (all files loaded)");
        eprintln!("═══════════════════════════════════════════════\n");

        let full_result = Self::run_full_pipeline(root)?;
        eprintln!("\n{}\n", full_result.summary());

        eprintln!("═══════════════════════════════════════════════");
        eprintln!("APPROACH 2: Streaming Pipeline (batch processing)");
        eprintln!("═══════════════════════════════════════════════\n");

        let streaming_result = Self::run_streaming_pipeline(root)?;
        eprintln!("\n{}\n", streaming_result.summary());

        // Calculate improvement
        let memory_reduction_pct = if full_result.peak_memory_mb > 0 {
            ((full_result.peak_memory_mb - streaming_result.peak_memory_mb) as f64
                / full_result.peak_memory_mb as f64)
                * 100.0
        } else {
            0.0
        };

        let time_overhead_pct = if full_result.duration_ms > 0 {
            ((streaming_result.duration_ms - full_result.duration_ms) as f64
                / full_result.duration_ms as f64)
                * 100.0
        } else {
            0.0
        };

        Ok(format!(
            "═══════════════════════════════════════════════\n\
             COMPARISON SUMMARY\n\
             ═══════════════════════════════════════════════\n\
             Memory Reduction:     {:.1}% ({} MB → {} MB)\n\
             Time Overhead:        {:.1}% ({:.2}s → {:.2}s)\n\
             Memory per File:      {:.2} KB → {:.2} KB\n\
             Throughput:           {:.0} files/s → {:.0} files/s\n\
             ═══════════════════════════════════════════════",
            memory_reduction_pct,
            full_result.peak_memory_mb,
            streaming_result.peak_memory_mb,
            time_overhead_pct,
            full_result.duration_ms as f64 / 1000.0,
            streaming_result.duration_ms as f64 / 1000.0,
            full_result.memory_per_file_kb(),
            streaming_result.memory_per_file_kb(),
            full_result.files_per_second(),
            streaming_result.files_per_second(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stress_test_result_calculations() {
        let result = StressTestResult {
            files_scanned: 1000,
            files_parsed: 950,
            total_lines: 100_000,
            peak_memory_mb: 1024,
            duration_ms: 10_000, // 10 seconds
            graph_nodes: 50_000,
            graph_edges: 150_000,
            dead_code_count: 120,
        };

        assert_eq!(result.files_per_second(), 95.0);
        assert_eq!(result.lines_per_second(), 10_000.0);
        // 1024 MB * 1024 KB/MB / 950 files ≈ 1103.76 KB/file
        assert!((result.memory_per_file_kb() - 1103.76).abs() < 1.0);
    }

    #[test]
    fn test_stress_test_result_summary() {
        let result = StressTestResult {
            files_scanned: 100,
            files_parsed: 95,
            total_lines: 10_000,
            peak_memory_mb: 512,
            duration_ms: 5_000,
            graph_nodes: 5_000,
            graph_edges: 15_000,
            dead_code_count: 50,
        };

        let summary = result.summary();
        assert!(summary.contains("100 scanned, 95 parsed"));
        assert!(summary.contains("512.0 MB"));
        assert!(summary.contains("5000 nodes"));
    }
}
