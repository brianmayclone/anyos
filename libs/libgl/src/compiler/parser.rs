//! GLSL ES 1.00 recursive-descent parser.
//!
//! Converts a token stream into an [`ast::TranslationUnit`]. Supports the Phase 1
//! subset: variable declarations with qualifiers, function definitions (main only),
//! assignments, return statements, and expressions with operators, swizzles, and calls.

use alloc::string::String;
use alloc::vec::Vec;
use super::lexer::Token;
use super::ast::*;
use crate::types::*;

/// Parser state.
struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    shader_type: GLenum,
}

/// Parse a token stream into an AST.
pub fn parse(tokens: &[Token], shader_type: GLenum) -> Result<TranslationUnit, String> {
    let mut p = Parser { tokens, pos: 0, shader_type };
    let mut decls = Vec::new();

    while !p.at_end() {
        decls.push(p.parse_declaration()?);
    }

    Ok(TranslationUnit { declarations: decls })
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<&Token> {
        let tok = self.tokens.get(self.pos);
        self.pos += 1;
        tok
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        match self.peek() {
            Some(tok) if tok == expected => { self.advance(); Ok(()) }
            Some(tok) => Err(alloc::format!("Expected {:?}, got {:?}", expected, tok)),
            None => Err(String::from("Unexpected end of input")),
        }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.advance() {
            Some(Token::Ident(s)) => Ok(s.clone()),
            Some(tok) => Err(alloc::format!("Expected identifier, got {:?}", tok)),
            None => Err(String::from("Expected identifier, got end of input")),
        }
    }

    fn parse_declaration(&mut self) -> Result<Declaration, String> {
        // Precision declaration
        if matches!(self.peek(), Some(Token::Precision)) {
            return self.parse_precision();
        }

        // Try qualifier
        let qualifier = self.parse_qualifier();

        // Type specifier
        let type_spec = self.parse_type_spec()?;

        // Name
        let name = self.expect_ident()?;

        // Function definition?
        if matches!(self.peek(), Some(Token::LParen)) {
            return self.parse_function_def(type_spec, name);
        }

        // Variable declaration
        let init = if matches!(self.peek(), Some(Token::Eq)) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };

        self.expect(&Token::Semicolon)?;

        Ok(Declaration::Variable(VarDecl {
            qualifier,
            type_spec,
            name,
            initializer: init,
        }))
    }

    fn parse_precision(&mut self) -> Result<Declaration, String> {
        self.advance(); // consume 'precision'
        let prec = match self.peek() {
            Some(Token::LowP) => { self.advance(); PrecisionQualifier::LowP }
            Some(Token::MediumP) => { self.advance(); PrecisionQualifier::MediumP }
            Some(Token::HighP) => { self.advance(); PrecisionQualifier::HighP }
            _ => PrecisionQualifier::MediumP,
        };
        let ty = self.parse_type_spec()?;
        self.expect(&Token::Semicolon)?;
        Ok(Declaration::Precision(prec, ty))
    }

    fn parse_qualifier(&mut self) -> StorageQualifier {
        match self.peek() {
            Some(Token::Attribute) => { self.advance(); StorageQualifier::Attribute }
            Some(Token::Varying) => { self.advance(); StorageQualifier::Varying }
            Some(Token::Uniform) => { self.advance(); StorageQualifier::Uniform }
            Some(Token::Const) => { self.advance(); StorageQualifier::Const }
            _ => StorageQualifier::None,
        }
    }

    fn parse_type_spec(&mut self) -> Result<TypeSpec, String> {
        match self.advance() {
            Some(Token::Void) => Ok(TypeSpec::Void),
            Some(Token::Float) => Ok(TypeSpec::Float),
            Some(Token::Int) => Ok(TypeSpec::Int),
            Some(Token::Bool) => Ok(TypeSpec::Bool),
            Some(Token::Vec2) => Ok(TypeSpec::Vec2),
            Some(Token::Vec3) => Ok(TypeSpec::Vec3),
            Some(Token::Vec4) => Ok(TypeSpec::Vec4),
            Some(Token::Mat3) => Ok(TypeSpec::Mat3),
            Some(Token::Mat4) => Ok(TypeSpec::Mat4),
            Some(Token::Sampler2D) => Ok(TypeSpec::Sampler2D),
            Some(tok) => Err(alloc::format!("Expected type, got {:?}", tok)),
            None => Err(String::from("Expected type, got end of input")),
        }
    }

    fn parse_function_def(&mut self, ret_type: TypeSpec, name: String) -> Result<Declaration, String> {
        self.expect(&Token::LParen)?;
        let mut params = Vec::new();
        if !matches!(self.peek(), Some(Token::RParen)) {
            loop {
                // Skip qualifier if present in param
                let _ = self.parse_qualifier();
                let ty = self.parse_type_spec()?;
                let pname = self.expect_ident()?;
                params.push((ty, pname));
                if matches!(self.peek(), Some(Token::Comma)) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect(&Token::RParen)?;

        let body = self.parse_block()?;

        Ok(Declaration::Function(FunctionDef {
            return_type: ret_type,
            name,
            params,
            body,
        }))
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect(&Token::LBrace)?;
        let mut stmts = Vec::new();
        while !matches!(self.peek(), Some(Token::RBrace) | None) {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&Token::RBrace)?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        // Return
        if matches!(self.peek(), Some(Token::Return)) {
            self.advance();
            if matches!(self.peek(), Some(Token::Semicolon)) {
                self.advance();
                return Ok(Stmt::Return(None));
            }
            let expr = self.parse_expr()?;
            self.expect(&Token::Semicolon)?;
            return Ok(Stmt::Return(Some(expr)));
        }

        // Discard
        if matches!(self.peek(), Some(Token::Discard)) {
            self.advance();
            self.expect(&Token::Semicolon)?;
            return Ok(Stmt::Discard);
        }

        // If statement
        if matches!(self.peek(), Some(Token::If)) {
            return self.parse_if();
        }

        // For loop
        if matches!(self.peek(), Some(Token::For)) {
            return self.parse_for();
        }

        // Check for type spec (local variable declaration)
        if self.is_type_token() {
            return self.parse_local_var_decl();
        }

        // Expression or assignment
        let expr = self.parse_expr()?;

        // Assignment?
        match self.peek() {
            Some(Token::Eq) => {
                self.advance();
                let rhs = self.parse_expr()?;
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::Assign(expr, rhs))
            }
            Some(Token::PlusEq) => {
                self.advance();
                let rhs = self.parse_expr()?;
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::CompoundAssign(expr, CompoundOp::Add, rhs))
            }
            Some(Token::MinusEq) => {
                self.advance();
                let rhs = self.parse_expr()?;
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::CompoundAssign(expr, CompoundOp::Sub, rhs))
            }
            Some(Token::StarEq) => {
                self.advance();
                let rhs = self.parse_expr()?;
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::CompoundAssign(expr, CompoundOp::Mul, rhs))
            }
            Some(Token::SlashEq) => {
                self.advance();
                let rhs = self.parse_expr()?;
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::CompoundAssign(expr, CompoundOp::Div, rhs))
            }
            _ => {
                self.expect(&Token::Semicolon)?;
                Ok(Stmt::Expr(expr))
            }
        }
    }

    fn parse_if(&mut self) -> Result<Stmt, String> {
        self.advance(); // consume 'if'
        self.expect(&Token::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(&Token::RParen)?;
        let then_block = self.parse_block()?;
        let else_block = if matches!(self.peek(), Some(Token::Else)) {
            self.advance();
            Some(self.parse_block()?)
        } else {
            None
        };
        Ok(Stmt::If(cond, then_block, else_block))
    }

    fn parse_for(&mut self) -> Result<Stmt, String> {
        self.advance(); // consume 'for'
        self.expect(&Token::LParen)?;
        let init = self.parse_stmt()?;
        let cond = self.parse_expr()?;
        self.expect(&Token::Semicolon)?;
        // Increment — parse as expression then wrap
        let inc_expr = self.parse_expr()?;
        // Check for assignment in increment
        let inc = if matches!(self.peek(), Some(Token::Eq)) {
            self.advance();
            let rhs = self.parse_expr()?;
            Stmt::Assign(inc_expr, rhs)
        } else {
            Stmt::Expr(inc_expr)
        };
        self.expect(&Token::RParen)?;
        let body = self.parse_block()?;
        Ok(Stmt::For(Box::new(init), cond, Box::new(inc), body))
    }

    fn parse_local_var_decl(&mut self) -> Result<Stmt, String> {
        let ty = self.parse_type_spec()?;
        let name = self.expect_ident()?;
        let init = if matches!(self.peek(), Some(Token::Eq)) {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(&Token::Semicolon)?;
        Ok(Stmt::VarDecl(VarDecl {
            qualifier: StorageQualifier::None,
            type_spec: ty,
            name,
            initializer: init,
        }))
    }

    fn is_type_token(&self) -> bool {
        matches!(self.peek(),
            Some(Token::Void | Token::Float | Token::Int | Token::Bool |
                 Token::Vec2 | Token::Vec3 | Token::Vec4 |
                 Token::Mat3 | Token::Mat4 | Token::Sampler2D))
    }

    // ── Expression Parsing (Pratt-style precedence climbing) ────────────

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Result<Expr, String> {
        let cond = self.parse_or()?;
        if matches!(self.peek(), Some(Token::Question)) {
            self.advance();
            let then_expr = self.parse_expr()?;
            self.expect(&Token::Colon)?;
            let else_expr = self.parse_expr()?;
            Ok(Expr::Ternary(Box::new(cond), Box::new(then_expr), Box::new(else_expr)))
        } else {
            Ok(cond)
        }
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Some(Token::Or)) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Binary(Box::new(left), BinOp::Or, Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_equality()?;
        while matches!(self.peek(), Some(Token::And)) {
            self.advance();
            let right = self.parse_equality()?;
            left = Expr::Binary(Box::new(left), BinOp::And, Box::new(right));
        }
        Ok(left)
    }

    fn parse_equality(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_comparison()?;
        loop {
            match self.peek() {
                Some(Token::EqEq) => {
                    self.advance();
                    let right = self.parse_comparison()?;
                    left = Expr::Binary(Box::new(left), BinOp::Eq, Box::new(right));
                }
                Some(Token::NotEq) => {
                    self.advance();
                    let right = self.parse_comparison()?;
                    left = Expr::Binary(Box::new(left), BinOp::NotEq, Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_additive()?;
        loop {
            match self.peek() {
                Some(Token::Less) => {
                    self.advance();
                    let right = self.parse_additive()?;
                    left = Expr::Binary(Box::new(left), BinOp::Less, Box::new(right));
                }
                Some(Token::LessEq) => {
                    self.advance();
                    let right = self.parse_additive()?;
                    left = Expr::Binary(Box::new(left), BinOp::LessEq, Box::new(right));
                }
                Some(Token::Greater) => {
                    self.advance();
                    let right = self.parse_additive()?;
                    left = Expr::Binary(Box::new(left), BinOp::Greater, Box::new(right));
                }
                Some(Token::GreaterEq) => {
                    self.advance();
                    let right = self.parse_additive()?;
                    left = Expr::Binary(Box::new(left), BinOp::GreaterEq, Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_multiplicative()?;
        loop {
            match self.peek() {
                Some(Token::Plus) => {
                    self.advance();
                    let right = self.parse_multiplicative()?;
                    left = Expr::Binary(Box::new(left), BinOp::Add, Box::new(right));
                }
                Some(Token::Minus) => {
                    self.advance();
                    let right = self.parse_multiplicative()?;
                    left = Expr::Binary(Box::new(left), BinOp::Sub, Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, String> {
        let mut left = self.parse_unary()?;
        loop {
            match self.peek() {
                Some(Token::Star) => {
                    self.advance();
                    let right = self.parse_unary()?;
                    left = Expr::Binary(Box::new(left), BinOp::Mul, Box::new(right));
                }
                Some(Token::Slash) => {
                    self.advance();
                    let right = self.parse_unary()?;
                    left = Expr::Binary(Box::new(left), BinOp::Div, Box::new(right));
                }
                _ => break,
            }
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Token::Minus) => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary(UnaryOp::Neg, Box::new(expr)))
            }
            Some(Token::Not) => {
                self.advance();
                let expr = self.parse_unary()?;
                Ok(Expr::Unary(UnaryOp::Not, Box::new(expr)))
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;

        loop {
            match self.peek() {
                Some(Token::Dot) => {
                    self.advance();
                    let field = self.expect_ident()?;
                    // Swizzle if all chars are xyzw or rgba
                    if is_swizzle(&field) && field.len() >= 1 {
                        expr = Expr::Swizzle(Box::new(expr), field);
                    } else {
                        expr = Expr::Field(Box::new(expr), field);
                    }
                }
                Some(Token::LBracket) => {
                    self.advance();
                    let index = self.parse_expr()?;
                    self.expect(&Token::RBracket)?;
                    expr = Expr::Index(Box::new(expr), Box::new(index));
                }
                _ => break,
            }
        }

        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        match self.peek().cloned() {
            Some(Token::IntLiteral(v)) => { self.advance(); Ok(Expr::IntLit(v)) }
            Some(Token::FloatLiteral(v)) => { self.advance(); Ok(Expr::FloatLit(v)) }
            Some(Token::BoolLiteral(v)) => { self.advance(); Ok(Expr::BoolLit(v)) }
            Some(Token::LParen) => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            // Type constructors: vec4(...), mat4(...), float(...), etc.
            Some(Token::Vec2 | Token::Vec3 | Token::Vec4 |
                 Token::Mat3 | Token::Mat4 |
                 Token::Float | Token::Int | Token::Bool) => {
                let name = match self.advance().unwrap() {
                    Token::Vec2 => "vec2",
                    Token::Vec3 => "vec3",
                    Token::Vec4 => "vec4",
                    Token::Mat3 => "mat3",
                    Token::Mat4 => "mat4",
                    Token::Float => "float",
                    Token::Int => "int",
                    Token::Bool => "bool",
                    _ => unreachable!(),
                };
                self.expect(&Token::LParen)?;
                let args = self.parse_arg_list()?;
                self.expect(&Token::RParen)?;
                Ok(Expr::Call(String::from(name), args))
            }
            Some(Token::Ident(name)) => {
                self.advance();
                // Function call?
                if matches!(self.peek(), Some(Token::LParen)) {
                    self.advance();
                    let args = self.parse_arg_list()?;
                    self.expect(&Token::RParen)?;
                    Ok(Expr::Call(name, args))
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            Some(tok) => Err(alloc::format!("Unexpected token in expression: {:?}", tok)),
            None => Err(String::from("Unexpected end of input in expression")),
        }
    }

    fn parse_arg_list(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        if matches!(self.peek(), Some(Token::RParen)) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            if matches!(self.peek(), Some(Token::Comma)) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(args)
    }
}

/// Check if a string is a valid swizzle pattern (only xyzw/rgba/stpq characters).
fn is_swizzle(s: &str) -> bool {
    if s.is_empty() || s.len() > 4 { return false; }
    s.bytes().all(|b| matches!(b, b'x' | b'y' | b'z' | b'w' | b'r' | b'g' | b'b' | b'a' | b's' | b't' | b'p' | b'q'))
}
