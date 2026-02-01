//! Tree edit distance computation for Type-4 semantic clone verification.
//!
//! Provides a simplified Zhang-Shasha algorithm for computing the edit distance
//! between two labeled ordered trees, plus utilities for converting source code
//! to labeled trees using indentation/brace heuristics.

use std::cmp;

/// A labeled ordered tree node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabeledTree {
    /// The label of this node (e.g., "if", "for", "call", "assign").
    pub label: String,
    /// Ordered children of this node.
    pub children: Vec<LabeledTree>,
}

impl LabeledTree {
    /// Create a new labeled tree node.
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            children: Vec::new(),
        }
    }

    /// Create a new labeled tree node with children.
    pub fn with_children(label: impl Into<String>, children: Vec<LabeledTree>) -> Self {
        Self {
            label: label.into(),
            children,
        }
    }

    /// Total number of nodes in this tree.
    pub fn size(&self) -> usize {
        1 + self.children.iter().map(|c| c.size()).sum::<usize>()
    }
}

/// Flattened tree representation for Zhang-Shasha algorithm.
/// All indices are 1-based (index 0 is unused / sentinel).
struct IndexedTree {
    /// Labels in postorder (1-indexed; labels[0] is unused).
    labels: Vec<String>,
    /// leftmost_leaf[i] = postorder index of leftmost leaf descendant of node i.
    leftmost_leaf: Vec<usize>,
    /// key_roots: set of key root postorder indices.
    key_roots: Vec<usize>,
    /// Number of nodes.
    n: usize,
}

impl IndexedTree {
    /// Build an indexed tree from a LabeledTree via postorder traversal.
    fn from_tree(tree: &LabeledTree) -> Self {
        let n = tree.size();
        let mut labels = vec![String::new(); n + 1];
        let mut leftmost_leaf = vec![0usize; n + 1];
        let mut parent = vec![0usize; n + 1];
        let mut idx = 1usize;

        Self::fill_postorder(tree, &mut labels, &mut leftmost_leaf, &mut parent, &mut idx);

        // Compute key roots
        let mut key_roots = Vec::new();
        // A node is a key root if it's the root, or its leftmost leaf differs from parent's
        for i in 1..=n {
            if parent[i] == 0 || leftmost_leaf[i] != leftmost_leaf[parent[i]] {
                key_roots.push(i);
            }
        }
        key_roots.sort_unstable();

        Self {
            labels,
            leftmost_leaf,
            key_roots,
            n,
        }
    }

    fn fill_postorder(
        node: &LabeledTree,
        labels: &mut [String],
        leftmost_leaf: &mut [usize],
        parent: &mut [usize],
        idx: &mut usize,
    ) -> usize {
        let mut child_indices = Vec::new();
        let mut my_leftmost = 0;

        for child in &node.children {
            let child_idx = Self::fill_postorder(child, labels, leftmost_leaf, parent, idx);
            child_indices.push(child_idx);
            if my_leftmost == 0 {
                my_leftmost = leftmost_leaf[child_idx];
            }
        }

        let my_idx = *idx;
        *idx += 1;

        labels[my_idx] = node.label.clone();

        if node.children.is_empty() {
            leftmost_leaf[my_idx] = my_idx;
        } else {
            leftmost_leaf[my_idx] = my_leftmost;
        }

        // Set parent for all children
        for ci in child_indices {
            parent[ci] = my_idx;
        }
        // Root has parent = 0 (default)

        my_idx
    }
}

/// Compute tree edit distance between two labeled trees using the Zhang-Shasha algorithm.
///
/// Operations and their costs:
/// - Delete a node: cost 1
/// - Insert a node: cost 1
/// - Rename a node: cost 0 if labels match, cost 1 otherwise
pub fn tree_edit_distance(a: &LabeledTree, b: &LabeledTree) -> usize {
    let ta = IndexedTree::from_tree(a);
    let tb = IndexedTree::from_tree(b);

    if ta.n == 0 && tb.n == 0 {
        return 0;
    }
    if ta.n == 0 {
        return tb.n;
    }
    if tb.n == 0 {
        return ta.n;
    }

    // td[i][j] = tree distance between subtree rooted at postorder node i in a
    //            and subtree rooted at postorder node j in b
    let mut td = vec![vec![0usize; tb.n + 1]; ta.n + 1];

    for &x in &ta.key_roots {
        for &y in &tb.key_roots {
            // Forest distance computation for key root pair (x, y)
            let lx = ta.leftmost_leaf[x]; // leftmost leaf of x
            let ly = tb.leftmost_leaf[y]; // leftmost leaf of y

            // fd[i - lx + 1][j - ly + 1] = forest distance
            // We offset by lx and ly so indices start at 0
            let rows = x - lx + 2; // +1 for the 0-row, +1 because range is inclusive
            let cols = y - ly + 2;
            let mut fd = vec![vec![0usize; cols]; rows];

            // Base cases
            fd[0][0] = 0;
            for i in 1..rows {
                fd[i][0] = fd[i - 1][0] + 1;
            }
            for j in 1..cols {
                fd[0][j] = fd[0][j - 1] + 1;
            }

            for i in 1..rows {
                for j in 1..cols {
                    let node_a = lx + i - 1; // postorder index in a (1-based)
                    let node_b = ly + j - 1; // postorder index in b (1-based)

                    let cost = if ta.labels[node_a] == tb.labels[node_b] {
                        0
                    } else {
                        1
                    };

                    if ta.leftmost_leaf[node_a] == lx && tb.leftmost_leaf[node_b] == ly {
                        // Both nodes have their leftmost leaf equal to the leftmost leaf
                        // of the current key root pair. This means we're comparing whole subtrees.
                        fd[i][j] = cmp::min(
                            cmp::min(fd[i - 1][j] + 1, fd[i][j - 1] + 1),
                            fd[i - 1][j - 1] + cost,
                        );
                        td[node_a][node_b] = fd[i][j];
                    } else {
                        // Use previously computed tree distance for the subtrees
                        let p = ta.leftmost_leaf[node_a] - lx; // offset into fd
                        let q = tb.leftmost_leaf[node_b] - ly;
                        fd[i][j] = cmp::min(
                            cmp::min(fd[i - 1][j] + 1, fd[i][j - 1] + 1),
                            fd[p][q] + td[node_a][node_b],
                        );
                    }
                }
            }
        }
    }

    td[ta.n][tb.n]
}

/// Compute normalized tree edit distance between two labeled trees.
///
/// Returns a value in [0.0, 1.0] where 0.0 means identical trees and 1.0 means
/// completely different. Normalized by the size of the larger tree.
pub fn normalized_tree_edit_distance(a: &LabeledTree, b: &LabeledTree) -> f64 {
    let a_size = a.size();
    let b_size = b.size();
    let max_size = cmp::max(a_size, b_size);

    if max_size == 0 {
        return 0.0;
    }

    let dist = tree_edit_distance(a, b);
    dist as f64 / max_size as f64
}

/// Convert source code to a simplified labeled tree using indentation/brace heuristics.
///
/// This creates a tree structure where:
/// - The root node represents the function
/// - Control flow keywords (if, else, for, while, etc.) create branches
/// - Other statements become leaf nodes labeled by their primary keyword/action
pub fn source_to_labeled_tree(source: &str) -> LabeledTree {
    let lines: Vec<&str> = source.lines().collect();
    if lines.is_empty() {
        return LabeledTree::new("empty");
    }

    // Determine if brace-delimited or indentation-based
    let has_braces = lines.iter().any(|l| l.contains('{') || l.contains('}'));

    if has_braces {
        brace_based_tree(&lines)
    } else {
        indent_based_tree(&lines)
    }
}

/// Build a labeled tree from brace-delimited source code.
fn brace_based_tree(lines: &[&str]) -> LabeledTree {
    let mut root = LabeledTree::new("function");
    let mut stack: Vec<LabeledTree> = Vec::new();
    stack.push(LabeledTree::new("function"));

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("/*") {
            continue;
        }

        let label = classify_line(trimmed);

        // Count braces on this line
        let open_braces = trimmed.chars().filter(|&c| c == '{').count();
        let close_braces = trimmed.chars().filter(|&c| c == '}').count();

        if label != "brace" {
            if let Some(current) = stack.last_mut() {
                current.children.push(LabeledTree::new(label));
            }
        }

        // Push for open braces (new scope)
        for _ in 0..open_braces {
            let scope_label = classify_line(trimmed);
            stack.push(LabeledTree::new(scope_label));
        }

        // Pop for close braces (end scope)
        for _ in 0..close_braces {
            if stack.len() > 1 {
                let completed = stack.pop().unwrap();
                if let Some(parent_node) = stack.last_mut() {
                    if !completed.children.is_empty() {
                        parent_node.children.push(completed);
                    }
                }
            }
        }
    }

    // Collapse remaining stack
    while stack.len() > 1 {
        let completed = stack.pop().unwrap();
        if let Some(parent_node) = stack.last_mut() {
            if !completed.children.is_empty() {
                parent_node.children.push(completed);
            }
        }
    }

    root = stack.pop().unwrap_or(root);
    root
}

/// Build a labeled tree from indentation-based source code (e.g., Python).
fn indent_based_tree(lines: &[&str]) -> LabeledTree {
    let mut root = LabeledTree::new("function");
    let mut stack: Vec<(usize, LabeledTree)> = Vec::new(); // (indent_level, node)
    stack.push((0, LabeledTree::new("function")));

    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();
        let label = classify_line(trimmed);

        // Pop nodes that are at the same or deeper indentation
        while stack.len() > 1 && stack.last().is_some_and(|(lvl, _)| *lvl >= indent) {
            let (_, completed) = stack.pop().unwrap();
            if let Some((_, parent_node)) = stack.last_mut() {
                parent_node.children.push(completed);
            }
        }

        // If this line starts a new block (ends with ':'), push a new scope
        if trimmed.ends_with(':') {
            stack.push((indent, LabeledTree::new(label)));
        } else if let Some((_, current)) = stack.last_mut() {
            current.children.push(LabeledTree::new(label));
        }
    }

    // Collapse remaining stack
    while stack.len() > 1 {
        let (_, completed) = stack.pop().unwrap();
        if let Some((_, parent_node)) = stack.last_mut() {
            parent_node.children.push(completed);
        }
    }

    root = stack.pop().map(|(_, n)| n).unwrap_or(root);
    root
}

/// Classify a source code line into a semantic label.
fn classify_line(trimmed: &str) -> &'static str {
    // Control flow
    if trimmed.starts_with("if ") || trimmed.starts_with("if(") || trimmed == "if" {
        return "if";
    }
    if trimmed.starts_with("else if")
        || trimmed.starts_with("elif")
        || trimmed.starts_with("} else if")
    {
        return "elif";
    }
    if trimmed.starts_with("else") || trimmed.starts_with("} else") {
        return "else";
    }
    if trimmed.starts_with("for ") || trimmed.starts_with("for(") {
        return "for";
    }
    if trimmed.starts_with("while ") || trimmed.starts_with("while(") {
        return "while";
    }
    if trimmed.starts_with("loop") {
        return "loop";
    }
    if trimmed.starts_with("match ") || trimmed.starts_with("switch") {
        return "switch";
    }
    if trimmed.starts_with("case ") || trimmed.starts_with("default:") {
        return "case";
    }

    // Function-related
    if trimmed.starts_with("return") {
        return "return";
    }
    if trimmed.starts_with("def ")
        || trimmed.starts_with("fn ")
        || trimmed.starts_with("func ")
        || trimmed.starts_with("function ")
        || trimmed.contains("function ")
    {
        return "funcdef";
    }

    // Error handling
    if trimmed.starts_with("try") {
        return "try";
    }
    if trimmed.starts_with("catch") || trimmed.starts_with("except") {
        return "catch";
    }
    if trimmed.starts_with("finally") {
        return "finally";
    }
    if trimmed.starts_with("throw") || trimmed.starts_with("raise") {
        return "throw";
    }

    // Loop control
    if trimmed.starts_with("break") {
        return "break";
    }
    if trimmed.starts_with("continue") {
        return "continue";
    }

    // Declarations / assignments
    if trimmed.starts_with("let ")
        || trimmed.starts_with("var ")
        || trimmed.starts_with("const ")
        || trimmed.starts_with("mut ")
    {
        return "declare";
    }

    // Only closing brace
    if trimmed == "}" || trimmed == "};" {
        return "brace";
    }

    // Assignment (contains = but not == or !=)
    if trimmed.contains(" = ") && !trimmed.contains("==") && !trimmed.contains("!=") {
        return "assign";
    }

    // Function calls
    if trimmed.contains('(') && !trimmed.starts_with("//") {
        return "call";
    }

    "stmt"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_labeled_tree_size() {
        let tree = LabeledTree::with_children(
            "root",
            vec![
                LabeledTree::new("a"),
                LabeledTree::with_children("b", vec![LabeledTree::new("c")]),
            ],
        );
        assert_eq!(tree.size(), 4);
    }

    #[test]
    fn test_identical_trees_distance_zero() {
        let a = LabeledTree::with_children(
            "if",
            vec![LabeledTree::new("assign"), LabeledTree::new("return")],
        );
        let b = a.clone();
        assert_eq!(tree_edit_distance(&a, &b), 0);
        assert!((normalized_tree_edit_distance(&a, &b) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_single_node_rename() {
        let a = LabeledTree::new("if");
        let b = LabeledTree::new("while");
        assert_eq!(tree_edit_distance(&a, &b), 1);
        assert!((normalized_tree_edit_distance(&a, &b) - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_insert_and_delete() {
        let a = LabeledTree::with_children("root", vec![LabeledTree::new("a")]);
        let b =
            LabeledTree::with_children("root", vec![LabeledTree::new("a"), LabeledTree::new("b")]);
        // Need to insert one node "b"
        assert_eq!(tree_edit_distance(&a, &b), 1);
    }

    #[test]
    fn test_empty_to_tree() {
        // A leaf node vs a tree with children
        let a = LabeledTree::new("root");
        let b =
            LabeledTree::with_children("root", vec![LabeledTree::new("a"), LabeledTree::new("b")]);
        // root->root (0 cost rename), then insert a and b = 2
        assert_eq!(tree_edit_distance(&a, &b), 2);
    }

    #[test]
    fn test_symmetric_distance() {
        let a = LabeledTree::with_children(
            "root",
            vec![
                LabeledTree::new("a"),
                LabeledTree::with_children("b", vec![LabeledTree::new("c")]),
            ],
        );
        let b = LabeledTree::with_children(
            "root",
            vec![
                LabeledTree::new("x"),
                LabeledTree::with_children("b", vec![LabeledTree::new("y")]),
            ],
        );
        assert_eq!(tree_edit_distance(&a, &b), tree_edit_distance(&b, &a));
    }

    #[test]
    fn test_normalized_range() {
        let a = LabeledTree::with_children(
            "func",
            vec![
                LabeledTree::new("assign"),
                LabeledTree::new("call"),
                LabeledTree::new("return"),
            ],
        );
        let b = LabeledTree::with_children(
            "func",
            vec![
                LabeledTree::with_children("for", vec![LabeledTree::new("call")]),
                LabeledTree::new("return"),
            ],
        );
        let dist = normalized_tree_edit_distance(&a, &b);
        assert!(
            (0.0..=1.0).contains(&dist),
            "Normalized distance should be in [0, 1], got {dist}"
        );
    }

    #[test]
    fn test_source_to_labeled_tree_python() {
        let source =
            "def fib(n):\n    if n <= 1:\n        return n\n    return fib(n-1) + fib(n-2)\n";
        let tree = source_to_labeled_tree(source);
        assert!(tree.size() >= 3, "Tree should have at least a few nodes");
    }

    #[test]
    fn test_source_to_labeled_tree_javascript() {
        let source = r#"function fib(n) {
    if (n <= 1) {
        return n;
    }
    return fib(n - 1) + fib(n - 2);
}"#;
        let tree = source_to_labeled_tree(source);
        assert!(tree.size() >= 3, "Tree should have at least a few nodes");
    }

    #[test]
    fn test_classify_line() {
        assert_eq!(classify_line("if (x > 0) {"), "if");
        assert_eq!(classify_line("else {"), "else");
        assert_eq!(classify_line("for i in range(10):"), "for");
        assert_eq!(classify_line("while (true) {"), "while");
        assert_eq!(classify_line("return result;"), "return");
        assert_eq!(classify_line("let x = 5;"), "declare");
        assert_eq!(classify_line("x = 5"), "assign");
        assert_eq!(classify_line("print(x)"), "call");
    }

    #[test]
    fn test_deeper_tree_distance() {
        // A deeper tree to exercise the algorithm more
        let a = LabeledTree::with_children(
            "func",
            vec![
                LabeledTree::with_children("if", vec![LabeledTree::new("return")]),
                LabeledTree::with_children(
                    "for",
                    vec![LabeledTree::new("assign"), LabeledTree::new("call")],
                ),
                LabeledTree::new("return"),
            ],
        );
        let b = LabeledTree::with_children(
            "func",
            vec![
                LabeledTree::with_children("if", vec![LabeledTree::new("return")]),
                LabeledTree::with_children(
                    "while",
                    vec![LabeledTree::new("assign"), LabeledTree::new("call")],
                ),
                LabeledTree::new("return"),
            ],
        );
        // Only difference is "for" vs "while" => distance should be 1
        let dist = tree_edit_distance(&a, &b);
        assert_eq!(
            dist, 1,
            "Should only differ in for vs while, got distance {dist}"
        );
    }
}
