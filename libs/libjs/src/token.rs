//! JavaScript token types for the lexer.

use alloc::string::String;

/// Source position for error reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub line: u32,
}

/// A token produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

/// All JavaScript token types.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    Number(f64),
    String(String),
    Template(String),      // template literal segment
    RegExp(String, String), // pattern, flags
    Bool(bool),
    Null,
    Undefined,

    // Identifiers and keywords
    Ident(String),

    // Keywords
    Var,
    Let,
    Const,
    Function,
    Return,
    If,
    Else,
    While,
    For,
    Do,
    Break,
    Continue,
    Switch,
    Case,
    Default,
    New,
    Delete,
    Typeof,
    Void,
    In,
    Instanceof,
    This,
    Class,
    Extends,
    Super,
    Import,
    Export,
    From,
    As,
    Try,
    Catch,
    Finally,
    Throw,
    Yield,
    Async,
    Await,
    Of,
    With,
    Debugger,

    // Punctuation
    LParen,     // (
    RParen,     // )
    LBrace,     // {
    RBrace,     // }
    LBracket,   // [
    RBracket,   // ]
    Semicolon,  // ;
    Comma,      // ,
    Dot,        // .
    DotDotDot,  // ...
    Colon,      // :
    Question,   // ?
    QuestionDot, // ?.
    QuestionQuestion, // ??
    Arrow,      // =>

    // Operators
    Plus,       // +
    Minus,      // -
    Star,       // *
    StarStar,   // **
    Slash,      // /
    Percent,    // %
    Amp,        // &
    AmpAmp,     // &&
    Pipe,       // |
    PipePipe,   // ||
    Caret,      // ^
    Tilde,      // ~
    Bang,       // !
    Lt,         // <
    Gt,         // >
    LtEq,      // <=
    GtEq,      // >=
    EqEq,       // ==
    BangEq,     // !=
    EqEqEq,     // ===
    BangEqEq,   // !==
    LtLt,       // <<
    GtGt,       // >>
    GtGtGt,     // >>>

    // Assignment
    Eq,         // =
    PlusEq,     // +=
    MinusEq,    // -=
    StarEq,     // *=
    SlashEq,    // /=
    PercentEq,  // %=
    AmpEq,      // &=
    PipeEq,     // |=
    CaretEq,    // ^=
    LtLtEq,     // <<=
    GtGtEq,     // >>=
    GtGtGtEq,   // >>>=
    StarStarEq,  // **=
    AmpAmpEq,   // &&=
    PipePipeEq,  // ||=
    QuestionQuestionEq, // ??=

    // Increment/Decrement
    PlusPlus,   // ++
    MinusMinus, // --

    // Special
    Eof,
}

impl TokenKind {
    /// Returns true if this token is an assignment operator.
    pub fn is_assignment(&self) -> bool {
        matches!(
            self,
            TokenKind::Eq
                | TokenKind::PlusEq
                | TokenKind::MinusEq
                | TokenKind::StarEq
                | TokenKind::SlashEq
                | TokenKind::PercentEq
                | TokenKind::AmpEq
                | TokenKind::PipeEq
                | TokenKind::CaretEq
                | TokenKind::LtLtEq
                | TokenKind::GtGtEq
                | TokenKind::GtGtGtEq
                | TokenKind::StarStarEq
                | TokenKind::AmpAmpEq
                | TokenKind::PipePipeEq
                | TokenKind::QuestionQuestionEq
        )
    }
}
