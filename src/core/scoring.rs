//! Unified confidence and risk scoring across all analysis engines.
//!
//! Provides structural and pattern-based signals that adjust base
//! severity/confidence from rules into calibrated priority scores.

use super::types::{Confidence, NodeKind, Severity, Visibility};

/// Structural signals from the code graph about a node.
#[derive(Debug, Clone, Default)]
pub struct StructuralSignals {
    /// Visibility of the code element.
    pub visibility: Option<Visibility>,
    /// Number of incoming edges (callers).
    pub in_degree: usize,
    /// Number of outgoing edges (callees).
    pub out_degree: usize,
    /// Whether this node is an entry point.
    pub is_entry_point: bool,
    /// Lines of code in this element.
    pub lines_of_code: usize,
    /// Kind of the node.
    pub node_kind: Option<NodeKind>,
    /// PageRank score (0.0-1.0, if computed).
    pub pagerank: Option<f64>,
    /// Whether this is a test node.
    pub is_test: bool,
}

/// Pattern-based signals from the match context.
#[derive(Debug, Clone, Default)]
pub struct PatternSignals {
    /// Known false positive indicators present.
    pub known_fp_indicators: bool,
    /// The match appears to be a placeholder/example.
    pub is_placeholder: bool,
    /// Dynamic dispatch indicators (reflection, eval, getattr).
    pub dynamic_dispatch_indicators: bool,
    /// User input nearby (request, params, argv, etc.).
    pub user_input_nearby: bool,
    /// Match is in a test file.
    pub is_test_file: bool,
    /// Match is in a comment or docstring.
    pub is_in_comment: bool,
    /// Number of similar findings in the same file.
    pub sibling_findings: usize,
}

/// A computed priority score combining all signals.
#[derive(Debug, Clone)]
pub struct PriorityScore {
    /// Final numeric score (0.0-10.0, higher = more important).
    pub score: f64,
    /// Adjusted severity after signal analysis.
    pub severity: Severity,
    /// Adjusted confidence after signal analysis.
    pub confidence: Confidence,
    /// Human-readable explanation of the score.
    pub explanation: String,
}

/// Compute a priority score from base values and signals.
pub fn compute_priority(
    base_severity: Severity,
    base_confidence: Confidence,
    structural: &StructuralSignals,
    pattern: &PatternSignals,
) -> PriorityScore {
    let mut severity_num = severity_to_f64(base_severity);
    let mut confidence_num = confidence_to_f64(base_confidence);
    let mut reasons = Vec::new();

    // Pattern-based adjustments
    if pattern.known_fp_indicators || pattern.is_placeholder {
        confidence_num *= 0.3;
        reasons.push("placeholder/FP indicators detected");
    }
    if pattern.is_in_comment {
        confidence_num *= 0.1;
        reasons.push("in comment");
    }
    if pattern.is_test_file {
        severity_num *= 0.5;
        reasons.push("in test file");
    }
    if pattern.user_input_nearby {
        confidence_num = (confidence_num * 1.3).min(1.0);
        reasons.push("user input nearby");
    }
    if pattern.dynamic_dispatch_indicators {
        confidence_num *= 0.5;
        reasons.push("dynamic dispatch possible");
    }
    if pattern.sibling_findings > 5 {
        confidence_num *= 0.8;
        reasons.push("many similar findings (possible FP pattern)");
    }

    // Structural adjustments
    if structural.is_entry_point {
        severity_num = (severity_num * 1.2).min(1.0);
        reasons.push("entry point");
    }
    if let Some(pr) = structural.pagerank {
        if pr > 0.05 {
            severity_num = (severity_num * 1.1).min(1.0);
            reasons.push("high centrality");
        }
    }
    if structural.visibility == Some(Visibility::Private) && structural.in_degree == 0 {
        confidence_num = (confidence_num * 1.2).min(1.0);
        reasons.push("private with no callers");
    }
    if structural.is_test {
        severity_num *= 0.5;
    }

    let score = (severity_num * 0.6 + confidence_num * 0.4) * 10.0;
    let adjusted_severity = f64_to_severity(severity_num);
    let adjusted_confidence = f64_to_confidence(confidence_num);

    let explanation = if reasons.is_empty() {
        "base score".to_string()
    } else {
        reasons.join("; ")
    };

    PriorityScore {
        score,
        severity: adjusted_severity,
        confidence: adjusted_confidence,
        explanation,
    }
}

/// Adjust dead code confidence based on structural and pattern signals.
pub fn adjust_dead_code_confidence(base: Confidence, structural: &StructuralSignals) -> Confidence {
    let mut conf = confidence_to_f64(base);

    // Private + no callers + not entry point → very confident it's dead
    if structural.visibility == Some(Visibility::Private)
        && structural.in_degree == 0
        && !structural.is_entry_point
    {
        conf = (conf * 1.3).min(1.0);
    }

    // Public visibility → less confident (could be called externally)
    if structural.visibility == Some(Visibility::Public) {
        conf *= 0.6;
    }

    // High centrality (PageRank) + unreachable → suspicious, lower confidence
    if let Some(pr) = structural.pagerank {
        if pr > 0.05 {
            conf *= 0.5;
        }
    }

    // Dynamic dispatch indicators → lower confidence
    if structural.lines_of_code > 100 {
        // Large unused functions are more suspicious (but could be intentional dead code)
        conf = (conf * 1.1).min(1.0);
    }

    f64_to_confidence(conf)
}

/// Adjust security finding confidence based on pattern signals.
pub fn adjust_security_confidence(base: Confidence, pattern: &PatternSignals) -> Confidence {
    let mut conf = confidence_to_f64(base);

    if pattern.known_fp_indicators || pattern.is_placeholder {
        conf *= 0.3;
    }
    if pattern.user_input_nearby {
        conf = (conf * 1.3).min(1.0);
    }
    if pattern.is_in_comment {
        conf *= 0.1;
    }
    if pattern.is_test_file {
        conf *= 0.5;
    }

    f64_to_confidence(conf)
}

/// Compute clone detection confidence from similarity and structural info.
pub fn clone_confidence(similarity: f64, lines: usize) -> Confidence {
    let base: f64 = if similarity > 0.95 {
        1.0
    } else if similarity > 0.8 {
        0.8
    } else if similarity > 0.6 {
        0.6
    } else {
        0.4
    };

    // Larger clones are more confident
    let size_factor: f64 = if lines > 50 {
        1.1
    } else if lines > 20 {
        1.0
    } else if lines > 10 {
        0.9
    } else {
        0.7
    };

    f64_to_confidence((base * size_factor).min(1.0))
}

fn severity_to_f64(s: Severity) -> f64 {
    match s {
        Severity::Critical => 1.0,
        Severity::High => 0.8,
        Severity::Medium => 0.6,
        Severity::Low => 0.4,
        Severity::Info => 0.2,
    }
}

fn f64_to_severity(v: f64) -> Severity {
    if v >= 0.9 {
        Severity::Critical
    } else if v >= 0.7 {
        Severity::High
    } else if v >= 0.5 {
        Severity::Medium
    } else if v >= 0.3 {
        Severity::Low
    } else {
        Severity::Info
    }
}

fn confidence_to_f64(c: Confidence) -> f64 {
    match c {
        Confidence::Certain => 1.0,
        Confidence::High => 0.8,
        Confidence::Medium => 0.6,
        Confidence::Low => 0.3,
    }
}

fn f64_to_confidence(v: f64) -> Confidence {
    if v >= 0.9 {
        Confidence::Certain
    } else if v >= 0.7 {
        Confidence::High
    } else if v >= 0.45 {
        Confidence::Medium
    } else {
        Confidence::Low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_priority_base() {
        let structural = StructuralSignals::default();
        let pattern = PatternSignals::default();
        let score = compute_priority(Severity::High, Confidence::High, &structural, &pattern);
        assert!(score.score > 5.0);
        assert_eq!(score.severity, Severity::High);
    }

    #[test]
    fn test_placeholder_lowers_confidence() {
        let structural = StructuralSignals::default();
        let pattern = PatternSignals {
            is_placeholder: true,
            ..Default::default()
        };
        let score = compute_priority(Severity::High, Confidence::High, &structural, &pattern);
        assert!(score.confidence < Confidence::High);
    }

    #[test]
    fn test_dead_code_private_high_confidence() {
        let structural = StructuralSignals {
            visibility: Some(Visibility::Private),
            in_degree: 0,
            is_entry_point: false,
            ..Default::default()
        };
        let conf = adjust_dead_code_confidence(Confidence::High, &structural);
        assert!(conf >= Confidence::High);
    }

    #[test]
    fn test_dead_code_public_lower_confidence() {
        let structural = StructuralSignals {
            visibility: Some(Visibility::Public),
            in_degree: 0,
            is_entry_point: false,
            ..Default::default()
        };
        let conf = adjust_dead_code_confidence(Confidence::High, &structural);
        assert!(conf <= Confidence::High);
    }

    #[test]
    fn test_clone_confidence_high_similarity() {
        let conf = clone_confidence(0.98, 30);
        assert!(conf >= Confidence::High);
    }

    #[test]
    fn test_clone_confidence_low_similarity() {
        let conf = clone_confidence(0.5, 5);
        assert!(conf <= Confidence::Medium);
    }

    #[test]
    fn test_security_confidence_with_user_input() {
        let pattern = PatternSignals {
            user_input_nearby: true,
            ..Default::default()
        };
        let conf = adjust_security_confidence(Confidence::Medium, &pattern);
        assert!(conf >= Confidence::Medium);
    }
}
