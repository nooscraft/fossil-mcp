//! AST visitor and transformer traits.

use super::node::ASTNode;

/// Visitor pattern for read-only AST traversal.
pub trait ASTVisitor {
    fn visit_node(&mut self, node: &ASTNode) {
        self.walk(node);
    }

    fn walk(&mut self, node: &ASTNode) {
        for child in node.children() {
            self.visit_node(child);
        }
    }
}

/// Transformer pattern for AST rewriting.
pub trait ASTTransformer {
    fn transform_node(&mut self, node: ASTNode) -> ASTNode {
        self.walk_transform(node)
    }

    fn walk_transform(&mut self, node: ASTNode) -> ASTNode {
        match node {
            ASTNode::Function {
                name,
                params,
                return_type,
                body,
                span,
                visibility,
                is_async,
                is_method,
                generic_params,
                attributes,
            } => ASTNode::Function {
                name,
                params,
                return_type,
                body: body.map(|b| b.into_iter().map(|n| self.transform_node(n)).collect()),
                span,
                visibility,
                is_async,
                is_method,
                generic_params,
                attributes,
            },
            ASTNode::Module {
                name,
                items,
                span,
                visibility,
            } => ASTNode::Module {
                name,
                items: items.into_iter().map(|n| self.transform_node(n)).collect(),
                span,
                visibility,
            },
            ASTNode::Trait {
                name,
                methods,
                span,
                visibility,
                generic_params,
            } => ASTNode::Trait {
                name,
                methods: methods
                    .into_iter()
                    .map(|n| self.transform_node(n))
                    .collect(),
                span,
                visibility,
                generic_params,
            },
            ASTNode::ImplBlock {
                type_name,
                trait_name,
                methods,
                span,
                generic_params,
            } => ASTNode::ImplBlock {
                type_name,
                trait_name,
                methods: methods
                    .into_iter()
                    .map(|n| self.transform_node(n))
                    .collect(),
                span,
                generic_params,
            },
            other => other,
        }
    }
}
