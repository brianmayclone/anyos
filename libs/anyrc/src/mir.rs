use crate::typeck::TyKind;
use crate::intern::Symbol;
use crate::diagnostics::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Local(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub block: BlockId,
    pub statement_index: usize,
}

pub struct MirBody {
    pub basic_blocks: Vec<BasicBlock>,
    pub locals: Vec<LocalDecl>,
    pub arg_count: usize,
    pub name: Symbol,
    pub span: Span,
}

pub struct LocalDecl {
    pub ty: TyKind,
    pub mutability: crate::ast::Mutability,
    pub name: Option<Symbol>,
    pub span: Span,
}

pub struct BasicBlock {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
}

pub enum StatementKind {
    Assign(Place, Rvalue),
    StorageLive(Local),
    StorageDead(Local),
    Nop,
}

pub enum Rvalue {
    Use(Operand),
    Ref(BorrowKind, Place),
    BinaryOp(crate::ast::BinOp, Operand, Operand),
    UnaryOp(crate::ast::UnOp, Operand),
    Cast(Operand, TyKind),
    Aggregate(AggregateKind, Vec<Operand>),
    Discriminant(Place),
    Len(Place),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowKind {
    Shared,
    Mutable,
}

pub enum AggregateKind {
    Tuple,
    Array,
    Adt(crate::hir::DefId, usize),
}

#[derive(Debug, Clone)]
pub enum Operand {
    Copy(Place),
    Move(Place),
    Constant(Constant),
}

#[derive(Debug, Clone)]
pub struct Constant {
    pub ty: TyKind,
    pub value: ConstValue,
}

#[derive(Debug, Clone)]
pub enum ConstValue {
    Int(i128),
    Uint(u128),
    Float(f64),
    Bool(bool),
    Char(char),
    Str(String),
    FnItem(crate::intern::Symbol),
    /// Reference to a static variable by its symbol name
    StaticRef(crate::intern::Symbol),
    Unit,
}

#[derive(Debug, Clone)]
pub struct Place {
    pub local: Local,
    pub projections: Vec<Projection>,
}

impl Place {
    pub fn local(local: Local) -> Self {
        Place { local, projections: vec![] }
    }
}

#[derive(Debug, Clone)]
pub enum Projection {
    Field(usize),
    Index(Local),
    Deref,
}

pub enum Terminator {
    Goto(BlockId),
    SwitchInt {
        operand: Operand,
        targets: Vec<(u128, BlockId)>,
        default: BlockId,
    },
    Call {
        func: Operand,
        args: Vec<Operand>,
        dest: Place,
        target: BlockId,
    },
    Return,
    Unreachable,
}
