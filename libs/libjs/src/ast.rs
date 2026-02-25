//! JavaScript Abstract Syntax Tree (AST) node types.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// A complete JavaScript program (list of statements).
#[derive(Debug, Clone)]
pub struct Program {
    pub body: Vec<Stmt>,
}

/// Statement nodes.
#[derive(Debug, Clone)]
pub enum Stmt {
    /// Expression statement
    Expr(Expr),

    /// Variable declaration: `var x = 1;` / `let x = 1;` / `const x = 1;`
    VarDecl {
        kind: VarKind,
        decls: Vec<VarDeclarator>,
    },

    /// Block: `{ ... }`
    Block(Vec<Stmt>),

    /// If statement: `if (cond) then else`
    If {
        condition: Expr,
        consequent: Box<Stmt>,
        alternate: Option<Box<Stmt>>,
    },

    /// While loop: `while (cond) body`
    While {
        condition: Expr,
        body: Box<Stmt>,
    },

    /// Do-while loop: `do body while (cond)`
    DoWhile {
        body: Box<Stmt>,
        condition: Expr,
    },

    /// For loop: `for (init; test; update) body`
    For {
        init: Option<Box<ForInit>>,
        test: Option<Expr>,
        update: Option<Expr>,
        body: Box<Stmt>,
    },

    /// For-in loop: `for (left in right) body`
    ForIn {
        left: Box<ForInit>,
        right: Expr,
        body: Box<Stmt>,
    },

    /// For-of loop: `for (left of right) body`
    ForOf {
        left: Box<ForInit>,
        right: Expr,
        body: Box<Stmt>,
    },

    /// Return: `return expr?;`
    Return(Option<Expr>),

    /// Break: `break label?;`
    Break(Option<String>),

    /// Continue: `continue label?;`
    Continue(Option<String>),

    /// Switch statement
    Switch {
        discriminant: Expr,
        cases: Vec<SwitchCase>,
    },

    /// Throw: `throw expr;`
    Throw(Expr),

    /// Try-catch-finally
    Try {
        block: Vec<Stmt>,
        catch: Option<CatchClause>,
        finally: Option<Vec<Stmt>>,
    },

    /// Function declaration: `function name(params) { body }`
    FunctionDecl {
        name: String,
        params: Vec<Param>,
        body: Vec<Stmt>,
        is_async: bool,
    },

    /// Class declaration
    ClassDecl {
        name: String,
        super_class: Option<Expr>,
        body: Vec<ClassMember>,
    },

    /// Labeled statement: `label: stmt`
    Labeled {
        label: String,
        body: Box<Stmt>,
    },

    /// Empty statement: `;`
    Empty,

    /// Debugger: `debugger;`
    Debugger,
}

/// Expression nodes.
#[derive(Debug, Clone)]
pub enum Expr {
    /// Numeric literal
    Number(f64),

    /// String literal
    String(String),

    /// Boolean literal
    Bool(bool),

    /// Null literal
    Null,

    /// Undefined
    Undefined,

    /// Template literal (simplified — just a string for now)
    Template(String),

    /// Identifier reference
    Ident(String),

    /// `this` keyword
    This,

    /// Array literal: `[a, b, c]`
    Array(Vec<Option<Expr>>),

    /// Object literal: `{ key: value, ... }`
    Object(Vec<ObjProp>),

    /// Member access: `obj.prop`
    Member {
        object: Box<Expr>,
        property: String,
        computed: bool,
    },

    /// Computed member access: `obj[expr]`
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },

    /// Function call: `func(args)`
    Call {
        callee: Box<Expr>,
        arguments: Vec<Expr>,
    },

    /// new expression: `new Ctor(args)`
    New {
        callee: Box<Expr>,
        arguments: Vec<Expr>,
    },

    /// Unary expression: `!x`, `-x`, `typeof x`, etc.
    Unary {
        op: UnaryOp,
        argument: Box<Expr>,
        prefix: bool,
    },

    /// Update expression: `x++`, `++x`
    Update {
        op: UpdateOp,
        argument: Box<Expr>,
        prefix: bool,
    },

    /// Binary expression: `a + b`, `a === b`, etc.
    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    /// Logical expression: `a && b`, `a || b`, `a ?? b`
    Logical {
        op: LogicalOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    /// Assignment expression: `a = b`, `a += b`, etc.
    Assign {
        op: AssignOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    /// Conditional expression: `cond ? then : else`
    Conditional {
        test: Box<Expr>,
        consequent: Box<Expr>,
        alternate: Box<Expr>,
    },

    /// Comma expression: `a, b`
    Sequence(Vec<Expr>),

    /// Function expression: `function(params) { body }`
    FunctionExpr {
        name: Option<String>,
        params: Vec<Param>,
        body: Vec<Stmt>,
        is_async: bool,
    },

    /// Arrow function: `(params) => body`
    Arrow {
        params: Vec<Param>,
        body: ArrowBody,
        is_async: bool,
    },

    /// Spread element: `...expr`
    Spread(Box<Expr>),

    /// Typeof: `typeof expr`
    Typeof(Box<Expr>),

    /// Void: `void expr`
    Void(Box<Expr>),

    /// Delete: `delete expr`
    Delete(Box<Expr>),

    /// Yield: `yield expr`
    Yield(Option<Box<Expr>>),

    /// Await: `await expr`
    Await(Box<Expr>),

    /// Class expression
    ClassExpr {
        name: Option<String>,
        super_class: Option<Box<Expr>>,
        body: Vec<ClassMember>,
    },

    /// Optional chaining: `a?.b`
    OptionalChain {
        object: Box<Expr>,
        property: String,
    },

    /// Tagged template: tag`template`
    TaggedTemplate {
        tag: Box<Expr>,
        template: String,
    },
}

/// Arrow function body — either an expression or a block.
#[derive(Debug, Clone)]
pub enum ArrowBody {
    Expr(Box<Expr>),
    Block(Vec<Stmt>),
}

/// Variable declaration kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

/// Variable declarator: `name = init?`
#[derive(Debug, Clone)]
pub struct VarDeclarator {
    pub name: Pattern,
    pub init: Option<Expr>,
}

/// Binding pattern (simplified).
#[derive(Debug, Clone)]
pub enum Pattern {
    Ident(String),
    Array(Vec<Option<Pattern>>),
    Object(Vec<ObjPatProp>),
    Assign(Box<Pattern>, Box<Expr>), // pattern = default
    /// Rest element: `...binding` in array or object destructuring.
    Rest(Box<Pattern>),
}

/// Object pattern property.
#[derive(Debug, Clone)]
pub struct ObjPatProp {
    pub key: String,
    pub value: Pattern,
}

/// Function parameter.
#[derive(Debug, Clone)]
pub struct Param {
    pub pattern: Pattern,
    pub default: Option<Expr>,
    /// True when this is a rest parameter (`...name`); always the last param.
    pub is_rest: bool,
}

/// Object property in literal.
#[derive(Debug, Clone)]
pub struct ObjProp {
    pub key: PropKey,
    pub value: Expr,
    pub kind: PropKind,
    pub shorthand: bool,
}

/// Property key.
#[derive(Debug, Clone)]
pub enum PropKey {
    Ident(String),
    String(String),
    Number(f64),
    Computed(Box<Expr>),
}

/// Property kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropKind {
    Init,
    Get,
    Set,
    Method,
}

/// For loop initializer.
#[derive(Debug, Clone)]
pub enum ForInit {
    VarDecl {
        kind: VarKind,
        decls: Vec<VarDeclarator>,
    },
    Expr(Expr),
}

/// Switch case.
#[derive(Debug, Clone)]
pub struct SwitchCase {
    pub test: Option<Expr>, // None for default
    pub consequent: Vec<Stmt>,
}

/// Catch clause.
#[derive(Debug, Clone)]
pub struct CatchClause {
    pub param: Option<Pattern>,
    pub body: Vec<Stmt>,
}

/// Class member.
#[derive(Debug, Clone)]
pub struct ClassMember {
    pub key: PropKey,
    pub kind: ClassMemberKind,
    pub is_static: bool,
}

/// Class member kind.
#[derive(Debug, Clone)]
pub enum ClassMemberKind {
    Method {
        params: Vec<Param>,
        body: Vec<Stmt>,
    },
    Property {
        value: Option<Expr>,
    },
    Constructor {
        params: Vec<Param>,
        body: Vec<Stmt>,
    },
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,      // -
    Pos,      // +
    Not,      // !
    BitNot,   // ~
    Typeof,
    Void,
    Delete,
}

/// Update operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOp {
    Inc, // ++
    Dec, // --
}

/// Binary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,      // +
    Sub,      // -
    Mul,      // *
    Div,      // /
    Mod,      // %
    Exp,      // **
    Eq,       // ==
    Ne,       // !=
    StrictEq, // ===
    StrictNe, // !==
    Lt,       // <
    Le,       // <=
    Gt,       // >
    Ge,       // >=
    BitAnd,   // &
    BitOr,    // |
    BitXor,   // ^
    Shl,      // <<
    Shr,      // >>
    UShr,     // >>>
    In,       // in
    InstanceOf, // instanceof
}

/// Logical operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,            // &&
    Or,             // ||
    NullishCoalesce, // ??
}

/// Assignment operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,     // =
    AddAssign,  // +=
    SubAssign,  // -=
    MulAssign,  // *=
    DivAssign,  // /=
    ModAssign,  // %=
    ExpAssign,  // **=
    BitAndAssign, // &=
    BitOrAssign,  // |=
    BitXorAssign, // ^=
    ShlAssign,    // <<=
    ShrAssign,    // >>=
    UShrAssign,   // >>>=
    AndAssign,    // &&=
    OrAssign,     // ||=
    NullishAssign, // ??=
}
