#![no_std]
#![no_main]

use anyos_std::String;
use anyos_std::Vec;
use anyos_std::fs;
use anyos_std::format;
use alloc::string::ToString;
use alloc::boxed::Box;
use alloc::vec;

anyos_std::entry!(main);

// ─── Value type ──────────────────────────────────────────────────────────────

#[derive(Clone)]
enum Value {
    Str(String),
    Num(f64),
    Uninit,
}

impl Value {
    fn as_str(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Num(n) => format_float(*n),
            Value::Uninit => String::new(),
        }
    }

    fn as_num(&self) -> f64 {
        match self {
            Value::Num(n) => *n,
            Value::Str(s) => parse_float(s),
            Value::Uninit => 0.0,
        }
    }

    fn is_true(&self) -> bool {
        match self {
            Value::Num(n) => *n != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Uninit => false,
        }
    }
}

fn parse_float(s: &str) -> f64 {
    let s = s.trim();
    if s.is_empty() {
        return 0.0;
    }
    let mut neg = false;
    let mut chars = s.chars().peekable();
    if chars.peek() == Some(&'-') {
        neg = true;
        chars.next();
    } else if chars.peek() == Some(&'+') {
        chars.next();
    }
    let mut int_part: f64 = 0.0;
    while let Some(&c) = chars.peek() {
        if c >= '0' && c <= '9' {
            int_part = int_part * 10.0 + (c as u32 - '0' as u32) as f64;
            chars.next();
        } else {
            break;
        }
    }
    let mut frac_part: f64 = 0.0;
    if chars.peek() == Some(&'.') {
        chars.next();
        let mut divisor = 10.0;
        while let Some(&c) = chars.peek() {
            if c >= '0' && c <= '9' {
                frac_part += (c as u32 - '0' as u32) as f64 / divisor;
                divisor *= 10.0;
                chars.next();
            } else {
                break;
            }
        }
    }
    let result = int_part + frac_part;
    if neg { -result } else { result }
}

fn format_float(n: f64) -> String {
    // Check if it's an integer
    let i = n as i64;
    if (i as f64 - n).abs() < 1e-9 {
        return format!("{}", i);
    }
    // Format with up to 6 decimal places, strip trailing zeros
    let neg = n < 0.0;
    let abs = if neg { -n } else { n };
    let int = abs as u64;
    let frac = ((abs - int as f64) * 1000000.0 + 0.5) as u64;
    let mut s = if neg {
        format!("-{}.{:06}", int, frac)
    } else {
        format!("{}.{:06}", int, frac)
    };
    // Trim trailing zeros after decimal point
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

// ─── Token / Expression types ────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum Token {
    Str(String),
    Num(f64),
    Regex(String),
    Field(u32),         // $0, $1, $NF...
    Var(String),
    Plus, Minus, Star, Slash, Percent,
    Eq, Ne, Lt, Gt, Le, Ge,
    Match, NotMatch,    // ~ !~
    And, Or, Not,
    Assign, PlusAssign, MinusAssign, StarAssign, SlashAssign,
    Incr, Decr,         // ++ --
    Concat,             // implicit concatenation
    LParen, RParen,
    LBrace, RBrace,
    Semi, Comma, Pipe,
    If, Else, While, For, Do,
    Print, Printf, Getline,
    Begin, End,
    Next, Exit,
    In,                 // "in" keyword for arrays
    Delete,
    LBracket, RBracket,
    Length, Substr, Index, Split, Sub, Gsub, Sprintf, Tolower, Toupper,
    Sin, Cos, Sqrt, Log, Exp, Int, Rand, Srand,
    Eof,
}

// ─── Lexer ───────────────────────────────────────────────────────────────────

struct Lexer {
    src: Vec<char>,
    pos: usize,
}

impl Lexer {
    fn new(src: &str) -> Self {
        Lexer { src: src.chars().collect(), pos: 0 }
    }

    fn peek_char(&self) -> Option<char> {
        self.src.get(self.pos).copied()
    }

    fn next_char(&mut self) -> Option<char> {
        let c = self.src.get(self.pos).copied();
        if c.is_some() { self.pos += 1; }
        c
    }

    fn skip_ws_and_comments(&mut self) {
        while let Some(c) = self.peek_char() {
            if c == ' ' || c == '\t' || c == '\r' {
                self.next_char();
            } else if c == '#' {
                while let Some(c2) = self.next_char() {
                    if c2 == '\n' { break; }
                }
            } else {
                break;
            }
        }
    }

    fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();
        loop {
            self.skip_ws_and_comments();
            let ch = match self.peek_char() {
                Some(c) => c,
                None => { tokens.push(Token::Eof); break; }
            };

            match ch {
                '\n' | ';' => { self.next_char(); tokens.push(Token::Semi); }
                '{' => { self.next_char(); tokens.push(Token::LBrace); }
                '}' => { self.next_char(); tokens.push(Token::RBrace); }
                '(' => { self.next_char(); tokens.push(Token::LParen); }
                ')' => { self.next_char(); tokens.push(Token::RParen); }
                '[' => { self.next_char(); tokens.push(Token::LBracket); }
                ']' => { self.next_char(); tokens.push(Token::RBracket); }
                ',' => { self.next_char(); tokens.push(Token::Comma); }
                '|' => {
                    self.next_char();
                    if self.peek_char() == Some('|') { self.next_char(); tokens.push(Token::Or); }
                    else { tokens.push(Token::Pipe); }
                }
                '+' => {
                    self.next_char();
                    if self.peek_char() == Some('+') { self.next_char(); tokens.push(Token::Incr); }
                    else if self.peek_char() == Some('=') { self.next_char(); tokens.push(Token::PlusAssign); }
                    else { tokens.push(Token::Plus); }
                }
                '-' => {
                    self.next_char();
                    if self.peek_char() == Some('-') { self.next_char(); tokens.push(Token::Decr); }
                    else if self.peek_char() == Some('=') { self.next_char(); tokens.push(Token::MinusAssign); }
                    else { tokens.push(Token::Minus); }
                }
                '*' => {
                    self.next_char();
                    if self.peek_char() == Some('=') { self.next_char(); tokens.push(Token::StarAssign); }
                    else { tokens.push(Token::Star); }
                }
                '%' => { self.next_char(); tokens.push(Token::Percent); }
                '~' => { self.next_char(); tokens.push(Token::Match); }
                '!' => {
                    self.next_char();
                    if self.peek_char() == Some('=') { self.next_char(); tokens.push(Token::Ne); }
                    else if self.peek_char() == Some('~') { self.next_char(); tokens.push(Token::NotMatch); }
                    else { tokens.push(Token::Not); }
                }
                '=' => {
                    self.next_char();
                    if self.peek_char() == Some('=') { self.next_char(); tokens.push(Token::Eq); }
                    else { tokens.push(Token::Assign); }
                }
                '<' => {
                    self.next_char();
                    if self.peek_char() == Some('=') { self.next_char(); tokens.push(Token::Le); }
                    else { tokens.push(Token::Lt); }
                }
                '>' => {
                    self.next_char();
                    if self.peek_char() == Some('=') { self.next_char(); tokens.push(Token::Ge); }
                    else { tokens.push(Token::Gt); }
                }
                '&' => {
                    self.next_char();
                    if self.peek_char() == Some('&') { self.next_char(); tokens.push(Token::And); }
                }
                '/' => {
                    // Determine if / starts a regex or is division
                    // Regex if preceded by nothing, operator, or keyword
                    let is_regex = if let Some(prev) = tokens.last() {
                        matches!(prev, Token::Semi | Token::LBrace | Token::LParen
                            | Token::Comma | Token::Not | Token::And | Token::Or
                            | Token::Match | Token::NotMatch | Token::Eq | Token::Ne
                            | Token::Lt | Token::Gt | Token::Le | Token::Ge
                            | Token::Plus | Token::Minus | Token::Star | Token::Slash
                            | Token::Percent | Token::Assign | Token::Begin | Token::End
                            | Token::Print | Token::Printf | Token::If | Token::While
                            | Token::For | Token::Do)
                    } else {
                        true
                    };
                    if is_regex {
                        self.next_char(); // consume /
                        let mut pat = String::new();
                        while let Some(c) = self.next_char() {
                            if c == '/' { break; }
                            if c == '\\' {
                                if let Some(esc) = self.next_char() {
                                    pat.push('\\');
                                    pat.push(esc);
                                }
                            } else {
                                pat.push(c);
                            }
                        }
                        tokens.push(Token::Regex(pat));
                    } else {
                        self.next_char();
                        if self.peek_char() == Some('=') { self.next_char(); tokens.push(Token::SlashAssign); }
                        else { tokens.push(Token::Slash); }
                    }
                }
                '$' => {
                    self.next_char();
                    let mut num = 0u32;
                    while let Some(d) = self.peek_char() {
                        if d >= '0' && d <= '9' {
                            num = num * 10 + (d as u32 - '0' as u32);
                            self.next_char();
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token::Field(num));
                }
                '"' => {
                    self.next_char(); // consume "
                    let mut s = String::new();
                    while let Some(c) = self.next_char() {
                        if c == '"' { break; }
                        if c == '\\' {
                            match self.next_char() {
                                Some('n') => s.push('\n'),
                                Some('t') => s.push('\t'),
                                Some('\\') => s.push('\\'),
                                Some('"') => s.push('"'),
                                Some(o) => { s.push('\\'); s.push(o); }
                                None => break,
                            }
                        } else {
                            s.push(c);
                        }
                    }
                    tokens.push(Token::Str(s));
                }
                _ if ch >= '0' && ch <= '9' || ch == '.' => {
                    let mut num_str = String::new();
                    let mut has_dot = false;
                    while let Some(&c) = self.src.get(self.pos) {
                        if c >= '0' && c <= '9' {
                            num_str.push(c);
                            self.pos += 1;
                        } else if c == '.' && !has_dot {
                            has_dot = true;
                            num_str.push(c);
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token::Num(parse_float(&num_str)));
                }
                _ if ch.is_alphabetic() || ch == '_' => {
                    let mut ident = String::new();
                    while let Some(&c) = self.src.get(self.pos) {
                        if c.is_alphanumeric() || c == '_' {
                            ident.push(c);
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    let tok = match ident.as_str() {
                        "BEGIN" => Token::Begin,
                        "END" => Token::End,
                        "if" => Token::If,
                        "else" => Token::Else,
                        "while" => Token::While,
                        "for" => Token::For,
                        "do" => Token::Do,
                        "print" => Token::Print,
                        "printf" => Token::Printf,
                        "getline" => Token::Getline,
                        "next" => Token::Next,
                        "exit" => Token::Exit,
                        "in" => Token::In,
                        "delete" => Token::Delete,
                        "length" => Token::Length,
                        "substr" => Token::Substr,
                        "index" => Token::Index,
                        "split" => Token::Split,
                        "sub" => Token::Sub,
                        "gsub" => Token::Gsub,
                        "sprintf" => Token::Sprintf,
                        "tolower" => Token::Tolower,
                        "toupper" => Token::Toupper,
                        "sin" => Token::Sin,
                        "cos" => Token::Cos,
                        "sqrt" => Token::Sqrt,
                        "log" => Token::Log,
                        "exp" => Token::Exp,
                        "int" => Token::Int,
                        "rand" => Token::Rand,
                        "srand" => Token::Srand,
                        _ => Token::Var(ident),
                    };
                    tokens.push(tok);
                }
                _ => {
                    self.next_char(); // skip unknown
                }
            }
        }
        tokens
    }
}

// ─── AST ─────────────────────────────────────────────────────────────────────

#[derive(Clone)]
enum Expr {
    Num(f64),
    Str(String),
    Field(Box<Expr>),
    Var(String),
    ArrayRef(String, Box<Expr>),
    BinOp(Box<Expr>, BinOp, Box<Expr>),
    UnaryOp(UnaryOp, Box<Expr>),
    Assign(Box<Expr>, Box<Expr>),
    CompoundAssign(Box<Expr>, BinOp, Box<Expr>),
    Incr(Box<Expr>, bool),  // bool = pre (true) or post (false)
    Decr(Box<Expr>, bool),
    Match(Box<Expr>, String),
    NotMatch(Box<Expr>, String),
    Concat(Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
    Getline,
    In(String, Box<Expr>),
}

#[derive(Clone, Copy)]
enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or,
}

#[derive(Clone, Copy)]
enum UnaryOp {
    Neg, Not,
}

#[derive(Clone)]
enum Stmt {
    ExprStmt(Expr),
    Print(Vec<Expr>, Option<String>),   // exprs, optional output file
    Printf(Vec<Expr>, Option<String>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    For(Box<Stmt>, Expr, Box<Stmt>, Box<Stmt>),
    ForIn(String, String, Box<Stmt>),
    Block(Vec<Stmt>),
    Next,
    Exit(Option<Expr>),
    Delete(String, Expr),
}

#[derive(Clone)]
enum Pattern {
    Begin,
    End,
    Expr(Expr),
    Regex(String),
    All,    // no pattern = match everything
}

#[derive(Clone)]
struct Rule {
    pattern: Pattern,
    action: Vec<Stmt>,
}

// ─── Parser ──────────────────────────────────────────────────────────────────

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or(&Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        t
    }

    fn expect(&mut self, expected: &Token) -> bool {
        if core::mem::discriminant(self.peek()) == core::mem::discriminant(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn skip_semis(&mut self) {
        while *self.peek() == Token::Semi {
            self.advance();
        }
    }

    fn parse_program(&mut self) -> Vec<Rule> {
        let mut rules = Vec::new();
        self.skip_semis();
        while *self.peek() != Token::Eof {
            if let Some(rule) = self.parse_rule() {
                rules.push(rule);
            }
            self.skip_semis();
        }
        rules
    }

    fn parse_rule(&mut self) -> Option<Rule> {
        let pattern = match self.peek().clone() {
            Token::Begin => { self.advance(); Pattern::Begin }
            Token::End => { self.advance(); Pattern::End }
            Token::LBrace => Pattern::All,
            Token::Regex(r) => { self.advance(); Pattern::Regex(r) }
            _ => {
                // Could be expression pattern or action-only
                if *self.peek() == Token::Eof {
                    return None;
                }
                let expr = self.parse_expr();
                Pattern::Expr(expr)
            }
        };

        let action = if *self.peek() == Token::LBrace {
            self.parse_block()
        } else {
            // Pattern without action: default is { print $0 }
            match &pattern {
                Pattern::Begin | Pattern::End => {
                    // BEGIN/END require a block
                    vec![Stmt::Print(vec![Expr::Field(Box::new(Expr::Num(0.0)))], None)]
                }
                _ => {
                    vec![Stmt::Print(vec![Expr::Field(Box::new(Expr::Num(0.0)))], None)]
                }
            }
        };

        Some(Rule { pattern, action })
    }

    fn parse_block(&mut self) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        self.expect(&Token::LBrace);
        self.skip_semis();
        while *self.peek() != Token::RBrace && *self.peek() != Token::Eof {
            stmts.push(self.parse_stmt());
            self.skip_semis();
        }
        self.expect(&Token::RBrace);
        stmts
    }

    fn parse_stmt(&mut self) -> Stmt {
        match self.peek().clone() {
            Token::Print => {
                self.advance();
                let mut exprs = Vec::new();
                let mut output_file = None;
                if *self.peek() != Token::Semi && *self.peek() != Token::RBrace
                    && *self.peek() != Token::Eof && *self.peek() != Token::Pipe
                {
                    exprs.push(self.parse_expr());
                    while *self.peek() == Token::Comma {
                        self.advance();
                        exprs.push(self.parse_expr());
                    }
                }
                // Handle > redirect in print
                if *self.peek() == Token::Gt {
                    self.advance();
                    let file_expr = self.parse_expr();
                    // For simplicity, evaluate as string later
                    output_file = match file_expr {
                        Expr::Str(s) => Some(s),
                        Expr::Var(v) => Some(v), // will resolve at runtime
                        _ => None,
                    };
                }
                if exprs.is_empty() {
                    exprs.push(Expr::Field(Box::new(Expr::Num(0.0))));
                }
                Stmt::Print(exprs, output_file)
            }
            Token::Printf => {
                self.advance();
                let mut exprs = Vec::new();
                let mut output_file = None;
                exprs.push(self.parse_expr());
                while *self.peek() == Token::Comma {
                    self.advance();
                    exprs.push(self.parse_expr());
                }
                if *self.peek() == Token::Gt {
                    self.advance();
                    let file_expr = self.parse_expr();
                    output_file = match file_expr {
                        Expr::Str(s) => Some(s),
                        Expr::Var(v) => Some(v),
                        _ => None,
                    };
                }
                Stmt::Printf(exprs, output_file)
            }
            Token::If => {
                self.advance();
                self.expect(&Token::LParen);
                let cond = self.parse_expr();
                self.expect(&Token::RParen);
                self.skip_semis();
                let then_stmt = self.parse_stmt();
                self.skip_semis();
                let else_stmt = if *self.peek() == Token::Else {
                    self.advance();
                    self.skip_semis();
                    Some(Box::new(self.parse_stmt()))
                } else {
                    None
                };
                Stmt::If(cond, Box::new(then_stmt), else_stmt)
            }
            Token::While => {
                self.advance();
                self.expect(&Token::LParen);
                let cond = self.parse_expr();
                self.expect(&Token::RParen);
                self.skip_semis();
                let body = self.parse_stmt();
                Stmt::While(cond, Box::new(body))
            }
            Token::Do => {
                self.advance();
                self.skip_semis();
                let body = self.parse_stmt();
                self.skip_semis();
                self.expect(&Token::While);
                self.expect(&Token::LParen);
                let cond = self.parse_expr();
                self.expect(&Token::RParen);
                Stmt::DoWhile(Box::new(body), cond)
            }
            Token::For => {
                self.advance();
                self.expect(&Token::LParen);
                // Check for for(var in array) form
                if let Token::Var(v) = self.peek().clone() {
                    let saved = self.pos;
                    self.advance();
                    if *self.peek() == Token::In {
                        self.advance();
                        if let Token::Var(arr) = self.advance() {
                            self.expect(&Token::RParen);
                            self.skip_semis();
                            let body = self.parse_stmt();
                            return Stmt::ForIn(v, arr, Box::new(body));
                        }
                    }
                    self.pos = saved; // backtrack
                }
                let init = self.parse_stmt();
                self.expect(&Token::Semi);
                let cond = self.parse_expr();
                self.expect(&Token::Semi);
                let update = self.parse_stmt();
                self.expect(&Token::RParen);
                self.skip_semis();
                let body = self.parse_stmt();
                Stmt::For(Box::new(init), cond, Box::new(update), Box::new(body))
            }
            Token::Next => { self.advance(); Stmt::Next }
            Token::Exit => {
                self.advance();
                let code = if *self.peek() != Token::Semi && *self.peek() != Token::RBrace {
                    Some(self.parse_expr())
                } else {
                    None
                };
                Stmt::Exit(code)
            }
            Token::Delete => {
                self.advance();
                if let Token::Var(name) = self.advance() {
                    if *self.peek() == Token::LBracket {
                        self.advance();
                        let idx = self.parse_expr();
                        self.expect(&Token::RBracket);
                        Stmt::Delete(name, idx)
                    } else {
                        Stmt::Delete(name, Expr::Str(String::new()))
                    }
                } else {
                    Stmt::ExprStmt(Expr::Num(0.0))
                }
            }
            Token::LBrace => {
                let stmts = self.parse_block();
                Stmt::Block(stmts)
            }
            _ => {
                let expr = self.parse_expr();
                Stmt::ExprStmt(expr)
            }
        }
    }

    fn parse_expr(&mut self) -> Expr {
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> Expr {
        let lhs = self.parse_or();
        match self.peek().clone() {
            Token::Assign => { self.advance(); let rhs = self.parse_assign(); Expr::Assign(Box::new(lhs), Box::new(rhs)) }
            Token::PlusAssign => { self.advance(); let rhs = self.parse_assign(); Expr::CompoundAssign(Box::new(lhs), BinOp::Add, Box::new(rhs)) }
            Token::MinusAssign => { self.advance(); let rhs = self.parse_assign(); Expr::CompoundAssign(Box::new(lhs), BinOp::Sub, Box::new(rhs)) }
            Token::StarAssign => { self.advance(); let rhs = self.parse_assign(); Expr::CompoundAssign(Box::new(lhs), BinOp::Mul, Box::new(rhs)) }
            Token::SlashAssign => { self.advance(); let rhs = self.parse_assign(); Expr::CompoundAssign(Box::new(lhs), BinOp::Div, Box::new(rhs)) }
            _ => lhs,
        }
    }

    fn parse_or(&mut self) -> Expr {
        let mut lhs = self.parse_and();
        while *self.peek() == Token::Or {
            self.advance();
            let rhs = self.parse_and();
            lhs = Expr::BinOp(Box::new(lhs), BinOp::Or, Box::new(rhs));
        }
        lhs
    }

    fn parse_and(&mut self) -> Expr {
        let mut lhs = self.parse_match();
        while *self.peek() == Token::And {
            self.advance();
            let rhs = self.parse_match();
            lhs = Expr::BinOp(Box::new(lhs), BinOp::And, Box::new(rhs));
        }
        lhs
    }

    fn parse_match(&mut self) -> Expr {
        let lhs = self.parse_comparison();
        match self.peek().clone() {
            Token::Match => {
                self.advance();
                if let Token::Regex(r) = self.advance() {
                    Expr::Match(Box::new(lhs), r)
                } else {
                    lhs
                }
            }
            Token::NotMatch => {
                self.advance();
                if let Token::Regex(r) = self.advance() {
                    Expr::NotMatch(Box::new(lhs), r)
                } else {
                    lhs
                }
            }
            _ => lhs,
        }
    }

    fn parse_comparison(&mut self) -> Expr {
        let lhs = self.parse_concat();
        match self.peek().clone() {
            Token::Eq => { self.advance(); Expr::BinOp(Box::new(lhs), BinOp::Eq, Box::new(self.parse_concat())) }
            Token::Ne => { self.advance(); Expr::BinOp(Box::new(lhs), BinOp::Ne, Box::new(self.parse_concat())) }
            Token::Lt => { self.advance(); Expr::BinOp(Box::new(lhs), BinOp::Lt, Box::new(self.parse_concat())) }
            Token::Gt => { self.advance(); Expr::BinOp(Box::new(lhs), BinOp::Gt, Box::new(self.parse_concat())) }
            Token::Le => { self.advance(); Expr::BinOp(Box::new(lhs), BinOp::Le, Box::new(self.parse_concat())) }
            Token::Ge => { self.advance(); Expr::BinOp(Box::new(lhs), BinOp::Ge, Box::new(self.parse_concat())) }
            _ => lhs,
        }
    }

    fn parse_concat(&mut self) -> Expr {
        let mut lhs = self.parse_add();
        // Concatenation: two expressions next to each other without an operator
        loop {
            match self.peek() {
                Token::Str(_) | Token::Num(_) | Token::Var(_) | Token::Field(_)
                | Token::LParen | Token::Not | Token::Minus
                | Token::Length | Token::Substr | Token::Index | Token::Split
                | Token::Sub | Token::Gsub | Token::Sprintf | Token::Tolower | Token::Toupper
                | Token::Sin | Token::Cos | Token::Sqrt | Token::Log | Token::Exp
                | Token::Int | Token::Rand | Token::Srand | Token::Incr | Token::Decr => {
                    let rhs = self.parse_add();
                    lhs = Expr::Concat(Box::new(lhs), Box::new(rhs));
                }
                _ => break,
            }
        }
        lhs
    }

    fn parse_add(&mut self) -> Expr {
        let mut lhs = self.parse_mul();
        loop {
            match self.peek() {
                Token::Plus => { self.advance(); lhs = Expr::BinOp(Box::new(lhs), BinOp::Add, Box::new(self.parse_mul())); }
                Token::Minus => { self.advance(); lhs = Expr::BinOp(Box::new(lhs), BinOp::Sub, Box::new(self.parse_mul())); }
                _ => break,
            }
        }
        lhs
    }

    fn parse_mul(&mut self) -> Expr {
        let mut lhs = self.parse_unary();
        loop {
            match self.peek() {
                Token::Star => { self.advance(); lhs = Expr::BinOp(Box::new(lhs), BinOp::Mul, Box::new(self.parse_unary())); }
                Token::Slash => { self.advance(); lhs = Expr::BinOp(Box::new(lhs), BinOp::Div, Box::new(self.parse_unary())); }
                Token::Percent => { self.advance(); lhs = Expr::BinOp(Box::new(lhs), BinOp::Mod, Box::new(self.parse_unary())); }
                _ => break,
            }
        }
        lhs
    }

    fn parse_unary(&mut self) -> Expr {
        match self.peek().clone() {
            Token::Minus => { self.advance(); Expr::UnaryOp(UnaryOp::Neg, Box::new(self.parse_unary())) }
            Token::Not => { self.advance(); Expr::UnaryOp(UnaryOp::Not, Box::new(self.parse_unary())) }
            Token::Incr => {
                self.advance();
                let expr = self.parse_postfix();
                Expr::Incr(Box::new(expr), true)
            }
            Token::Decr => {
                self.advance();
                let expr = self.parse_postfix();
                Expr::Decr(Box::new(expr), true)
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_primary();
        loop {
            match self.peek().clone() {
                Token::Incr => { self.advance(); expr = Expr::Incr(Box::new(expr), false); }
                Token::Decr => { self.advance(); expr = Expr::Decr(Box::new(expr), false); }
                Token::LBracket => {
                    self.advance();
                    let idx = self.parse_expr();
                    self.expect(&Token::RBracket);
                    if let Expr::Var(name) = expr {
                        expr = Expr::ArrayRef(name, Box::new(idx));
                    }
                }
                _ => break,
            }
        }
        expr
    }

    fn parse_primary(&mut self) -> Expr {
        match self.peek().clone() {
            Token::Num(n) => { self.advance(); Expr::Num(n) }
            Token::Str(s) => { self.advance(); Expr::Str(s) }
            Token::Field(n) => { self.advance(); Expr::Field(Box::new(Expr::Num(n as f64))) }
            Token::Var(name) => {
                self.advance();
                // Check for array access
                if *self.peek() == Token::LBracket {
                    self.advance();
                    let idx = self.parse_expr();
                    self.expect(&Token::RBracket);
                    Expr::ArrayRef(name, Box::new(idx))
                } else {
                    Expr::Var(name)
                }
            }
            Token::LParen => {
                self.advance();
                let e = self.parse_expr();
                self.expect(&Token::RParen);
                e
            }
            Token::Getline => { self.advance(); Expr::Getline }
            // Built-in functions
            Token::Length | Token::Substr | Token::Index | Token::Split
            | Token::Sub | Token::Gsub | Token::Sprintf | Token::Tolower | Token::Toupper
            | Token::Sin | Token::Cos | Token::Sqrt | Token::Log | Token::Exp
            | Token::Int | Token::Rand | Token::Srand => {
                let fname = match self.advance() {
                    Token::Length => "length",
                    Token::Substr => "substr",
                    Token::Index => "index",
                    Token::Split => "split",
                    Token::Sub => "sub",
                    Token::Gsub => "gsub",
                    Token::Sprintf => "sprintf",
                    Token::Tolower => "tolower",
                    Token::Toupper => "toupper",
                    Token::Sin => "sin",
                    Token::Cos => "cos",
                    Token::Sqrt => "sqrt",
                    Token::Log => "log",
                    Token::Exp => "exp",
                    Token::Int => "int",
                    Token::Rand => "rand",
                    Token::Srand => "srand",
                    _ => "",
                };
                let mut args = Vec::new();
                if *self.peek() == Token::LParen {
                    self.advance();
                    if *self.peek() != Token::RParen {
                        args.push(self.parse_expr());
                        while *self.peek() == Token::Comma {
                            self.advance();
                            args.push(self.parse_expr());
                        }
                    }
                    self.expect(&Token::RParen);
                }
                Expr::Call(String::from(fname), args)
            }
            _ => {
                self.advance();
                Expr::Num(0.0)
            }
        }
    }
}

// ─── Simple regex matcher ────────────────────────────────────────────────────

fn regex_match(text: &str, pattern: &str) -> bool {
    // Simple regex: supports . * + ? ^ $ [] literal chars
    // For a basic AWK, this covers common cases
    if pattern.is_empty() {
        return true;
    }
    let pat_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    // If pattern starts with ^, anchor at start
    if pat_chars[0] == '^' {
        return regex_match_at(&text_chars, 0, &pat_chars, 1);
    }

    // Try matching at every position
    for i in 0..=text_chars.len() {
        if regex_match_at(&text_chars, i, &pat_chars, 0) {
            return true;
        }
    }
    false
}

fn regex_match_at(text: &[char], ti: usize, pat: &[char], pi: usize) -> bool {
    if pi >= pat.len() {
        return true; // pattern exhausted = match
    }

    // Handle $ anchor at end
    if pat[pi] == '$' && pi + 1 >= pat.len() {
        return ti >= text.len();
    }

    // Check for quantifiers after current pattern element
    let has_star = pi + 1 < pat.len() && pat[pi + 1] == '*';
    let has_plus = pi + 1 < pat.len() && pat[pi + 1] == '+';
    let has_question = pi + 1 < pat.len() && pat[pi + 1] == '?';

    if has_star {
        // Match zero or more
        let mut t = ti;
        // Try matching zero first, then more
        if regex_match_at(text, t, pat, pi + 2) {
            return true;
        }
        while t < text.len() && char_matches(text[t], pat, pi) {
            t += 1;
            if regex_match_at(text, t, pat, pi + 2) {
                return true;
            }
        }
        return false;
    }

    if has_plus {
        // Match one or more
        let mut t = ti;
        while t < text.len() && char_matches(text[t], pat, pi) {
            t += 1;
            if regex_match_at(text, t, pat, pi + 2) {
                return true;
            }
        }
        return false;
    }

    if has_question {
        // Match zero or one
        if regex_match_at(text, ti, pat, pi + 2) {
            return true;
        }
        if ti < text.len() && char_matches(text[ti], pat, pi) {
            return regex_match_at(text, ti + 1, pat, pi + 2);
        }
        return false;
    }

    // Handle character class [...]
    if pat[pi] == '[' {
        let (matches, end_pi) = match_char_class(text.get(ti).copied(), pat, pi);
        if ti < text.len() && matches {
            return regex_match_at(text, ti + 1, pat, end_pi);
        }
        return false;
    }

    // Handle escape
    if pat[pi] == '\\' && pi + 1 < pat.len() {
        if ti < text.len() && text[ti] == pat[pi + 1] {
            return regex_match_at(text, ti + 1, pat, pi + 2);
        }
        return false;
    }

    // Handle . (any char)
    if pat[pi] == '.' {
        if ti < text.len() {
            return regex_match_at(text, ti + 1, pat, pi + 1);
        }
        return false;
    }

    // Literal match
    if ti < text.len() && text[ti] == pat[pi] {
        return regex_match_at(text, ti + 1, pat, pi + 1);
    }

    false
}

fn char_matches(ch: char, pat: &[char], pi: usize) -> bool {
    if pat[pi] == '.' {
        return true;
    }
    if pat[pi] == '[' {
        let (matches, _) = match_char_class(Some(ch), pat, pi);
        return matches;
    }
    if pat[pi] == '\\' && pi + 1 < pat.len() {
        return ch == pat[pi + 1];
    }
    ch == pat[pi]
}

fn match_char_class(ch: Option<char>, pat: &[char], pi: usize) -> (bool, usize) {
    // pi points to '['
    let ch = match ch {
        Some(c) => c,
        None => return (false, pi + 1),
    };
    let mut i = pi + 1;
    let negate = i < pat.len() && pat[i] == '^';
    if negate { i += 1; }
    let mut matched = false;
    while i < pat.len() && pat[i] != ']' {
        if i + 2 < pat.len() && pat[i + 1] == '-' {
            // Range a-z
            if ch >= pat[i] && ch <= pat[i + 2] {
                matched = true;
            }
            i += 3;
        } else {
            if ch == pat[i] {
                matched = true;
            }
            i += 1;
        }
    }
    let end = if i < pat.len() { i + 1 } else { i }; // skip ']'
    (if negate { !matched } else { matched }, end)
}

// ─── Interpreter ─────────────────────────────────────────────────────────────

struct Interpreter {
    vars: Vec<(String, Value)>,
    arrays: Vec<(String, Vec<(String, Value)>)>,
    fields: Vec<String>,
    nr: u32,
    fnr: u32,
    fs: String,
    ofs: String,
    ors: String,
    nf: u32,
    rng_state: u32,
}

impl Interpreter {
    fn new() -> Self {
        Interpreter {
            vars: Vec::new(),
            arrays: Vec::new(),
            fields: Vec::new(),
            nr: 0,
            fnr: 0,
            fs: String::from(" "),
            ofs: String::from(" "),
            ors: String::from("\n"),
            nf: 0,
            rng_state: 12345,
        }
    }

    fn get_var(&self, name: &str) -> Value {
        match name {
            "NR" => Value::Num(self.nr as f64),
            "FNR" => Value::Num(self.fnr as f64),
            "NF" => Value::Num(self.nf as f64),
            "FS" => Value::Str(self.fs.clone()),
            "OFS" => Value::Str(self.ofs.clone()),
            "ORS" => Value::Str(self.ors.clone()),
            "FILENAME" => {
                self.vars.iter().find(|(n, _)| n == "FILENAME")
                    .map(|(_, v)| v.clone()).unwrap_or(Value::Uninit)
            }
            _ => {
                self.vars.iter().find(|(n, _)| n == name)
                    .map(|(_, v)| v.clone()).unwrap_or(Value::Uninit)
            }
        }
    }

    fn set_var(&mut self, name: &str, val: Value) {
        match name {
            "NR" => self.nr = val.as_num() as u32,
            "FNR" => self.fnr = val.as_num() as u32,
            "NF" => self.nf = val.as_num() as u32,
            "FS" => self.fs = val.as_str(),
            "OFS" => self.ofs = val.as_str(),
            "ORS" => self.ors = val.as_str(),
            _ => {
                if let Some(existing) = self.vars.iter_mut().find(|(n, _)| n == name) {
                    existing.1 = val;
                } else {
                    self.vars.push((String::from(name), val));
                }
            }
        }
    }

    fn get_array(&self, name: &str, key: &str) -> Value {
        if let Some((_, entries)) = self.arrays.iter().find(|(n, _)| n == name) {
            entries.iter().find(|(k, _)| k == key)
                .map(|(_, v)| v.clone()).unwrap_or(Value::Uninit)
        } else {
            Value::Uninit
        }
    }

    fn set_array(&mut self, name: &str, key: &str, val: Value) {
        if let Some((_, entries)) = self.arrays.iter_mut().find(|(n, _)| n == name) {
            if let Some(entry) = entries.iter_mut().find(|(k, _)| k == key) {
                entry.1 = val;
            } else {
                entries.push((String::from(key), val));
            }
        } else {
            self.arrays.push((String::from(name), alloc::vec![(String::from(key), val)]));
        }
    }

    fn delete_array(&mut self, name: &str, key: &str) {
        if key.is_empty() {
            // Delete entire array
            self.arrays.retain(|(n, _)| n != name);
        } else if let Some((_, entries)) = self.arrays.iter_mut().find(|(n, _)| n == name) {
            entries.retain(|(k, _)| k != key);
        }
    }

    fn set_record(&mut self, line: &str) {
        self.fields.clear();
        self.fields.push(String::from(line)); // $0

        if self.fs == " " {
            // Default: split on whitespace
            for part in line.split_whitespace() {
                self.fields.push(String::from(part));
            }
        } else if self.fs.len() == 1 {
            let sep = self.fs.chars().next().unwrap();
            for part in line.split(sep) {
                self.fields.push(String::from(part));
            }
        } else {
            self.fields.push(String::from(line));
        }
        self.nf = (self.fields.len() - 1) as u32;
    }

    fn get_field(&self, n: u32) -> String {
        if (n as usize) < self.fields.len() {
            self.fields[n as usize].clone()
        } else {
            String::new()
        }
    }

    fn set_field(&mut self, n: u32, val: &str) {
        let idx = n as usize;
        while self.fields.len() <= idx {
            self.fields.push(String::new());
        }
        self.fields[idx] = String::from(val);
        if n > 0 {
            self.nf = (self.fields.len() - 1) as u32;
            // Rebuild $0
            let mut line = String::new();
            for i in 1..self.fields.len() {
                if i > 1 { line.push_str(&self.ofs); }
                line.push_str(&self.fields[i]);
            }
            self.fields[0] = line;
        }
    }

    fn eval_expr(&mut self, expr: &Expr) -> Value {
        match expr {
            Expr::Num(n) => Value::Num(*n),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Field(e) => {
                let n = self.eval_expr(e).as_num() as u32;
                Value::Str(self.get_field(n))
            }
            Expr::Var(name) => self.get_var(name),
            Expr::ArrayRef(name, idx) => {
                let key = self.eval_expr(idx).as_str();
                self.get_array(name, &key)
            }
            Expr::BinOp(lhs, op, rhs) => {
                let l = self.eval_expr(lhs);
                let r = self.eval_expr(rhs);
                match op {
                    BinOp::Add => Value::Num(l.as_num() + r.as_num()),
                    BinOp::Sub => Value::Num(l.as_num() - r.as_num()),
                    BinOp::Mul => Value::Num(l.as_num() * r.as_num()),
                    BinOp::Div => {
                        let d = r.as_num();
                        if d == 0.0 { Value::Num(0.0) } else { Value::Num(l.as_num() / d) }
                    }
                    BinOp::Mod => {
                        let d = r.as_num();
                        if d == 0.0 { Value::Num(0.0) } else { Value::Num(l.as_num() % d) }
                    }
                    BinOp::Eq => {
                        // String comparison if both look like strings
                        let ls = l.as_str();
                        let rs = r.as_str();
                        Value::Num(if ls == rs { 1.0 } else { 0.0 })
                    }
                    BinOp::Ne => {
                        let ls = l.as_str();
                        let rs = r.as_str();
                        Value::Num(if ls != rs { 1.0 } else { 0.0 })
                    }
                    BinOp::Lt => Value::Num(if l.as_num() < r.as_num() { 1.0 } else { 0.0 }),
                    BinOp::Gt => Value::Num(if l.as_num() > r.as_num() { 1.0 } else { 0.0 }),
                    BinOp::Le => Value::Num(if l.as_num() <= r.as_num() { 1.0 } else { 0.0 }),
                    BinOp::Ge => Value::Num(if l.as_num() >= r.as_num() { 1.0 } else { 0.0 }),
                    BinOp::And => Value::Num(if l.is_true() && r.is_true() { 1.0 } else { 0.0 }),
                    BinOp::Or => Value::Num(if l.is_true() || r.is_true() { 1.0 } else { 0.0 }),
                }
            }
            Expr::UnaryOp(op, e) => {
                let v = self.eval_expr(e);
                match op {
                    UnaryOp::Neg => Value::Num(-v.as_num()),
                    UnaryOp::Not => Value::Num(if v.is_true() { 0.0 } else { 1.0 }),
                }
            }
            Expr::Assign(target, val) => {
                let v = self.eval_expr(val);
                self.assign_to(target, v.clone());
                v
            }
            Expr::CompoundAssign(target, op, val) => {
                let current = self.eval_expr(target);
                let rhs = self.eval_expr(val);
                let result = match op {
                    BinOp::Add => Value::Num(current.as_num() + rhs.as_num()),
                    BinOp::Sub => Value::Num(current.as_num() - rhs.as_num()),
                    BinOp::Mul => Value::Num(current.as_num() * rhs.as_num()),
                    BinOp::Div => {
                        let d = rhs.as_num();
                        Value::Num(if d == 0.0 { 0.0 } else { current.as_num() / d })
                    }
                    _ => Value::Num(current.as_num()),
                };
                self.assign_to(target, result.clone());
                result
            }
            Expr::Incr(e, pre) => {
                let v = self.eval_expr(e).as_num();
                let new_v = Value::Num(v + 1.0);
                self.assign_to(e, new_v);
                if *pre { Value::Num(v + 1.0) } else { Value::Num(v) }
            }
            Expr::Decr(e, pre) => {
                let v = self.eval_expr(e).as_num();
                let new_v = Value::Num(v - 1.0);
                self.assign_to(e, new_v);
                if *pre { Value::Num(v - 1.0) } else { Value::Num(v) }
            }
            Expr::Match(e, pat) => {
                let s = self.eval_expr(e).as_str();
                Value::Num(if regex_match(&s, pat) { 1.0 } else { 0.0 })
            }
            Expr::NotMatch(e, pat) => {
                let s = self.eval_expr(e).as_str();
                Value::Num(if !regex_match(&s, pat) { 1.0 } else { 0.0 })
            }
            Expr::Concat(a, b) => {
                let sa = self.eval_expr(a).as_str();
                let sb = self.eval_expr(b).as_str();
                Value::Str(format!("{}{}", sa, sb))
            }
            Expr::Call(name, args) => self.call_func(name, args),
            Expr::Getline => Value::Num(0.0),
            Expr::In(name, idx) => {
                let key = self.eval_expr(idx).as_str();
                if let Some((_, entries)) = self.arrays.iter().find(|(n, _)| n == name) {
                    Value::Num(if entries.iter().any(|(k, _)| k == &key) { 1.0 } else { 0.0 })
                } else {
                    Value::Num(0.0)
                }
            }
        }
    }

    fn assign_to(&mut self, target: &Expr, val: Value) {
        match target {
            Expr::Var(name) => self.set_var(name, val),
            Expr::Field(e) => {
                let n = self.eval_expr(e).as_num() as u32;
                self.set_field(n, &val.as_str());
            }
            Expr::ArrayRef(name, idx) => {
                let key = self.eval_expr(idx).as_str();
                self.set_array(name, &key, val);
            }
            _ => {}
        }
    }

    fn call_func(&mut self, name: &str, args: &[Expr]) -> Value {
        match name {
            "length" => {
                let s = if args.is_empty() {
                    self.get_field(0)
                } else {
                    self.eval_expr(&args[0]).as_str()
                };
                Value::Num(s.len() as f64)
            }
            "substr" => {
                let s = self.eval_expr(&args[0]).as_str();
                let start = (self.eval_expr(&args[1]).as_num() as usize).saturating_sub(1); // 1-indexed
                let len = if args.len() > 2 {
                    self.eval_expr(&args[2]).as_num() as usize
                } else {
                    s.len()
                };
                let end = (start + len).min(s.len());
                if start < s.len() {
                    Value::Str(String::from(&s[start..end]))
                } else {
                    Value::Str(String::new())
                }
            }
            "index" => {
                let s = self.eval_expr(&args[0]).as_str();
                let needle = self.eval_expr(&args[1]).as_str();
                match s.find(&*needle) {
                    Some(pos) => Value::Num((pos + 1) as f64), // 1-indexed
                    None => Value::Num(0.0),
                }
            }
            "split" => {
                let s = self.eval_expr(&args[0]).as_str();
                let arr_name = match &args[1] {
                    Expr::Var(n) => n.clone(),
                    _ => String::from("_split"),
                };
                let sep = if args.len() > 2 {
                    self.eval_expr(&args[2]).as_str()
                } else {
                    self.fs.clone()
                };
                // Clear array
                self.delete_array(&arr_name, "");
                let parts: Vec<&str> = if sep == " " {
                    s.split_whitespace().collect()
                } else if sep.len() == 1 {
                    s.split(sep.chars().next().unwrap()).collect()
                } else {
                    alloc::vec![s.as_str()]
                };
                for (i, part) in parts.iter().enumerate() {
                    let key = format!("{}", i + 1);
                    self.set_array(&arr_name, &key, Value::Str(String::from(*part)));
                }
                Value::Num(parts.len() as f64)
            }
            "sub" | "gsub" => {
                let pat = self.eval_expr(&args[0]).as_str();
                let repl = self.eval_expr(&args[1]).as_str();
                let target_str = if args.len() > 2 {
                    self.eval_expr(&args[2]).as_str()
                } else {
                    self.get_field(0)
                };
                let global = name == "gsub";
                let (result, count) = regex_sub(&target_str, &pat, &repl, global);
                if args.len() > 2 {
                    self.assign_to(&args[2], Value::Str(result));
                } else {
                    self.set_field(0, &result);
                }
                Value::Num(count as f64)
            }
            "sprintf" => {
                if args.is_empty() {
                    return Value::Str(String::new());
                }
                let fmt = self.eval_expr(&args[0]).as_str();
                let vals: Vec<Value> = args[1..].iter().map(|a| self.eval_expr(a)).collect();
                Value::Str(awk_sprintf(&fmt, &vals))
            }
            "tolower" => {
                let s = self.eval_expr(&args[0]).as_str();
                let mut result = String::new();
                for c in s.chars() {
                    if c >= 'A' && c <= 'Z' {
                        result.push((c as u8 + 32) as char);
                    } else {
                        result.push(c);
                    }
                }
                Value::Str(result)
            }
            "toupper" => {
                let s = self.eval_expr(&args[0]).as_str();
                let mut result = String::new();
                for c in s.chars() {
                    if c >= 'a' && c <= 'z' {
                        result.push((c as u8 - 32) as char);
                    } else {
                        result.push(c);
                    }
                }
                Value::Str(result)
            }
            "sin" => {
                let n = self.eval_expr(&args[0]).as_num();
                Value::Num(libm::sin(n))
            }
            "cos" => {
                let n = self.eval_expr(&args[0]).as_num();
                Value::Num(libm::cos(n))
            }
            "sqrt" => {
                let n = self.eval_expr(&args[0]).as_num();
                Value::Num(libm::sqrt(n))
            }
            "log" => {
                let n = self.eval_expr(&args[0]).as_num();
                Value::Num(libm::log(n))
            }
            "exp" => {
                let n = self.eval_expr(&args[0]).as_num();
                Value::Num(libm::exp(n))
            }
            "int" => {
                let n = self.eval_expr(&args[0]).as_num();
                Value::Num((n as i64) as f64)
            }
            "rand" => {
                // Simple LCG
                self.rng_state = self.rng_state.wrapping_mul(1103515245).wrapping_add(12345);
                let val = ((self.rng_state >> 16) & 0x7FFF) as f64 / 32768.0;
                Value::Num(val)
            }
            "srand" => {
                let old = self.rng_state;
                if !args.is_empty() {
                    self.rng_state = self.eval_expr(&args[0]).as_num() as u32;
                }
                Value::Num(old as f64)
            }
            _ => Value::Uninit,
        }
    }

    fn exec_stmt(&mut self, stmt: &Stmt) -> StmtResult {
        match stmt {
            Stmt::ExprStmt(expr) => {
                self.eval_expr(expr);
                StmtResult::Continue
            }
            Stmt::Print(exprs, output_file) => {
                let mut out = String::new();
                for (i, expr) in exprs.iter().enumerate() {
                    if i > 0 { out.push_str(&self.ofs); }
                    out.push_str(&self.eval_expr(expr).as_str());
                }
                out.push_str(&self.ors);
                if let Some(file) = output_file {
                    let resolved = self.get_var(file).as_str();
                    let target = if resolved.is_empty() { file.as_str() } else { &resolved };
                    let existing = fs::read_to_string(target).unwrap_or_default();
                    let combined = format!("{}{}", existing, out);
                    let _ = fs::write_bytes(target, combined.as_bytes());
                } else {
                    anyos_std::print!("{}", out);
                }
                StmtResult::Continue
            }
            Stmt::Printf(exprs, output_file) => {
                if exprs.is_empty() {
                    return StmtResult::Continue;
                }
                let fmt = self.eval_expr(&exprs[0]).as_str();
                let vals: Vec<Value> = exprs[1..].iter().map(|a| self.eval_expr(a)).collect();
                let out = awk_sprintf(&fmt, &vals);
                if let Some(file) = output_file {
                    let resolved = self.get_var(file).as_str();
                    let target = if resolved.is_empty() { file.as_str() } else { &resolved };
                    let existing = fs::read_to_string(target).unwrap_or_default();
                    let combined = format!("{}{}", existing, out);
                    let _ = fs::write_bytes(target, combined.as_bytes());
                } else {
                    anyos_std::print!("{}", out);
                }
                StmtResult::Continue
            }
            Stmt::If(cond, then_stmt, else_stmt) => {
                let v = self.eval_expr(cond);
                if v.is_true() {
                    self.exec_stmt(then_stmt)
                } else if let Some(els) = else_stmt {
                    self.exec_stmt(els)
                } else {
                    StmtResult::Continue
                }
            }
            Stmt::While(cond, body) => {
                let mut limit = 100000u32;
                while self.eval_expr(cond).is_true() && limit > 0 {
                    limit -= 1;
                    match self.exec_stmt(body) {
                        StmtResult::Next => return StmtResult::Next,
                        StmtResult::Exit(c) => return StmtResult::Exit(c),
                        _ => {}
                    }
                }
                StmtResult::Continue
            }
            Stmt::DoWhile(body, cond) => {
                let mut limit = 100000u32;
                loop {
                    if limit == 0 { break; }
                    limit -= 1;
                    match self.exec_stmt(body) {
                        StmtResult::Next => return StmtResult::Next,
                        StmtResult::Exit(c) => return StmtResult::Exit(c),
                        _ => {}
                    }
                    if !self.eval_expr(cond).is_true() { break; }
                }
                StmtResult::Continue
            }
            Stmt::For(init, cond, update, body) => {
                self.exec_stmt(init);
                let mut limit = 100000u32;
                while self.eval_expr(cond).is_true() && limit > 0 {
                    limit -= 1;
                    match self.exec_stmt(body) {
                        StmtResult::Next => return StmtResult::Next,
                        StmtResult::Exit(c) => return StmtResult::Exit(c),
                        _ => {}
                    }
                    self.exec_stmt(update);
                }
                StmtResult::Continue
            }
            Stmt::ForIn(var, arr_name, body) => {
                let keys: Vec<String> = if let Some((_, entries)) = self.arrays.iter().find(|(n, _)| n == arr_name) {
                    entries.iter().map(|(k, _)| k.clone()).collect()
                } else {
                    Vec::new()
                };
                for key in keys {
                    self.set_var(var, Value::Str(key));
                    match self.exec_stmt(body) {
                        StmtResult::Next => return StmtResult::Next,
                        StmtResult::Exit(c) => return StmtResult::Exit(c),
                        _ => {}
                    }
                }
                StmtResult::Continue
            }
            Stmt::Block(stmts) => {
                for s in stmts {
                    match self.exec_stmt(s) {
                        StmtResult::Continue => {}
                        other => return other,
                    }
                }
                StmtResult::Continue
            }
            Stmt::Next => StmtResult::Next,
            Stmt::Exit(code) => {
                let c = code.as_ref().map(|e| self.eval_expr(e).as_num() as u32).unwrap_or(0);
                StmtResult::Exit(c)
            }
            Stmt::Delete(name, idx) => {
                let key = self.eval_expr(idx).as_str();
                self.delete_array(name, &key);
                StmtResult::Continue
            }
        }
    }

    fn run(&mut self, rules: &[Rule], lines: &[&str]) {
        // Check for -v variable assignments already set
        // Run BEGIN rules
        for rule in rules {
            if let Pattern::Begin = rule.pattern {
                for stmt in &rule.action {
                    match self.exec_stmt(stmt) {
                        StmtResult::Exit(c) => { anyos_std::process::exit(c); }
                        _ => {}
                    }
                }
            }
        }

        // Process each line
        'lines: for line in lines {
            self.nr += 1;
            self.fnr += 1;
            self.set_record(line);

            for rule in rules {
                let matches = match &rule.pattern {
                    Pattern::Begin | Pattern::End => false,
                    Pattern::All => true,
                    Pattern::Regex(pat) => regex_match(line, pat),
                    Pattern::Expr(expr) => self.eval_expr(expr).is_true(),
                };
                if matches {
                    for stmt in &rule.action {
                        match self.exec_stmt(stmt) {
                            StmtResult::Next => continue 'lines,
                            StmtResult::Exit(c) => {
                                // Run END rules before exit
                                self.run_end(rules);
                                anyos_std::process::exit(c);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Run END rules
        self.run_end(rules);
    }

    fn run_end(&mut self, rules: &[Rule]) {
        for rule in rules {
            if let Pattern::End = rule.pattern {
                for stmt in &rule.action {
                    match self.exec_stmt(stmt) {
                        StmtResult::Exit(c) => { anyos_std::process::exit(c); }
                        _ => {}
                    }
                }
            }
        }
    }
}

enum StmtResult {
    Continue,
    Next,
    Exit(u32),
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn regex_sub(text: &str, pattern: &str, replacement: &str, global: bool) -> (String, u32) {
    let text_chars: Vec<char> = text.chars().collect();
    let pat_chars: Vec<char> = pattern.chars().collect();
    let mut result = String::new();
    let mut count = 0u32;
    let mut i = 0;

    while i <= text_chars.len() {
        // Try to match at position i
        let match_len = find_match_length(&text_chars, i, &pat_chars);
        if match_len > 0 {
            result.push_str(replacement);
            count += 1;
            i += match_len;
            if !global {
                // Append rest
                for j in i..text_chars.len() {
                    result.push(text_chars[j]);
                }
                return (result, count);
            }
        } else {
            if i < text_chars.len() {
                result.push(text_chars[i]);
            }
            i += 1;
        }
    }
    (result, count)
}

fn find_match_length(text: &[char], start: usize, pat: &[char]) -> usize {
    // Try to find the shortest match at position start
    if pat.is_empty() { return 0; }
    for len in 1..=(text.len() - start) {
        if regex_match_at(text, start, pat, 0) {
            // Check if the match ends at start+len
            let sub: String = text[start..start+len].iter().collect();
            if regex_match(&sub, &pat.iter().collect::<String>()) {
                return len;
            }
        }
    }
    if regex_match_at(text, start, pat, 0) {
        return 0; // zero-length match possible with some patterns, skip
    }
    0
}

fn awk_sprintf(fmt: &str, vals: &[Value]) -> String {
    let mut result = String::new();
    let mut vi = 0;
    let mut chars = fmt.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            if chars.peek() == Some(&'%') {
                chars.next();
                result.push('%');
                continue;
            }
            // Parse format spec
            let mut width = 0i32;
            let mut left_align = false;
            let mut zero_pad = false;
            let mut precision: Option<u32> = None;

            // Flags
            loop {
                match chars.peek() {
                    Some(&'-') => { left_align = true; chars.next(); }
                    Some(&'0') => { zero_pad = true; chars.next(); }
                    _ => break,
                }
            }
            // Width
            while let Some(&d) = chars.peek() {
                if d >= '0' && d <= '9' {
                    width = width * 10 + (d as i32 - '0' as i32);
                    chars.next();
                } else {
                    break;
                }
            }
            // Precision
            if chars.peek() == Some(&'.') {
                chars.next();
                let mut prec = 0u32;
                while let Some(&d) = chars.peek() {
                    if d >= '0' && d <= '9' {
                        prec = prec * 10 + (d as u32 - '0' as u32);
                        chars.next();
                    } else {
                        break;
                    }
                }
                precision = Some(prec);
            }
            // Conversion char
            let conv = chars.next().unwrap_or('s');
            let val = if vi < vals.len() { &vals[vi] } else { &Value::Uninit };
            vi += 1;

            let formatted = match conv {
                'd' | 'i' => {
                    let n = val.as_num() as i64;
                    format!("{}", n)
                }
                'f' => {
                    let n = val.as_num();
                    let p = precision.unwrap_or(6) as usize;
                    format_float_prec(n, p)
                }
                'e' => {
                    let n = val.as_num();
                    format!("{}", n) // simplified
                }
                'g' => {
                    let n = val.as_num();
                    format_float(n)
                }
                's' => {
                    let s = val.as_str();
                    if let Some(p) = precision {
                        if (p as usize) < s.len() {
                            String::from(&s[..p as usize])
                        } else {
                            s
                        }
                    } else {
                        s
                    }
                }
                'c' => {
                    let n = val.as_num() as u32;
                    let ch = char::from_u32(n).unwrap_or('?');
                    let mut s = String::new();
                    s.push(ch);
                    s
                }
                'x' => {
                    let n = val.as_num() as u64;
                    format!("{:x}", n)
                }
                'o' => {
                    let n = val.as_num() as u64;
                    format!("{:o}", n)
                }
                _ => String::from("?"),
            };

            // Apply width and alignment
            let w = width as usize;
            if w > formatted.len() {
                let pad_char = if zero_pad && !left_align { '0' } else { ' ' };
                let padding = w - formatted.len();
                if left_align {
                    result.push_str(&formatted);
                    for _ in 0..padding { result.push(' '); }
                } else {
                    for _ in 0..padding { result.push(pad_char); }
                    result.push_str(&formatted);
                }
            } else {
                result.push_str(&formatted);
            }
        } else if c == '\\' {
            match chars.next() {
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some(o) => { result.push('\\'); result.push(o); }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn format_float_prec(n: f64, prec: usize) -> String {
    let neg = n < 0.0;
    let abs = if neg { -n } else { n };
    let int_part = abs as u64;
    let frac = abs - int_part as f64;

    let mut frac_str = String::new();
    let mut rem = frac;
    for _ in 0..prec {
        rem *= 10.0;
        let digit = rem as u32;
        frac_str.push(char::from_u32('0' as u32 + digit).unwrap_or('0'));
        rem -= digit as f64;
    }

    if prec > 0 {
        if neg {
            format!("-{}.{}", int_part, frac_str)
        } else {
            format!("{}.{}", int_part, frac_str)
        }
    } else {
        if neg {
            format!("-{}", int_part)
        } else {
            format!("{}", int_part)
        }
    }
}

// ─── Simple libm replacements ────────────────────────────────────────────────
// no_std doesn't have libm; provide basic implementations

mod libm {
    pub fn sqrt(x: f64) -> f64 {
        if x < 0.0 { return 0.0; }
        if x == 0.0 { return 0.0; }
        let mut guess = x / 2.0;
        for _ in 0..64 {
            guess = (guess + x / guess) / 2.0;
        }
        guess
    }

    pub fn sin(x: f64) -> f64 {
        // Taylor series: sin(x) = x - x^3/3! + x^5/5! - ...
        let x = normalize_angle(x);
        let mut result = 0.0;
        let mut term = x;
        let mut n = 1i32;
        for _ in 0..12 {
            result += term;
            n += 2;
            term *= -x * x / ((n - 1) as f64 * n as f64);
        }
        result
    }

    pub fn cos(x: f64) -> f64 {
        let x = normalize_angle(x);
        let mut result = 0.0;
        let mut term = 1.0;
        let mut n = 0i32;
        for _ in 0..12 {
            result += term;
            n += 2;
            term *= -x * x / ((n - 1) as f64 * n as f64);
        }
        result
    }

    pub fn log(x: f64) -> f64 {
        if x <= 0.0 { return f64::NEG_INFINITY; }
        // ln(x) using series: ln(1+u) = 2*(u + u^3/3 + u^5/5 + ...)
        // where u = (x-1)/(x+1)
        let mut mantissa = x;
        let mut exp = 0i32;
        while mantissa > 2.0 { mantissa /= 2.0; exp += 1; }
        while mantissa < 0.5 { mantissa *= 2.0; exp -= 1; }
        let u = (mantissa - 1.0) / (mantissa + 1.0);
        let u2 = u * u;
        let mut sum = u;
        let mut term = u;
        for k in 1..20 {
            term *= u2;
            sum += term / (2 * k + 1) as f64;
        }
        2.0 * sum + exp as f64 * 0.6931471805599453 // ln(2)
    }

    pub fn exp(x: f64) -> f64 {
        // e^x using Taylor series
        if x > 700.0 { return f64::INFINITY; }
        if x < -700.0 { return 0.0; }
        let mut result = 1.0;
        let mut term = 1.0;
        for n in 1..30 {
            term *= x / n as f64;
            result += term;
        }
        result
    }

    fn normalize_angle(x: f64) -> f64 {
        const TWO_PI: f64 = 6.283185307179586;
        let mut r = x % TWO_PI;
        if r > core::f64::consts::PI { r -= TWO_PI; }
        if r < -core::f64::consts::PI { r += TWO_PI; }
        r
    }
}

// ─── Read helpers ────────────────────────────────────────────────────────────

fn read_all(fd: u32) -> Vec<u8> {
    let mut data = Vec::new();
    let mut buf = [0u8; 512];
    loop {
        let n = fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX { break; }
        data.extend_from_slice(&buf[..n as usize]);
    }
    data
}

// ─── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let mut args_buf = [0u8; 256];
    let args_str = anyos_std::process::args(&mut args_buf);

    // Parse arguments: awk [-F fs] [-v var=val] 'program' [file ...]
    // or: awk [-F fs] [-v var=val] -f progfile [file ...]
    let mut fs_opt: Option<String> = None;
    let mut var_assigns: Vec<(String, String)> = Vec::new();
    let mut program_text: Option<String> = None;
    let mut files: Vec<String> = Vec::new();

    let mut parts: Vec<&str> = Vec::new();
    let mut in_quote = false;
    let mut quote_char = ' ';
    let mut start = 0;
    let bytes = args_str.as_bytes();
    let mut i = 0;
    // Custom splitting that respects quotes
    while i < bytes.len() {
        let c = bytes[i] as char;
        if in_quote {
            if c == quote_char {
                in_quote = false;
            }
            i += 1;
        } else if c == '\'' || c == '"' {
            in_quote = true;
            quote_char = c;
            i += 1;
        } else if c == ' ' || c == '\t' {
            if i > start {
                parts.push(&args_str[start..i]);
            }
            i += 1;
            start = i;
        } else {
            i += 1;
        }
    }
    if i > start {
        parts.push(&args_str[start..i]);
    }

    let mut pi = 0;
    while pi < parts.len() {
        let arg = parts[pi];
        if arg == "-F" && pi + 1 < parts.len() {
            pi += 1;
            fs_opt = Some(String::from(parts[pi]));
        } else if arg.starts_with("-F") {
            fs_opt = Some(String::from(&arg[2..]));
        } else if arg == "-v" && pi + 1 < parts.len() {
            pi += 1;
            let assign = parts[pi];
            if let Some(eq) = assign.find('=') {
                var_assigns.push((String::from(&assign[..eq]), String::from(&assign[eq+1..])));
            }
        } else if arg.starts_with("-v") {
            let assign = &arg[2..];
            if let Some(eq) = assign.find('=') {
                var_assigns.push((String::from(&assign[..eq]), String::from(&assign[eq+1..])));
            }
        } else if program_text.is_none() {
            // Strip surrounding quotes if present
            let mut p = arg;
            if (p.starts_with('\'') && p.ends_with('\''))
                || (p.starts_with('"') && p.ends_with('"')) {
                p = &p[1..p.len()-1];
            }
            program_text = Some(String::from(p));
        } else {
            files.push(String::from(arg));
        }
        pi += 1;
    }

    let program = match program_text {
        Some(p) => p,
        None => {
            anyos_std::println!("awk: no program text");
            return;
        }
    };

    // Lex and parse
    let mut lexer = Lexer::new(&program);
    let tokens = lexer.tokenize();
    let mut parser = Parser::new(tokens);
    let rules = parser.parse_program();

    // Set up interpreter
    let mut interp = Interpreter::new();
    if let Some(ref fs) = fs_opt {
        interp.fs = fs.clone();
    }
    for (k, v) in &var_assigns {
        interp.set_var(k, Value::Str(v.clone()));
    }

    // Read input
    if files.is_empty() {
        // Read from stdin
        let data = read_all(0);
        if let Ok(text) = core::str::from_utf8(&data) {
            let lines: Vec<&str> = text.lines().collect();
            interp.run(&rules, &lines);
        }
    } else {
        for file in &files {
            interp.fnr = 0;
            interp.set_var("FILENAME", Value::Str(file.clone()));
            let fd = fs::open(file, 0);
            if fd == u32::MAX {
                anyos_std::println!("awk: cannot open '{}'", file);
                continue;
            }
            let data = read_all(fd);
            fs::close(fd);
            if let Ok(text) = core::str::from_utf8(&data) {
                let lines: Vec<&str> = text.lines().collect();
                interp.run(&rules, &lines);
            }
        }
    }
}
