//! BigCloneBench benchmark harness for clone detection evaluation.
//!
//! Provides infrastructure for loading ground-truth clone pairs, evaluating
//! detected clones against them, and computing precision/recall/F1 metrics
//! per clone type.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::types::{CloneGroup, CloneType};

/// A code fragment for benchmarking.
#[derive(Debug, Clone)]
pub struct CodeFragment {
    /// Unique identifier for this fragment.
    pub id: usize,
    /// Source file path.
    pub file: String,
    /// Start line (1-indexed).
    pub start_line: usize,
    /// End line (1-indexed, inclusive).
    pub end_line: usize,
    /// Source code text (may be empty if not loaded).
    pub source: String,
    /// Functionality class identifier (for BigCloneBench grouping).
    pub functionality_id: Option<usize>,
}

/// Ground truth pair for evaluation.
#[derive(Debug, Clone)]
pub struct GroundTruthPair {
    /// Fragment ID of the first element.
    pub fragment_a: usize,
    /// Fragment ID of the second element.
    pub fragment_b: usize,
    /// Clone type classification.
    pub clone_type: CloneType,
    /// Whether this is a true clone pair (`true`) or a known non-clone (`false`).
    pub is_clone: bool,
}

/// Result of running a benchmark evaluation.
#[derive(Debug, Clone)]
pub struct BenchmarkResult {
    /// Number of correctly detected clone pairs.
    pub true_positives: usize,
    /// Number of detected pairs that are not in ground truth.
    pub false_positives: usize,
    /// Number of ground truth clones that were not detected.
    pub false_negatives: usize,
    /// Precision: TP / (TP + FP).
    pub precision: f64,
    /// Recall: TP / (TP + FN).
    pub recall: f64,
    /// F1 score: harmonic mean of precision and recall.
    pub f1: f64,
    /// Per clone-type breakdown.
    pub per_type: HashMap<CloneType, TypeResult>,
}

/// Per clone-type evaluation result.
#[derive(Debug, Clone)]
pub struct TypeResult {
    pub true_positives: usize,
    pub false_positives: usize,
    pub false_negatives: usize,
    pub precision: f64,
    pub recall: f64,
    pub f1: f64,
}

/// Clone detection benchmark harness.
///
/// Loads code fragments and ground truth pairs, runs a clone detector, and
/// computes evaluation metrics. Supports BigCloneBench IJaDataset format.
pub struct CloneBenchmark {
    /// Code fragments under test.
    pub fragments: Vec<CodeFragment>,
    /// Ground truth clone/non-clone pairs.
    pub ground_truth: Vec<GroundTruthPair>,
}

impl CloneBenchmark {
    /// Create from raw data.
    pub fn new(fragments: Vec<CodeFragment>, ground_truth: Vec<GroundTruthPair>) -> Self {
        Self {
            fragments,
            ground_truth,
        }
    }

    /// Load from BigCloneBench IJaDataset format.
    ///
    /// Expects:
    /// - `dir/functions.csv` -- fragment definitions: `id,file,start_line,end_line[,functionality_id]`
    /// - `dir/clones.csv` -- ground truth pairs: `id1,id2,type,is_clone`
    ///
    /// Source files are not loaded by this method; use `load_sources` afterwards
    /// if fragment source text is needed.
    pub fn from_bigclonebench(dir: &Path) -> Result<Self, crate::core::Error> {
        let functions_path = dir.join("functions.csv");
        let fragments = if functions_path.exists() {
            Self::parse_functions_csv(&functions_path)?
        } else {
            Vec::new()
        };

        let clones_path = dir.join("clones.csv");
        let ground_truth = if clones_path.exists() {
            Self::parse_clones_csv(&clones_path)?
        } else {
            Vec::new()
        };

        Ok(Self {
            fragments,
            ground_truth,
        })
    }

    /// Build a lookup from fragment ID to `(file, start_line, end_line)`.
    fn fragment_location_map(&self) -> HashMap<usize, String> {
        self.fragments
            .iter()
            .map(|f| {
                let key = format!("{}:{}:{}", f.file, f.start_line, f.end_line);
                (f.id, key)
            })
            .collect()
    }

    /// Evaluate detected clones against ground truth.
    ///
    /// Matching is done by `(file, start_line, end_line)` location keys.
    /// Returns aggregated and per-type metrics.
    pub fn evaluate(&self, detected_groups: &[CloneGroup]) -> BenchmarkResult {
        // Build set of detected pairs as canonical location keys
        let mut detected_pairs: HashSet<(String, String)> = HashSet::new();
        for group in detected_groups {
            for i in 0..group.instances.len() {
                for j in (i + 1)..group.instances.len() {
                    let a = &group.instances[i];
                    let b = &group.instances[j];
                    let key_a = format!("{}:{}:{}", a.file, a.start_line, a.end_line);
                    let key_b = format!("{}:{}:{}", b.file, b.start_line, b.end_line);
                    if key_a < key_b {
                        detected_pairs.insert((key_a, key_b));
                    } else {
                        detected_pairs.insert((key_b, key_a));
                    }
                }
            }
        }

        // Build fragment ID -> location key map
        let location_map = self.fragment_location_map();

        // Evaluate against ground truth
        let mut tp = 0usize;
        let mut fn_ = 0usize;
        let mut per_type_counts: HashMap<CloneType, (usize, usize)> = HashMap::new();

        for gt in &self.ground_truth {
            if !gt.is_clone {
                continue;
            }

            let key_a = match location_map.get(&gt.fragment_a) {
                Some(k) => k.clone(),
                None => continue,
            };
            let key_b = match location_map.get(&gt.fragment_b) {
                Some(k) => k.clone(),
                None => continue,
            };

            let canonical = if key_a < key_b {
                (key_a, key_b)
            } else {
                (key_b, key_a)
            };

            let entry = per_type_counts.entry(gt.clone_type).or_insert((0, 0));
            if detected_pairs.contains(&canonical) {
                tp += 1;
                entry.0 += 1;
            } else {
                fn_ += 1;
                entry.1 += 1;
            }
        }

        let fp = detected_pairs.len().saturating_sub(tp);

        let precision = if tp + fp > 0 {
            tp as f64 / (tp + fp) as f64
        } else {
            0.0
        };
        let recall = if tp + fn_ > 0 {
            tp as f64 / (tp + fn_) as f64
        } else {
            0.0
        };
        let f1 = if precision + recall > 0.0 {
            2.0 * precision * recall / (precision + recall)
        } else {
            0.0
        };

        // Build per-type results
        let per_type: HashMap<CloneType, TypeResult> = per_type_counts
            .into_iter()
            .map(|(ct, (type_tp, type_fn))| {
                // We cannot precisely attribute false positives per type without
                // ground truth labeling of detected pairs, so FP is set to 0
                // for per-type breakdown. The aggregate FP is accurate.
                let type_fp = 0usize;
                let p = if type_tp + type_fp > 0 {
                    type_tp as f64 / (type_tp + type_fp) as f64
                } else {
                    0.0
                };
                let r = if type_tp + type_fn > 0 {
                    type_tp as f64 / (type_tp + type_fn) as f64
                } else {
                    0.0
                };
                let f = if p + r > 0.0 {
                    2.0 * p * r / (p + r)
                } else {
                    0.0
                };
                (
                    ct,
                    TypeResult {
                        true_positives: type_tp,
                        false_positives: type_fp,
                        false_negatives: type_fn,
                        precision: p,
                        recall: r,
                        f1: f,
                    },
                )
            })
            .collect();

        BenchmarkResult {
            true_positives: tp,
            false_positives: fp,
            false_negatives: fn_,
            precision,
            recall,
            f1,
            per_type,
        }
    }

    /// Run evaluation with a sweep of similarity thresholds.
    ///
    /// Calls `detect_fn(threshold)` for each threshold and evaluates the result.
    pub fn sweep_thresholds(
        &self,
        thresholds: &[f64],
        detect_fn: impl Fn(f64) -> Vec<CloneGroup>,
    ) -> Vec<(f64, BenchmarkResult)> {
        thresholds
            .iter()
            .map(|&t| (t, self.evaluate(&detect_fn(t))))
            .collect()
    }

    /// Parse BigCloneBench `functions.csv`.
    ///
    /// Format: `id,file,start_line,end_line[,functionality_id]`
    fn parse_functions_csv(path: &Path) -> Result<Vec<CodeFragment>, crate::core::Error> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            crate::core::Error::config(format!("Cannot read {}: {e}", path.display()))
        })?;

        let mut fragments = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            if idx == 0 {
                continue; // skip header
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 4 {
                fragments.push(CodeFragment {
                    id: parts[0].trim().parse().unwrap_or(idx),
                    file: parts[1].trim().to_string(),
                    start_line: parts[2].trim().parse().unwrap_or(0),
                    end_line: parts[3].trim().parse().unwrap_or(0),
                    source: String::new(),
                    functionality_id: parts.get(4).and_then(|s| s.trim().parse().ok()),
                });
            }
        }

        Ok(fragments)
    }

    /// Parse BigCloneBench `clones.csv`.
    ///
    /// Format: `id1,id2,type,is_clone`
    /// Type values: `1`/`T1` = Type1, `2`/`T2` = Type2,
    /// `3`/`T3`/`VST3`/`ST3`/`MT3`/`WT3` = Type3, `4`/`T4` = Type4.
    fn parse_clones_csv(path: &Path) -> Result<Vec<GroundTruthPair>, crate::core::Error> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            crate::core::Error::config(format!("Cannot read {}: {e}", path.display()))
        })?;

        let mut pairs = Vec::new();
        for (idx, line) in content.lines().enumerate() {
            if idx == 0 {
                continue; // skip header
            }
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split(',').collect();
            if parts.len() >= 4 {
                let clone_type = match parts[2].trim() {
                    "1" | "T1" => CloneType::Type1,
                    "2" | "T2" => CloneType::Type2,
                    "3" | "T3" | "VST3" | "ST3" | "MT3" | "WT3" => CloneType::Type3,
                    "4" | "T4" => CloneType::Type4,
                    _ => CloneType::Type3,
                };
                let is_clone =
                    parts[3].trim() == "true" || parts[3].trim() == "1" || parts[3].trim() == "T";
                pairs.push(GroundTruthPair {
                    fragment_a: parts[0].trim().parse().unwrap_or(0),
                    fragment_b: parts[1].trim().parse().unwrap_or(0),
                    clone_type,
                    is_clone,
                });
            }
        }

        Ok(pairs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clones::types::CloneInstance;

    fn make_instance(file: &str, start: usize, end: usize) -> CloneInstance {
        CloneInstance {
            file: file.to_string(),
            start_line: start,
            end_line: end,
            start_byte: 0,
            end_byte: 0,
            function_name: None,
        }
    }

    #[test]
    fn test_empty_benchmark_zero_metrics() {
        let bench = CloneBenchmark::new(Vec::new(), Vec::new());
        let result = bench.evaluate(&[]);
        assert_eq!(result.true_positives, 0);
        assert_eq!(result.false_positives, 0);
        assert_eq!(result.false_negatives, 0);
        assert_eq!(result.precision, 0.0);
        assert_eq!(result.recall, 0.0);
        assert_eq!(result.f1, 0.0);
    }

    #[test]
    fn test_perfect_detection() {
        let fragments = vec![
            CodeFragment {
                id: 1,
                file: "a.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
            CodeFragment {
                id: 2,
                file: "b.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
        ];
        let ground_truth = vec![GroundTruthPair {
            fragment_a: 1,
            fragment_b: 2,
            clone_type: CloneType::Type1,
            is_clone: true,
        }];

        let bench = CloneBenchmark::new(fragments, ground_truth);

        // Detect exactly the right pair
        let detected = vec![CloneGroup::new(
            CloneType::Type1,
            vec![make_instance("a.py", 1, 10), make_instance("b.py", 1, 10)],
        )];

        let result = bench.evaluate(&detected);
        assert_eq!(result.true_positives, 1);
        assert_eq!(result.false_positives, 0);
        assert_eq!(result.false_negatives, 0);
        assert!((result.precision - 1.0).abs() < f64::EPSILON);
        assert!((result.recall - 1.0).abs() < f64::EPSILON);
        assert!((result.f1 - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_no_detection_zero_recall() {
        let fragments = vec![
            CodeFragment {
                id: 1,
                file: "a.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
            CodeFragment {
                id: 2,
                file: "b.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
        ];
        let ground_truth = vec![GroundTruthPair {
            fragment_a: 1,
            fragment_b: 2,
            clone_type: CloneType::Type3,
            is_clone: true,
        }];

        let bench = CloneBenchmark::new(fragments, ground_truth);
        let result = bench.evaluate(&[]); // no detections

        assert_eq!(result.true_positives, 0);
        assert_eq!(result.false_negatives, 1);
        assert_eq!(result.recall, 0.0);
        assert_eq!(result.f1, 0.0);
    }

    #[test]
    fn test_false_positive_detection() {
        let fragments = vec![
            CodeFragment {
                id: 1,
                file: "a.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
            CodeFragment {
                id: 2,
                file: "b.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
        ];
        // No ground truth clones
        let ground_truth = Vec::new();
        let bench = CloneBenchmark::new(fragments, ground_truth);

        // Detect a spurious pair
        let detected = vec![CloneGroup::new(
            CloneType::Type3,
            vec![make_instance("a.py", 1, 10), make_instance("b.py", 1, 10)],
        )];

        let result = bench.evaluate(&detected);
        assert_eq!(result.true_positives, 0);
        assert_eq!(result.false_positives, 1);
        assert_eq!(result.precision, 0.0);
    }

    #[test]
    fn test_csv_parsing_handles_missing_data() {
        let dir = tempfile::tempdir().unwrap();

        // Write functions.csv with some bad rows
        std::fs::write(
            dir.path().join("functions.csv"),
            "id,file,start_line,end_line\n\
             1,a.py,1,10\n\
             bad_row\n\
             3,c.py,5,20,42\n",
        )
        .unwrap();

        // Write clones.csv with various type formats
        std::fs::write(
            dir.path().join("clones.csv"),
            "id1,id2,type,is_clone\n\
             1,3,T1,true\n\
             1,3,VST3,1\n\
             short\n",
        )
        .unwrap();

        let bench = CloneBenchmark::from_bigclonebench(dir.path()).unwrap();
        // Should parse 2 valid fragments (bad_row skipped)
        assert_eq!(bench.fragments.len(), 2);
        assert_eq!(bench.fragments[0].id, 1);
        assert_eq!(bench.fragments[1].id, 3);
        assert_eq!(bench.fragments[1].functionality_id, Some(42));

        // Should parse 2 valid pairs (short skipped)
        assert_eq!(bench.ground_truth.len(), 2);
        assert_eq!(bench.ground_truth[0].clone_type, CloneType::Type1);
        assert!(bench.ground_truth[0].is_clone);
        assert_eq!(bench.ground_truth[1].clone_type, CloneType::Type3);
        assert!(bench.ground_truth[1].is_clone);
    }

    #[test]
    fn test_from_bigclonebench_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        // No CSV files at all
        let bench = CloneBenchmark::from_bigclonebench(dir.path()).unwrap();
        assert!(bench.fragments.is_empty());
        assert!(bench.ground_truth.is_empty());
    }

    #[test]
    fn test_non_clone_pairs_ignored() {
        let fragments = vec![
            CodeFragment {
                id: 1,
                file: "a.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
            CodeFragment {
                id: 2,
                file: "b.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
        ];
        // Ground truth says these are NOT clones
        let ground_truth = vec![GroundTruthPair {
            fragment_a: 1,
            fragment_b: 2,
            clone_type: CloneType::Type1,
            is_clone: false,
        }];

        let bench = CloneBenchmark::new(fragments, ground_truth);
        let result = bench.evaluate(&[]);

        // Non-clone pair should not count as FN
        assert_eq!(result.false_negatives, 0);
    }

    #[test]
    fn test_sweep_thresholds() {
        let bench = CloneBenchmark::new(Vec::new(), Vec::new());
        let results = bench.sweep_thresholds(&[0.3, 0.5, 0.7], |_threshold| Vec::new());
        assert_eq!(results.len(), 3);
        assert!((results[0].0 - 0.3).abs() < f64::EPSILON);
        assert!((results[1].0 - 0.5).abs() < f64::EPSILON);
        assert!((results[2].0 - 0.7).abs() < f64::EPSILON);
    }

    #[test]
    fn test_per_type_breakdown() {
        let fragments = vec![
            CodeFragment {
                id: 1,
                file: "a.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
            CodeFragment {
                id: 2,
                file: "b.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
            CodeFragment {
                id: 3,
                file: "c.py".to_string(),
                start_line: 1,
                end_line: 10,
                source: String::new(),
                functionality_id: None,
            },
        ];
        let ground_truth = vec![
            GroundTruthPair {
                fragment_a: 1,
                fragment_b: 2,
                clone_type: CloneType::Type1,
                is_clone: true,
            },
            GroundTruthPair {
                fragment_a: 1,
                fragment_b: 3,
                clone_type: CloneType::Type3,
                is_clone: true,
            },
        ];

        let bench = CloneBenchmark::new(fragments, ground_truth);

        // Only detect the Type1 pair
        let detected = vec![CloneGroup::new(
            CloneType::Type1,
            vec![make_instance("a.py", 1, 10), make_instance("b.py", 1, 10)],
        )];

        let result = bench.evaluate(&detected);
        assert_eq!(result.true_positives, 1);
        assert_eq!(result.false_negatives, 1);

        // Type1 should have 100% recall
        let t1 = result.per_type.get(&CloneType::Type1).unwrap();
        assert_eq!(t1.true_positives, 1);
        assert_eq!(t1.false_negatives, 0);

        // Type3 should have 0% recall
        let t3 = result.per_type.get(&CloneType::Type3).unwrap();
        assert_eq!(t3.true_positives, 0);
        assert_eq!(t3.false_negatives, 1);
    }
}
