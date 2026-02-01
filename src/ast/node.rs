//! Core AST node definitions.

use super::types::{AstVisibility, EnumVariant, Field, Parameter, Type};
use crate::core::SourceSpan;
use serde::{Deserialize, Serialize};

/// Language-agnostic AST node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ASTNode {
    Function {
        name: String,
        params: Vec<Parameter>,
        return_type: Option<Type>,
        body: Option<Vec<Self>>,
        span: SourceSpan,
        visibility: AstVisibility,
        is_async: bool,
        is_method: bool,
        generic_params: Vec<String>,
        attributes: Vec<String>,
    },
    Struct {
        name: String,
        fields: Vec<Field>,
        span: SourceSpan,
        visibility: AstVisibility,
        generic_params: Vec<String>,
        base_types: Vec<String>,
    },
    Enum {
        name: String,
        variants: Vec<EnumVariant>,
        span: SourceSpan,
        visibility: AstVisibility,
        generic_params: Vec<String>,
    },
    Trait {
        name: String,
        methods: Vec<Self>,
        span: SourceSpan,
        visibility: AstVisibility,
        generic_params: Vec<String>,
    },
    ImplBlock {
        type_name: String,
        trait_name: Option<String>,
        methods: Vec<Self>,
        span: SourceSpan,
        generic_params: Vec<String>,
    },
    Import {
        path: Vec<String>,
        items: Vec<String>,
        alias: Option<String>,
        span: SourceSpan,
    },
    Export {
        items: Vec<String>,
        is_default: bool,
        span: SourceSpan,
    },
    Module {
        name: String,
        items: Vec<Self>,
        span: SourceSpan,
        visibility: AstVisibility,
    },
    Constant {
        name: String,
        type_annotation: Option<Type>,
        value: Option<String>,
        span: SourceSpan,
        visibility: AstVisibility,
    },
    Variable {
        name: String,
        type_annotation: Option<Type>,
        value: Option<String>,
        span: SourceSpan,
        is_mutable: bool,
    },
    TypeAlias {
        name: String,
        target_type: Type,
        span: SourceSpan,
        visibility: AstVisibility,
        generic_params: Vec<String>,
    },
    Macro {
        name: String,
        span: SourceSpan,
        visibility: AstVisibility,
    },
}

impl ASTNode {
    /// Get the name of this node (if it has one).
    pub fn name(&self) -> Option<&str> {
        match self {
            ASTNode::Function { name, .. }
            | ASTNode::Struct { name, .. }
            | ASTNode::Enum { name, .. }
            | ASTNode::Trait { name, .. }
            | ASTNode::ImplBlock {
                type_name: name, ..
            }
            | ASTNode::Module { name, .. }
            | ASTNode::Constant { name, .. }
            | ASTNode::Variable { name, .. }
            | ASTNode::TypeAlias { name, .. }
            | ASTNode::Macro { name, .. } => Some(name),
            ASTNode::Import { .. } | ASTNode::Export { .. } => None,
        }
    }

    /// Get the source span of this node.
    pub fn span(&self) -> &SourceSpan {
        match self {
            ASTNode::Function { span, .. }
            | ASTNode::Struct { span, .. }
            | ASTNode::Enum { span, .. }
            | ASTNode::Trait { span, .. }
            | ASTNode::ImplBlock { span, .. }
            | ASTNode::Import { span, .. }
            | ASTNode::Export { span, .. }
            | ASTNode::Module { span, .. }
            | ASTNode::Constant { span, .. }
            | ASTNode::Variable { span, .. }
            | ASTNode::TypeAlias { span, .. }
            | ASTNode::Macro { span, .. } => span,
        }
    }

    /// Get child nodes (for recursive traversal).
    pub fn children(&self) -> Vec<&ASTNode> {
        match self {
            ASTNode::Function {
                body: Some(body), ..
            } => body.iter().collect(),
            ASTNode::Trait { methods, .. } => methods.iter().collect(),
            ASTNode::ImplBlock { methods, .. } => methods.iter().collect(),
            ASTNode::Module { items, .. } => items.iter().collect(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_function_node() {
        let node = ASTNode::Function {
            name: "hello".to_string(),
            params: vec![],
            return_type: None,
            body: None,
            span: SourceSpan::new(0, 10),
            visibility: AstVisibility::Public,
            is_async: false,
            is_method: false,
            generic_params: vec![],
            attributes: vec![],
        };
        assert_eq!(node.name(), Some("hello"));
    }

    #[test]
    fn test_import_no_name() {
        let node = ASTNode::Import {
            path: vec!["std".to_string()],
            items: vec![],
            alias: None,
            span: SourceSpan::new(0, 20),
        };
        assert_eq!(node.name(), None);
    }
}
