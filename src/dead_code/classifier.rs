//! Dead code classification and confidence scoring.

use std::collections::{HashMap, HashSet};

use crate::core::{
    CodeNode, Confidence, FossilType, NodeKind, RemovalImpact, Severity, Visibility,
};
use crate::dead_code::BddContextDetector;
use crate::graph::CodeGraph;
use petgraph::graph::NodeIndex;

/// A classified dead code finding.
#[derive(Debug, Clone)]
pub struct DeadCodeFinding {
    pub node_index: NodeIndex,
    pub name: String,
    pub full_name: String,
    pub kind: NodeKind,
    pub fossil_type: FossilType,
    pub confidence: Confidence,
    pub severity: Severity,
    pub removal_impact: RemovalImpact,
    pub reason: String,
    pub file: String,
    pub line_start: usize,
    pub line_end: usize,
    pub lines_of_code: usize,
}

/// Classifies unreachable nodes into dead code findings.
pub struct DeadCodeClassifier<'a> {
    graph: &'a CodeGraph,
    centrality_scores: Option<HashMap<NodeIndex, f64>>,
}

impl<'a> DeadCodeClassifier<'a> {
    pub fn new(graph: &'a CodeGraph) -> Self {
        Self {
            graph,
            centrality_scores: None,
        }
    }

    pub fn with_centrality(
        graph: &'a CodeGraph,
        centrality_scores: HashMap<NodeIndex, f64>,
    ) -> Self {
        Self {
            graph,
            centrality_scores: Some(centrality_scores),
        }
    }

    /// Classify all unreachable nodes.
    pub fn classify(
        &self,
        production_reachable: &HashSet<NodeIndex>,
        test_reachable: &HashSet<NodeIndex>,
    ) -> Vec<DeadCodeFinding> {
        let mut findings = Vec::new();

        for (idx, node) in self.graph.nodes() {
            if production_reachable.contains(&idx) {
                continue;
            }

            let is_test_only = test_reachable.contains(&idx);
            let fossil_type = if is_test_only {
                FossilType::TestOnlyCode
            } else {
                self.classify_type(node)
            };

            let confidence = self.compute_confidence(idx, node);
            let severity = self.compute_severity(node, &confidence);
            let removal_impact = if is_test_only {
                RemovalImpact::RisksBreakage
            } else {
                self.compute_removal_impact(node, &confidence)
            };
            let reason = self.generate_reason(node, &fossil_type, &confidence);

            findings.push(DeadCodeFinding {
                node_index: idx,
                name: node.name.clone(),
                full_name: node.full_name.clone(),
                kind: node.kind,
                fossil_type,
                confidence,
                severity,
                removal_impact,
                reason,
                file: node.location.file.clone(),
                line_start: node.location.line_start,
                line_end: node.location.line_end,
                lines_of_code: node.lines_of_code,
            });
        }

        // Sort by confidence (highest first), then by file and line
        findings.sort_by(|a, b| {
            b.confidence
                .cmp(&a.confidence)
                .then_with(|| a.file.cmp(&b.file))
                .then_with(|| a.line_start.cmp(&b.line_start))
        });

        findings
    }

    fn classify_type(&self, node: &CodeNode) -> FossilType {
        match node.kind {
            NodeKind::Function
            | NodeKind::AsyncFunction
            | NodeKind::Lambda
            | NodeKind::Closure
            | NodeKind::StaticMethod => FossilType::DeadFunction,
            NodeKind::Method | NodeKind::AsyncMethod | NodeKind::Constructor => {
                FossilType::DeadFunction
            }
            NodeKind::ImportDeclaration => FossilType::UnusedImport,
            NodeKind::ExportDeclaration => FossilType::UnusedExport,
            NodeKind::Variable => FossilType::UnusedVariable,
            NodeKind::Parameter => FossilType::UnusedParameter,
            NodeKind::Class
            | NodeKind::Struct
            | NodeKind::Enum
            | NodeKind::Trait
            | NodeKind::Interface => FossilType::Unreachable,
            _ => FossilType::Unreachable,
        }
    }

    fn compute_confidence(&self, idx: NodeIndex, node: &CodeNode) -> Confidence {
        let has_callers = self.graph.callers_of(idx).next().is_some();

        if has_callers {
            return Confidence::Low; // Has callers but still unreachable? Dynamic dispatch.
        }

        // Check for dynamic call indicators
        let has_dynamic_indicators = node.attributes.iter().any(|a| {
            a.contains("dynamic")
                || a.contains("reflect")
                || a.contains("eval")
                || a.contains("getattr")
        });

        if has_dynamic_indicators {
            return Confidence::Low;
        }

        // Check for behavior-driven markers (callbacks, middleware, lifecycle methods, etc.)
        // These indicate code is actually alive despite appearing unreachable
        let behavior_markers = BddContextDetector::detect_markers(node);
        if !behavior_markers.is_empty() {
            return Confidence::Low; // Behavior markers indicate code is likely alive
        }

        // If centrality data is available and this node is in the top percentile,
        // downgrade confidence by one level (high-centrality nodes are risky to call dead)
        if let Some(ref scores) = self.centrality_scores {
            if let Some(&score) = scores.get(&idx) {
                // Compute the 90th percentile threshold
                let mut all_scores: Vec<f64> = scores.values().copied().collect();
                all_scores.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                let p90_idx = (all_scores.len() as f64 * 0.9) as usize;
                let threshold = all_scores.get(p90_idx).copied().unwrap_or(f64::MAX);

                if score >= threshold {
                    // Downgrade confidence for high-centrality nodes
                    return match node.visibility {
                        Visibility::Private => Confidence::High, // was Certain
                        Visibility::Internal | Visibility::Protected => Confidence::Medium, // was High
                        Visibility::Public => Confidence::Low, // was Medium
                        Visibility::Unknown => Confidence::Low, // was Medium
                    };
                }
            }
        }

        // Visibility affects confidence: private unreachable is more certain
        match node.visibility {
            Visibility::Private => Confidence::Certain,
            Visibility::Internal | Visibility::Protected => Confidence::High,
            Visibility::Public => Confidence::Medium,
            Visibility::Unknown => Confidence::Medium,
        }
    }

    fn compute_severity(&self, node: &CodeNode, confidence: &Confidence) -> Severity {
        match confidence {
            Confidence::Certain => {
                if node.lines_of_code > 50 {
                    Severity::High
                } else {
                    Severity::Medium
                }
            }
            Confidence::High => Severity::Medium,
            Confidence::Medium => Severity::Low,
            Confidence::Low => Severity::Info,
        }
    }

    fn compute_removal_impact(&self, node: &CodeNode, confidence: &Confidence) -> RemovalImpact {
        if *confidence == Confidence::Certain {
            RemovalImpact::Safe
        } else if node.documentation.is_some() {
            RemovalImpact::HasDocumentation
        } else if node.visibility == Visibility::Public {
            RemovalImpact::RisksBreakage
        } else {
            RemovalImpact::Safe
        }
    }

    fn generate_reason(
        &self,
        node: &CodeNode,
        fossil_type: &FossilType,
        confidence: &Confidence,
    ) -> String {
        let kind_name = node.kind.to_string();
        let conf = confidence.to_string();

        // Check for behavior markers to provide more context
        let behavior_markers = BddContextDetector::detect_markers(node);
        let behavior_hint = if !behavior_markers.is_empty() {
            format!(
                " (detected: {:?})",
                behavior_markers
                    .iter()
                    .map(|m| format!("{:?}", m))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };

        match fossil_type {
            FossilType::DeadFunction => {
                format!(
                    "{} `{}` is never called ({} confidence){}",
                    kind_name, node.name, conf, behavior_hint
                )
            }
            FossilType::UnusedImport => {
                format!("import `{}` is never used", node.name)
            }
            FossilType::UnusedExport => {
                format!("export `{}` is never consumed", node.name)
            }
            FossilType::UnusedVariable => {
                format!("variable `{}` is never read", node.name)
            }
            FossilType::TestOnlyCode => {
                format!(
                    "{} `{}` is only reachable from test code",
                    kind_name, node.name
                )
            }
            _ => {
                format!(
                    "{} `{}` is unreachable ({} confidence){}",
                    kind_name, node.name, conf, behavior_hint
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CallEdge, Language, SourceLocation};

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
    fn test_classify_dead_function() {
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

        let reachable = graph.compute_production_reachable();
        let classifier = DeadCodeClassifier::new(&graph);
        let findings = classifier.classify(&reachable, &HashSet::new());

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].name, "dead_fn");
        assert_eq!(findings[0].fossil_type, FossilType::DeadFunction);
        assert_eq!(findings[0].confidence, Confidence::Certain);
    }

    #[test]
    fn test_classify_test_only() {
        let mut graph = CodeGraph::new();
        let main = make_node("main", NodeKind::Function, Visibility::Public);
        let test_helper = make_node("test_helper", NodeKind::Function, Visibility::Private);

        let main_idx = graph.add_node(main);
        let test_idx = graph.add_node(test_helper);

        graph.add_entry_point(main_idx);
        graph.add_test_entry_point(test_idx);

        let prod_reachable = graph.compute_production_reachable();
        let test_reachable = graph.compute_test_reachable();

        let classifier = DeadCodeClassifier::new(&graph);
        let findings = classifier.classify(&prod_reachable, &test_reachable);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].fossil_type, FossilType::TestOnlyCode);
    }

    #[test]
    fn test_centrality_downgrades_confidence_for_private() {
        // Build a graph with a single unreachable private function
        let mut graph = CodeGraph::new();
        let main = make_node("main", NodeKind::Function, Visibility::Public);
        let main_idx = graph.add_node(main);
        graph.add_entry_point(main_idx);

        let dead_private = make_node("dead_private", NodeKind::Function, Visibility::Private);
        let dead_idx = graph.add_node(dead_private);

        // Without centrality, private unreachable => Certain
        let classifier_no_cent = DeadCodeClassifier::new(&graph);
        let reachable = graph.compute_production_reachable();
        let findings = classifier_no_cent.classify(&reachable, &HashSet::new());
        let finding = findings.iter().find(|f| f.name == "dead_private").unwrap();
        assert_eq!(finding.confidence, Confidence::Certain);

        // With centrality where dead_private is in the top 90th percentile,
        // it should be downgraded to High
        let mut centrality_scores = HashMap::new();
        // Add 10 low-score entries to establish the distribution
        for i in 0..10 {
            centrality_scores.insert(NodeIndex::new(i + 100), 0.1);
        }
        // dead_private gets a high centrality score (in top 10%)
        centrality_scores.insert(dead_idx, 0.99);

        let classifier_with_cent = DeadCodeClassifier::with_centrality(&graph, centrality_scores);
        let findings = classifier_with_cent.classify(&reachable, &HashSet::new());
        let finding = findings.iter().find(|f| f.name == "dead_private").unwrap();
        assert_eq!(
            finding.confidence,
            Confidence::High,
            "High-centrality private node should be downgraded from Certain to High"
        );
    }

    #[test]
    fn test_centrality_downgrades_confidence_for_public() {
        // Public unreachable => Medium; with high centrality => Low
        let mut graph = CodeGraph::new();
        let main = make_node("main", NodeKind::Function, Visibility::Public);
        let main_idx = graph.add_node(main);
        graph.add_entry_point(main_idx);

        let dead_pub = make_node("dead_public", NodeKind::Function, Visibility::Public);
        let dead_idx = graph.add_node(dead_pub);

        let reachable = graph.compute_production_reachable();

        let mut centrality_scores = HashMap::new();
        for i in 0..10 {
            centrality_scores.insert(NodeIndex::new(i + 100), 0.05);
        }
        centrality_scores.insert(dead_idx, 0.95);

        let classifier = DeadCodeClassifier::with_centrality(&graph, centrality_scores);
        let findings = classifier.classify(&reachable, &HashSet::new());
        let finding = findings.iter().find(|f| f.name == "dead_public").unwrap();
        assert_eq!(
            finding.confidence,
            Confidence::Low,
            "High-centrality public node should be downgraded from Medium to Low"
        );
    }

    #[test]
    fn test_centrality_no_downgrade_for_low_centrality() {
        // If the node is NOT in the top 90th percentile, no downgrade
        let mut graph = CodeGraph::new();
        let main = make_node("main", NodeKind::Function, Visibility::Public);
        let main_idx = graph.add_node(main);
        graph.add_entry_point(main_idx);

        let dead_private = make_node("dead_private", NodeKind::Function, Visibility::Private);
        let dead_idx = graph.add_node(dead_private);

        let reachable = graph.compute_production_reachable();

        let mut centrality_scores = HashMap::new();
        // dead_private has a LOW centrality score
        centrality_scores.insert(dead_idx, 0.01);
        // Add high-score entries so dead_private is NOT in top 10%
        for i in 0..10 {
            centrality_scores.insert(NodeIndex::new(i + 100), 0.9);
        }

        let classifier = DeadCodeClassifier::with_centrality(&graph, centrality_scores);
        let findings = classifier.classify(&reachable, &HashSet::new());
        let finding = findings.iter().find(|f| f.name == "dead_private").unwrap();
        assert_eq!(
            finding.confidence,
            Confidence::Certain,
            "Low-centrality private node should remain Certain (no downgrade)"
        );
    }
}
