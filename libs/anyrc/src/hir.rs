use crate::intern::Symbol;
use crate::diagnostics::Span;
use crate::ast::{BinOp, UnOp, Mutability, Visibility, Literal};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HirId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefId(pub u32);

// ── Top-level ──

pub struct HirCrate {
    pub items: Vec<HirItem>,
}

pub struct HirItem {
    pub id: HirId,
    pub kind: HirItemKind,
    pub span: Span,
}

pub enum HirItemKind {
    Fn(HirFnDef),
    Struct(HirStructDef),
    Enum(HirEnumDef),
    Impl(HirImplBlock),
    Trait(HirTraitDef),
    TypeAlias(HirTypeAlias),
    Const(HirConstDef),
    Static(HirStaticDef),
    Use(HirUseTree),
    Mod(HirModDef),
    ExternBlock(HirExternBlock),
}

// ── Items ──

pub struct HirFnDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub generics: HirGenerics,
    pub params: Vec<HirParam>,
    pub ret_ty: Option<Box<HirTy>>,
    pub body: Option<HirBlock>,
    pub vis: Visibility,
    pub is_unsafe: bool,
    pub is_const: bool,
    pub abi: Option<String>,
    pub no_mangle: bool,
    pub is_panic_handler: bool,
}

pub struct HirParam {
    pub id: HirId,
    pub pat: HirPattern,
    pub ty: HirTy,
    pub span: Span,
}

pub struct HirStructDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub generics: HirGenerics,
    pub fields: Vec<HirFieldDef>,
    pub vis: Visibility,
}

pub struct HirFieldDef {
    pub id: HirId,
    pub name: Symbol,
    pub ty: HirTy,
    pub vis: Visibility,
    pub span: Span,
}

pub struct HirEnumDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub generics: HirGenerics,
    pub variants: Vec<HirVariant>,
    pub vis: Visibility,
}

pub struct HirVariant {
    pub id: HirId,
    pub name: Symbol,
    pub fields: HirVariantFields,
    pub discriminant: Option<Box<HirExpr>>,
    pub span: Span,
}

pub enum HirVariantFields {
    Unit,
    Tuple(Vec<HirTy>),
    Struct(Vec<HirFieldDef>),
}

pub struct HirImplBlock {
    pub id: HirId,
    pub generics: HirGenerics,
    pub trait_ref: Option<HirPath>,
    pub self_ty: HirTy,
    pub items: Vec<HirItem>,
    pub is_unsafe: bool,
}

pub struct HirTraitDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub generics: HirGenerics,
    pub supertraits: Vec<HirTraitBound>,
    pub items: Vec<HirItem>,
    pub vis: Visibility,
    pub is_unsafe: bool,
}

pub struct HirTypeAlias {
    pub def_id: DefId,
    pub name: Symbol,
    pub generics: HirGenerics,
    pub ty: Option<Box<HirTy>>,
    pub vis: Visibility,
}

pub struct HirConstDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub ty: HirTy,
    pub value: Option<Box<HirExpr>>,
    pub vis: Visibility,
}

pub struct HirStaticDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub ty: HirTy,
    pub value: Option<Box<HirExpr>>,
    pub vis: Visibility,
    pub is_mut: bool,
}

pub struct HirUseTree {
    pub id: HirId,
    pub path: Vec<Symbol>,
    pub kind: HirUseTreeKind,
    pub span: Span,
}

pub enum HirUseTreeKind {
    Simple(Option<Symbol>),
    Glob,
    Nested(Vec<HirUseTree>),
}

pub struct HirModDef {
    pub def_id: DefId,
    pub name: Symbol,
    pub items: Option<Vec<HirItem>>,
    pub vis: Visibility,
}

pub struct HirExternBlock {
    pub id: HirId,
    pub abi: Option<String>,
    pub items: Vec<HirItem>,
}

// ── Expressions ──

pub struct HirExpr {
    pub id: HirId,
    pub kind: HirExprKind,
    pub span: Span,
}

pub enum HirExprKind {
    Lit(Literal),
    Path(HirPath),
    Binary(BinOp, Box<HirExpr>, Box<HirExpr>),
    Unary(UnOp, Box<HirExpr>),
    Call(Box<HirExpr>, Vec<HirExpr>),
    MethodCall(Box<HirExpr>, Symbol, Vec<HirTy>, Vec<HirExpr>),
    Field(Box<HirExpr>, Symbol),
    Index(Box<HirExpr>, Box<HirExpr>),
    Block(HirBlock),
    If(Box<HirExpr>, HirBlock, Option<Box<HirExpr>>),
    Match(Box<HirExpr>, Vec<HirMatchArm>),
    Loop(HirBlock, Option<Symbol>),
    // while is desugared to loop { if !cond { break; } body }
    Closure(Vec<HirParam>, Option<Box<HirTy>>, Box<HirExpr>, bool),
    Return(Option<Box<HirExpr>>),
    Break(Option<Symbol>, Option<Box<HirExpr>>),
    Continue(Option<Symbol>),
    Assign(Box<HirExpr>, Box<HirExpr>),
    AssignOp(BinOp, Box<HirExpr>, Box<HirExpr>),
    Ref(Box<HirExpr>, Mutability),
    Deref(Box<HirExpr>),
    Cast(Box<HirExpr>, HirTy),
    Struct(HirPath, Vec<HirFieldExpr>, Option<Box<HirExpr>>),
    Tuple(Vec<HirExpr>),
    Array(Vec<HirExpr>),
    ArrayRepeat(Box<HirExpr>, Box<HirExpr>),
    Range(Option<Box<HirExpr>>, Option<Box<HirExpr>>, bool),
    Unsafe(HirBlock),
    Paren(Box<HirExpr>),
    // Question mark operator kept as-is (needs type info for desugaring)
    Try(Box<HirExpr>),
    // For loop kept as-is for now (needs trait resolution for full desugaring)
    For(HirPattern, Box<HirExpr>, HirBlock, Option<Symbol>),
    InlineAsm(HirInlineAsm),
}

pub struct HirInlineAsm {
    pub template: Vec<String>,
    pub operands: Vec<HirAsmOperand>,
    pub options: Vec<String>,
}

pub enum HirAsmOperand {
    In { reg: HirAsmReg, expr: Box<HirExpr> },
    Out { reg: HirAsmReg, expr: Option<Box<HirExpr>> },
    InOut { reg: HirAsmReg, expr: Box<HirExpr> },
}

pub enum HirAsmReg {
    Named(String),
    Class(String),
}

pub struct HirFieldExpr {
    pub name: Symbol,
    pub value: HirExpr,
    pub span: Span,
}

pub struct HirMatchArm {
    pub id: HirId,
    pub pat: HirPattern,
    pub guard: Option<Box<HirExpr>>,
    pub body: Box<HirExpr>,
    pub span: Span,
}

// ── Statements ──

pub enum HirStmt {
    Let(HirId, HirPattern, Option<HirTy>, Option<Box<HirExpr>>, Span),
    Expr(HirExpr),
    Semi(HirExpr, Span),
    Item(HirItem),
}

pub struct HirBlock {
    pub id: HirId,
    pub stmts: Vec<HirStmt>,
    pub span: Span,
}

// ── Patterns ──

pub enum HirPattern {
    Ident(HirId, Symbol, Mutability, Option<Box<HirPattern>>, Span),
    Literal(Literal, Span),
    Tuple(Vec<HirPattern>, Span),
    Struct(HirPath, Vec<HirFieldPat>, bool, Span),
    TupleStruct(HirPath, Vec<HirPattern>, Span),
    Wildcard(Span),
    Ref(Box<HirPattern>, Mutability, Span),
    Or(Vec<HirPattern>, Span),
    Range(Option<Box<HirExpr>>, Option<Box<HirExpr>>, bool, Span),
    Path(HirPath),
}

pub struct HirFieldPat {
    pub name: Symbol,
    pub pat: HirPattern,
    pub span: Span,
}

// ── Types ──

pub enum HirTy {
    Path(HirPath),
    Reference(Option<Symbol>, Box<HirTy>, Mutability, Span),
    RawPtr(Box<HirTy>, Mutability, Span),
    Tuple(Vec<HirTy>, Span),
    Array(Box<HirTy>, Box<HirExpr>, Span),
    Slice(Box<HirTy>, Span),
    FnPtr(Vec<HirTy>, Option<Box<HirTy>>, Span),
    Infer(Span),
    Never(Span),
}

// ── Generics / Paths ──

pub struct HirGenerics {
    pub params: Vec<HirGenericParam>,
    pub span: Span,
}

pub enum HirGenericParam {
    Type(Symbol, Vec<HirTraitBound>, Option<HirTy>, Span),
    Lifetime(Symbol, Vec<Symbol>, Span),
    Const(Symbol, HirTy, Span),
}

pub struct HirTraitBound {
    pub path: HirPath,
    pub span: Span,
}

pub struct HirPath {
    pub segments: Vec<HirPathSegment>,
    pub span: Span,
}

pub struct HirPathSegment {
    pub ident: Symbol,
    pub args: Option<HirGenericArgs>,
}

pub struct HirGenericArgs {
    pub args: Vec<HirGenericArg>,
    pub span: Span,
}

pub enum HirGenericArg {
    Type(HirTy),
    Lifetime(Symbol),
    Const(HirExpr),
}
