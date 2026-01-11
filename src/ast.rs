//! Abstract Syntax Tree definitions for Intent
//!
//! Defines the core AST nodes representing Intent programs.

use serde::{Deserialize, Serialize};

/// A complete Intent program
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Program {
    pub statements: Vec<Statement>,
}

/// Top-level statements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Statement {
    /// Variable declaration: `let x = expr;` or `let mut x = expr;`
    Let {
        name: String,
        mutable: bool,
        type_annotation: Option<TypeExpr>,
        value: Option<Expression>,
        /// Optional pattern for destructuring: `let (a, b) = expr;`
        pattern: Option<Pattern>,
    },
    
    /// Function declaration
    Function {
        name: String,
        params: Vec<Parameter>,
        return_type: Option<TypeExpr>,
        contract: Option<Contract>,
        body: Block,
        attributes: Vec<Attribute>,
        /// Generic type parameters: `fn foo<T, U>()`
        type_params: Vec<String>,
        /// Effect annotation: `fn foo() with io`
        effects: Vec<String>,
    },
    
    /// Type alias declaration: `type Name = Type;`
    TypeAlias {
        name: String,
        type_params: Vec<String>,
        target: TypeExpr,
    },
    
    /// Struct declaration
    Struct {
        name: String,
        fields: Vec<Field>,
        attributes: Vec<Attribute>,
        /// Generic type parameters: `struct Foo<T>`
        type_params: Vec<String>,
    },
    
    /// Enum declaration
    Enum {
        name: String,
        variants: Vec<EnumVariant>,
        attributes: Vec<Attribute>,
        /// Generic type parameters: `enum Option<T>`
        type_params: Vec<String>,
    },
    
    /// Implementation block
    Impl {
        type_name: String,
        methods: Vec<Statement>,
        invariants: Vec<Expression>,
    },
    
    /// Module declaration
    Module {
        name: String,
        body: Vec<Statement>,
    },
    
    /// Use/import statement
    Use {
        path: Vec<String>,
    },
    
    /// Expression statement
    Expression(Expression),
    
    /// Return statement
    Return(Option<Expression>),
    
    /// If statement
    If {
        condition: Expression,
        then_branch: Block,
        else_branch: Option<Block>,
    },
    
    /// While loop
    While {
        condition: Expression,
        body: Block,
    },
    
    /// Infinite loop
    Loop {
        body: Block,
    },
    
    /// Break statement
    Break,
    
    /// Continue statement
    Continue,
    
    /// Protocol declaration for concurrency
    Protocol {
        name: String,
        steps: Vec<ProtocolStep>,
    },
    
    /// Intent annotation
    Intent {
        description: String,
        target: Box<Statement>,
    },
}

/// Expression nodes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expression {
    /// Integer literal
    Integer(i64),
    
    /// Float literal
    Float(f64),
    
    /// String literal
    String(String),
    
    /// Boolean literal
    Bool(bool),
    
    /// Unit value ()
    Unit,
    
    /// Variable reference
    Identifier(String),
    
    /// Binary operation
    Binary {
        left: Box<Expression>,
        operator: BinaryOp,
        right: Box<Expression>,
    },
    
    /// Unary operation
    Unary {
        operator: UnaryOp,
        operand: Box<Expression>,
    },
    
    /// Function call
    Call {
        function: Box<Expression>,
        arguments: Vec<Expression>,
    },
    
    /// Method call
    MethodCall {
        object: Box<Expression>,
        method: String,
        arguments: Vec<Expression>,
    },
    
    /// Field access
    FieldAccess {
        object: Box<Expression>,
        field: String,
    },
    
    /// Index access
    Index {
        object: Box<Expression>,
        index: Box<Expression>,
    },
    
    /// Array literal
    Array(Vec<Expression>),
    
    /// Struct literal
    StructLiteral {
        name: String,
        fields: Vec<(String, Expression)>,
    },
    
    /// Enum variant access (EnumName::Variant or EnumName::Variant(args))
    EnumVariant {
        enum_name: String,
        variant: String,
        arguments: Vec<Expression>,
    },
    
    /// Lambda/closure
    Lambda {
        params: Vec<Parameter>,
        body: Box<Expression>,
    },
    
    /// Block expression
    Block(Block),
    
    /// If expression
    IfExpr {
        condition: Box<Expression>,
        then_branch: Box<Expression>,
        else_branch: Box<Expression>,
    },
    
    /// Match expression
    Match {
        scrutinee: Box<Expression>,
        arms: Vec<MatchArm>,
    },
    
    /// Assignment
    Assign {
        target: Box<Expression>,
        value: Box<Expression>,
    },
    
    /// Await expression
    Await(Box<Expression>),
    
    /// Try expression (for error propagation)
    Try(Box<Expression>),
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum BinaryOp {
    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    
    // Comparison
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    
    // Logical
    And,
    Or,
}

/// Unary operators
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum UnaryOp {
    Neg,
    Not,
}

/// Function parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub type_annotation: Option<TypeExpr>,
    pub default: Option<Expression>,
}

/// Struct field
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Field {
    pub name: String,
    pub type_annotation: TypeExpr,
    pub public: bool,
}

/// Enum variant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Option<Vec<TypeExpr>>,
}

/// Type expression
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TypeExpr {
    /// Named type like `Int`, `String`, `MyStruct`
    Named(String),
    
    /// Array type `[T]`
    Array(Box<TypeExpr>),
    
    /// Tuple type `(T1, T2, ...)`
    Tuple(Vec<TypeExpr>),
    
    /// Function type `(T1, T2) -> T3`
    Function {
        params: Vec<TypeExpr>,
        return_type: Box<TypeExpr>,
    },
    
    /// Generic type `T<A, B>`
    Generic {
        name: String,
        args: Vec<TypeExpr>,
    },
    
    /// Optional type `T?`
    Optional(Box<TypeExpr>),
    
    /// Union type `T | U`
    Union(Vec<TypeExpr>),
    
    /// Result type with effect `T / E`
    WithEffect {
        value_type: Box<TypeExpr>,
        effect: Box<TypeExpr>,
    },
}

/// Contract specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub requires: Vec<Expression>,
    pub ensures: Vec<Expression>,
}

/// Block of statements
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub statements: Vec<Statement>,
}

/// Match arm
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub guard: Option<Expression>,
    pub body: Expression,
}

/// Pattern for matching
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Pattern {
    /// Wildcard `_`
    Wildcard,
    
    /// Variable binding
    Variable(String),
    
    /// Literal pattern
    Literal(Expression),
    
    /// Struct pattern
    Struct {
        name: String,
        fields: Vec<(String, Pattern)>,
    },
    
    /// Enum variant pattern
    Variant {
        name: String,
        variant: String,
        fields: Option<Vec<Pattern>>,
    },
    
    /// Tuple pattern
    Tuple(Vec<Pattern>),
    
    /// Array pattern
    Array(Vec<Pattern>),
}

/// Attribute/annotation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attribute {
    pub name: String,
    pub args: Vec<Expression>,
}

/// Protocol step for concurrency
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProtocolStep {
    Send {
        message_type: String,
        payload: Option<TypeExpr>,
    },
    Receive {
        message_type: String,
        payload: Option<TypeExpr>,
    },
    Choice(Vec<Vec<ProtocolStep>>),
    Loop(Vec<ProtocolStep>),
    End,
}

impl Program {
    pub fn new() -> Self {
        Program {
            statements: Vec::new(),
        }
    }
}

impl Default for Program {
    fn default() -> Self {
        Self::new()
    }
}

impl Block {
    pub fn new() -> Self {
        Block {
            statements: Vec::new(),
        }
    }
}

impl Default for Block {
    fn default() -> Self {
        Self::new()
    }
}
