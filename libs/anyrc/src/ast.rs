use crate::prelude::*;
use crate::intern::Symbol;
use crate::diagnostics::Span;

// Top-level
pub struct Crate {
    pub attrs: Vec<Attribute>,
    pub items: Vec<Item>,
}

pub enum Item {
    Fn(FnDef),
    Struct(StructDef),
    Enum(EnumDef),
    Impl(ImplBlock),
    Trait(TraitDef),
    TypeAlias(TypeAliasDef),
    Const(ConstDef),
    Static(StaticDef),
    Use(UseTree),
    Mod(ModDef),
    ExternCrate(ExternCrateDef),
    MacroDef(MacroRulesDef),
    MacroCall(Path, Vec<TokenTree>, Span),
    ExternBlock(ExternBlockDef),
}

pub struct FnDef {
    pub name: Symbol,
    pub generics: Generics,
    pub params: Vec<Param>,
    pub ret_ty: Option<Box<Ty>>,
    pub where_clause: WhereClause,
    pub body: Option<Block>,
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub is_unsafe: bool,
    pub is_const: bool,
    pub abi: Option<String>,
    pub span: Span,
}

pub struct Param {
    pub pat: Pattern,
    pub ty: Ty,
    pub span: Span,
}

pub struct StructDef {
    pub name: Symbol,
    pub generics: Generics,
    pub fields: Vec<FieldDef>,
    pub vis: Visibility,
    pub attrs: Vec<Attribute>,
    pub is_union: bool,
    pub span: Span,
}

pub struct FieldDef {
    pub name: Symbol,
    pub ty: Ty,
    pub vis: Visibility,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

pub struct EnumDef {
    pub name: Symbol,
    pub generics: Generics,
    pub variants: Vec<Variant>,
    pub vis: Visibility,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

pub struct Variant {
    pub name: Symbol,
    pub fields: VariantFields,
    pub discriminant: Option<Box<Expr>>,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

pub enum VariantFields {
    Unit,
    Tuple(Vec<Ty>),
    Struct(Vec<FieldDef>),
}

pub struct ImplBlock {
    pub generics: Generics,
    pub trait_ref: Option<Path>,
    pub self_ty: Ty,
    pub items: Vec<Item>,
    pub attrs: Vec<Attribute>,
    pub is_unsafe: bool,
    pub span: Span,
}

pub struct TraitDef {
    pub name: Symbol,
    pub generics: Generics,
    pub supertraits: Vec<TraitBound>,
    pub items: Vec<Item>,
    pub vis: Visibility,
    pub is_unsafe: bool,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

pub struct TypeAliasDef {
    pub name: Symbol,
    pub generics: Generics,
    pub ty: Option<Box<Ty>>,
    pub vis: Visibility,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

pub struct ConstDef {
    pub name: Symbol,
    pub ty: Ty,
    pub value: Option<Box<Expr>>,
    pub vis: Visibility,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

pub struct StaticDef {
    pub name: Symbol,
    pub ty: Ty,
    pub value: Option<Box<Expr>>,
    pub vis: Visibility,
    pub is_mut: bool,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

pub struct UseTree {
    pub vis: Visibility,
    pub path: Vec<Symbol>,
    pub kind: UseTreeKind,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

pub enum UseTreeKind {
    Simple(Option<Symbol>),
    Glob,
    Nested(Vec<UseTree>),
}

pub struct ModDef {
    pub name: Symbol,
    pub items: Option<Vec<Item>>,
    pub attrs: Vec<Attribute>,
    pub vis: Visibility,
    pub span: Span,
}

pub struct ExternCrateDef {
    pub name: Symbol,
    pub alias: Option<Symbol>,
    pub span: Span,
}

pub struct MacroRulesDef {
    pub name: Symbol,
    pub rules: Vec<MacroRule>,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

#[derive(Clone)]
pub struct MacroRule {
    pub pattern: Vec<TokenTree>,
    pub body: Vec<TokenTree>,
}

#[derive(Debug, Clone)]
pub enum TokenTree {
    Token(crate::lexer::Token),
    Delimited(Delimiter, Vec<TokenTree>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delimiter {
    Paren,
    Bracket,
    Brace,
}

pub struct ExternBlockDef {
    pub abi: Option<String>,
    pub items: Vec<Item>,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

// Expressions
pub enum Expr {
    Lit(Literal, Span),
    Path(Path),
    QualifiedPath(QualifiedPath),
    Binary(BinOp, Box<Expr>, Box<Expr>, Span),
    Unary(UnOp, Box<Expr>, Span),
    Call(Box<Expr>, Vec<Expr>, Span),
    MethodCall(Box<Expr>, Symbol, Vec<Ty>, Vec<Expr>, Span),
    Field(Box<Expr>, Symbol, Span),
    Index(Box<Expr>, Box<Expr>, Span),
    Block(Block),
    If(Box<Expr>, Block, Option<Box<Expr>>, Span),
    Match(Box<Expr>, Vec<MatchArm>, Span),
    Loop(Block, Option<Symbol>, Span),
    While(Box<Expr>, Block, Option<Symbol>, Span),
    For(Pattern, Box<Expr>, Block, Option<Symbol>, Span),
    Closure(Vec<Param>, Option<Box<Ty>>, Box<Expr>, bool, Span),
    Return(Option<Box<Expr>>, Span),
    Break(Option<Symbol>, Option<Box<Expr>>, Span),
    Continue(Option<Symbol>, Span),
    Assign(Box<Expr>, Box<Expr>, Span),
    AssignOp(BinOp, Box<Expr>, Box<Expr>, Span),
    Ref(Box<Expr>, Mutability, Span),
    Deref(Box<Expr>, Span),
    Cast(Box<Expr>, Ty, Span),
    Struct(Path, Vec<FieldExpr>, Option<Box<Expr>>, Span),
    Tuple(Vec<Expr>, Span),
    Array(Vec<Expr>, Span),
    ArrayRepeat(Box<Expr>, Box<Expr>, Span),
    Range(Option<Box<Expr>>, Option<Box<Expr>>, bool, Span),
    MacroCall(Path, Vec<TokenTree>, Span),
    Unsafe(Block, Span),
    Paren(Box<Expr>, Span),
    InlineAsm(InlineAsm),
    IfLet(Pattern, Box<Expr>, Block, Option<Box<Expr>>, Span),
    WhileLet(Pattern, Box<Expr>, Block, Option<Symbol>, Span),
}

pub struct InlineAsm {
    pub template: Vec<String>,
    pub operands: Vec<AsmOperand>,
    pub options: Vec<String>,
    pub span: Span,
}

pub enum AsmOperand {
    In { reg: AsmReg, expr: Box<Expr> },
    Out { reg: AsmReg, expr: Option<Box<Expr>> },
    InOut { reg: AsmReg, expr: Box<Expr>, out_expr: Option<Box<Expr>> },
    Const { expr: Box<Expr> },
    Sym { path: Path },
}

pub enum AsmReg {
    Named(String),
    Class(String),
}

pub struct FieldExpr {
    pub name: Symbol,
    pub value: Expr,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

pub struct MatchArm {
    pub attrs: Vec<Attribute>,
    pub pat: Pattern,
    pub guard: Option<Box<Expr>>,
    pub body: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Literal {
    Int(u128),
    Float(f64),
    String(String),
    Char(char),
    Bool(bool),
    ByteString(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add, Sub, Mul, Div, Rem,
    BitAnd, BitOr, BitXor, Shl, Shr,
    Eq, Ne, Lt, Le, Gt, Ge,
    And, Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Deref,
}

// Patterns
pub enum Pattern {
    Ident(Symbol, Mutability, Option<Box<Pattern>>, Span),
    Literal(Literal, Span),
    Tuple(Vec<Pattern>, Span),
    Slice(Vec<Pattern>, Span),
    Struct(Path, Vec<FieldPat>, bool, Span),
    TupleStruct(Path, Vec<Pattern>, Span),
    Wildcard(Span),
    Rest(Span),
    Ref(Box<Pattern>, Mutability, Span),
    RefBinding(Box<Pattern>, Mutability, Span),
    Or(Vec<Pattern>, Span),
    Range(Option<Box<Expr>>, Option<Box<Expr>>, bool, Span),
    Path(Path),
}

pub struct FieldPat {
    pub name: Symbol,
    pub pat: Pattern,
    pub attrs: Vec<Attribute>,
    pub span: Span,
}

// Types
pub enum Ty {
    Path(Path),
    QualifiedPath(QualifiedPath),
    Reference(Option<Symbol>, Box<Ty>, Mutability, Span),
    RawPtr(Box<Ty>, Mutability, Span),
    Tuple(Vec<Ty>, Span),
    Array(Box<Ty>, Box<Expr>, Span),
    Slice(Box<Ty>, Span),
    FnPtr(Vec<Ty>, Option<Box<Ty>>, Span),
    DynTrait(Vec<TraitBound>, Span),
    Infer(Span),
    Never(Span),
}

// Generics
pub struct Generics {
    pub params: Vec<GenericParam>,
    pub span: Span,
}

pub enum GenericParam {
    Type(Symbol, Vec<TraitBound>, Option<Ty>, Span),
    Lifetime(Symbol, Vec<Symbol>, Span),
    Const(Symbol, Ty, Span),
}

pub struct TraitBound {
    pub path: Path,
    pub span: Span,
}

pub struct WhereClause {
    pub predicates: Vec<WherePredicate>,
    pub span: Span,
}

pub enum WherePredicate {
    Type(Ty, Vec<TraitBound>, Span),
    Lifetime(Symbol, Vec<Symbol>, Span),
}

// Common
pub struct Path {
    pub segments: Vec<PathSegment>,
    pub span: Span,
}

pub struct QualifiedPath {
    pub self_ty: Box<Ty>,
    pub trait_path: Option<Path>,
    pub path: Path,
    pub span: Span,
}

pub struct PathSegment {
    pub ident: Symbol,
    pub args: Option<GenericArgs>,
}

pub struct GenericArgs {
    pub args: Vec<GenericArg>,
    pub span: Span,
}

pub enum GenericArg {
    Type(Ty),
    Lifetime(Symbol),
    Const(Expr),
}

pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

pub enum Stmt {
    Attributed(Vec<Attribute>, Box<Stmt>, Span),
    Let(Pattern, Option<Ty>, Option<Box<Expr>>, Span),
    Expr(Expr),
    Semi(Expr, Span),
    Item(Item),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Private,
    Public,
    PubCrate,
    PubSuper,
    PubSelf,
    PubIn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mutability {
    Immutable,
    Mut,
}

pub struct Attribute {
    pub path: Path,
    pub args: AttrArgs,
    pub span: Span,
}

pub enum AttrArgs {
    Empty,
    Delimited(Vec<TokenTree>),
    Eq(Box<Expr>),
}
