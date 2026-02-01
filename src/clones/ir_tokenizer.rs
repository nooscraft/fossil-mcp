//! Language-agnostic IR token extraction for cross-language clone detection.
//!
//! Converts tree-sitter AST nodes into a language-independent intermediate
//! representation suitable for MinHash-based similarity comparison.

use xxhash_rust::xxh3::xxh3_64;

/// Language-agnostic IR token types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IRToken {
    FuncDef,
    IfBranch,
    ElseBranch,
    LoopStart,
    LoopEnd,
    Return,
    Assign,
    AugAssign,
    Call,
    Param,
    VarRef,
    TypeRef,
    LiteralInt,
    LiteralFloat,
    LiteralString,
    LiteralBool,
    OpAdd,
    OpSub,
    OpMul,
    OpDiv,
    OpMod,
    OpEq,
    OpNeq,
    OpLt,
    OpGt,
    OpLte,
    OpGte,
    OpAnd,
    OpOr,
    OpNot,
    ArrayAccess,
    FieldAccess,
    TryCatch,
    Throw,
    Break,
    Continue,
    Switch,
    Case,
    Lambda,
    Yield,
    Await,
    Import,
    ClassDef,
}

impl IRToken {
    /// Convert to a deterministic u8 discriminant for hashing.
    fn discriminant(self) -> u8 {
        match self {
            IRToken::FuncDef => 1,
            IRToken::IfBranch => 2,
            IRToken::ElseBranch => 3,
            IRToken::LoopStart => 4,
            IRToken::LoopEnd => 5,
            IRToken::Return => 6,
            IRToken::Assign => 7,
            IRToken::AugAssign => 8,
            IRToken::Call => 9,
            IRToken::Param => 10,
            IRToken::VarRef => 11,
            IRToken::TypeRef => 12,
            IRToken::LiteralInt => 13,
            IRToken::LiteralFloat => 14,
            IRToken::LiteralString => 15,
            IRToken::LiteralBool => 16,
            IRToken::OpAdd => 17,
            IRToken::OpSub => 18,
            IRToken::OpMul => 19,
            IRToken::OpDiv => 20,
            IRToken::OpMod => 21,
            IRToken::OpEq => 22,
            IRToken::OpNeq => 23,
            IRToken::OpLt => 24,
            IRToken::OpGt => 25,
            IRToken::OpLte => 26,
            IRToken::OpGte => 27,
            IRToken::OpAnd => 28,
            IRToken::OpOr => 29,
            IRToken::OpNot => 30,
            IRToken::ArrayAccess => 31,
            IRToken::FieldAccess => 32,
            IRToken::TryCatch => 33,
            IRToken::Throw => 34,
            IRToken::Break => 35,
            IRToken::Continue => 36,
            IRToken::Switch => 37,
            IRToken::Case => 38,
            IRToken::Lambda => 39,
            IRToken::Yield => 40,
            IRToken::Await => 41,
            IRToken::Import => 42,
            IRToken::ClassDef => 43,
        }
    }
}

/// Extract language-agnostic IR tokens from a tree-sitter node.
pub fn extract_ir_tokens(node: tree_sitter::Node<'_>, source: &str) -> Vec<IRToken> {
    let mut tokens = Vec::new();
    extract_recursive(node, source, &mut tokens);
    tokens
}

fn extract_recursive(node: tree_sitter::Node<'_>, source: &str, tokens: &mut Vec<IRToken>) {
    let kind = node.kind();

    // Map tree-sitter node kinds to IR tokens
    match kind {
        // Function definitions
        "function_definition"
        | "function_declaration"
        | "method_definition"
        | "method_declaration"
        | "function_item"
        | "func_literal" => {
            tokens.push(IRToken::FuncDef);
        }

        // Conditionals
        "if_statement" | "if_expression" | "if_let_expression" => {
            tokens.push(IRToken::IfBranch);
        }
        "else_clause" | "else" => {
            tokens.push(IRToken::ElseBranch);
        }

        // Loops
        "while_statement" | "while_expression" | "for_statement" | "for_expression"
        | "for_in_statement" | "loop_expression" | "do_statement" => {
            tokens.push(IRToken::LoopStart);
        }

        // Returns
        "return_statement" | "return_expression" => {
            tokens.push(IRToken::Return);
        }

        // Assignments
        "assignment"
        | "assignment_expression"
        | "assignment_statement"
        | "variable_declarator"
        | "let_declaration"
        | "short_var_declaration"
        | "local_variable_declaration" => {
            tokens.push(IRToken::Assign);
        }
        "augmented_assignment" | "compound_assignment_expr" | "update_expression" => {
            tokens.push(IRToken::AugAssign);
        }

        // Calls
        "call" | "call_expression" | "method_invocation" | "function_call" => {
            tokens.push(IRToken::Call);
        }

        // Parameters
        "parameter" | "formal_parameter" | "required_parameter" | "optional_parameter"
        | "typed_parameter" => {
            tokens.push(IRToken::Param);
        }

        // Identifiers (variable references)
        "identifier" | "property_identifier" | "shorthand_property_identifier" => {
            tokens.push(IRToken::VarRef);
        }

        // Type references
        "type_identifier" | "type_annotation" | "type_reference" | "predefined_type" => {
            tokens.push(IRToken::TypeRef);
        }

        // Literals
        "integer" | "integer_literal" | "decimal_integer_literal" | "number" => {
            tokens.push(IRToken::LiteralInt);
        }
        "float" | "float_literal" | "decimal_floating_point_literal" => {
            tokens.push(IRToken::LiteralFloat);
        }
        "string" | "string_literal" | "template_string" | "raw_string_literal" => {
            tokens.push(IRToken::LiteralString);
        }
        "true" | "false" | "boolean" => {
            tokens.push(IRToken::LiteralBool);
        }

        // Operators
        "binary_expression" | "binary_operator" => {
            if let Some(op) = node.child_by_field_name("operator") {
                let op_text = op.utf8_text(source.as_bytes()).unwrap_or("");
                match op_text {
                    "+" => tokens.push(IRToken::OpAdd),
                    "-" => tokens.push(IRToken::OpSub),
                    "*" => tokens.push(IRToken::OpMul),
                    "/" => tokens.push(IRToken::OpDiv),
                    "%" => tokens.push(IRToken::OpMod),
                    "==" => tokens.push(IRToken::OpEq),
                    "!=" => tokens.push(IRToken::OpNeq),
                    "<" => tokens.push(IRToken::OpLt),
                    ">" => tokens.push(IRToken::OpGt),
                    "<=" => tokens.push(IRToken::OpLte),
                    ">=" => tokens.push(IRToken::OpGte),
                    "&&" | "and" => tokens.push(IRToken::OpAnd),
                    "||" | "or" => tokens.push(IRToken::OpOr),
                    _ => {}
                }
            }
        }
        "unary_expression" | "not_operator" => {
            tokens.push(IRToken::OpNot);
        }

        // Subscript/field access
        "subscript" | "subscript_expression" | "element_access_expression" => {
            tokens.push(IRToken::ArrayAccess);
        }
        "attribute" | "member_expression" | "field_expression" | "field_access" => {
            tokens.push(IRToken::FieldAccess);
        }

        // Exception handling
        "try_statement" | "try_expression" => {
            tokens.push(IRToken::TryCatch);
        }
        "throw_statement" | "throw_expression" | "raise_statement" => {
            tokens.push(IRToken::Throw);
        }

        // Control flow
        "break_statement" | "break_expression" => {
            tokens.push(IRToken::Break);
        }
        "continue_statement" | "continue_expression" => {
            tokens.push(IRToken::Continue);
        }
        "switch_statement" | "match_expression" | "switch_expression" => {
            tokens.push(IRToken::Switch);
        }
        "case_clause" | "match_arm" | "switch_case" => {
            tokens.push(IRToken::Case);
        }

        // Lambda/closures
        "lambda" | "arrow_function" | "closure_expression" => {
            tokens.push(IRToken::Lambda);
        }

        // Async
        "yield_expression" | "yield_statement" => {
            tokens.push(IRToken::Yield);
        }
        "await_expression" => {
            tokens.push(IRToken::Await);
        }

        // Imports
        "import_statement" | "import_declaration" | "use_declaration" => {
            tokens.push(IRToken::Import);
        }

        // Classes
        "class_definition" | "class_declaration" | "struct_item" | "impl_item" => {
            tokens.push(IRToken::ClassDef);
        }

        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_recursive(child, source, tokens);
    }
}

/// Convert IR tokens to shingles (n-gram hashes) for MinHash comparison.
pub fn ir_tokens_to_shingles(tokens: &[IRToken], shingle_size: usize) -> Vec<u64> {
    if tokens.len() < shingle_size {
        return Vec::new();
    }

    tokens
        .windows(shingle_size)
        .map(|window| {
            let bytes: Vec<u8> = window.iter().map(|t| t.discriminant()).collect();
            xxh3_64(&bytes)
        })
        .collect()
}

/// Extract IR tokens from source text using keyword-based heuristics.
///
/// This is a text-based alternative to `extract_ir_tokens()` that does not
/// require a tree-sitter `Node`. It scans lines of source code within the
/// given 1-indexed line range and emits IR tokens based on keyword patterns.
///
/// This is intentionally less precise than tree-sitter-based extraction but
/// works across all languages and does not require parser initialization.
pub fn extract_ir_tokens_from_source(
    source: &str,
    start_line: usize,
    end_line: usize,
) -> Vec<IRToken> {
    let mut tokens = Vec::new();
    let lines: Vec<&str> = source.lines().collect();

    let start_idx = start_line.saturating_sub(1);
    let end_idx = end_line.min(lines.len());

    for line in &lines[start_idx..end_idx] {
        let trimmed = line.trim();

        // Function definitions
        if trimmed.starts_with("def ")
            || trimmed.starts_with("fn ")
            || trimmed.starts_with("pub fn ")
            || trimmed.starts_with("func ")
            || trimmed.starts_with("function ")
            || trimmed.contains("function ")
        {
            tokens.push(IRToken::FuncDef);
        }

        // Conditionals
        if trimmed.starts_with("if ")
            || trimmed.starts_with("if(")
            || trimmed.starts_with("} else if")
            || trimmed.starts_with("elif ")
        {
            tokens.push(IRToken::IfBranch);
        }
        if trimmed.starts_with("else") || trimmed.starts_with("} else") {
            tokens.push(IRToken::ElseBranch);
        }

        // Loops
        if trimmed.starts_with("for ")
            || trimmed.starts_with("for(")
            || trimmed.starts_with("while ")
            || trimmed.starts_with("while(")
            || trimmed.starts_with("loop ")
            || trimmed.starts_with("loop{")
            || trimmed == "loop"
            || trimmed.starts_with("do ")
            || trimmed.starts_with("do{")
        {
            tokens.push(IRToken::LoopStart);
        }

        // Return
        if trimmed.starts_with("return ") || trimmed.starts_with("return;") || trimmed == "return" {
            tokens.push(IRToken::Return);
        }

        // Assignments (heuristic: contains = but not == or !=)
        if (trimmed.contains(" = ") || trimmed.contains(" := "))
            && !trimmed.contains("==")
            && !trimmed.starts_with("if ")
            && !trimmed.starts_with("while ")
        {
            tokens.push(IRToken::Assign);
        }

        // Augmented assignments
        if trimmed.contains(" += ")
            || trimmed.contains(" -= ")
            || trimmed.contains(" *= ")
            || trimmed.contains(" /= ")
        {
            tokens.push(IRToken::AugAssign);
        }

        // Function calls (heuristic: contains identifier followed by parentheses)
        if trimmed.contains('(')
            && !trimmed.starts_with("def ")
            && !trimmed.starts_with("fn ")
            && !trimmed.starts_with("func ")
            && !trimmed.starts_with("function ")
            && !trimmed.starts_with("if ")
            && !trimmed.starts_with("if(")
            && !trimmed.starts_with("for ")
            && !trimmed.starts_with("for(")
            && !trimmed.starts_with("while ")
            && !trimmed.starts_with("while(")
            && !trimmed.starts_with("class ")
        {
            tokens.push(IRToken::Call);
        }

        // Try/catch
        if trimmed.starts_with("try ")
            || trimmed.starts_with("try{")
            || trimmed == "try"
            || trimmed.starts_with("try:")
        {
            tokens.push(IRToken::TryCatch);
        }
        if trimmed.starts_with("catch ")
            || trimmed.starts_with("except ")
            || trimmed.starts_with("rescue ")
        {
            tokens.push(IRToken::TryCatch);
        }

        // Throw/raise
        if trimmed.starts_with("throw ") || trimmed.starts_with("raise ") {
            tokens.push(IRToken::Throw);
        }

        // Break/continue
        if trimmed == "break" || trimmed == "break;" {
            tokens.push(IRToken::Break);
        }
        if trimmed == "continue" || trimmed == "continue;" {
            tokens.push(IRToken::Continue);
        }

        // Switch/match
        if trimmed.starts_with("switch ") || trimmed.starts_with("match ") {
            tokens.push(IRToken::Switch);
        }
        if trimmed.starts_with("case ") {
            tokens.push(IRToken::Case);
        }

        // Class definitions
        if trimmed.starts_with("class ")
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("impl ")
        {
            tokens.push(IRToken::ClassDef);
        }

        // Imports
        if trimmed.starts_with("import ")
            || trimmed.starts_with("from ")
            || trimmed.starts_with("use ")
            || trimmed.starts_with("require(")
        {
            tokens.push(IRToken::Import);
        }

        // Comparisons in the line
        if trimmed.contains(" == ") || trimmed.contains(" === ") {
            tokens.push(IRToken::OpEq);
        }
        if trimmed.contains(" != ") || trimmed.contains(" !== ") {
            tokens.push(IRToken::OpNeq);
        }
        if trimmed.contains(" && ") || trimmed.contains(" and ") {
            tokens.push(IRToken::OpAnd);
        }
        if trimmed.contains(" || ") || trimmed.contains(" or ") {
            tokens.push(IRToken::OpOr);
        }
    }

    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ir_token_discriminants_unique() {
        use std::collections::HashSet;
        let all_tokens = vec![
            IRToken::FuncDef,
            IRToken::IfBranch,
            IRToken::ElseBranch,
            IRToken::LoopStart,
            IRToken::LoopEnd,
            IRToken::Return,
            IRToken::Assign,
            IRToken::AugAssign,
            IRToken::Call,
            IRToken::Param,
            IRToken::VarRef,
            IRToken::TypeRef,
        ];
        let discriminants: HashSet<u8> = all_tokens.iter().map(|t| t.discriminant()).collect();
        assert_eq!(discriminants.len(), all_tokens.len());
    }

    #[test]
    fn test_shingle_generation() {
        let tokens = vec![
            IRToken::FuncDef,
            IRToken::Param,
            IRToken::Assign,
            IRToken::Call,
            IRToken::Return,
        ];
        let shingles = ir_tokens_to_shingles(&tokens, 3);
        assert_eq!(shingles.len(), 3); // 5 - 3 + 1 = 3 windows
    }

    #[test]
    fn test_empty_tokens() {
        let shingles = ir_tokens_to_shingles(&[], 3);
        assert!(shingles.is_empty());
    }

    #[test]
    fn test_same_pattern_same_hash() {
        let pattern = vec![IRToken::Assign, IRToken::Call, IRToken::Return];
        let s1 = ir_tokens_to_shingles(&pattern, 3);
        let s2 = ir_tokens_to_shingles(&pattern, 3);
        assert_eq!(s1, s2);
    }

    // ---- Text-based IR extraction tests ----

    #[test]
    fn test_extract_ir_tokens_from_source_python() {
        let source = "def foo(x):\n    y = x + 1\n    if y > 0:\n        return y\n    return 0\n";
        let tokens = extract_ir_tokens_from_source(source, 1, 5);

        assert!(
            tokens.contains(&IRToken::FuncDef),
            "Should find function definition"
        );
        assert!(tokens.contains(&IRToken::Assign), "Should find assignment");
        assert!(tokens.contains(&IRToken::IfBranch), "Should find if branch");
        assert!(tokens.contains(&IRToken::Return), "Should find return");
    }

    #[test]
    fn test_extract_ir_tokens_from_source_javascript() {
        let source = "function bar(x) {\n    for (let i = 0; i < x; i++) {\n        console.log(i);\n    }\n    return x;\n}\n";
        let tokens = extract_ir_tokens_from_source(source, 1, 6);

        assert!(
            tokens.contains(&IRToken::FuncDef),
            "Should find function def"
        );
        assert!(tokens.contains(&IRToken::LoopStart), "Should find loop");
        assert!(tokens.contains(&IRToken::Return), "Should find return");
    }

    #[test]
    fn test_extract_ir_tokens_from_source_partial_range() {
        let source = "import os\n\ndef foo():\n    x = 1\n    return x\n\ndef bar():\n    y = 2\n    return y\n";
        // Only extract from lines 3-5 (the foo function)
        let tokens = extract_ir_tokens_from_source(source, 3, 5);

        assert!(tokens.contains(&IRToken::FuncDef));
        assert!(tokens.contains(&IRToken::Assign));
        assert!(tokens.contains(&IRToken::Return));
        // Should NOT contain import (line 1)
        assert!(!tokens.contains(&IRToken::Import));
    }

    #[test]
    fn test_extract_ir_tokens_from_source_empty_range() {
        let source = "# just a comment\n";
        let tokens = extract_ir_tokens_from_source(source, 1, 1);
        assert!(
            tokens.is_empty(),
            "Comment-only source should produce no IR tokens"
        );
    }
}
