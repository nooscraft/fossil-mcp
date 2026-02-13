//! Threshold evaluation: determine pass/fail based on configurable limits.

use crate::config::CiConfig;
use crate::core::Confidence;

use super::ThresholdViolation;

/// Evaluates whether findings exceed configured thresholds.
pub struct ThresholdEvaluator {
    config: CiConfig,
}

impl ThresholdEvaluator {
    /// Create a new evaluator with the given config.
    pub fn new(config: CiConfig) -> Self {
        Self { config }
    }

    /// Evaluate findings and return violations.
    /// Returns empty vec if all thresholds are met.
    pub fn evaluate(
        &self,
        dead_code_count: usize,
        clone_count: usize,
        scaffolding_count: usize,
    ) -> Vec<ThresholdViolation> {
        let mut violations = Vec::new();

        // Check dead code threshold
        if let Some(threshold) = self.config.max_dead_code {
            if dead_code_count > threshold {
                violations.push(ThresholdViolation {
                    category: "dead_code".to_string(),
                    threshold,
                    actual: dead_code_count,
                    message: format!(
                        "Dead code findings ({}) exceed threshold ({})",
                        dead_code_count, threshold
                    ),
                });
            }
        }

        // Check clones threshold
        if let Some(threshold) = self.config.max_clones {
            if clone_count > threshold {
                violations.push(ThresholdViolation {
                    category: "clones".to_string(),
                    threshold,
                    actual: clone_count,
                    message: format!(
                        "Clone groups ({}) exceed threshold ({})",
                        clone_count, threshold
                    ),
                });
            }
        }

        // Check scaffolding threshold
        if let Some(threshold) = self.config.max_scaffolding {
            if scaffolding_count > threshold {
                violations.push(ThresholdViolation {
                    category: "scaffolding".to_string(),
                    threshold,
                    actual: scaffolding_count,
                    message: format!(
                        "Scaffolding artifacts ({}) exceed threshold ({})",
                        scaffolding_count, threshold
                    ),
                });
            }
        }

        // Check fail_on_scaffolding flag
        if self.config.fail_on_scaffolding == Some(true) && scaffolding_count > 0 {
            violations.push(ThresholdViolation {
                category: "scaffolding".to_string(),
                threshold: 0,
                actual: scaffolding_count,
                message: format!(
                    "Scaffolding artifacts ({}) found but fail_on_scaffolding is enabled",
                    scaffolding_count
                ),
            });
        }

        violations
    }

    /// Get the minimum confidence filter if configured.
    pub fn min_confidence(&self) -> Option<Confidence> {
        self.config.min_confidence.as_ref().and_then(|s| {
            match s.to_lowercase().as_str() {
                "certain" => Some(Confidence::Certain),
                "high" => Some(Confidence::High),
                "medium" => Some(Confidence::Medium),
                "low" => Some(Confidence::Low),
                _ => None,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_thresholds_no_violations() {
        let config = CiConfig::default();
        let evaluator = ThresholdEvaluator::new(config);
        let violations = evaluator.evaluate(100, 50, 10);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_dead_code_threshold_exceeded() {
        let config = CiConfig {
            max_dead_code: Some(10),
            ..Default::default()
        };
        let evaluator = ThresholdEvaluator::new(config);
        let violations = evaluator.evaluate(15, 0, 0);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "dead_code");
        assert_eq!(violations[0].actual, 15);
        assert_eq!(violations[0].threshold, 10);
    }

    #[test]
    fn test_multiple_threshold_violations() {
        let config = CiConfig {
            max_dead_code: Some(5),
            max_clones: Some(3),
            ..Default::default()
        };
        let evaluator = ThresholdEvaluator::new(config);
        let violations = evaluator.evaluate(10, 5, 0);
        assert_eq!(violations.len(), 2);
        assert!(violations.iter().any(|v| v.category == "dead_code"));
        assert!(violations.iter().any(|v| v.category == "clones"));
    }

    #[test]
    fn test_fail_on_scaffolding_flag() {
        let config = CiConfig {
            fail_on_scaffolding: Some(true),
            ..Default::default()
        };
        let evaluator = ThresholdEvaluator::new(config);
        let violations = evaluator.evaluate(0, 0, 5);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].category, "scaffolding");
    }

    #[test]
    fn test_min_confidence_parsing() {
        let config = CiConfig {
            min_confidence: Some("high".to_string()),
            ..Default::default()
        };
        let evaluator = ThresholdEvaluator::new(config);
        assert_eq!(evaluator.min_confidence(), Some(Confidence::High));
    }
}
