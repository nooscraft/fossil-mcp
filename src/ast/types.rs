//! AST supporting types.

use serde::{Deserialize, Serialize};

/// Visibility modifier in AST context.
/// (Named `AstVisibility` to avoid collision with `crate::core::Visibility`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AstVisibility {
    Public,
    Private,
    Protected,
    Internal,
}

/// Type annotation representation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Type {
    Named(String),
    Generic {
        base: String,
        params: Vec<Self>,
    },
    Function {
        params: Vec<Self>,
        return_type: Box<Self>,
    },
    Tuple(Vec<Self>),
    Array {
        element_type: Box<Self>,
        size: Option<usize>,
    },
    Reference {
        inner: Box<Self>,
        is_mut: bool,
    },
    Optional(Box<Self>),
    Union(Vec<Self>),
    Unknown,
}

/// Function/method parameter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub type_annotation: Option<Type>,
    pub default_value: Option<String>,
}

/// Struct/class field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub type_annotation: Option<Type>,
    pub visibility: AstVisibility,
}

/// Enum variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub value: Option<String>,
    pub fields: Vec<Field>,
}
