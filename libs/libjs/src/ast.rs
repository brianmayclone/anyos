//! JavaScript Abstract Syntax Tree (AST) node types.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

/// A complete JavaScript program (list of statements).
#[derive(Debug, Clone)]
pub struct Program {
    pub body: Vec<Stmt>,
    /// Source line number for each statement (parallel to `body`).
    /// Set by the parser; used by the compiler to generate line maps.
    pub stmt_lines: Vec<u32>,
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
    While { condition: Expr, body: Box<Stmt> },

    /// Do-while loop: `do body while (cond)`
    DoWhile { body: Box<Stmt>, condition: Expr },

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
        is_generator: bool,
    },

    /// Class declaration
    ClassDecl {
        name: String,
        super_class: Option<Expr>,
        body: Vec<ClassMember>,
    },

    /// Labeled statement: `label: stmt`
    Labeled { label: String, body: Box<Stmt> },

    /// Empty statement: `;`
    Empty,

    /// Debugger: `debugger;`
    Debugger,

    /// With statement: `with (expr) stmt`
    With {
        object: Expr,
        body: Box<Stmt>,
    },

    /// Import declaration: `import { a, b } from 'module'`
    Import {
        specifiers: Vec<ImportSpecifier>,
        source: String,
    },

    /// Export declaration: `export { a, b }`, `export default expr`, `export function ...`
    Export(ExportDecl),
}

/// Import specifier.
#[derive(Debug, Clone)]
pub enum ImportSpecifier {
    /// `import name from 'mod'`  (default import)
    Default(String),
    /// `import { name }` or `import { name as alias }`
    Named { imported: String, local: String },
    /// `import * as name from 'mod'`
    Namespace(String),
}

/// Export declaration.
#[derive(Debug, Clone)]
pub enum ExportDecl {
    /// `export default expr`
    Default(Expr),
    /// `export { name1, name2 as alias2 }`
    Named(Vec<ExportSpecifier>),
    /// `export function name() {}`, `export class name {}`, `export const x = ...`
    Decl(Box<Stmt>),
    /// `export { ... } from 'module'` (re-export)
    ReExport {
        specifiers: Vec<ExportSpecifier>,
        source: String,
    },
}

/// Export specifier: `name` or `name as alias`.
#[derive(Debug, Clone)]
pub struct ExportSpecifier {
    pub local: String,
    pub exported: String,
}

/// Expression nodes.
#[derive(Debug, Clone)]
pub enum Expr {
    /// Numeric literal
    Number(f64),

    /// BigInt literal (decimal string, e.g. "123" for 123n)
    BigIntLit(String),

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
    Index { object: Box<Expr>, index: Box<Expr> },

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
        is_generator: bool,
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

    /// Yield delegate: `yield* expr`
    YieldDelegate(Box<Expr>),

    /// Await: `await expr`
    Await(Box<Expr>),

    /// `new.target` meta-property
    NewTarget,

    /// Class expression
    ClassExpr {
        name: Option<String>,
        super_class: Option<Box<Expr>>,
        body: Vec<ClassMember>,
    },

    /// Optional chaining: `a?.b`
    OptionalChain { object: Box<Expr>, property: String },

    /// Optional call: `a?.(args)`
    OptionalCall {
        callee: Box<Expr>,
        arguments: Vec<Expr>,
    },

    /// Tagged template: tag`template`
    TaggedTemplate { tag: Box<Expr>, template: String },

    /// Regular expression literal: `/pattern/flags`
    RegExp { pattern: String, flags: String },
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
    pub computed: Option<Expr>,
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
        is_generator: bool,
        is_async: bool,
    },
    Property {
        value: Option<Expr>,
    },
    Constructor {
        params: Vec<Param>,
        body: Vec<Stmt>,
    },
    Getter {
        body: Vec<Stmt>,
    },
    Setter {
        param: String,
        body: Vec<Stmt>,
    },
    StaticBlock {
        body: Vec<Stmt>,
    },
}

/// Unary operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,    // -
    Pos,    // +
    Not,    // !
    BitNot, // ~
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
    Add,        // +
    Sub,        // -
    Mul,        // *
    Div,        // /
    Mod,        // %
    Exp,        // **
    Eq,         // ==
    Ne,         // !=
    StrictEq,   // ===
    StrictNe,   // !==
    Lt,         // <
    Le,         // <=
    Gt,         // >
    Ge,         // >=
    BitAnd,     // &
    BitOr,      // |
    BitXor,     // ^
    Shl,        // <<
    Shr,        // >>
    UShr,       // >>>
    In,         // in
    InstanceOf, // instanceof
}

/// Logical operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalOp {
    And,             // &&
    Or,              // ||
    NullishCoalesce, // ??
}

/// Assignment operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOp {
    Assign,        // =
    AddAssign,     // +=
    SubAssign,     // -=
    MulAssign,     // *=
    DivAssign,     // /=
    ModAssign,     // %=
    ExpAssign,     // **=
    BitAndAssign,  // &=
    BitOrAssign,   // |=
    BitXorAssign,  // ^=
    ShlAssign,     // <<=
    ShrAssign,     // >>=
    UShrAssign,    // >>>=
    AndAssign,     // &&=
    OrAssign,      // ||=
    NullishAssign, // ??=
}

/// Return a summary of an expression tree (for diagnostics). Depth-limited.
pub fn expr_summary(expr: &Expr, depth: usize) -> String {
    if depth > 5 {
        return String::from("...");
    }
    match expr {
        Expr::Number(n) => alloc::format!("Number({})", n),
        Expr::String(s) => alloc::format!("String({:?})", &s[..s.len().min(30)]),
        Expr::Bool(b) => alloc::format!("Bool({})", b),
        Expr::Null => String::from("Null"),
        Expr::Undefined => String::from("Undefined"),
        Expr::Ident(name) => alloc::format!("Ident({})", name),
        Expr::Unary { op, argument, .. } => {
            alloc::format!("Unary({:?}, {})", op, expr_summary(argument, depth + 1))
        }
        Expr::Binary { op, left, right } => alloc::format!(
            "Binary({:?}, {}, {})",
            op,
            expr_summary(left, depth + 1),
            expr_summary(right, depth + 1)
        ),
        Expr::Call { callee, arguments } => alloc::format!(
            "Call({}, {} args)",
            expr_summary(callee, depth + 1),
            arguments.len()
        ),
        Expr::FunctionExpr { name, params, .. } => alloc::format!(
            "FunctionExpr({}, {} params)",
            name.as_deref().unwrap_or("anon"),
            params.len()
        ),
        Expr::Arrow { params, .. } => alloc::format!("Arrow({} params)", params.len()),
        Expr::Member {
            object,
            property,
            computed,
        } => {
            if *computed {
                alloc::format!("Member({}, [computed])", expr_summary(object, depth + 1))
            } else {
                alloc::format!("Member({}, .{})", expr_summary(object, depth + 1), property)
            }
        }
        Expr::Assign { op, left, right } => alloc::format!(
            "Assign({:?}, {}, {})",
            op,
            expr_summary(left, depth + 1),
            expr_summary(right, depth + 1)
        ),
        Expr::Sequence(exprs) => alloc::format!("Sequence({} exprs)", exprs.len()),
        _ => alloc::format!("{:?}", core::mem::discriminant(expr)),
    }
}

/// Return a short string naming the Stmt variant (for diagnostics).
pub fn stmt_variant_name(stmt: &Stmt) -> &'static str {
    match stmt {
        Stmt::Expr(_) => "Expr",
        Stmt::VarDecl { .. } => "VarDecl",
        Stmt::Block(_) => "Block",
        Stmt::If { .. } => "If",
        Stmt::While { .. } => "While",
        Stmt::DoWhile { .. } => "DoWhile",
        Stmt::For { .. } => "For",
        Stmt::ForIn { .. } => "ForIn",
        Stmt::ForOf { .. } => "ForOf",
        Stmt::Return(_) => "Return",
        Stmt::Break(_) => "Break",
        Stmt::Continue(_) => "Continue",
        Stmt::Switch { .. } => "Switch",
        Stmt::Throw(_) => "Throw",
        Stmt::Try { .. } => "Try",
        Stmt::FunctionDecl { .. } => "FunctionDecl",
        Stmt::ClassDecl { .. } => "ClassDecl",
        Stmt::Labeled { .. } => "Labeled",
        Stmt::Empty => "Empty",
        Stmt::Debugger => "Debugger",
        Stmt::Import { .. } => "Import",
        Stmt::Export(_) => "Export",
        Stmt::With { .. } => "With",
    }
}
