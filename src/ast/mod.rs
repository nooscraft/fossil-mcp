//! # fossil_ast
//!
//! Language-agnostic AST types, visitor, and transformer for Fossil.

mod node;
mod types;
mod visitor;

pub use node::ASTNode;
pub use types::{AstVisibility, EnumVariant, Field, Parameter, Type};
pub use visitor::{ASTTransformer, ASTVisitor};
