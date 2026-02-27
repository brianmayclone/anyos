//! GLSL AST (Abstract Syntax Tree) definitions.
//!
//! Represents parsed GLSL source as a tree of declarations, statements, and
//! expressions. Used as the intermediate representation between parser and IR lowering.

use alloc::string::String;
use alloc::vec::Vec;

/// Top-level translation unit.
#[derive(Debug, Clone)]
pub struct TranslationUnit {
    pub declarations: Vec<Declaration>,
}

/// A top-level declaration.
#[derive(Debug, Clone)]
pub enum Declaration {
    /// `precision mediump float;`
    Precision(PrecisionQualifier, TypeSpec),
    /// Variable declaration: `uniform mat4 uMVP;`
    Variable(VarDecl),
    /// Function definition (Phase 3+).
    Function(FunctionDef),
}

/// Precision qualifiers (parsed and ignored for computation).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PrecisionQualifier {
    LowP,
    MediumP,
    HighP,
}

/// Type specifier.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeSpec {
    Void,
    Float,
    Int,
    Bool,
    Vec2,
    Vec3,
    Vec4,
    Mat3,
    Mat4,
    Sampler2D,
}

impl TypeSpec {
    /// Number of float components for this type.
    pub fn components(&self) -> u32 {
        match self {
            TypeSpec::Float | TypeSpec::Int | TypeSpec::Bool => 1,
            TypeSpec::Vec2 => 2,
            TypeSpec::Vec3 => 3,
            TypeSpec::Vec4 => 4,
            TypeSpec::Mat3 => 9,
            TypeSpec::Mat4 => 16,
            TypeSpec::Sampler2D => 1,
            TypeSpec::Void => 0,
        }
    }
}

/// Storage qualifier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StorageQualifier {
    None,
    Attribute,
    Varying,
    Uniform,
    Const,
}

/// Variable declaration.
#[derive(Debug, Clone)]
pub struct VarDecl {
    pub qualifier: StorageQualifier,
    pub type_spec: TypeSpec,
    pub name: String,
    pub initializer: Option<Expr>,
}

/// Function definition.
#[derive(Debug, Clone)]
pub struct FunctionDef {
    pub return_type: TypeSpec,
    pub name: String,
    pub params: Vec<(TypeSpec, String)>,
    pub body: Vec<Stmt>,
}

/// Statement.
#[derive(Debug, Clone)]
pub enum Stmt {
    /// Variable declaration as a statement.
    VarDecl(VarDecl),
    /// Assignment: `lhs = rhs;`
    Assign(Expr, Expr),
    /// Compound assignment: `lhs += rhs;`
    CompoundAssign(Expr, CompoundOp, Expr),
    /// Return statement.
    Return(Option<Expr>),
    /// Expression statement (e.g., function call).
    Expr(Expr),
    /// If statement (Phase 3+).
    If(Expr, Vec<Stmt>, Option<Vec<Stmt>>),
    /// For loop (Phase 3+).
    For(Box<Stmt>, Expr, Box<Stmt>, Vec<Stmt>),
    /// Discard (fragment shader only, Phase 3+).
    Discard,
}

/// Compound assignment operators.
#[derive(Debug, Clone, Copy)]
pub enum CompoundOp {
    Add,
    Sub,
    Mul,
    Div,
}

/// Expression.
#[derive(Debug, Clone)]
pub enum Expr {
    /// Integer literal.
    IntLit(i32),
    /// Float literal.
    FloatLit(f32),
    /// Boolean literal.
    BoolLit(bool),
    /// Variable reference.
    Ident(String),
    /// Binary operation.
    Binary(Box<Expr>, BinOp, Box<Expr>),
    /// Unary operation (prefix).
    Unary(UnaryOp, Box<Expr>),
    /// Ternary `cond ? a : b`.
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    /// Function call or type constructor: `vec4(...)`, `normalize(...)`.
    Call(String, Vec<Expr>),
    /// Swizzle: `v.xyz`.
    Swizzle(Box<Expr>, String),
    /// Field/member access: `v.x` (single component — also handled as swizzle).
    Field(Box<Expr>, String),
    /// Array index: `a[i]`.
    Index(Box<Expr>, Box<Expr>),
}

/// Binary operators.
#[derive(Debug, Clone, Copy)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    And,
    Or,
}

/// Unary operators.
#[derive(Debug, Clone, Copy)]
pub enum UnaryOp {
    Neg,
    Not,
}
