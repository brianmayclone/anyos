//! Recursive descent JavaScript parser.
//!
//! Parses a token stream into an AST (Abstract Syntax Tree).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use alloc::string::ToString;
use alloc::format;

use crate::token::{Token, TokenKind};
use crate::ast::*;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    pub errors: Vec<String>,
    /// When true, the `in` keyword is NOT treated as a binary operator in
    /// relational expressions.  This implements the [~In] grammar parameter
    /// required by the ECMAScript spec for `for` loop initializers.
    no_in: bool,
    /// Parser recursion depth — prevents stack overflow on deeply nested input.
    depth: usize,
}

/// Maximum parser recursion depth before bailing out with a SyntaxError.
const MAX_PARSER_DEPTH: usize = 128;

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0, errors: Vec::new(), no_in: false, depth: 0 }
    }

    /// Return the number of tokens that were NOT consumed by parse_program().
    pub fn remaining_tokens(&self) -> usize {
        if self.pos >= self.tokens.len() { 0 } else { self.tokens.len() - self.pos }
    }

    /// Current token index (for diagnostics).
    pub fn current_pos(&self) -> usize { self.pos }

    /// Get token at a specific index (for diagnostics).
    pub fn token_at(&self, idx: usize) -> Option<&Token> {
        self.tokens.get(idx)
    }

    fn syntax_error(&mut self, msg: &str) {
        self.errors.push(String::from(msg));
    }

    /// Parse a complete JavaScript program.
    pub fn parse_program(&mut self) -> Program {
        let mut body = Vec::new();
        while !self.at_end() {
            if let Some(stmt) = self.parse_statement() {
                body.push(stmt);
            }
        }
        Program { body }
    }

    // ── Helpers ──

    fn peek(&self) -> &TokenKind {
        if self.pos < self.tokens.len() {
            &self.tokens[self.pos].kind
        } else {
            &TokenKind::Eof
        }
    }

    fn peek2(&self) -> &TokenKind {
        if self.pos + 1 < self.tokens.len() {
            &self.tokens[self.pos + 1].kind
        } else {
            &TokenKind::Eof
        }
    }

    fn peek3(&self) -> &TokenKind {
        if self.pos + 2 < self.tokens.len() {
            &self.tokens[self.pos + 2].kind
        } else {
            &TokenKind::Eof
        }
    }

    fn advance(&mut self) -> &Token {
        let tok = &self.tokens[self.pos.min(self.tokens.len() - 1)];
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        tok
    }

    fn at_end(&self) -> bool {
        matches!(self.peek(), TokenKind::Eof)
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if core::mem::discriminant(self.peek()) == core::mem::discriminant(kind) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind) {
        if !self.eat(kind) {
            // Skip to recover
            self.pos += 1;
        }
    }

    fn eat_semicolon(&mut self) {
        self.eat(&TokenKind::Semicolon);
    }

    /// Check if a token is a contextual keyword that can be used as an identifier
    /// (function name, variable name, etc.).  These are NOT reserved words per
    /// ECMAScript — they only have special meaning in specific syntactic contexts
    /// (e.g. `import ... as ...`, `for ... of ...`).
    fn is_contextual_keyword(&self, kind: &TokenKind) -> bool {
        matches!(kind,
            TokenKind::As | TokenKind::From | TokenKind::Of |
            TokenKind::Async | TokenKind::Yield | TokenKind::Await |
            TokenKind::Let | TokenKind::With |
            // Some keywords used as identifiers in minified bundles
            TokenKind::Catch | TokenKind::Finally | TokenKind::Extends |
            TokenKind::Export | TokenKind::Import | TokenKind::Default
        )
    }

    fn ident_str(&mut self) -> String {
        match self.peek().clone() {
            TokenKind::Ident(s) => {
                self.pos += 1;
                s
            }
            // Keywords are valid property names after `.` in member expressions.
            TokenKind::Delete => { self.pos += 1; String::from("delete") }
            TokenKind::In => { self.pos += 1; String::from("in") }
            TokenKind::Return => { self.pos += 1; String::from("return") }
            TokenKind::New => { self.pos += 1; String::from("new") }
            TokenKind::Class => { self.pos += 1; String::from("class") }
            TokenKind::Super => { self.pos += 1; String::from("super") }
            TokenKind::This => { self.pos += 1; String::from("this") }
            TokenKind::Default => { self.pos += 1; String::from("default") }
            TokenKind::Var => { self.pos += 1; String::from("var") }
            TokenKind::Let => { self.pos += 1; String::from("let") }
            TokenKind::Const => { self.pos += 1; String::from("const") }
            TokenKind::Function => { self.pos += 1; String::from("function") }
            TokenKind::If => { self.pos += 1; String::from("if") }
            TokenKind::Else => { self.pos += 1; String::from("else") }
            TokenKind::While => { self.pos += 1; String::from("while") }
            TokenKind::For => { self.pos += 1; String::from("for") }
            TokenKind::Do => { self.pos += 1; String::from("do") }
            TokenKind::Switch => { self.pos += 1; String::from("switch") }
            TokenKind::Case => { self.pos += 1; String::from("case") }
            TokenKind::Break => { self.pos += 1; String::from("break") }
            TokenKind::Continue => { self.pos += 1; String::from("continue") }
            TokenKind::Throw => { self.pos += 1; String::from("throw") }
            TokenKind::Try => { self.pos += 1; String::from("try") }
            TokenKind::Catch => { self.pos += 1; String::from("catch") }
            TokenKind::Finally => { self.pos += 1; String::from("finally") }
            TokenKind::Typeof => { self.pos += 1; String::from("typeof") }
            TokenKind::Void => { self.pos += 1; String::from("void") }
            TokenKind::Instanceof => { self.pos += 1; String::from("instanceof") }
            TokenKind::Extends => { self.pos += 1; String::from("extends") }
            TokenKind::Import => { self.pos += 1; String::from("import") }
            TokenKind::Export => { self.pos += 1; String::from("export") }
            TokenKind::Async => { self.pos += 1; String::from("async") }
            TokenKind::Await => { self.pos += 1; String::from("await") }
            TokenKind::Yield => { self.pos += 1; String::from("yield") }
            TokenKind::Of => { self.pos += 1; String::from("of") }
            TokenKind::From => { self.pos += 1; String::from("from") }
            TokenKind::As => { self.pos += 1; String::from("as") }
            TokenKind::With => { self.pos += 1; String::from("with") }
            TokenKind::Debugger => { self.pos += 1; String::from("debugger") }
            _ => {
                self.pos += 1;
                String::from("_error_")
            }
        }
    }

    // ── Statements ──

    fn parse_statement(&mut self) -> Option<Stmt> {
        self.depth += 1;
        if self.depth > MAX_PARSER_DEPTH {
            self.depth -= 1;
            self.syntax_error("Maximum nesting depth exceeded");
            return None;
        }
        let result = self.parse_statement_inner();
        self.depth -= 1;
        result
    }

    fn parse_statement_inner(&mut self) -> Option<Stmt> {
        match self.peek() {
            TokenKind::Semicolon => {
                self.pos += 1;
                Some(Stmt::Empty)
            }
            TokenKind::LBrace => Some(self.parse_block_stmt()),
            TokenKind::Var | TokenKind::Let | TokenKind::Const => Some(self.parse_var_decl()),
            TokenKind::If => Some(self.parse_if()),
            TokenKind::While => Some(self.parse_while()),
            TokenKind::Do => Some(self.parse_do_while()),
            TokenKind::For => Some(self.parse_for()),
            TokenKind::Return => Some(self.parse_return()),
            TokenKind::Break => Some(self.parse_break()),
            TokenKind::Continue => Some(self.parse_continue()),
            TokenKind::Switch => Some(self.parse_switch()),
            TokenKind::Throw => Some(self.parse_throw()),
            TokenKind::Try => Some(self.parse_try()),
            TokenKind::Function => {
                if matches!(self.peek2(), TokenKind::Ident(_) | TokenKind::Star) || self.is_contextual_keyword(self.peek2()) {
                    Some(self.parse_function_decl(false))
                } else {
                    Some(self.parse_expr_stmt())
                }
            }
            TokenKind::Async => {
                if matches!(self.peek2(), TokenKind::Function) {
                    self.pos += 1; // skip async
                    Some(self.parse_function_decl(true))
                } else {
                    Some(self.parse_expr_stmt())
                }
            }
            TokenKind::Class => Some(self.parse_class_decl()),
            TokenKind::Debugger => {
                self.pos += 1;
                self.eat_semicolon();
                Some(Stmt::Debugger)
            }
            TokenKind::Import => Some(self.parse_import()),
            TokenKind::Export => Some(self.parse_export()),
            TokenKind::Eof => None,
            // Check for labeled statement
            TokenKind::Ident(_) => {
                if matches!(self.peek2(), TokenKind::Colon) {
                    let label = self.ident_str();
                    self.expect(&TokenKind::Colon);
                    let body = self.parse_statement().unwrap_or(Stmt::Empty);
                    Some(Stmt::Labeled {
                        label,
                        body: Box::new(body),
                    })
                } else {
                    Some(self.parse_expr_stmt())
                }
            }
            _ => Some(self.parse_expr_stmt()),
        }
    }

    fn parse_block_stmt(&mut self) -> Stmt {
        self.expect(&TokenKind::LBrace);
        let stmts = self.parse_block_body();
        self.expect(&TokenKind::RBrace);
        Stmt::Block(stmts)
    }

    fn parse_block_body(&mut self) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        let entry_pos = self.pos;
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            let before = self.pos;
            if let Some(stmt) = self.parse_statement() {
                stmts.push(stmt);
            }
            if self.pos == before { self.pos += 1; }
        }
        stmts
    }

    fn parse_var_decl(&mut self) -> Stmt {
        let kind = match self.peek() {
            TokenKind::Var => { self.pos += 1; VarKind::Var }
            TokenKind::Let => { self.pos += 1; VarKind::Let }
            TokenKind::Const => { self.pos += 1; VarKind::Const }
            _ => { self.pos += 1; VarKind::Var }
        };

        let mut decls = Vec::new();
        loop {
            let name = self.parse_binding_pattern();
            let init = if self.eat(&TokenKind::Eq) {
                Some(self.parse_assignment_expr())
            } else {
                None
            };
            decls.push(VarDeclarator { name, init });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.eat_semicolon();
        Stmt::VarDecl { kind, decls }
    }

    fn parse_binding_pattern(&mut self) -> Pattern {
        match self.peek() {
            TokenKind::LBracket => self.parse_array_pattern(),
            TokenKind::LBrace => self.parse_object_pattern(),
            _ => Pattern::Ident(self.ident_str()),
        }
    }

    fn parse_array_pattern(&mut self) -> Pattern {
        self.expect(&TokenKind::LBracket);
        let mut elements = Vec::new();
        while !matches!(self.peek(), TokenKind::RBracket | TokenKind::Eof) {
            if self.eat(&TokenKind::Comma) {
                elements.push(None);
                continue;
            }
            // Rest element: `...binding` — must be last, no initializer
            if self.eat(&TokenKind::DotDotDot) {
                let inner = self.parse_binding_pattern();
                // Rest element cannot have a default initializer: [...x = val] is a SyntaxError
                if matches!(self.peek(), TokenKind::Eq) {
                    self.syntax_error("Rest element may not have a default initializer");
                }
                elements.push(Some(Pattern::Rest(Box::new(inner))));
                // Rest must be last — if there are more elements, that's a SyntaxError
                if self.eat(&TokenKind::Comma) {
                    if !matches!(self.peek(), TokenKind::RBracket) {
                        self.syntax_error("Rest element must be last element");
                    }
                }
                break;
            }
            let pat = self.parse_binding_pattern();
            let pat = if self.eat(&TokenKind::Eq) {
                let def = self.parse_assignment_expr();
                Pattern::Assign(Box::new(pat), Box::new(def))
            } else {
                pat
            };
            elements.push(Some(pat));
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBracket);
        Pattern::Array(elements)
    }

    fn parse_object_pattern(&mut self) -> Pattern {
        self.expect(&TokenKind::LBrace);
        let mut props = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            // Rest element: `...binding` — must be last
            if self.eat(&TokenKind::DotDotDot) {
                let inner = self.parse_binding_pattern();
                // Use empty key as sentinel; compiler checks Pattern::Rest on value
                props.push(ObjPatProp { key: String::new(), value: Pattern::Rest(Box::new(inner)) });
                self.eat(&TokenKind::Comma);
                break;
            }
            let key = self.ident_str();
            let value = if self.eat(&TokenKind::Colon) {
                self.parse_binding_pattern()
            } else {
                Pattern::Ident(key.clone())
            };
            let value = if self.eat(&TokenKind::Eq) {
                let def = self.parse_assignment_expr();
                Pattern::Assign(Box::new(value), Box::new(def))
            } else {
                value
            };
            props.push(ObjPatProp { key, value });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBrace);
        Pattern::Object(props)
    }

    fn parse_if(&mut self) -> Stmt {
        self.expect(&TokenKind::If);
        self.expect(&TokenKind::LParen);
        let condition = self.parse_expression();
        self.expect(&TokenKind::RParen);
        let consequent = Box::new(self.parse_statement().unwrap_or(Stmt::Empty));
        let alternate = if self.eat(&TokenKind::Else) {
            Some(Box::new(self.parse_statement().unwrap_or(Stmt::Empty)))
        } else {
            None
        };
        Stmt::If { condition, consequent, alternate }
    }

    fn parse_while(&mut self) -> Stmt {
        self.expect(&TokenKind::While);
        self.expect(&TokenKind::LParen);
        let condition = self.parse_expression();
        self.expect(&TokenKind::RParen);
        let body = Box::new(self.parse_statement().unwrap_or(Stmt::Empty));
        Stmt::While { condition, body }
    }

    fn parse_do_while(&mut self) -> Stmt {
        self.expect(&TokenKind::Do);
        let body = Box::new(self.parse_statement().unwrap_or(Stmt::Empty));
        self.expect(&TokenKind::While);
        self.expect(&TokenKind::LParen);
        let condition = self.parse_expression();
        self.expect(&TokenKind::RParen);
        self.eat_semicolon();
        Stmt::DoWhile { body, condition }
    }

    fn parse_for(&mut self) -> Stmt {
        self.expect(&TokenKind::For);
        self.expect(&TokenKind::LParen);

        // Check for for-in / for-of
        let init = match self.peek() {
            TokenKind::Semicolon => None,
            TokenKind::Var | TokenKind::Let | TokenKind::Const => {
                let kind = match self.peek() {
                    TokenKind::Var => { self.pos += 1; VarKind::Var }
                    TokenKind::Let => { self.pos += 1; VarKind::Let }
                    TokenKind::Const => { self.pos += 1; VarKind::Const }
                    _ => { self.pos += 1; VarKind::Var }
                };
                let name = self.parse_binding_pattern();
                // Check for for-in / for-of
                if matches!(self.peek(), TokenKind::In) {
                    self.pos += 1;
                    let right = self.parse_expression();
                    self.expect(&TokenKind::RParen);
                    let body = Box::new(self.parse_statement().unwrap_or(Stmt::Empty));
                    return Stmt::ForIn {
                        left: Box::new(ForInit::VarDecl {
                            kind,
                            decls: vec![VarDeclarator { name, init: None }],
                        }),
                        right,
                        body,
                    };
                }
                if matches!(self.peek(), TokenKind::Of) {
                    self.pos += 1;
                    let right = self.parse_assignment_expr();
                    self.expect(&TokenKind::RParen);
                    let body = Box::new(self.parse_statement().unwrap_or(Stmt::Empty));
                    return Stmt::ForOf {
                        left: Box::new(ForInit::VarDecl {
                            kind,
                            decls: vec![VarDeclarator { name, init: None }],
                        }),
                        right,
                        body,
                    };
                }
                // Use [~In] for var-init expressions per ES spec
                self.no_in = true;
                let init_val = if self.eat(&TokenKind::Eq) {
                    Some(self.parse_assignment_expr())
                } else {
                    None
                };
                let mut decls = vec![VarDeclarator { name, init: init_val }];
                while self.eat(&TokenKind::Comma) {
                    let n = self.parse_binding_pattern();
                    let i = if self.eat(&TokenKind::Eq) {
                        Some(self.parse_assignment_expr())
                    } else {
                        None
                    };
                    decls.push(VarDeclarator { name: n, init: i });
                }
                self.no_in = false;
                Some(Box::new(ForInit::VarDecl { kind, decls }))
            }
            _ => {
                // Parse with [~In] to prevent `in` being consumed as a
                // binary operator — this is required by the ES spec so that
                // `for(x in obj)` is correctly recognised as for-in.
                self.no_in = true;
                let expr = self.parse_expression();
                self.no_in = false;
                // Check for for-in / for-of
                if matches!(self.peek(), TokenKind::In) {
                    self.pos += 1;
                    let right = self.parse_expression();
                    self.expect(&TokenKind::RParen);
                    let body = Box::new(self.parse_statement().unwrap_or(Stmt::Empty));
                    return Stmt::ForIn {
                        left: Box::new(ForInit::Expr(expr)),
                        right,
                        body,
                    };
                }
                if matches!(self.peek(), TokenKind::Of) {
                    self.pos += 1;
                    let right = self.parse_assignment_expr();
                    self.expect(&TokenKind::RParen);
                    let body = Box::new(self.parse_statement().unwrap_or(Stmt::Empty));
                    return Stmt::ForOf {
                        left: Box::new(ForInit::Expr(expr)),
                        right,
                        body,
                    };
                }
                Some(Box::new(ForInit::Expr(expr)))
            }
        };

        self.expect(&TokenKind::Semicolon);
        let test = if !matches!(self.peek(), TokenKind::Semicolon) {
            Some(self.parse_expression())
        } else {
            None
        };
        self.expect(&TokenKind::Semicolon);
        let update = if !matches!(self.peek(), TokenKind::RParen) {
            Some(self.parse_expression())
        } else {
            None
        };
        self.expect(&TokenKind::RParen);
        let body = Box::new(self.parse_statement().unwrap_or(Stmt::Empty));
        Stmt::For { init, test, update, body }
    }

    fn parse_return(&mut self) -> Stmt {
        self.expect(&TokenKind::Return);
        let value = if matches!(self.peek(), TokenKind::Semicolon | TokenKind::RBrace | TokenKind::Eof) {
            None
        } else {
            Some(self.parse_expression())
        };
        self.eat_semicolon();
        Stmt::Return(value)
    }

    fn parse_break(&mut self) -> Stmt {
        self.expect(&TokenKind::Break);
        let label = if let TokenKind::Ident(s) = self.peek().clone() {
            self.pos += 1;
            Some(s)
        } else {
            None
        };
        self.eat_semicolon();
        Stmt::Break(label)
    }

    fn parse_continue(&mut self) -> Stmt {
        self.expect(&TokenKind::Continue);
        let label = if let TokenKind::Ident(s) = self.peek().clone() {
            self.pos += 1;
            Some(s)
        } else {
            None
        };
        self.eat_semicolon();
        Stmt::Continue(label)
    }

    fn parse_switch(&mut self) -> Stmt {
        self.expect(&TokenKind::Switch);
        self.expect(&TokenKind::LParen);
        let discriminant = self.parse_expression();
        self.expect(&TokenKind::RParen);
        self.expect(&TokenKind::LBrace);

        let mut cases = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            let test = if self.eat(&TokenKind::Case) {
                Some(self.parse_expression())
            } else {
                self.expect(&TokenKind::Default);
                None
            };
            self.expect(&TokenKind::Colon);
            let mut consequent = Vec::new();
            while !matches!(
                self.peek(),
                TokenKind::Case | TokenKind::Default | TokenKind::RBrace | TokenKind::Eof
            ) {
                if let Some(stmt) = self.parse_statement() {
                    consequent.push(stmt);
                }
            }
            cases.push(SwitchCase { test, consequent });
        }
        self.expect(&TokenKind::RBrace);
        Stmt::Switch { discriminant, cases }
    }

    fn parse_throw(&mut self) -> Stmt {
        self.expect(&TokenKind::Throw);
        let argument = self.parse_expression();
        self.eat_semicolon();
        Stmt::Throw(argument)
    }

    fn parse_try(&mut self) -> Stmt {
        self.expect(&TokenKind::Try);
        self.expect(&TokenKind::LBrace);
        let block = self.parse_block_body();
        self.expect(&TokenKind::RBrace);

        let catch = if self.eat(&TokenKind::Catch) {
            let param = if self.eat(&TokenKind::LParen) {
                let p = self.parse_binding_pattern();
                self.expect(&TokenKind::RParen);
                Some(p)
            } else {
                None
            };
            self.expect(&TokenKind::LBrace);
            let body = self.parse_block_body();
            self.expect(&TokenKind::RBrace);
            Some(CatchClause { param, body })
        } else {
            None
        };

        let finally = if self.eat(&TokenKind::Finally) {
            self.expect(&TokenKind::LBrace);
            let body = self.parse_block_body();
            self.expect(&TokenKind::RBrace);
            Some(body)
        } else {
            None
        };

        Stmt::Try { block, catch, finally }
    }

    fn parse_import(&mut self) -> Stmt {
        self.expect(&TokenKind::Import);
        let mut specifiers = Vec::new();

        // import 'module'  (side-effect only)
        if let TokenKind::String(ref s) = self.peek().clone() {
            let source = s.clone();
            self.pos += 1;
            self.eat_semicolon();
            return Stmt::Import { specifiers, source };
        }

        // import * as name from 'module'
        if self.eat(&TokenKind::Star) {
            self.expect(&TokenKind::As);
            let local = self.ident_str();
            specifiers.push(ImportSpecifier::Namespace(local));
        }
        // import { ... } from 'module'
        else if matches!(self.peek(), TokenKind::LBrace) {
            self.pos += 1;
            while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
                let imported = self.ident_str();
                let local = if self.eat(&TokenKind::As) {
                    self.ident_str()
                } else {
                    imported.clone()
                };
                specifiers.push(ImportSpecifier::Named { imported, local });
                if !self.eat(&TokenKind::Comma) { break; }
            }
            self.expect(&TokenKind::RBrace);
        }
        // import name from 'module'  or  import name, { ... } from 'module'
        else if let TokenKind::Ident(_) = self.peek() {
            let default_name = self.ident_str();
            specifiers.push(ImportSpecifier::Default(default_name));
            // import name, { ... }
            if self.eat(&TokenKind::Comma) {
                if self.eat(&TokenKind::LBrace) {
                    while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
                        let imported = self.ident_str();
                        let local = if self.eat(&TokenKind::As) {
                            self.ident_str()
                        } else {
                            imported.clone()
                        };
                        specifiers.push(ImportSpecifier::Named { imported, local });
                        if !self.eat(&TokenKind::Comma) { break; }
                    }
                    self.expect(&TokenKind::RBrace);
                } else if self.eat(&TokenKind::Star) {
                    self.expect(&TokenKind::As);
                    let local = self.ident_str();
                    specifiers.push(ImportSpecifier::Namespace(local));
                }
            }
        }

        // from 'module'
        self.expect(&TokenKind::From);
        let source = if let TokenKind::String(ref s) = self.peek().clone() {
            let src = s.clone();
            self.pos += 1;
            src
        } else {
            self.pos += 1;
            String::from("")
        };
        self.eat_semicolon();
        Stmt::Import { specifiers, source }
    }

    fn parse_export(&mut self) -> Stmt {
        self.expect(&TokenKind::Export);

        // export default expr
        if self.eat(&TokenKind::Default) {
            let expr = if matches!(self.peek(), TokenKind::Function) {
                // export default function name() {}
                let stmt = self.parse_function_decl(false);
                match stmt {
                    Stmt::FunctionDecl { name, params, body, is_async, is_generator } => {
                        Expr::FunctionExpr { name: Some(name), params, body, is_async, is_generator }
                    }
                    _ => Expr::Undefined,
                }
            } else if matches!(self.peek(), TokenKind::Class) {
                let stmt = self.parse_class_decl();
                match stmt {
                    Stmt::ClassDecl { name, super_class, body } => {
                        Expr::ClassExpr { name: Some(name), super_class: super_class.map(Box::new), body }
                    }
                    _ => Expr::Undefined,
                }
            } else {
                let expr = self.parse_assignment_expr();
                self.eat_semicolon();
                expr
            };
            return Stmt::Export(ExportDecl::Default(expr));
        }

        // export { name1, name2 as alias }
        if matches!(self.peek(), TokenKind::LBrace) {
            self.pos += 1;
            let mut specifiers = Vec::new();
            while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
                let local = self.ident_str();
                let exported = if self.eat(&TokenKind::As) {
                    self.ident_str()
                } else {
                    local.clone()
                };
                specifiers.push(ExportSpecifier { local, exported });
                if !self.eat(&TokenKind::Comma) { break; }
            }
            self.expect(&TokenKind::RBrace);
            // Re-export: export { ... } from 'module'
            if self.eat(&TokenKind::From) {
                let source = if let TokenKind::String(ref s) = self.peek().clone() {
                    let src = s.clone();
                    self.pos += 1;
                    src
                } else {
                    self.pos += 1;
                    String::from("")
                };
                self.eat_semicolon();
                return Stmt::Export(ExportDecl::ReExport { specifiers, source });
            }
            self.eat_semicolon();
            return Stmt::Export(ExportDecl::Named(specifiers));
        }

        // export function/class/var/let/const
        let decl = match self.peek() {
            TokenKind::Function => self.parse_function_decl(false),
            TokenKind::Async => {
                self.pos += 1;
                self.parse_function_decl(true)
            }
            TokenKind::Class => self.parse_class_decl(),
            TokenKind::Var | TokenKind::Let | TokenKind::Const => self.parse_var_decl(),
            _ => {
                // export *
                if self.eat(&TokenKind::Star) {
                    self.expect(&TokenKind::From);
                    let source = if let TokenKind::String(ref s) = self.peek().clone() {
                        let src = s.clone();
                        self.pos += 1;
                        src
                    } else {
                        self.pos += 1;
                        String::from("")
                    };
                    self.eat_semicolon();
                    return Stmt::Export(ExportDecl::ReExport {
                        specifiers: Vec::new(),
                        source,
                    });
                }
                self.parse_expr_stmt()
            }
        };
        Stmt::Export(ExportDecl::Decl(Box::new(decl)))
    }

    fn parse_function_decl(&mut self, is_async: bool) -> Stmt {
        self.expect(&TokenKind::Function);
        let is_generator = self.eat(&TokenKind::Star);
        let name = self.ident_str();
        let params = self.parse_params();
        self.expect(&TokenKind::LBrace);
        let body = self.parse_block_body();
        self.expect(&TokenKind::RBrace);
        Stmt::FunctionDecl { name, params, body, is_async, is_generator }
    }

    fn parse_class_decl(&mut self) -> Stmt {
        self.expect(&TokenKind::Class);
        let name = self.ident_str();
        let super_class = if self.eat(&TokenKind::Extends) {
            Some(self.parse_assignment_expr())
        } else {
            None
        };
        let body = self.parse_class_body();
        Stmt::ClassDecl { name, super_class, body }
    }

    fn parse_class_body(&mut self) -> Vec<ClassMember> {
        self.expect(&TokenKind::LBrace);
        let mut members = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            if self.eat(&TokenKind::Semicolon) {
                continue;
            }
            let is_static = if let TokenKind::Ident(s) = self.peek() {
                if s == "static" && !matches!(self.peek2(), TokenKind::LParen | TokenKind::Eq) {
                    self.pos += 1;
                    true
                } else {
                    false
                }
            } else {
                false
            };

            // Static block: static { ... } (ES2022)
            if is_static && matches!(self.peek(), TokenKind::LBrace) {
                self.pos += 1; // consume '{'
                let body = self.parse_block_body();
                self.expect(&TokenKind::RBrace);
                members.push(ClassMember {
                    key: PropKey::Ident(String::from("")),
                    kind: ClassMemberKind::StaticBlock { body },
                    is_static: true,
                });
                continue;
            }

            // Private field/method: #name, * #name(), or #name = value
            // Check for generator star before private ident: `* #method()`
            let priv_is_generator = if matches!(self.peek(), TokenKind::Star)
                && matches!(self.peek2(), TokenKind::PrivateIdent(_))
            {
                self.pos += 1; // consume *
                true
            } else {
                false
            };
            if let TokenKind::PrivateIdent(ref name) = self.peek().clone() {
                let priv_name = name.clone(); // e.g. "#count"
                self.pos += 1;
                if matches!(self.peek(), TokenKind::LParen) {
                    // Private method: #method() { } or * #method() { }
                    let params = self.parse_params();
                    self.expect(&TokenKind::LBrace);
                    let body = self.parse_block_body();
                    self.expect(&TokenKind::RBrace);
                    members.push(ClassMember {
                        key: PropKey::Ident(priv_name),
                        kind: ClassMemberKind::Method { params, body, is_generator: priv_is_generator, is_async: false },
                        is_static,
                    });
                } else {
                    // Private field: #name = value
                    let value = if self.eat(&TokenKind::Eq) {
                        Some(self.parse_assignment_expr())
                    } else {
                        None
                    };
                    self.eat_semicolon();
                    members.push(ClassMember {
                        key: PropKey::Ident(priv_name),
                        kind: ClassMemberKind::Property { value },
                        is_static,
                    });
                }
                continue;
            }

            // Check for get/set accessor in class body (ES spec §14.3)
            let is_get = matches!(self.peek(), TokenKind::Ident(ref s) if s == "get");
            let is_set = matches!(self.peek(), TokenKind::Ident(ref s) if s == "set");
            if (is_get || is_set) && !matches!(self.peek2(), TokenKind::LParen | TokenKind::Eq | TokenKind::Semicolon | TokenKind::RBrace) {
                self.pos += 1; // skip 'get'/'set'
                let key = self.parse_prop_key();
                if is_get {
                    self.expect(&TokenKind::LParen);
                    self.expect(&TokenKind::RParen);
                    self.expect(&TokenKind::LBrace);
                    let body = self.parse_block_body();
                    self.expect(&TokenKind::RBrace);
                    members.push(ClassMember {
                        key,
                        kind: ClassMemberKind::Getter { body },
                        is_static,
                    });
                } else {
                    self.expect(&TokenKind::LParen);
                    let param = self.ident_str();
                    self.expect(&TokenKind::RParen);
                    self.expect(&TokenKind::LBrace);
                    let body = self.parse_block_body();
                    self.expect(&TokenKind::RBrace);
                    members.push(ClassMember {
                        key,
                        kind: ClassMemberKind::Setter { param, body },
                        is_static,
                    });
                }
                continue;
            }

            // Check for async method: async name() { ... } or async * name() { ... }
            let is_async = if matches!(self.peek(), TokenKind::Async)
                && !matches!(self.peek2(), TokenKind::LParen | TokenKind::Eq | TokenKind::Semicolon | TokenKind::Colon | TokenKind::RBrace)
            {
                self.pos += 1; // skip 'async'
                true
            } else {
                false
            };

            // Generator method: * name() { ... }
            let is_generator = self.eat(&TokenKind::Star);

            let key = self.parse_prop_key();

            if matches!(self.peek(), TokenKind::LParen) {
                // Method
                let params = self.parse_params();
                self.expect(&TokenKind::LBrace);
                let body = self.parse_block_body();
                self.expect(&TokenKind::RBrace);

                let is_ctor = matches!(&key, PropKey::Ident(s) if s == "constructor") && !is_static;
                let kind = if is_ctor {
                    ClassMemberKind::Constructor { params, body }
                } else {
                    ClassMemberKind::Method { params, body, is_generator, is_async }
                };
                members.push(ClassMember { key, kind, is_static });
            } else {
                // Property
                let value = if self.eat(&TokenKind::Eq) {
                    Some(self.parse_assignment_expr())
                } else {
                    None
                };
                self.eat_semicolon();
                members.push(ClassMember {
                    key,
                    kind: ClassMemberKind::Property { value },
                    is_static,
                });
            }
        }
        self.expect(&TokenKind::RBrace);
        members
    }

    fn parse_prop_key(&mut self) -> PropKey {
        match self.peek().clone() {
            TokenKind::Ident(s) => { self.pos += 1; PropKey::Ident(s) }
            TokenKind::String(s) => { self.pos += 1; PropKey::String(s) }
            TokenKind::Number(n) => { self.pos += 1; PropKey::Number(n) }
            TokenKind::LBracket => {
                self.pos += 1;
                let expr = self.parse_assignment_expr();
                self.expect(&TokenKind::RBracket);
                PropKey::Computed(Box::new(expr))
            }
            _ => {
                // Per ES spec §12.1.1, all keywords are valid as property names.
                let s = self.ident_str();
                PropKey::Ident(s)
            }
        }
    }

    fn parse_params(&mut self) -> Vec<Param> {
        self.expect(&TokenKind::LParen);
        let mut params = Vec::new();
        while !matches!(self.peek(), TokenKind::RParen | TokenKind::Eof) {
            // Rest parameter
            if self.eat(&TokenKind::DotDotDot) {
                let pattern = self.parse_binding_pattern();
                params.push(Param { pattern, default: None, is_rest: true });
                break;
            }
            let pattern = self.parse_binding_pattern();
            let default = if self.eat(&TokenKind::Eq) {
                Some(self.parse_assignment_expr())
            } else {
                None
            };
            params.push(Param { pattern, default, is_rest: false });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen);
        params
    }

    fn parse_expr_stmt(&mut self) -> Stmt {
        let expr = self.parse_expression();
        self.eat_semicolon();
        Stmt::Expr(expr)
    }

    // ── Expressions ──

    fn parse_expression(&mut self) -> Expr {
        self.depth += 1;
        if self.depth > MAX_PARSER_DEPTH {
            self.depth -= 1;
            self.syntax_error("Maximum nesting depth exceeded");
            return Expr::Undefined;
        }
        let expr = self.parse_assignment_expr();
        self.depth -= 1;
        if matches!(self.peek(), TokenKind::Comma) {
            let mut exprs = vec![expr];
            while self.eat(&TokenKind::Comma) {
                exprs.push(self.parse_assignment_expr());
            }
            Expr::Sequence(exprs)
        } else {
            expr
        }
    }

    fn parse_assignment_expr(&mut self) -> Expr {
        // Arrow function: (params) => body  or  ident => body
        if self.is_arrow_function() {
            return self.parse_arrow_function(false);
        }

        // Async arrow function: async (params) => body  or  async ident => body
        if self.is_async_arrow_function() {
            self.pos += 1; // skip 'async'
            return self.parse_arrow_function(true);
        }

        let left = self.parse_conditional_expr();

        if let Some(op) = self.assignment_op() {
            self.pos += 1;
            let right = self.parse_assignment_expr();
            Expr::Assign {
                op,
                left: Box::new(left),
                right: Box::new(right),
            }
        } else {
            left
        }
    }

    fn is_arrow_function(&self) -> bool {
        // Simple ident => ...
        if matches!(self.peek(), TokenKind::Ident(_)) && matches!(self.peek2(), TokenKind::Arrow) {
            return true;
        }
        // (params) => ... — scan ahead for matching paren then arrow
        if matches!(self.peek(), TokenKind::LParen) {
            let mut depth = 0;
            let mut i = self.pos;
            while i < self.tokens.len() {
                match &self.tokens[i].kind {
                    TokenKind::LParen => depth += 1,
                    TokenKind::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            // Check if next is =>
                            if i + 1 < self.tokens.len()
                                && matches!(self.tokens[i + 1].kind, TokenKind::Arrow)
                            {
                                return true;
                            }
                            break;
                        }
                    }
                    TokenKind::Eof => break,
                    _ => {}
                }
                i += 1;
            }
        }
        false
    }

    /// Check if current position is `async (params) => ...` or `async ident => ...`.
    fn is_async_arrow_function(&self) -> bool {
        if !matches!(self.peek(), TokenKind::Async) {
            return false;
        }
        let next = if self.pos + 1 < self.tokens.len() {
            &self.tokens[self.pos + 1].kind
        } else {
            return false;
        };
        // async ident => ...
        if matches!(next, TokenKind::Ident(_)) {
            if self.pos + 2 < self.tokens.len()
                && matches!(self.tokens[self.pos + 2].kind, TokenKind::Arrow)
            {
                return true;
            }
        }
        // async (params) => ...
        if matches!(next, TokenKind::LParen) {
            let mut depth = 0;
            let mut i = self.pos + 1;
            while i < self.tokens.len() {
                match &self.tokens[i].kind {
                    TokenKind::LParen => depth += 1,
                    TokenKind::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            if i + 1 < self.tokens.len()
                                && matches!(self.tokens[i + 1].kind, TokenKind::Arrow)
                            {
                                return true;
                            }
                            break;
                        }
                    }
                    TokenKind::Eof => break,
                    _ => {}
                }
                i += 1;
            }
        }
        false
    }

    fn parse_arrow_function(&mut self, is_async: bool) -> Expr {
        let params = if matches!(self.peek(), TokenKind::Ident(_)) && matches!(self.peek2(), TokenKind::Arrow) {
            let name = self.ident_str();
            vec![Param {
                pattern: Pattern::Ident(name),
                default: None,
                is_rest: false,
            }]
        } else {
            self.parse_params()
        };

        self.expect(&TokenKind::Arrow);

        let body = if matches!(self.peek(), TokenKind::LBrace) {
            self.expect(&TokenKind::LBrace);
            let stmts = self.parse_block_body();
            self.expect(&TokenKind::RBrace);
            ArrowBody::Block(stmts)
        } else {
            ArrowBody::Expr(Box::new(self.parse_assignment_expr()))
        };

        Expr::Arrow { params, body, is_async }
    }

    fn assignment_op(&self) -> Option<AssignOp> {
        match self.peek() {
            TokenKind::Eq => Some(AssignOp::Assign),
            TokenKind::PlusEq => Some(AssignOp::AddAssign),
            TokenKind::MinusEq => Some(AssignOp::SubAssign),
            TokenKind::StarEq => Some(AssignOp::MulAssign),
            TokenKind::SlashEq => Some(AssignOp::DivAssign),
            TokenKind::PercentEq => Some(AssignOp::ModAssign),
            TokenKind::StarStarEq => Some(AssignOp::ExpAssign),
            TokenKind::AmpEq => Some(AssignOp::BitAndAssign),
            TokenKind::PipeEq => Some(AssignOp::BitOrAssign),
            TokenKind::CaretEq => Some(AssignOp::BitXorAssign),
            TokenKind::LtLtEq => Some(AssignOp::ShlAssign),
            TokenKind::GtGtEq => Some(AssignOp::ShrAssign),
            TokenKind::GtGtGtEq => Some(AssignOp::UShrAssign),
            TokenKind::AmpAmpEq => Some(AssignOp::AndAssign),
            TokenKind::PipePipeEq => Some(AssignOp::OrAssign),
            TokenKind::QuestionQuestionEq => Some(AssignOp::NullishAssign),
            _ => None,
        }
    }

    fn parse_conditional_expr(&mut self) -> Expr {
        let expr = self.parse_nullish_coalesce();
        if self.eat(&TokenKind::Question) {
            let consequent = self.parse_assignment_expr();
            self.expect(&TokenKind::Colon);
            let alternate = self.parse_assignment_expr();
            Expr::Conditional {
                test: Box::new(expr),
                consequent: Box::new(consequent),
                alternate: Box::new(alternate),
            }
        } else {
            expr
        }
    }

    fn parse_nullish_coalesce(&mut self) -> Expr {
        let mut left = self.parse_logical_or();
        while self.eat(&TokenKind::QuestionQuestion) {
            let right = self.parse_logical_or();
            left = Expr::Logical {
                op: LogicalOp::NullishCoalesce,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_logical_or(&mut self) -> Expr {
        let mut left = self.parse_logical_and();
        while self.eat(&TokenKind::PipePipe) {
            let right = self.parse_logical_and();
            left = Expr::Logical {
                op: LogicalOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_logical_and(&mut self) -> Expr {
        let mut left = self.parse_bitwise_or();
        while self.eat(&TokenKind::AmpAmp) {
            let right = self.parse_bitwise_or();
            left = Expr::Logical {
                op: LogicalOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_bitwise_or(&mut self) -> Expr {
        let mut left = self.parse_bitwise_xor();
        while matches!(self.peek(), TokenKind::Pipe) {
            self.pos += 1;
            let right = self.parse_bitwise_xor();
            left = Expr::Binary {
                op: BinaryOp::BitOr,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_bitwise_xor(&mut self) -> Expr {
        let mut left = self.parse_bitwise_and();
        while matches!(self.peek(), TokenKind::Caret) {
            self.pos += 1;
            let right = self.parse_bitwise_and();
            left = Expr::Binary {
                op: BinaryOp::BitXor,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_bitwise_and(&mut self) -> Expr {
        let mut left = self.parse_equality();
        while matches!(self.peek(), TokenKind::Amp) {
            self.pos += 1;
            let right = self.parse_equality();
            left = Expr::Binary {
                op: BinaryOp::BitAnd,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_equality(&mut self) -> Expr {
        let mut left = self.parse_relational();
        loop {
            let op = match self.peek() {
                TokenKind::EqEq => BinaryOp::Eq,
                TokenKind::BangEq => BinaryOp::Ne,
                TokenKind::EqEqEq => BinaryOp::StrictEq,
                TokenKind::BangEqEq => BinaryOp::StrictNe,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_relational();
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_relational(&mut self) -> Expr {
        let mut left = self.parse_shift();
        loop {
            let op = match self.peek() {
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::LtEq => BinaryOp::Le,
                TokenKind::GtEq => BinaryOp::Ge,
                TokenKind::Instanceof => BinaryOp::InstanceOf,
                TokenKind::In if !self.no_in => BinaryOp::In,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_shift();
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_shift(&mut self) -> Expr {
        let mut left = self.parse_additive();
        loop {
            let op = match self.peek() {
                TokenKind::LtLt => BinaryOp::Shl,
                TokenKind::GtGt => BinaryOp::Shr,
                TokenKind::GtGtGt => BinaryOp::UShr,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_additive();
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_additive(&mut self) -> Expr {
        let mut left = self.parse_multiplicative();
        loop {
            let op = match self.peek() {
                TokenKind::Plus => BinaryOp::Add,
                TokenKind::Minus => BinaryOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_multiplicative();
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_multiplicative(&mut self) -> Expr {
        let mut left = self.parse_exponentiation();
        loop {
            let op = match self.peek() {
                TokenKind::Star => BinaryOp::Mul,
                TokenKind::Slash => BinaryOp::Div,
                TokenKind::Percent => BinaryOp::Mod,
                _ => break,
            };
            self.pos += 1;
            let right = self.parse_exponentiation();
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        left
    }

    fn parse_exponentiation(&mut self) -> Expr {
        let base = self.parse_unary();
        if self.eat(&TokenKind::StarStar) {
            let exp = self.parse_exponentiation(); // right-associative
            Expr::Binary {
                op: BinaryOp::Exp,
                left: Box::new(base),
                right: Box::new(exp),
            }
        } else {
            base
        }
    }

    fn parse_unary(&mut self) -> Expr {
        match self.peek() {
            TokenKind::Bang => {
                self.pos += 1;
                let arg = self.parse_unary();
                Expr::Unary {
                    op: UnaryOp::Not,
                    argument: Box::new(arg),
                    prefix: true,
                }
            }
            TokenKind::Tilde => {
                self.pos += 1;
                let arg = self.parse_unary();
                Expr::Unary {
                    op: UnaryOp::BitNot,
                    argument: Box::new(arg),
                    prefix: true,
                }
            }
            TokenKind::Minus => {
                self.pos += 1;
                let arg = self.parse_unary();
                Expr::Unary {
                    op: UnaryOp::Neg,
                    argument: Box::new(arg),
                    prefix: true,
                }
            }
            TokenKind::Plus => {
                self.pos += 1;
                let arg = self.parse_unary();
                Expr::Unary {
                    op: UnaryOp::Pos,
                    argument: Box::new(arg),
                    prefix: true,
                }
            }
            TokenKind::Typeof => {
                self.pos += 1;
                let arg = self.parse_unary();
                Expr::Typeof(Box::new(arg))
            }
            TokenKind::Void => {
                self.pos += 1;
                let arg = self.parse_unary();
                Expr::Void(Box::new(arg))
            }
            TokenKind::Delete => {
                self.pos += 1;
                let arg = self.parse_unary();
                Expr::Delete(Box::new(arg))
            }
            TokenKind::Await => {
                self.pos += 1;
                let arg = self.parse_unary();
                Expr::Await(Box::new(arg))
            }
            TokenKind::PlusPlus => {
                self.pos += 1;
                let arg = self.parse_unary();
                Expr::Update {
                    op: UpdateOp::Inc,
                    argument: Box::new(arg),
                    prefix: true,
                }
            }
            TokenKind::MinusMinus => {
                self.pos += 1;
                let arg = self.parse_unary();
                Expr::Update {
                    op: UpdateOp::Dec,
                    argument: Box::new(arg),
                    prefix: true,
                }
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expr = self.parse_call_expr();
        // Postfix ++ / --
        match self.peek() {
            TokenKind::PlusPlus => {
                self.pos += 1;
                expr = Expr::Update {
                    op: UpdateOp::Inc,
                    argument: Box::new(expr),
                    prefix: false,
                };
            }
            TokenKind::MinusMinus => {
                self.pos += 1;
                expr = Expr::Update {
                    op: UpdateOp::Dec,
                    argument: Box::new(expr),
                    prefix: false,
                };
            }
            _ => {}
        }
        expr
    }

    fn parse_call_expr(&mut self) -> Expr {
        let mut expr = self.parse_left_hand_side_expr();

        loop {
            match self.peek() {
                TokenKind::LParen => {
                    let args = self.parse_arguments();
                    expr = Expr::Call {
                        callee: Box::new(expr),
                        arguments: args,
                    };
                }
                TokenKind::Dot => {
                    self.pos += 1;
                    // Handle private field access: obj.#field
                    let prop = if let TokenKind::PrivateIdent(ref name) = self.peek().clone() {
                        let n = name.clone();
                        self.pos += 1;
                        n
                    } else {
                        self.ident_str()
                    };
                    expr = Expr::Member {
                        object: Box::new(expr),
                        property: prop,
                        computed: false,
                    };
                }
                TokenKind::QuestionDot => {
                    self.pos += 1;
                    if matches!(self.peek(), TokenKind::LParen) {
                        // Optional call: expr?.(args)
                        let args = self.parse_arguments();
                        expr = Expr::OptionalCall {
                            callee: Box::new(expr),
                            arguments: args,
                        };
                    } else if matches!(self.peek(), TokenKind::LBracket) {
                        // Optional computed access: expr?.[key]
                        self.pos += 1;
                        let index = self.parse_expression();
                        self.expect(&TokenKind::RBracket);
                        expr = Expr::Index {
                            object: Box::new(expr),
                            index: Box::new(index),
                        };
                    } else {
                        // Optional property access: expr?.prop
                        let prop = self.ident_str();
                        expr = Expr::OptionalChain {
                            object: Box::new(expr),
                            property: prop,
                        };
                    }
                }
                TokenKind::LBracket => {
                    self.pos += 1;
                    let index = self.parse_expression();
                    self.expect(&TokenKind::RBracket);
                    expr = Expr::Index {
                        object: Box::new(expr),
                        index: Box::new(index),
                    };
                }
                // Tagged template: expr`template`
                TokenKind::Template(ref s) => {
                    let template = s.clone();
                    self.pos += 1;
                    expr = Expr::TaggedTemplate {
                        tag: Box::new(expr),
                        template,
                    };
                }
                _ => break,
            }
        }
        expr
    }

    fn parse_arguments(&mut self) -> Vec<Expr> {
        self.expect(&TokenKind::LParen);
        let mut args = Vec::new();
        while !matches!(self.peek(), TokenKind::RParen | TokenKind::Eof) {
            if self.eat(&TokenKind::DotDotDot) {
                let expr = self.parse_assignment_expr();
                args.push(Expr::Spread(Box::new(expr)));
            } else {
                args.push(self.parse_assignment_expr());
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen);
        args
    }

    fn parse_left_hand_side_expr(&mut self) -> Expr {
        if matches!(self.peek(), TokenKind::New) {
            if matches!(self.peek2(), TokenKind::Dot) {
                // `new.target` meta-property
                self.pos += 1; // consume `new`
                self.pos += 1; // consume `.`
                let ident = self.ident_str();
                if ident == "target" {
                    return Expr::NewTarget;
                }
                // Invalid: `new.<something>` other than `target`
                self.syntax_error("expected 'target' after 'new.'");
                return Expr::Undefined;
            }
            self.pos += 1;
            let callee = self.parse_left_hand_side_expr();
            let arguments = if matches!(self.peek(), TokenKind::LParen) {
                self.parse_arguments()
            } else {
                Vec::new()
            };
            return Expr::New {
                callee: Box::new(callee),
                arguments,
            };
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Expr {
        self.depth += 1;
        if self.depth > MAX_PARSER_DEPTH {
            self.depth -= 1;
            self.syntax_error("Maximum nesting depth exceeded");
            self.pos = self.tokens.len(); // skip to end
            return Expr::Undefined;
        }
        let result = self.parse_primary_inner();
        self.depth -= 1;
        result
    }

    fn parse_primary_inner(&mut self) -> Expr {
        match self.peek().clone() {
            TokenKind::Number(n) => {
                self.pos += 1;
                Expr::Number(n)
            }
            TokenKind::String(s) => {
                self.pos += 1;
                Expr::String(s)
            }
            TokenKind::Template(s) => {
                self.pos += 1;
                Expr::Template(s)
            }
            TokenKind::RegExp(pattern, flags) => {
                self.pos += 1;
                Expr::RegExp { pattern, flags }
            }
            TokenKind::Bool(b) => {
                self.pos += 1;
                Expr::Bool(b)
            }
            TokenKind::Null => {
                self.pos += 1;
                Expr::Null
            }
            TokenKind::Undefined => {
                self.pos += 1;
                Expr::Undefined
            }
            TokenKind::This => {
                self.pos += 1;
                Expr::This
            }
            TokenKind::Ident(s) => {
                self.pos += 1;
                Expr::Ident(s)
            }
            TokenKind::LParen => {
                self.pos += 1;
                let expr = self.parse_expression();
                self.expect(&TokenKind::RParen);
                expr
            }
            TokenKind::LBracket => self.parse_array_literal(),
            TokenKind::LBrace => self.parse_object_literal(),
            TokenKind::Function => self.parse_function_expr(false),
            TokenKind::Async => {
                if matches!(self.peek2(), TokenKind::Function) {
                    self.pos += 1;
                    self.parse_function_expr(true)
                } else {
                    self.pos += 1;
                    Expr::Ident(String::from("async"))
                }
            }
            TokenKind::Class => self.parse_class_expr(),
            TokenKind::Super => {
                self.pos += 1;
                Expr::Ident(String::from("super"))
            }
            TokenKind::Yield => {
                self.pos += 1;
                // yield* expr  (delegate)
                if self.eat(&TokenKind::Star) {
                    let arg = self.parse_assignment_expr();
                    Expr::YieldDelegate(Box::new(arg))
                } else if matches!(self.peek(), TokenKind::Semicolon | TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket | TokenKind::Comma | TokenKind::Colon | TokenKind::Eof) {
                    Expr::Yield(None)
                } else {
                    let arg = self.parse_assignment_expr();
                    Expr::Yield(Some(Box::new(arg)))
                }
            }
            _ => {
                // Error recovery: skip token and return undefined
                self.pos += 1;
                Expr::Undefined
            }
        }
    }

    fn parse_array_literal(&mut self) -> Expr {
        self.expect(&TokenKind::LBracket);
        let mut elements = Vec::new();
        while !matches!(self.peek(), TokenKind::RBracket | TokenKind::Eof) {
            if self.eat(&TokenKind::Comma) {
                elements.push(None);
                continue;
            }
            if self.eat(&TokenKind::DotDotDot) {
                let expr = self.parse_assignment_expr();
                elements.push(Some(Expr::Spread(Box::new(expr))));
            } else {
                elements.push(Some(self.parse_assignment_expr()));
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBracket);
        Expr::Array(elements)
    }

    fn parse_object_literal(&mut self) -> Expr {
        self.expect(&TokenKind::LBrace);
        let mut props = Vec::new();
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            if self.eat(&TokenKind::DotDotDot) {
                let expr = self.parse_assignment_expr();
                props.push(ObjProp {
                    key: PropKey::Ident(String::from("...")),
                    value: expr,
                    kind: PropKind::Init,
                    shorthand: false,
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                continue;
            }

            // Check for async method shorthand: { async foo() { } }
            // async is a modifier when followed by an identifier/keyword + '(' (not colon/comma)
            let is_async_method = matches!(self.peek(), TokenKind::Async)
                && !matches!(self.peek2(), TokenKind::Colon | TokenKind::Comma | TokenKind::RBrace | TokenKind::LParen);
            if is_async_method {
                self.pos += 1; // skip 'async'
                let is_generator = self.eat(&TokenKind::Star);
                let key = self.parse_prop_key();
                let params = self.parse_params();
                self.expect(&TokenKind::LBrace);
                let body = self.parse_block_body();
                self.expect(&TokenKind::RBrace);
                props.push(ObjProp {
                    key,
                    value: Expr::FunctionExpr {
                        name: None,
                        params,
                        body,
                        is_async: true,
                        is_generator,
                    },
                    kind: PropKind::Method,
                    shorthand: false,
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                continue;
            }

            // Check for get/set accessor
            let is_get = matches!(self.peek(), TokenKind::Ident(ref s) if s == "get");
            let is_set = matches!(self.peek(), TokenKind::Ident(ref s) if s == "set");
            if (is_get || is_set) && !matches!(self.peek2(), TokenKind::Colon | TokenKind::Comma | TokenKind::RBrace | TokenKind::LParen) {
                let accessor_kind = if is_get { PropKind::Get } else { PropKind::Set };
                self.pos += 1; // skip 'get' or 'set'
                let key = self.parse_prop_key();
                let params = self.parse_params();
                self.expect(&TokenKind::LBrace);
                let body = self.parse_block_body();
                self.expect(&TokenKind::RBrace);
                props.push(ObjProp {
                    key,
                    value: Expr::FunctionExpr {
                        name: None,
                        params,
                        body,
                        is_async: false,
                        is_generator: false,
                    },
                    kind: accessor_kind,
                    shorthand: false,
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                continue;
            }

            let key = self.parse_prop_key();

            // Shorthand property: { x } → { x: x }
            // CoverInitializedName: { x = default } → { x: x = default }
            if matches!(self.peek(), TokenKind::Comma | TokenKind::RBrace | TokenKind::Eq) {
                if let PropKey::Ident(ref name) = key {
                    let value = if self.eat(&TokenKind::Eq) {
                        // { x = default } — shorthand with default (destructuring pattern)
                        let default = self.parse_assignment_expr();
                        Expr::Assign {
                            op: AssignOp::Assign,
                            left: Box::new(Expr::Ident(name.clone())),
                            right: Box::new(default),
                        }
                    } else {
                        Expr::Ident(name.clone())
                    };
                    props.push(ObjProp {
                        key: key.clone(),
                        value,
                        kind: PropKind::Init,
                        shorthand: true,
                    });
                    self.eat(&TokenKind::Comma);
                    continue;
                }
            }

            // Method shorthand: { foo() { } }
            if matches!(self.peek(), TokenKind::LParen) {
                let params = self.parse_params();
                self.expect(&TokenKind::LBrace);
                let body = self.parse_block_body();
                self.expect(&TokenKind::RBrace);
                props.push(ObjProp {
                    key,
                    value: Expr::FunctionExpr {
                        name: None,
                        params,
                        body,
                        is_async: false,
                        is_generator: false,
                    },
                    kind: PropKind::Method,
                    shorthand: false,
                });
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                continue;
            }

            self.expect(&TokenKind::Colon);
            let value = self.parse_assignment_expr();
            props.push(ObjProp {
                key,
                value,
                kind: PropKind::Init,
                shorthand: false,
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBrace);
        Expr::Object(props)
    }

    fn parse_function_expr(&mut self, is_async: bool) -> Expr {
        self.expect(&TokenKind::Function);
        let is_generator = self.eat(&TokenKind::Star);
        let name = if let TokenKind::Ident(s) = self.peek().clone() {
            self.pos += 1;
            Some(s)
        } else {
            None
        };
        let params = self.parse_params();
        self.expect(&TokenKind::LBrace);
        let body = self.parse_block_body();
        self.expect(&TokenKind::RBrace);
        Expr::FunctionExpr { name, params, body, is_async, is_generator }
    }

    fn parse_class_expr(&mut self) -> Expr {
        self.expect(&TokenKind::Class);
        let name = if let TokenKind::Ident(s) = self.peek().clone() {
            self.pos += 1;
            Some(s)
        } else {
            None
        };
        let super_class = if self.eat(&TokenKind::Extends) {
            Some(Box::new(self.parse_assignment_expr()))
        } else {
            None
        };
        let body = self.parse_class_body();
        Expr::ClassExpr { name, super_class, body }
    }
}
