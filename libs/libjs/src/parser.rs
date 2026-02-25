//! Recursive descent JavaScript parser.
//!
//! Parses a token stream into an AST (Abstract Syntax Tree).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use alloc::vec;
use alloc::string::ToString;

use crate::token::{Token, TokenKind};
use crate::ast::*;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser { tokens, pos: 0 }
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

    fn ident_str(&mut self) -> String {
        match self.peek().clone() {
            TokenKind::Ident(s) => {
                self.pos += 1;
                s
            }
            _ => {
                self.pos += 1;
                String::from("_error_")
            }
        }
    }

    // ── Statements ──

    fn parse_statement(&mut self) -> Option<Stmt> {
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
                if matches!(self.peek2(), TokenKind::Ident(_) | TokenKind::Star) {
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
        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
            if let Some(stmt) = self.parse_statement() {
                stmts.push(stmt);
            }
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
                Some(Box::new(ForInit::VarDecl { kind, decls }))
            }
            _ => {
                let expr = self.parse_expression();
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

    fn parse_function_decl(&mut self, is_async: bool) -> Stmt {
        self.expect(&TokenKind::Function);
        let name = self.ident_str();
        let params = self.parse_params();
        self.expect(&TokenKind::LBrace);
        let body = self.parse_block_body();
        self.expect(&TokenKind::RBrace);
        Stmt::FunctionDecl { name, params, body, is_async }
    }

    fn parse_class_decl(&mut self) -> Stmt {
        self.expect(&TokenKind::Class);
        let name = self.ident_str();
        let super_class = if self.eat(&TokenKind::Extends) {
            Some(self.parse_left_hand_side_expr())
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
                    ClassMemberKind::Method { params, body }
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
                // Use keyword as identifier name
                let s = match self.peek() {
                    TokenKind::Default | TokenKind::New |
                    TokenKind::Delete | TokenKind::In | TokenKind::Return |
                    TokenKind::Class | TokenKind::Super | TokenKind::This => {
                        self.pos += 1;
                        String::from("_kw_")
                    }
                    _ => {
                        self.pos += 1;
                        String::from("_error_")
                    }
                };
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
                params.push(Param { pattern, default: None });
                break;
            }
            let pattern = self.parse_binding_pattern();
            let default = if self.eat(&TokenKind::Eq) {
                Some(self.parse_assignment_expr())
            } else {
                None
            };
            params.push(Param { pattern, default });
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
        let expr = self.parse_assignment_expr();
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

    fn parse_arrow_function(&mut self, is_async: bool) -> Expr {
        let params = if matches!(self.peek(), TokenKind::Ident(_)) && matches!(self.peek2(), TokenKind::Arrow) {
            let name = self.ident_str();
            vec![Param {
                pattern: Pattern::Ident(name),
                default: None,
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
                TokenKind::In => BinaryOp::In,
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
                    let prop = self.ident_str();
                    expr = Expr::Member {
                        object: Box::new(expr),
                        property: prop,
                        computed: false,
                    };
                }
                TokenKind::QuestionDot => {
                    self.pos += 1;
                    let prop = self.ident_str();
                    expr = Expr::OptionalChain {
                        object: Box::new(expr),
                        property: prop,
                    };
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
        if matches!(self.peek(), TokenKind::New) && !matches!(self.peek2(), TokenKind::Dot) {
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
                if matches!(self.peek(), TokenKind::Semicolon | TokenKind::RBrace | TokenKind::RParen | TokenKind::RBracket | TokenKind::Comma | TokenKind::Colon | TokenKind::Eof) {
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

            let key = self.parse_prop_key();

            // Shorthand property: { x } → { x: x }
            if matches!(self.peek(), TokenKind::Comma | TokenKind::RBrace) {
                if let PropKey::Ident(ref name) = key {
                    props.push(ObjProp {
                        key: key.clone(),
                        value: Expr::Ident(name.clone()),
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
        Expr::FunctionExpr { name, params, body, is_async }
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
            Some(Box::new(self.parse_left_hand_side_expr()))
        } else {
            None
        };
        let body = self.parse_class_body();
        Expr::ClassExpr { name, super_class, body }
    }
}
