//! Type system for Intent
//!
//! Defines the type representation and type checking logic.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Runtime value types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Type {
    /// Unit type
    Unit,

    /// Integer type
    Int,

    /// Floating point type
    Float,

    /// Boolean type
    Bool,

    /// String type
    String,

    /// Array type
    Array(Box<Type>),

    /// Tuple type
    Tuple(Vec<Type>),

    /// Function type
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
    },

    /// Named type (struct, enum, etc.)
    Named(String),

    /// Generic type
    Generic { name: String, args: Vec<Type> },

    /// Optional type
    Optional(Box<Type>),

    /// Map type
    Map {
        key_type: Box<Type>,
        value_type: Box<Type>,
    },

    /// Union type
    Union(Vec<Type>),

    /// Any type (for gradual typing)
    Any,

    /// Never type (for functions that don't return)
    Never,
}

/// Type environment for type checking
#[derive(Debug, Clone)]
pub struct TypeEnv {
    bindings: HashMap<String, Type>,
    parent: Option<Box<TypeEnv>>,
}

impl TypeEnv {
    pub fn new() -> Self {
        TypeEnv {
            bindings: HashMap::new(),
            parent: None,
        }
    }

    pub fn with_parent(parent: TypeEnv) -> Self {
        TypeEnv {
            bindings: HashMap::new(),
            parent: Some(Box::new(parent)),
        }
    }

    pub fn define(&mut self, name: String, typ: Type) {
        self.bindings.insert(name, typ);
    }

    pub fn lookup(&self, name: &str) -> Option<Type> {
        if let Some(typ) = self.bindings.get(name) {
            Some(typ.clone())
        } else if let Some(ref parent) = self.parent {
            parent.lookup(name)
        } else {
            None
        }
    }
}

impl Default for TypeEnv {
    fn default() -> Self {
        Self::new()
    }
}

impl Type {
    /// Check if two types are compatible
    pub fn is_compatible(&self, other: &Type) -> bool {
        match (self, other) {
            (Type::Any, _) | (_, Type::Any) => true,
            (Type::Unit, Type::Unit) => true,
            (Type::Int, Type::Int) => true,
            (Type::Float, Type::Float) => true,
            (Type::Int, Type::Float) | (Type::Float, Type::Int) => true, // Allow numeric coercion
            (Type::Bool, Type::Bool) => true,
            (Type::String, Type::String) => true,
            (Type::Array(a), Type::Array(b)) => a.is_compatible(b),
            (Type::Optional(a), Type::Optional(b)) => a.is_compatible(b),
            (
                Type::Map {
                    key_type: k1,
                    value_type: v1,
                },
                Type::Map {
                    key_type: k2,
                    value_type: v2,
                },
            ) => k1.is_compatible(k2) && v1.is_compatible(v2),
            (Type::Union(types), other) | (other, Type::Union(types)) => {
                types.iter().any(|t| t.is_compatible(other))
            }
            (Type::Named(a), Type::Named(b)) => a == b,
            (Type::Tuple(a), Type::Tuple(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.is_compatible(y))
            }
            (
                Type::Function {
                    params: p1,
                    return_type: r1,
                },
                Type::Function {
                    params: p2,
                    return_type: r2,
                },
            ) => {
                p1.len() == p2.len()
                    && p1.iter().zip(p2.iter()).all(|(x, y)| x.is_compatible(y))
                    && r1.is_compatible(r2)
            }
            _ => false,
        }
    }

    /// Get a human-readable name for the type
    pub fn name(&self) -> String {
        match self {
            Type::Unit => "()".to_string(),
            Type::Int => "Int".to_string(),
            Type::Float => "Float".to_string(),
            Type::Bool => "Bool".to_string(),
            Type::String => "String".to_string(),
            Type::Array(inner) => format!("[{}]", inner.name()),
            Type::Tuple(types) => {
                let names: Vec<_> = types.iter().map(|t| t.name()).collect();
                format!("({})", names.join(", "))
            }
            Type::Function {
                params,
                return_type,
            } => {
                let param_names: Vec<_> = params.iter().map(|t| t.name()).collect();
                format!("({}) -> {}", param_names.join(", "), return_type.name())
            }
            Type::Named(name) => name.clone(),
            Type::Generic { name, args } => {
                let arg_names: Vec<_> = args.iter().map(|t| t.name()).collect();
                format!("{}<{}>", name, arg_names.join(", "))
            }
            Type::Optional(inner) => format!("{}?", inner.name()),
            Type::Map {
                key_type,
                value_type,
            } => {
                format!("Map<{}, {}>", key_type.name(), value_type.name())
            }
            Type::Union(types) => {
                let names: Vec<_> = types.iter().map(|t| t.name()).collect();
                names.join(" | ")
            }
            Type::Any => "Any".to_string(),
            Type::Never => "Never".to_string(),
        }
    }
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}
