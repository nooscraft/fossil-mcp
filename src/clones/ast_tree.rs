//! AST-based tree construction using tree-sitter.
//!
//! Converts tree-sitter AST nodes to [`LabeledTree`] instances for use with
//! tree edit distance algorithms (both Zhang-Shasha and APTED). Provides proper
//! structural representation based on the actual AST, unlike the heuristic
//! line-based approach in `crate::tree_edit_distance::source_to_labeled_tree`.
//!
//! The conversion is language-agnostic: tree-sitter grammars use consistent
//! naming conventions across languages, so the same label mapping works for
//! Python, JavaScript, Rust, Go, etc.
//!
//! # Usage
//!
//! When a `tree_sitter::Node` is available (e.g., from cross-language detection
//! or direct parsing), use [`ast_to_labeled_tree`] for accurate structural
//! representation. When only source text and a language are available, use
//! [`parse_to_labeled_tree`] as a convenience wrapper.

use super::tree_edit_distance::LabeledTree;

/// Convert a tree-sitter AST node to a [`LabeledTree`].
///
/// Traverses the tree-sitter AST, mapping node kinds to abstract labels
/// and filtering out trivial nodes (punctuation, braces, semicolons, comments).
///
/// The resulting tree has meaningful structural depth that reflects the actual
/// AST nesting, which is critical for accurate tree edit distance computation.
///
/// # Arguments
/// * `node` - A tree-sitter AST node to convert
/// * `source` - The original source text (used for leaf-node text extraction)
///
/// # Returns
/// A [`LabeledTree`] with abstract labels suitable for tree edit distance.
pub fn ast_to_labeled_tree(node: tree_sitter::Node<'_>, source: &str) -> LabeledTree {
    let label = node_to_label(node, source);
    let mut children = Vec::new();

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if !is_trivial_node(child) {
                children.push(ast_to_labeled_tree(child, source));
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    LabeledTree::with_children(label, children)
}

/// Map a tree-sitter node to an abstract label.
///
/// Uses the node's kind (grammar type) as the primary label, with normalization
/// to provide consistent labels across different languages. For identifiers and
/// literals, uses abstract placeholders (`$ID`, `$NUM`, `$STR`, etc.) to enable
/// Type-2 (renamed) clone detection at the structural level.
///
/// # Label categories
///
/// - **Control flow**: `if`, `else`, `elif`, `for`, `while`, `do_while`, `switch`, `case`
/// - **Definitions**: `funcdef`, `lambda`, `classdef`
/// - **Statements**: `return`, `expr_stmt`, `assign`, `declare`
/// - **Expressions**: `call`, `binop`, `unop`, `compare`, `member`, `subscript`
/// - **Identifiers**: `$ID` (normalized)
/// - **Literals**: `$NUM`, `$STR`, `$BOOL`, `$NULL` (normalized)
/// - **Structural**: `params`, `param`, `args`, `arg`, `block`
/// - **Other**: uses the raw tree-sitter node kind
fn node_to_label(node: tree_sitter::Node<'_>, _source: &str) -> String {
    let kind = node.kind();

    match kind {
        // Control flow - normalize across languages
        "if_statement" | "if_expression" => "if".to_string(),
        "else_clause" | "else" => "else".to_string(),
        "elif_clause" => "elif".to_string(),
        "for_statement" | "for_in_statement" | "for_expression" => "for".to_string(),
        "while_statement" | "while_expression" => "while".to_string(),
        "do_statement" => "do_while".to_string(),
        "switch_statement" | "match_expression" => "switch".to_string(),
        "case_clause" | "match_arm" => "case".to_string(),
        "try_statement" => "try".to_string(),
        "catch_clause" | "except_clause" => "catch".to_string(),
        "finally_clause" => "finally".to_string(),

        // Function definitions
        "function_definition"
        | "function_declaration"
        | "method_definition"
        | "function_item"
        | "method_declaration" => "funcdef".to_string(),
        "arrow_function" | "lambda" | "lambda_expression" => "lambda".to_string(),

        // Class definitions
        "class_definition" | "class_declaration" => "classdef".to_string(),

        // Statements
        "return_statement" => "return".to_string(),
        "expression_statement" => "expr_stmt".to_string(),
        "assignment" | "assignment_expression" => "assign".to_string(),
        "augmented_assignment" => "aug_assign".to_string(),
        "variable_declaration" | "lexical_declaration" | "let_declaration" => "declare".to_string(),

        // Expressions
        "call_expression" | "call" => "call".to_string(),
        "binary_expression" | "binary_operator" => "binop".to_string(),
        "unary_expression" | "unary_operator" => "unop".to_string(),
        "comparison_operator" => "compare".to_string(),
        "boolean_operator" => "boolop".to_string(),
        "subscript" | "subscript_expression" => "subscript".to_string(),
        "attribute" | "member_expression" | "field_expression" => "member".to_string(),

        // Identifiers - normalize to $ID for rename-insensitive comparison
        "identifier" | "property_identifier" | "field_identifier" | "type_identifier" => {
            "$ID".to_string()
        }

        // Literals - normalize by type
        "integer" | "integer_literal" | "number" | "float" | "float_literal" => "$NUM".to_string(),
        "string" | "string_literal" | "template_string" | "string_content" => "$STR".to_string(),
        "true" | "false" | "boolean" => "$BOOL".to_string(),
        "none" | "null" | "nil" => "$NULL".to_string(),

        // Parameters
        "parameters" | "formal_parameters" | "parameter_list" => "params".to_string(),
        "parameter" | "simple_parameter" | "typed_parameter" | "typed_default_parameter" => {
            "param".to_string()
        }

        // Arguments
        "argument_list" | "arguments" => "args".to_string(),
        "argument" | "keyword_argument" => "arg".to_string(),

        // Block / body
        "block" | "statement_block" | "compound_statement" => "block".to_string(),

        // Import/module
        "import_statement" | "import_declaration" => "import".to_string(),

        // Operators (when they appear as named nodes)
        "=" | "+=" | "-=" | "*=" | "/=" => "op_assign".to_string(),
        "+" | "-" | "*" | "/" | "%" | "**" => "op_arith".to_string(),
        "==" | "!=" | "<" | ">" | "<=" | ">=" => "op_cmp".to_string(),
        "&&" | "||" | "and" | "or" | "not" | "!" => "op_logic".to_string(),

        // Default: use the node kind directly
        _ => kind.to_string(),
    }
}

/// Check if a tree-sitter node is trivial and should be filtered out.
///
/// Trivial nodes are punctuation, delimiters, comments, and whitespace-related
/// nodes that add noise without structural meaning.
fn is_trivial_node(node: tree_sitter::Node<'_>) -> bool {
    let kind = node.kind();
    matches!(
        kind,
        "(" | ")"
            | "{"
            | "}"
            | "["
            | "]"
            | ";"
            | ","
            | ":"
            | "."
            | "->"
            | "=>"
            | "::"
            | "comment"
            | "line_comment"
            | "block_comment"
            | "newline"
            | "indent"
            | "dedent"
            | "NEWLINE"
            | "INDENT"
            | "DEDENT"
            | "\n"
    )
}

/// Build a [`LabeledTree`] from source code using tree-sitter parsing.
///
/// This is a convenience function that parses source with the given tree-sitter
/// language and converts the resulting AST to a [`LabeledTree`].
///
/// # Arguments
/// * `source` - Source code text to parse
/// * `ts_language` - The tree-sitter [`Language`](tree_sitter::Language) grammar to use
///
/// # Returns
/// `Some(LabeledTree)` on successful parse, `None` if parsing fails.
///
/// # Example
/// ```ignore
/// let lang: tree_sitter::Language = tree_sitter_python::LANGUAGE.into();
/// let tree = parse_to_labeled_tree("def foo(): pass", lang);
/// assert!(tree.is_some());
/// ```
pub fn parse_to_labeled_tree(
    source: &str,
    ts_language: tree_sitter::Language,
) -> Option<LabeledTree> {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&ts_language).ok()?;
    let tree = parser.parse(source, None)?;
    Some(ast_to_labeled_tree(tree.root_node(), source))
}

/// Build a [`LabeledTree`] from a specific byte range within already-parsed source.
///
/// Useful when you have a tree-sitter node representing a function or block
/// and want to convert just that subtree.
///
/// # Arguments
/// * `node` - The tree-sitter node to convert (e.g., a function definition)
/// * `source` - The full source text
/// * `max_depth` - Maximum tree depth to convert (0 = unlimited)
///
/// # Returns
/// A [`LabeledTree`] with depth limited to `max_depth` if specified.
pub fn ast_to_labeled_tree_bounded(
    node: tree_sitter::Node<'_>,
    source: &str,
    max_depth: usize,
) -> LabeledTree {
    ast_to_labeled_tree_inner(node, source, max_depth, 0)
}

/// Internal recursive builder with depth tracking.
fn ast_to_labeled_tree_inner(
    node: tree_sitter::Node<'_>,
    source: &str,
    max_depth: usize,
    current_depth: usize,
) -> LabeledTree {
    let label = node_to_label(node, source);

    // If we've reached max depth (and it's not unlimited), return a leaf
    if max_depth > 0 && current_depth >= max_depth {
        return LabeledTree::new(label);
    }

    let mut children = Vec::new();
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child = cursor.node();
            if !is_trivial_node(child) {
                children.push(ast_to_labeled_tree_inner(
                    child,
                    source,
                    max_depth,
                    current_depth + 1,
                ));
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    LabeledTree::with_children(label, children)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: parse source with a given tree-sitter language and return the labeled tree.
    fn parse_tree(source: &str, ts_language: tree_sitter::Language) -> LabeledTree {
        parse_to_labeled_tree(source, ts_language).expect("Parsing should succeed")
    }

    /// Helper: get the Python tree-sitter language.
    fn python_lang() -> tree_sitter::Language {
        tree_sitter_python::LANGUAGE.into()
    }

    /// Helper: get the JavaScript tree-sitter language.
    fn js_lang() -> tree_sitter::Language {
        tree_sitter_javascript::LANGUAGE.into()
    }

    // --- Basic conversion tests ---

    #[test]
    fn test_ast_tree_has_proper_nesting() {
        let source = "def foo(x):\n    if x > 0:\n        return x\n    return 0\n";
        let tree = parse_tree(source, python_lang());

        // The tree should have depth > 2 (root -> funcdef -> if -> return)
        assert!(
            tree.size() > 4,
            "AST tree should have multiple nested nodes, got size {}",
            tree.size()
        );

        // Should not be flat (all children of root)
        fn max_depth(t: &LabeledTree) -> usize {
            if t.children.is_empty() {
                1
            } else {
                1 + t.children.iter().map(max_depth).max().unwrap_or(0)
            }
        }
        let depth = max_depth(&tree);
        assert!(
            depth >= 3,
            "AST tree should have depth >= 3 for nested code, got {depth}"
        );
    }

    #[test]
    fn test_filtering_removes_punctuation() {
        let source = "x = [1, 2, 3]";
        let tree = parse_tree(source, python_lang());

        // Check that no node in the tree has a punctuation label
        fn has_punctuation(t: &LabeledTree) -> bool {
            let punct = ["(", ")", "{", "}", "[", "]", ";", ",", ":", "."];
            if punct.contains(&t.label.as_str()) {
                return true;
            }
            t.children.iter().any(has_punctuation)
        }
        assert!(
            !has_punctuation(&tree),
            "AST tree should not contain punctuation nodes"
        );
    }

    #[test]
    fn test_filtering_removes_comments() {
        let source = "# This is a comment\nx = 1\n# Another comment\ny = 2\n";
        let tree = parse_tree(source, python_lang());

        fn has_comment(t: &LabeledTree) -> bool {
            if t.label == "comment" || t.label == "line_comment" || t.label == "block_comment" {
                return true;
            }
            t.children.iter().any(has_comment)
        }
        assert!(
            !has_comment(&tree),
            "AST tree should not contain comment nodes"
        );
    }

    #[test]
    fn test_labels_are_abstract() {
        let source = "def foo(x):\n    return x + 1\n";
        let tree = parse_tree(source, python_lang());

        // Collect all labels
        fn collect_labels(t: &LabeledTree, labels: &mut Vec<String>) {
            labels.push(t.label.clone());
            for child in &t.children {
                collect_labels(child, labels);
            }
        }
        let mut labels = Vec::new();
        collect_labels(&tree, &mut labels);

        // Should contain abstract labels, not raw text like "foo" or "x"
        assert!(
            labels
                .iter()
                .any(|l| l == "funcdef" || l == "$ID" || l == "return" || l == "params"),
            "Should contain abstract labels like funcdef, $ID, return. Got: {labels:?}"
        );
    }

    #[test]
    fn test_identifiers_normalized() {
        let source = "x = foo(bar)";
        let tree = parse_tree(source, python_lang());

        fn has_id(t: &LabeledTree) -> bool {
            if t.label == "$ID" {
                return true;
            }
            t.children.iter().any(has_id)
        }
        assert!(has_id(&tree), "Identifiers should be normalized to $ID");
    }

    #[test]
    fn test_literals_normalized() {
        let source = "x = 42\ny = \"hello\"\nz = True\n";
        let tree = parse_tree(source, python_lang());

        fn collect_labels(t: &LabeledTree, labels: &mut Vec<String>) {
            labels.push(t.label.clone());
            for child in &t.children {
                collect_labels(child, labels);
            }
        }
        let mut labels = Vec::new();
        collect_labels(&tree, &mut labels);

        assert!(
            labels.contains(&"$NUM".to_string()),
            "Numeric literals should be normalized to $NUM. Labels: {labels:?}"
        );
        assert!(
            labels.contains(&"$STR".to_string()),
            "String literals should be normalized to $STR. Labels: {labels:?}"
        );
    }

    // --- Cross-language consistency tests ---

    #[test]
    fn test_python_and_js_similar_labels() {
        let python_source = "def add(a, b):\n    return a + b\n";
        let js_source = "function add(a, b) { return a + b; }";

        let py_tree = parse_tree(python_source, python_lang());
        let js_tree = parse_tree(js_source, js_lang());

        fn collect_labels(t: &LabeledTree) -> Vec<String> {
            let mut labels = vec![t.label.clone()];
            for child in &t.children {
                labels.extend(collect_labels(child));
            }
            labels
        }

        let py_labels = collect_labels(&py_tree);
        let js_labels = collect_labels(&js_tree);

        // Both should contain funcdef, params, return, $ID, binop
        let common_labels = ["funcdef", "return", "$ID", "params"];
        for expected in &common_labels {
            let expected_str = expected.to_string();
            assert!(
                py_labels.contains(&expected_str),
                "Python tree should contain '{expected}'. Labels: {py_labels:?}"
            );
            assert!(
                js_labels.contains(&expected_str),
                "JavaScript tree should contain '{expected}'. Labels: {js_labels:?}"
            );
        }
    }

    // --- Empty/minimal source tests ---

    #[test]
    fn test_empty_source_produces_minimal_tree() {
        let source = "";
        let tree = parse_tree(source, python_lang());
        // Empty source should produce a tree with just the root module node
        assert!(
            tree.size() >= 1,
            "Empty source should produce at least a root node"
        );
    }

    #[test]
    fn test_single_statement() {
        let source = "pass";
        let tree = parse_tree(source, python_lang());
        assert!(tree.size() >= 1);
    }

    // --- Bounded depth tests ---

    #[test]
    fn test_bounded_depth_limits_tree() {
        let source =
            "def foo(x):\n    if x > 0:\n        for i in range(x):\n            print(i)\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&python_lang())
            .expect("Language should set");
        let parsed = parser.parse(source, None).expect("Parse should succeed");

        let full_tree = ast_to_labeled_tree(parsed.root_node(), source);
        let bounded_tree = ast_to_labeled_tree_bounded(parsed.root_node(), source, 3);

        fn max_depth(t: &LabeledTree) -> usize {
            if t.children.is_empty() {
                1
            } else {
                1 + t.children.iter().map(max_depth).max().unwrap_or(0)
            }
        }

        let full_depth = max_depth(&full_tree);
        let bounded_depth = max_depth(&bounded_tree);

        assert!(
            bounded_depth <= full_depth,
            "Bounded tree (depth {bounded_depth}) should not be deeper than full tree (depth {full_depth})"
        );
        // max_depth=3 means we recurse 3 levels from root (depth 0, 1, 2),
        // and at depth 3 we create leaves. The max_depth counting function
        // returns 1 for a leaf, so the result is at most max_depth + 1.
        assert!(
            bounded_depth <= 4,
            "Bounded tree depth should be <= max_depth+1 (4), got {bounded_depth}"
        );
        assert!(
            bounded_depth < full_depth,
            "Bounded tree (depth {bounded_depth}) should be shallower than full tree (depth {full_depth})"
        );
    }

    #[test]
    fn test_bounded_depth_zero_means_unlimited() {
        let source = "def foo(x):\n    return x + 1\n";
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&python_lang())
            .expect("Language should set");
        let parsed = parser.parse(source, None).expect("Parse should succeed");

        let full_tree = ast_to_labeled_tree(parsed.root_node(), source);
        let unbounded_tree = ast_to_labeled_tree_bounded(parsed.root_node(), source, 0);

        assert_eq!(
            full_tree.size(),
            unbounded_tree.size(),
            "Depth 0 (unlimited) should produce the same tree"
        );
    }

    // --- Integration with tree edit distance ---

    #[test]
    fn test_ast_trees_usable_with_ted() {
        use crate::clones::tree_edit_distance::tree_edit_distance;

        let source_a = "def add(a, b):\n    return a + b\n";
        let source_b = "def sub(a, b):\n    return a - b\n";

        let tree_a = parse_tree(source_a, python_lang());
        let tree_b = parse_tree(source_b, python_lang());

        // Should produce a valid distance
        let dist = tree_edit_distance(&tree_a, &tree_b);
        // The only structural difference is the operator (+ vs -)
        // The function names differ but are both $ID
        assert!(
            dist < tree_a.size().max(tree_b.size()),
            "Similar functions should have distance less than max size"
        );
    }

    #[test]
    fn test_ast_trees_usable_with_apted() {
        use crate::clones::apted::apted_distance;

        let source_a = "def add(a, b):\n    return a + b\n";
        let source_b = "def sub(a, b):\n    return a - b\n";

        let tree_a = parse_tree(source_a, python_lang());
        let tree_b = parse_tree(source_b, python_lang());

        let dist = apted_distance(&tree_a, &tree_b);
        assert!(
            dist < tree_a.size().max(tree_b.size()),
            "Similar functions should have small APTED distance"
        );
    }

    // --- Label mapping coverage ---

    #[test]
    fn test_trivial_node_detection() {
        // Test the is_trivial_node function indirectly: parse code with lots of
        // punctuation and verify none of it appears in the tree
        let source = "result = foo(a, b, c)";
        let tree = parse_tree(source, python_lang());

        fn find_label(t: &LabeledTree, target: &str) -> bool {
            if t.label == target {
                return true;
            }
            t.children.iter().any(|c| find_label(c, target))
        }

        assert!(!find_label(&tree, "("), "Should not contain '('");
        assert!(!find_label(&tree, ")"), "Should not contain ')'");
        assert!(!find_label(&tree, ","), "Should not contain ','");
    }

    #[test]
    fn test_control_flow_labels() {
        let source = r#"
def foo(x):
    if x > 0:
        return 1
    else:
        return 0
    for i in range(x):
        pass
    while True:
        break
"#;
        let tree = parse_tree(source, python_lang());

        fn collect_labels(t: &LabeledTree) -> Vec<String> {
            let mut labels = vec![t.label.clone()];
            for child in &t.children {
                labels.extend(collect_labels(child));
            }
            labels
        }
        let labels = collect_labels(&tree);

        assert!(labels.contains(&"if".to_string()), "Should contain 'if'");
        assert!(
            labels.contains(&"else".to_string()),
            "Should contain 'else'"
        );
        assert!(labels.contains(&"for".to_string()), "Should contain 'for'");
        assert!(
            labels.contains(&"while".to_string()),
            "Should contain 'while'"
        );
        assert!(
            labels.contains(&"funcdef".to_string()),
            "Should contain 'funcdef'"
        );
        assert!(
            labels.contains(&"return".to_string()),
            "Should contain 'return'"
        );
    }

    #[test]
    fn test_js_function_labels() {
        let source = r#"
function greet(name) {
    if (name === "world") {
        return "Hello, World!";
    }
    return "Hello, " + name;
}
"#;
        let tree = parse_tree(source, js_lang());

        fn collect_labels(t: &LabeledTree) -> Vec<String> {
            let mut labels = vec![t.label.clone()];
            for child in &t.children {
                labels.extend(collect_labels(child));
            }
            labels
        }
        let labels = collect_labels(&tree);

        assert!(
            labels.contains(&"funcdef".to_string()),
            "JS function should have 'funcdef' label. Labels: {labels:?}"
        );
        assert!(
            labels.contains(&"if".to_string()),
            "JS if-statement should have 'if' label. Labels: {labels:?}"
        );
        assert!(
            labels.contains(&"return".to_string()),
            "JS return should have 'return' label. Labels: {labels:?}"
        );
    }
}
