use crate::prelude::*;
use crate::ast::*;
use crate::diagnostics::Span;
use crate::intern::{Interner, Symbol};
use crate::lexer::{IntSuffix, Keyword, Lexer, Token, TokenKind};

pub struct Parser<'a> {
    tokens: Vec<Token>,
    pos: usize,
    prev_span: Span,
    interner: &'a mut Interner,
    /// When true, `{` starts a block, not a struct literal (if/while/match/for conditions)
    no_struct_literal: bool,
    /// Statement parsing uses this to stop after block-like expressions before
    /// a following token that starts the next statement (for example `} *ptr`).
    stop_stmt_after_block_expr: bool,
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a str, interner: &'a mut Interner) -> Self {
        let mut lexer = Lexer::new(src, interner);
        let mut tokens = Vec::new();
        loop {
            let tok = lexer.next_token();
            let is_eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        Self {
            tokens,
            pos: 0,
            prev_span: Span::dummy(),
            interner,
            no_struct_literal: false,
            stop_stmt_after_block_expr: false,
        }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn ty_from_int_suffix(&mut self, suffix: &IntSuffix, span: Span) -> Ty {
        let name = match suffix {
            IntSuffix::I8 => "i8",
            IntSuffix::I16 => "i16",
            IntSuffix::I32 => "i32",
            IntSuffix::I64 => "i64",
            IntSuffix::I128 => "i128",
            IntSuffix::Isize => "isize",
            IntSuffix::U8 => "u8",
            IntSuffix::U16 => "u16",
            IntSuffix::U32 => "u32",
            IntSuffix::U64 => "u64",
            IntSuffix::U128 => "u128",
            IntSuffix::Usize => "usize",
        };
        Ty::Path(Path {
            segments: vec![PathSegment {
                ident: self.interner.intern(name),
                args: None,
            }],
            span,
        })
    }

    fn bump(&mut self) -> Token {
        let tok = self.tokens[self.pos.min(self.tokens.len() - 1)].clone();
        self.prev_span = tok.span;
        if self.pos < self.tokens.len() - 1 {
            self.pos += 1;
        }
        tok
    }

    fn at(&self, kind: &TokenKind) -> bool {
        core::mem::discriminant(&self.current().kind) == core::mem::discriminant(kind)
            || self.current().kind == *kind
    }

    fn at_exact(&self, kind: &TokenKind) -> bool {
        self.current().kind == *kind
    }

    fn at_kw(&self, kw: Keyword) -> bool {
        self.current().kind == TokenKind::Kw(kw)
    }

    fn at_ident(&self) -> bool {
        matches!(self.current().kind, TokenKind::Ident(_))
    }

    fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_exact(&mut self, kind: &TokenKind) -> bool {
        if self.at_exact(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: &TokenKind) -> Token {
        if self.at(kind) {
            self.bump()
        } else {
            panic!(
                "expected {:?}, got {:?} at {:?}; near {}",
                kind,
                self.current().kind,
                self.current().span,
                self.token_window()
            );
        }
    }

    fn expect_exact(&mut self, kind: &TokenKind) -> Token {
        if self.at_exact(kind) {
            self.bump()
        } else {
            panic!(
                "expected {:?}, got {:?} at {:?}; near {}",
                kind,
                self.current().kind,
                self.current().span,
                self.token_window()
            );
        }
    }

    fn expect_ident(&mut self) -> Symbol {
        match self.current().kind {
            TokenKind::Ident(sym) => {
                self.bump();
                sym
            }
            TokenKind::Kw(Keyword::Ref) => {
                self.bump();
                self.interner.intern("ref")
            }
            _ => panic!(
                "expected identifier, got {:?} at {:?}; near {}",
                self.current().kind,
                self.current().span,
                self.token_window()
            ),
        }
    }

    fn token_window(&self) -> String {
        let start = self.pos.saturating_sub(5);
        let end = (self.pos + 6).min(self.tokens.len());
        let mut out = String::new();
        for idx in start..end {
            if idx > start {
                out.push(' ');
            }
            if idx == self.pos {
                out.push_str(">>");
            }
            match self.tokens[idx].kind {
                TokenKind::Ident(sym) => out.push_str(&format!(
                    "Ident({})@{:?}",
                    self.interner.resolve(sym),
                    self.tokens[idx].span
                )),
                TokenKind::Lifetime(sym) => out.push_str(&format!(
                    "Lifetime({})@{:?}",
                    self.interner.resolve(sym),
                    self.tokens[idx].span
                )),
                TokenKind::DocComment(ref text, inner) => out.push_str(&format!(
                    "DocComment(inner={}, {:?})@{:?}",
                    inner,
                    text,
                    self.tokens[idx].span
                )),
                _ => out.push_str(&format!("{:?}@{:?}", self.tokens[idx].kind, self.tokens[idx].span)),
            }
            if idx == self.pos {
                out.push_str("<<");
            }
        }
        out
    }

    fn expect_ident_or_self(&mut self) -> Symbol {
        match self.current().kind {
            TokenKind::Dollar if matches!(self.peek_kind(), TokenKind::Kw(Keyword::Crate)) => {
                self.bump();
                self.bump();
                self.interner.intern("crate")
            }
            TokenKind::Ident(sym) => {
                self.bump();
                sym
            }
            TokenKind::Kw(Keyword::SelfValue) => {
                self.bump();
                self.interner.intern("self")
            }
            TokenKind::Kw(Keyword::SelfType) => {
                self.bump();
                self.interner.intern("Self")
            }
            TokenKind::Kw(Keyword::Super) => {
                self.bump();
                self.interner.intern("super")
            }
            TokenKind::Kw(Keyword::Crate) => {
                self.bump();
                self.interner.intern("crate")
            }
            TokenKind::Kw(Keyword::Ref) => {
                self.bump();
                self.interner.intern("ref")
            }
            _ => panic!(
                "expected identifier, got {:?} at {:?}; near {}",
                self.current().kind,
                self.current().span,
                self.token_window()
            ),
        }
    }

    fn span_from(&self, start: Span) -> Span {
        Span::new(start.start(), self.prev_span.end())
    }

    // ── Expressions (Pratt parsing) ──

    pub fn parse_expr(&mut self) -> Expr {
        self.parse_expr_bp(0)
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Expr {
        let start = self.current().span;
        let stmt_boundary = self.stop_stmt_after_block_expr && min_bp == 0;
        let old_stop_stmt_after_block_expr = self.stop_stmt_after_block_expr;
        if stmt_boundary {
            self.stop_stmt_after_block_expr = false;
        }
        let mut lhs = self.parse_prefix_expr();

        loop {
            // Postfix operators
            if let Some(post_bp) = self.postfix_bp() {
                if matches!(self.current().kind, TokenKind::LParen)
                    && self.expr_can_end_stmt_without_semicolon(&lhs)
                    && self.current().span.start() > self.prev_span.end()
                {
                    break;
                }
                if post_bp < min_bp {
                    break;
                }
                lhs = self.parse_postfix(lhs, start);
                continue;
            }

            // `as` cast
            if self.at_kw(Keyword::As) {
                let (l_bp, _r_bp) = (24u8, 25u8);
                if l_bp < min_bp {
                    break;
                }
                self.bump();
                let ty = self.parse_ty();
                lhs = Expr::Cast(Box::new(lhs), ty, self.span_from(start));
                continue;
            }

            // Infix operators
            if let Some((l_bp, r_bp, op)) = self.infix_bp() {
                if stmt_boundary && self.expr_can_end_stmt_without_semicolon(&lhs) {
                    break;
                }
                if l_bp < min_bp {
                    break;
                }
                self.bump();
                let span = self.span_from(start);
                if let InfixOp::Range(inclusive) = op {
                    let rhs = if self.at_expr_start() {
                        Some(Box::new(self.parse_expr_bp(r_bp)))
                    } else {
                        None
                    };
                    lhs = Expr::Range(Some(Box::new(lhs)), rhs, inclusive, span);
                    continue;
                }
                let rhs = self.parse_expr_bp(r_bp);
                lhs = match op {
                    InfixOp::Binary(binop) => Expr::Binary(binop, Box::new(lhs), Box::new(rhs), span),
                    InfixOp::Assign => Expr::Assign(Box::new(lhs), Box::new(rhs), span),
                    InfixOp::AssignOp(binop) => {
                        Expr::AssignOp(binop, Box::new(lhs), Box::new(rhs), span)
                    }
                    InfixOp::Range(_) => unreachable!(),
                };
                continue;
            }

            break;
        }

        self.stop_stmt_after_block_expr = old_stop_stmt_after_block_expr;
        lhs
    }

    fn parse_prefix_expr(&mut self) -> Expr {
        let start = self.current().span;

        // Outer expression attributes, e.g. `foo(#[inline(always)] |x| x)`.
        if self.at_exact(&TokenKind::Hash) {
            let attrs = self.parse_attrs();
            let expr = self.parse_prefix_expr();
            return Expr::Attributed(attrs, Box::new(expr), self.span_from(start));
        }

        // Unary minus
        if self.at_exact(&TokenKind::Minus) {
            self.bump();
            let operand = self.parse_expr_bp(27);
            return Expr::Unary(UnOp::Neg, Box::new(operand), self.span_from(start));
        }

        // Unary not
        if self.at_exact(&TokenKind::Not) {
            self.bump();
            let operand = self.parse_expr_bp(27);
            return Expr::Unary(UnOp::Not, Box::new(operand), self.span_from(start));
        }

        // Unary deref
        if self.at_exact(&TokenKind::Star) {
            self.bump();
            let operand = self.parse_expr_bp(27);
            return Expr::Deref(Box::new(operand), self.span_from(start));
        }

        // Tokenized `&&expr` is two immutable borrows in expression position.
        if self.at_exact(&TokenKind::AndAnd) {
            self.bump();
            let inner_start = self.current().span;
            let operand = self.parse_expr_bp(27);
            let inner = Expr::Ref(Box::new(operand), Mutability::Immutable, self.span_from(inner_start));
            return Expr::Ref(Box::new(inner), Mutability::Immutable, self.span_from(start));
        }

        // Reference & and &mut
        if self.at_exact(&TokenKind::Amp) {
            self.bump();
            let mutability = if self.eat_exact(&TokenKind::Kw(Keyword::Mut)) {
                Mutability::Mut
            } else {
                Mutability::Immutable
            };
            let operand = self.parse_expr_bp(27);
            return Expr::Ref(Box::new(operand), mutability, self.span_from(start));
        }

        // return
        if self.at_kw(Keyword::Return) {
            self.bump();
            let val = if self.at_expr_start() {
                Some(Box::new(self.parse_expr()))
            } else {
                None
            };
            return Expr::Return(val, self.span_from(start));
        }

        // break
        if self.at_kw(Keyword::Break) {
            self.bump();
            let label = if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                if let TokenKind::Lifetime(sym) = self.bump().kind {
                    Some(sym)
                } else {
                    None
                }
            } else {
                None
            };
            let val = if self.at_expr_start() && !self.at_exact(&TokenKind::LBrace) {
                Some(Box::new(self.parse_expr()))
            } else {
                None
            };
            return Expr::Break(label, val, self.span_from(start));
        }

        // continue
        if self.at_kw(Keyword::Continue) {
            self.bump();
            let label = if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                if let TokenKind::Lifetime(sym) = self.bump().kind {
                    Some(sym)
                } else {
                    None
                }
            } else {
                None
            };
            return Expr::Continue(label, self.span_from(start));
        }

        // Labelled loop/while/for: 'label: loop { ... }
        if matches!(self.current().kind, TokenKind::Lifetime(_))
            && self.peek_kind() == &TokenKind::Colon
        {
            let label = if let TokenKind::Lifetime(sym) = self.bump().kind {
                sym
            } else {
                unreachable!()
            };
            self.expect_exact(&TokenKind::Colon);
            let expr = self.parse_prefix_expr();
            return match expr {
                Expr::Loop(block, _, span) => Expr::Loop(block, Some(label), span),
                Expr::While(cond, block, _, span) => Expr::While(cond, block, Some(label), span),
                Expr::WhileLet(pat, scrutinee, block, _, span) => {
                    Expr::WhileLet(pat, scrutinee, block, Some(label), span)
                }
                Expr::For(pat, iter, block, _, span) => Expr::For(pat, iter, block, Some(label), span),
                other => other,
            };
        }

        // closure: |params| expr  or  move |params| expr
        if self.at_exact(&TokenKind::Pipe)
            || self.at_exact(&TokenKind::OrOr)
            || (self.at_kw(Keyword::Move)
                && (self.peek_kind() == &TokenKind::Pipe
                    || self.peek_kind() == &TokenKind::OrOr))
        {
            return self.parse_closure(start);
        }

        // Range prefix: ..expr, ..=expr
        if self.at_exact(&TokenKind::DotDot) {
            self.bump();
            let rhs = if self.at_expr_start() {
                Some(Box::new(self.parse_expr_bp(5)))
            } else {
                None
            };
            return Expr::Range(None, rhs, false, self.span_from(start));
        }
        if self.at_exact(&TokenKind::DotDotEq) {
            self.bump();
            let rhs = Some(Box::new(self.parse_expr_bp(5)));
            return Expr::Range(None, rhs, true, self.span_from(start));
        }

        self.parse_primary_expr()
    }

    fn parse_primary_expr(&mut self) -> Expr {
        let start = self.current().span;

        // Literals
        match &self.current().kind {
            TokenKind::IntLit(_, _) => {
                if let TokenKind::IntLit(v, suffix) = self.bump().kind {
                    let lit = Expr::Lit(Literal::Int(v), self.span_from(start));
                    if let Some(suffix) = suffix {
                        return Expr::Cast(Box::new(lit), self.ty_from_int_suffix(&suffix, self.span_from(start)), self.span_from(start));
                    }
                    return lit;
                }
            }
            TokenKind::FloatLit(_) => {
                if let TokenKind::FloatLit(v) = self.bump().kind {
                    return Expr::Lit(Literal::Float(v), self.span_from(start));
                }
            }
            TokenKind::StringLit(_) => {
                if let TokenKind::StringLit(s) = self.bump().kind {
                    return Expr::Lit(Literal::String(s), self.span_from(start));
                }
            }
            TokenKind::CharLit(_) => {
                if let TokenKind::CharLit(c) = self.bump().kind {
                    return Expr::Lit(Literal::Char(c), self.span_from(start));
                }
            }
            TokenKind::ByteStringLit(_) => {
                if let TokenKind::ByteStringLit(v) = self.bump().kind {
                    return Expr::Lit(Literal::ByteString(v), self.span_from(start));
                }
            }
            TokenKind::Kw(Keyword::True) => {
                self.bump();
                return Expr::Lit(Literal::Bool(true), self.span_from(start));
            }
            TokenKind::Kw(Keyword::False) => {
                self.bump();
                return Expr::Lit(Literal::Bool(false), self.span_from(start));
            }
            _ => {}
        }

        // if
        if self.at_kw(Keyword::If) {
            return self.parse_if_expr(start);
        }

        // match
        if self.at_kw(Keyword::Match) {
            return self.parse_match_expr(start);
        }

        // loop
        if self.at_kw(Keyword::Loop) {
            self.bump();
            let block = self.parse_block();
            return Expr::Loop(block, None, self.span_from(start));
        }

        // while / while let
        if self.at_kw(Keyword::While) {
            self.bump();
            if self.at_kw(Keyword::Let) {
                self.bump(); // let
                let pat = self.parse_pattern();
                self.expect_exact(&TokenKind::Eq);
                let scrutinee = self.parse_expr_no_struct();
                let block = self.parse_block();
                return Expr::WhileLet(pat, Box::new(scrutinee), block, None, self.span_from(start));
            }
            let cond = self.parse_expr_no_struct();
            let block = self.parse_block();
            return Expr::While(Box::new(cond), block, None, self.span_from(start));
        }

        // for
        if self.at_kw(Keyword::For) {
            self.bump();
            let pat = self.parse_pattern();
            self.expect_exact(&TokenKind::Kw(Keyword::In));
            let iter = self.parse_expr_no_struct();
            let block = self.parse_block();
            return Expr::For(pat, Box::new(iter), block, None, self.span_from(start));
        }

        // unsafe block
        if self.at_kw(Keyword::Unsafe) {
            self.bump();
            let block = self.parse_block();
            return Expr::Unsafe(block, self.span_from(start));
        }

        // Block expression
        if self.at_exact(&TokenKind::LBrace) {
            return Expr::Block(self.with_struct_literals_allowed(|this| this.parse_block()));
        }

        // Paren / Tuple
        if self.at_exact(&TokenKind::LParen) {
            self.bump();
            if self.at_exact(&TokenKind::RParen) {
                self.bump();
                return Expr::Tuple(vec![], self.span_from(start));
            }
            let first = self.with_struct_literals_allowed(|this| this.parse_expr());
            if self.at_exact(&TokenKind::Comma) {
                // tuple
                self.bump();
                let mut elems = vec![first];
                while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                    elems.push(self.with_struct_literals_allowed(|this| this.parse_expr()));
                    if !self.eat_exact(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect_exact(&TokenKind::RParen);
                return Expr::Tuple(elems, self.span_from(start));
            }
            self.expect_exact(&TokenKind::RParen);
            return Expr::Paren(Box::new(first), self.span_from(start));
        }

        // Array
        if self.at_exact(&TokenKind::LBracket) {
            self.bump();
            if self.at_exact(&TokenKind::RBracket) {
                self.bump();
                return Expr::Array(vec![], self.span_from(start));
            }
            let first = self.with_struct_literals_allowed(|this| this.parse_expr());
            // [val; count]
            if self.at_exact(&TokenKind::Semi) {
                self.bump();
                let count = self.with_struct_literals_allowed(|this| this.parse_expr());
                self.expect_exact(&TokenKind::RBracket);
                return Expr::ArrayRepeat(Box::new(first), Box::new(count), self.span_from(start));
            }
            // [a, b, c]
            let mut elems = vec![first];
            while self.eat_exact(&TokenKind::Comma) {
                if self.at_exact(&TokenKind::RBracket) {
                    break;
                }
                elems.push(self.with_struct_literals_allowed(|this| this.parse_expr()));
            }
            self.expect_exact(&TokenKind::RBracket);
            return Expr::Array(elems, self.span_from(start));
        }

        // Qualified path: <T as Trait>::Assoc or <[T]>::method
        if self.at_exact(&TokenKind::Lt) {
            let qpath = self.parse_qualified_path(start);
            if qpath.trait_path.is_none()
                && qpath.path.segments.len() == 1
                && self.at_exact(&TokenKind::Not)
            {
                self.bump();
                let tts = self.parse_macro_args();
                return Expr::MacroCall(qpath.path, tts, self.span_from(start));
            }
            return Expr::QualifiedPath(qpath);
        }

        // Path or struct literal or macro call
        if self.at_ident()
            || self.at_kw(Keyword::SelfValue)
            || self.at_kw(Keyword::SelfType)
            || self.at_kw(Keyword::Super)
            || self.at_kw(Keyword::Crate)
            || self.at_exact(&TokenKind::Dollar)
            || self.at_exact(&TokenKind::ColonColon)
        {
            let path = self.parse_path_expr();

            // Macro call: path!(...)
            if self.at_exact(&TokenKind::Not) {
                // Check for asm! built-in
                if path.segments.len() == 1 {
                    let name = self.interner.resolve(path.segments[0].ident);
                    if name == "asm" {
                        self.bump(); // !
                        return self.parse_inline_asm(start);
                    }
                }
                self.bump();
                let tts = self.parse_macro_args();
                return Expr::MacroCall(path, tts, self.span_from(start));
            }

            // Struct literal: Path { field: val }
            if self.at_exact(&TokenKind::LBrace) && !self.no_struct_literal {
                return self.parse_struct_literal(path, start);
            }

            return Expr::Path(path);
        }

        panic!(
            "unexpected token in expression: {:?} at {:?}; near {}",
            self.current().kind,
            self.current().span,
            self.token_window()
        );
    }

    fn parse_if_expr(&mut self, start: Span) -> Expr {
        self.bump(); // if
        // if let <pattern> = <expr> <block> [else <block>]
        if self.at_kw(Keyword::Let) {
            self.bump(); // let
            let pat = self.parse_pattern();
            self.expect_exact(&TokenKind::Eq);
            let scrutinee = self.parse_expr_no_struct_bp(9);
            if self.eat_exact(&TokenKind::AndAnd) {
                let guard = self.parse_expr_no_struct();
                let then_block = self.parse_block();
                let else_branch = if self.at_kw(Keyword::Else) {
                    self.bump();
                    if self.at_kw(Keyword::If) {
                        let s = self.current().span;
                        Some(Box::new(self.parse_if_expr(s)))
                    } else {
                        Some(Box::new(Expr::Block(self.parse_block())))
                    }
                } else {
                    None
                };
                let span = self.span_from(start);
                let then_expr = Expr::Block(then_block);
                let else_expr = else_branch
                    .map(|expr| *expr)
                    .unwrap_or_else(|| Expr::Tuple(vec![], span));
                return Expr::Match(
                    Box::new(scrutinee),
                    vec![
                        MatchArm {
                            attrs: Vec::new(),
                            pat,
                            guard: Some(Box::new(guard)),
                            body: Box::new(then_expr),
                            span,
                        },
                        MatchArm {
                            attrs: Vec::new(),
                            pat: Pattern::Wildcard(span),
                            guard: None,
                            body: Box::new(else_expr),
                            span,
                        },
                    ],
                    span,
                );
            }
            let then_block = self.parse_block();
            let else_branch = if self.at_kw(Keyword::Else) {
                self.bump();
                if self.at_kw(Keyword::If) {
                    let s = self.current().span;
                    Some(Box::new(self.parse_if_expr(s)))
                } else {
                    Some(Box::new(Expr::Block(self.parse_block())))
                }
            } else {
                None
            };
            return Expr::IfLet(pat, Box::new(scrutinee), then_block, else_branch, self.span_from(start));
        }
        let cond = self.parse_expr_no_struct();
        let then_block = self.parse_block();
        let else_branch = if self.at_kw(Keyword::Else) {
            self.bump();
            if self.at_kw(Keyword::If) {
                let s = self.current().span;
                Some(Box::new(self.parse_if_expr(s)))
            } else {
                Some(Box::new(Expr::Block(self.parse_block())))
            }
        } else {
            None
        };
        Expr::If(Box::new(cond), then_block, else_branch, self.span_from(start))
    }

    fn parse_match_expr(&mut self, start: Span) -> Expr {
        self.bump(); // match
        let scrutinee = self.parse_expr_no_struct();
        self.expect_exact(&TokenKind::LBrace);
        let mut arms = Vec::new();
        while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
            let attrs = self.parse_attrs();
            let arm_start = self.current().span;
            let pat = self.parse_pattern();
            let guard = if self.at_kw(Keyword::If) {
                self.bump();
                Some(Box::new(self.parse_expr_no_struct()))
            } else {
                None
            };
            self.expect_exact(&TokenKind::FatArrow);
            let body = Box::new(self.parse_expr());
            let span = self.span_from(arm_start);
            arms.push(MatchArm { attrs, pat, guard, body, span });
            if self.eat_exact(&TokenKind::Comma) {
                continue;
            }
        }
        self.expect_exact(&TokenKind::RBrace);
        Expr::Match(Box::new(scrutinee), arms, self.span_from(start))
    }

    fn parse_closure(&mut self, start: Span) -> Expr {
        let is_move = self.eat_exact(&TokenKind::Kw(Keyword::Move));
        let params = if self.at_exact(&TokenKind::OrOr) {
            self.bump();
            vec![]
        } else {
            self.expect_exact(&TokenKind::Pipe);
            let mut params = Vec::new();
            while !self.at_exact(&TokenKind::Pipe) && !self.at_exact(&TokenKind::Eof) {
                let p_start = self.current().span;
                let attrs = self.parse_attrs();
                let pat = self.parse_pattern_no_or();
                let ty = if self.eat_exact(&TokenKind::Colon) {
                    self.parse_ty()
                } else {
                    Ty::Infer(self.prev_span)
                };
                params.push(Param {
                    pat,
                    ty,
                    attrs,
                    span: self.span_from(p_start),
                });
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_exact(&TokenKind::Pipe);
            params
        };
        let ret_ty = if self.at_exact(&TokenKind::Arrow) {
            self.bump();
            Some(Box::new(self.parse_ty()))
        } else {
            None
        };
        let body = Box::new(self.with_struct_literals_allowed(|this| this.parse_expr()));
        Expr::Closure(params, ret_ty, body, is_move, self.span_from(start))
    }

    fn parse_struct_literal(&mut self, path: Path, start: Span) -> Expr {
        self.expect_exact(&TokenKind::LBrace);
        let mut fields = Vec::new();
        let mut base = None;
        while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
            let attrs = self.parse_attrs();
            if self.at_exact(&TokenKind::DotDot) {
                self.bump();
                base = Some(Box::new(self.parse_expr()));
                break;
            }
            let f_start = self.current().span;
            let name = self.expect_ident();
            let value = if self.eat_exact(&TokenKind::Colon) {
                self.parse_expr()
            } else {
                Expr::Path(Path {
                    segments: vec![PathSegment { ident: name, args: None }],
                    span: self.span_from(f_start),
                })
            };
            fields.push(FieldExpr {
                name,
                value,
                attrs,
                span: self.span_from(f_start),
            });
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        self.expect_exact(&TokenKind::RBrace);
        Expr::Struct(path, fields, base, self.span_from(start))
    }

    fn parse_expr_no_struct(&mut self) -> Expr {
        self.parse_expr_no_struct_bp(0)
    }

    fn parse_expr_no_struct_bp(&mut self, min_bp: u8) -> Expr {
        let old = self.no_struct_literal;
        self.no_struct_literal = true;
        let expr = self.parse_expr_bp(min_bp);
        self.no_struct_literal = old;
        expr
    }

    fn with_struct_literals_allowed<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let old = self.no_struct_literal;
        self.no_struct_literal = false;
        let result = f(self);
        self.no_struct_literal = old;
        result
    }

    fn parse_macro_args(&mut self) -> Vec<TokenTree> {
        if self.at_exact(&TokenKind::LParen) {
            self.bump();
            let tts = self.collect_token_trees(&TokenKind::RParen);
            self.expect_exact(&TokenKind::RParen);
            tts
        } else if self.at_exact(&TokenKind::LBracket) {
            self.bump();
            let tts = self.collect_token_trees(&TokenKind::RBracket);
            self.expect_exact(&TokenKind::RBracket);
            tts
        } else if self.at_exact(&TokenKind::LBrace) {
            self.bump();
            let tts = self.collect_token_trees(&TokenKind::RBrace);
            self.expect_exact(&TokenKind::RBrace);
            tts
        } else {
            vec![]
        }
    }

    fn parse_delimited_token_trees(&mut self) -> Vec<TokenTree> {
        if self.at_exact(&TokenKind::LParen) {
            self.bump();
            let tts = self.collect_token_trees(&TokenKind::RParen);
            self.expect_exact(&TokenKind::RParen);
            tts
        } else if self.at_exact(&TokenKind::LBracket) {
            self.bump();
            let tts = self.collect_token_trees(&TokenKind::RBracket);
            self.expect_exact(&TokenKind::RBracket);
            tts
        } else if self.at_exact(&TokenKind::LBrace) {
            self.bump();
            let tts = self.collect_token_trees(&TokenKind::RBrace);
            self.expect_exact(&TokenKind::RBrace);
            tts
        } else {
            panic!(
                "expected delimited token tree, got {:?} at {:?}",
                self.current().kind,
                self.current().span
            );
        }
    }

    fn collect_token_trees(&mut self, close: &TokenKind) -> Vec<TokenTree> {
        let mut tts = Vec::new();
        while !self.at_exact(close) && !self.at_exact(&TokenKind::Eof) {
            if let TokenKind::DocComment(text, inner) = self.current().kind.clone() {
                let span = self.bump().span;
                tts.extend(self.doc_comment_token_trees(text, inner, span));
            } else if self.at_exact(&TokenKind::LParen) {
                self.bump();
                let inner = self.collect_token_trees(&TokenKind::RParen);
                self.expect_exact(&TokenKind::RParen);
                tts.push(TokenTree::Delimited(Delimiter::Paren, inner));
            } else if self.at_exact(&TokenKind::LBracket) {
                self.bump();
                let inner = self.collect_token_trees(&TokenKind::RBracket);
                self.expect_exact(&TokenKind::RBracket);
                tts.push(TokenTree::Delimited(Delimiter::Bracket, inner));
            } else if self.at_exact(&TokenKind::LBrace) {
                self.bump();
                let inner = self.collect_token_trees(&TokenKind::RBrace);
                self.expect_exact(&TokenKind::RBrace);
                tts.push(TokenTree::Delimited(Delimiter::Brace, inner));
            } else {
                tts.push(TokenTree::Token(self.bump()));
            }
        }
        tts
    }

    fn doc_comment_token_trees(&mut self, text: String, inner: bool, span: Span) -> Vec<TokenTree> {
        let doc = self.interner.intern("doc");
        let mut out = Vec::new();
        out.push(TokenTree::Token(Token { kind: TokenKind::Hash, span }));
        if inner {
            out.push(TokenTree::Token(Token { kind: TokenKind::Not, span }));
        }
        out.push(TokenTree::Delimited(
            Delimiter::Bracket,
            vec![
                TokenTree::Token(Token { kind: TokenKind::Ident(doc), span }),
                TokenTree::Token(Token { kind: TokenKind::Eq, span }),
                TokenTree::Token(Token { kind: TokenKind::StringLit(text), span }),
            ],
        ));
        out
    }

    fn doc_comment_attr(&mut self, text: String, span: Span) -> Attribute {
        let doc = self.interner.intern("doc");
        Attribute {
            path: Path {
                segments: vec![PathSegment {
                    ident: doc,
                    args: None,
                }],
                span,
            },
            args: AttrArgs::Eq(Box::new(Expr::Lit(Literal::String(text), span))),
            span,
        }
    }

    fn parse_path_expr(&mut self) -> Path {
        let start = self.current().span;
        let mut segments = Vec::new();

        // Leading ::
        if self.at_exact(&TokenKind::ColonColon) {
            self.bump();
        }

        let ident = self.expect_ident_or_self();
        segments.push(PathSegment { ident, args: None });

        while self.at_exact(&TokenKind::ColonColon) {
            self.bump();
            // turbofish ::<T>
            if self.at_exact(&TokenKind::Lt) {
                let args = self.parse_generic_args();
                if let Some(last) = segments.last_mut() {
                    last.args = Some(args);
                }
                // might continue with more ::segments
                if !self.at_exact(&TokenKind::ColonColon) {
                    break;
                }
                self.bump();
            }
            let ident = self.expect_ident_or_self();
            segments.push(PathSegment { ident, args: None });
        }

        Path {
            segments,
            span: self.span_from(start),
        }
    }

    fn parse_qualified_path(&mut self, start: Span) -> QualifiedPath {
        self.expect_type_arg_lt();
        let self_ty = self.parse_ty();
        let trait_path = if self.at_kw(Keyword::As) {
            self.bump();
            Some(self.parse_path_ty())
        } else {
            None
        };
        self.expect_exact(&TokenKind::Gt);
        self.expect_exact(&TokenKind::ColonColon);
        let mut path = self.parse_path_expr();
        path.span = self.span_from(start);
        QualifiedPath {
            self_ty: Box::new(self_ty),
            trait_path,
            path,
            span: self.span_from(start),
        }
    }

    fn postfix_bp(&self) -> Option<u8> {
        match &self.current().kind {
            TokenKind::Dot => Some(29),
            TokenKind::LParen => Some(29),
            TokenKind::LBracket => Some(29),
            TokenKind::Question => Some(29),
            _ => None,
        }
    }

    fn parse_postfix(&mut self, lhs: Expr, start: Span) -> Expr {
        match &self.current().kind {
            TokenKind::Dot => {
                self.bump();
                // Tuple field access: expr.0
                if let TokenKind::IntLit(n, _) = self.current().kind {
                    let idx = n as u32;
                    self.bump();
                    let name = self.interner.intern(&idx.to_string());
                    return Expr::Field(Box::new(lhs), name, self.span_from(start));
                }
                let name = self.expect_ident();
                // Method call with turbofish: expr.method::<T>(args)
                if self.at_exact(&TokenKind::ColonColon) && self.peek_kind_at(1) == Some(&TokenKind::Lt) {
                    self.bump(); // ::
                    let generic_args = self.parse_generic_args();
                    let ty_args: Vec<Ty> = generic_args
                        .args
                        .into_iter()
                        .filter_map(|a| match a {
                            GenericArg::Type(t) => Some(t),
                            _ => None,
                        })
                        .collect();
                    self.expect_exact(&TokenKind::LParen);
                    let args = self.parse_call_args();
                    self.expect_exact(&TokenKind::RParen);
                    return Expr::MethodCall(
                        Box::new(lhs),
                        name,
                        ty_args,
                        args,
                        self.span_from(start),
                    );
                }
                // Method call: expr.method(args)
                if self.at_exact(&TokenKind::LParen) {
                    self.bump();
                    let args = self.parse_call_args();
                    self.expect_exact(&TokenKind::RParen);
                    return Expr::MethodCall(
                        Box::new(lhs),
                        name,
                        vec![],
                        args,
                        self.span_from(start),
                    );
                }
                // Field access
                Expr::Field(Box::new(lhs), name, self.span_from(start))
            }
            TokenKind::LParen => {
                self.bump();
                let args = self.parse_call_args();
                self.expect_exact(&TokenKind::RParen);
                Expr::Call(Box::new(lhs), args, self.span_from(start))
            }
            TokenKind::LBracket => {
                self.bump();
                let index = self.with_struct_literals_allowed(|this| this.parse_expr());
                self.expect_exact(&TokenKind::RBracket);
                Expr::Index(Box::new(lhs), Box::new(index), self.span_from(start))
            }
            TokenKind::Question => {
                self.bump();
                // Desugar ? to a method-call-like thing; for now just wrap.
                // We'll represent it as a MacroCall to a special path, or just
                // reuse the ast. Actually, there's no Expr::Try variant, so let's
                // use a macro call with an empty path for now. We can add Try later.
                // For simplicity, just produce a Call to "try" - but really we should
                // add an Expr variant. Let's just panic for now since tests don't need it.
                // Actually let's just produce a method call with a special name.
                let try_sym = self.interner.intern("?");
                Expr::MethodCall(Box::new(lhs), try_sym, vec![], vec![], self.span_from(start))
            }
            _ => unreachable!(),
        }
    }

    fn parse_call_args(&mut self) -> Vec<Expr> {
        let mut args = Vec::new();
        while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
            args.push(self.with_struct_literals_allowed(|this| this.parse_expr()));
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        args
    }

    fn infix_bp(&self) -> Option<(u8, u8, InfixOp)> {
        match &self.current().kind {
            // Assignment (right-assoc)
            TokenKind::Eq => Some((2, 1, InfixOp::Assign)),
            TokenKind::PlusEq => Some((2, 1, InfixOp::AssignOp(BinOp::Add))),
            TokenKind::MinusEq => Some((2, 1, InfixOp::AssignOp(BinOp::Sub))),
            TokenKind::StarEq => Some((2, 1, InfixOp::AssignOp(BinOp::Mul))),
            TokenKind::SlashEq => Some((2, 1, InfixOp::AssignOp(BinOp::Div))),
            TokenKind::PercentEq => Some((2, 1, InfixOp::AssignOp(BinOp::Rem))),
            TokenKind::AmpEq => Some((2, 1, InfixOp::AssignOp(BinOp::BitAnd))),
            TokenKind::PipeEq => Some((2, 1, InfixOp::AssignOp(BinOp::BitOr))),
            TokenKind::CaretEq => Some((2, 1, InfixOp::AssignOp(BinOp::BitXor))),
            TokenKind::ShlEq => Some((2, 1, InfixOp::AssignOp(BinOp::Shl))),
            TokenKind::ShrEq => Some((2, 1, InfixOp::AssignOp(BinOp::Shr))),
            // Range
            TokenKind::DotDot => Some((4, 3, InfixOp::Range(false))),
            TokenKind::DotDotEq => Some((4, 3, InfixOp::Range(true))),
            // Logical
            TokenKind::OrOr => Some((6, 7, InfixOp::Binary(BinOp::Or))),
            TokenKind::AndAnd => Some((8, 9, InfixOp::Binary(BinOp::And))),
            // Comparison
            TokenKind::EqEq => Some((10, 11, InfixOp::Binary(BinOp::Eq))),
            TokenKind::Ne => Some((10, 11, InfixOp::Binary(BinOp::Ne))),
            TokenKind::Lt => Some((10, 11, InfixOp::Binary(BinOp::Lt))),
            TokenKind::Le => Some((10, 11, InfixOp::Binary(BinOp::Le))),
            TokenKind::Gt => Some((10, 11, InfixOp::Binary(BinOp::Gt))),
            TokenKind::Ge => Some((10, 11, InfixOp::Binary(BinOp::Ge))),
            // Bitwise
            TokenKind::Pipe => Some((12, 13, InfixOp::Binary(BinOp::BitOr))),
            TokenKind::Caret => Some((14, 15, InfixOp::Binary(BinOp::BitXor))),
            TokenKind::Amp => Some((16, 17, InfixOp::Binary(BinOp::BitAnd))),
            // Shift
            TokenKind::Shl => Some((18, 19, InfixOp::Binary(BinOp::Shl))),
            TokenKind::Shr => Some((18, 19, InfixOp::Binary(BinOp::Shr))),
            // Additive
            TokenKind::Plus => Some((20, 21, InfixOp::Binary(BinOp::Add))),
            TokenKind::Minus => Some((20, 21, InfixOp::Binary(BinOp::Sub))),
            // Multiplicative
            TokenKind::Star => Some((22, 23, InfixOp::Binary(BinOp::Mul))),
            TokenKind::Slash => Some((22, 23, InfixOp::Binary(BinOp::Div))),
            TokenKind::Percent => Some((22, 23, InfixOp::Binary(BinOp::Rem))),
            _ => None,
        }
    }

    fn at_expr_start(&self) -> bool {
        matches!(
            &self.current().kind,
            TokenKind::IntLit(_, _)
                | TokenKind::FloatLit(_)
                | TokenKind::StringLit(_)
                | TokenKind::CharLit(_)
                | TokenKind::ByteStringLit(_)
                | TokenKind::Ident(_)
                | TokenKind::Kw(Keyword::True)
                | TokenKind::Kw(Keyword::False)
                | TokenKind::Kw(Keyword::If)
                | TokenKind::Kw(Keyword::Match)
                | TokenKind::Kw(Keyword::Loop)
                | TokenKind::Kw(Keyword::While)
                | TokenKind::Kw(Keyword::For)
                | TokenKind::Kw(Keyword::Unsafe)
                | TokenKind::Kw(Keyword::Return)
                | TokenKind::Kw(Keyword::Break)
                | TokenKind::Kw(Keyword::Continue)
                | TokenKind::Kw(Keyword::Move)
                | TokenKind::Kw(Keyword::SelfValue)
                | TokenKind::Kw(Keyword::SelfType)
                | TokenKind::Kw(Keyword::Super)
                | TokenKind::Kw(Keyword::Crate)
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::Pipe
                | TokenKind::OrOr
                | TokenKind::Amp
                | TokenKind::AndAnd
                | TokenKind::Star
                | TokenKind::Minus
                | TokenKind::Not
                | TokenKind::DotDot
                | TokenKind::DotDotEq
                | TokenKind::ColonColon
        )
    }

    fn peek_kind(&self) -> &TokenKind {
        let next = (self.pos + 1).min(self.tokens.len() - 1);
        &self.tokens[next].kind
    }

    fn peek_kind_at(&self, offset: usize) -> Option<&TokenKind> {
        let idx = self.pos + offset;
        if idx < self.tokens.len() {
            Some(&self.tokens[idx].kind)
        } else {
            None
        }
    }

    // ── Statements & Blocks ──

    pub fn parse_block(&mut self) -> Block {
        let start = self.current().span;
        self.expect_exact(&TokenKind::LBrace);
        let mut stmts = Vec::new();
        while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
            stmts.push(self.parse_stmt());
        }
        self.expect_exact(&TokenKind::RBrace);
        Block {
            stmts,
            span: self.span_from(start),
        }
    }

    fn parse_stmt(&mut self) -> Stmt {
        let start = self.current().span;

        if self.at_exact(&TokenKind::Hash) || matches!(self.current().kind, TokenKind::DocComment(_, false)) {
            let attrs = self.parse_attrs();
            let vis = self.parse_visibility();
            if self.at_item_start_after_leading() {
                return Stmt::Item(
                    self.parse_item_after_leading(attrs, vis, start)
                        .expect("item after attributes"),
                );
            }
            let stmt = self.parse_stmt_no_attrs(start);
            return Stmt::Attributed(attrs, Box::new(stmt), self.span_from(start));
        }

        self.parse_stmt_no_attrs(start)
    }

    fn parse_stmt_no_attrs(&mut self, start: Span) -> Stmt {
        // let statement
        if self.at_kw(Keyword::Let) {
            self.bump();
            let pat = self.parse_pattern();
            let ty = if self.eat_exact(&TokenKind::Colon) {
                Some(self.parse_ty())
            } else {
                None
            };
            let init = if self.eat_exact(&TokenKind::Eq) {
                Some(Box::new(self.parse_expr()))
            } else {
                None
            };
            if self.at_kw(Keyword::Else) {
                self.bump();
                let else_block = self.parse_block();
                self.expect_exact(&TokenKind::Semi);
                let init_expr = init
                    .map(|expr| *expr)
                    .unwrap_or_else(|| panic!("let-else requires an initializer"));
                return self.desugar_let_else(pat, ty, init_expr, else_block, self.span_from(start));
            }
            self.expect_exact(&TokenKind::Semi);
            return Stmt::Let(pat, ty, init, self.span_from(start));
        }

        // Item in block
        if self.at_item_start() {
            if let Some(item) = self.parse_item() {
                return Stmt::Item(item);
            }
        }

        // Expression statement
        let old_stop_stmt_after_block_expr = self.stop_stmt_after_block_expr;
        self.stop_stmt_after_block_expr = true;
        let expr = self.parse_expr();
        self.stop_stmt_after_block_expr = old_stop_stmt_after_block_expr;
        if self.eat_exact(&TokenKind::Semi) {
            Stmt::Semi(expr, self.span_from(start))
        } else if self.expr_can_end_stmt_without_semicolon(&expr) && !self.at_exact(&TokenKind::RBrace) {
            // Block-like expressions such as `while { ... }` and `if { ... }`
            // may omit the semicolon in statement position. Without this split,
            // the Pratt parser can accidentally continue into the following line
            // and parse e.g. `while { ... } (*ptr).field = value;` as one giant
            // assignment/call expression.
            Stmt::Semi(expr, self.span_from(start))
        } else {
            // Trailing expression (no semi) - only valid as last stmt in block
            Stmt::Expr(expr)
        }
    }

    fn expr_can_end_stmt_without_semicolon(&self, expr: &Expr) -> bool {
        matches!(
            expr,
            Expr::Block(_)
                | Expr::If(_, _, _, _)
                | Expr::IfLet(_, _, _, _, _)
                | Expr::Match(_, _, _)
                | Expr::Loop(_, _, _)
                | Expr::While(_, _, _, _)
                | Expr::WhileLet(_, _, _, _, _)
                | Expr::For(_, _, _, _, _)
                | Expr::Unsafe(_, _)
        )
    }

    fn desugar_let_else(
        &mut self,
        original_pat: Pattern,
        ty: Option<Ty>,
        init_expr: Expr,
        else_block: Block,
        span: Span,
    ) -> Stmt {
        let mut bindings = Vec::new();
        collect_pattern_bindings(&original_pat, &mut bindings);
        let success_pattern = Pattern::Tuple(
            bindings
                .iter()
                .map(|(name, mutability, binding_span)| {
                    Pattern::Ident(*name, *mutability, None, *binding_span)
                })
                .collect(),
            span,
        );
        let success_expr = Expr::Tuple(
            bindings
                .iter()
                .map(|(name, _, binding_span)| Expr::Path(Path {
                    segments: vec![PathSegment {
                        ident: *name,
                        args: None,
                    }],
                    span: *binding_span,
                }))
                .collect(),
            span,
        );
        let match_expr = Expr::Match(
            Box::new(init_expr),
            vec![
                MatchArm {
                    attrs: Vec::new(),
                    pat: original_pat,
                    guard: None,
                    body: Box::new(success_expr),
                    span,
                },
                MatchArm {
                    attrs: Vec::new(),
                    pat: Pattern::Wildcard(span),
                    guard: None,
                    body: Box::new(Expr::Block(else_block)),
                    span,
                },
            ],
            span,
        );
        Stmt::Let(success_pattern, ty, Some(Box::new(match_expr)), span)
    }

    fn at_item_start(&self) -> bool {
        if matches!(
            &self.current().kind,
            TokenKind::Kw(Keyword::Fn)
                | TokenKind::Kw(Keyword::Struct)
                | TokenKind::Kw(Keyword::Enum)
                | TokenKind::Kw(Keyword::Impl)
                | TokenKind::Kw(Keyword::Trait)
                | TokenKind::Kw(Keyword::Use)
                | TokenKind::Kw(Keyword::Mod)
                | TokenKind::Kw(Keyword::Const)
                | TokenKind::Kw(Keyword::Static)
                | TokenKind::Kw(Keyword::Extern)
                | TokenKind::Kw(Keyword::Type)
                | TokenKind::Kw(Keyword::Pub)
                | TokenKind::Kw(Keyword::Unsafe)
                | TokenKind::Hash
        ) {
            return true;
        }
        match &self.current().kind {
            TokenKind::Ident(sym) => matches!(self.interner.resolve(*sym), "macro_rules" | "union"),
            _ => false,
        }
    }

    fn at_item_start_after_leading(&self) -> bool {
        match &self.current().kind {
            TokenKind::Kw(Keyword::Fn)
            | TokenKind::Kw(Keyword::Struct)
            | TokenKind::Kw(Keyword::Enum)
            | TokenKind::Kw(Keyword::Impl)
            | TokenKind::Kw(Keyword::Trait)
            | TokenKind::Kw(Keyword::Use)
            | TokenKind::Kw(Keyword::Mod)
            | TokenKind::Kw(Keyword::Const)
            | TokenKind::Kw(Keyword::Static)
            | TokenKind::Kw(Keyword::Extern)
            | TokenKind::Kw(Keyword::Type) => true,
            TokenKind::Kw(Keyword::Unsafe) => matches!(
                self.peek_kind(),
                TokenKind::Kw(Keyword::Fn)
                    | TokenKind::Kw(Keyword::Trait)
                    | TokenKind::Kw(Keyword::Impl)
                    | TokenKind::Kw(Keyword::Extern)
            ),
            TokenKind::Ident(sym) => matches!(self.interner.resolve(*sym), "macro_rules" | "union"),
            _ => false,
        }
    }

    // ── Items ──

    pub fn parse_crate(&mut self) -> Crate {
        // Parse inner attributes (#![...])
        let mut attrs = Vec::new();
        while (self.at_exact(&TokenKind::Hash) && self.peek_kind_at(1) == Some(&TokenKind::Not))
            || matches!(self.current().kind, TokenKind::DocComment(_, true))
        {
            if let TokenKind::DocComment(text, true) = self.current().kind.clone() {
                let span = self.bump().span;
                attrs.push(self.doc_comment_attr(text, span));
                continue;
            }
            let start = self.current().span;
            self.bump(); // #
            self.bump(); // !
            self.expect_exact(&TokenKind::LBracket);
            let path = self.parse_path_ty();
            let args = if self.at_exact(&TokenKind::LParen) {
                self.bump();
                let tts = self.collect_token_trees(&TokenKind::RParen);
                self.expect_exact(&TokenKind::RParen);
                AttrArgs::Delimited(tts)
            } else if self.eat_exact(&TokenKind::Eq) {
                AttrArgs::Eq(Box::new(self.parse_expr()))
            } else {
                AttrArgs::Empty
            };
            self.expect_exact(&TokenKind::RBracket);
            attrs.push(Attribute { path, args, span: self.span_from(start) });
        }

        let mut items = Vec::new();
        while !self.at_exact(&TokenKind::Eof) {
            if let Some(item) = self.parse_item() {
                items.push(item);
            } else {
                break;
            }
        }
        Crate { attrs, items }
    }

    fn parse_item(&mut self) -> Option<Item> {
        let start = self.current().span;
        let attrs = self.parse_attrs();
        let vis = self.parse_visibility();
        self.parse_item_after_leading(attrs, vis, start)
    }

    fn parse_item_after_leading(
        &mut self,
        attrs: Vec<Attribute>,
        vis: Visibility,
        start: Span,
    ) -> Option<Item> {
        match &self.current().kind {
            TokenKind::Kw(Keyword::Fn) => {
                Some(Item::Fn(self.parse_fn_def(vis, attrs, false, false, None, start)))
            }
            TokenKind::Kw(Keyword::Struct) => {
                Some(Item::Struct(self.parse_struct_def(vis, attrs, start, false)))
            }
            TokenKind::Kw(Keyword::Enum) => {
                Some(Item::Enum(self.parse_enum_def(vis, attrs, start)))
            }
            TokenKind::Kw(Keyword::Impl) => Some(Item::Impl(self.parse_impl_block(attrs, start))),
            TokenKind::Kw(Keyword::Trait) => {
                Some(Item::Trait(self.parse_trait_def(vis, attrs, false, start)))
            }
            TokenKind::Kw(Keyword::Use) => Some(Item::Use(self.parse_use_tree_item(vis, attrs, start))),
            TokenKind::Kw(Keyword::Mod) => Some(Item::Mod(self.parse_mod_def(vis, attrs, start))),
            TokenKind::Kw(Keyword::Const) => {
                // `const fn` → parse as function with is_const=true
                if self.peek_kind() == &TokenKind::Kw(Keyword::Fn) {
                    self.bump(); // eat `const`
                    Some(Item::Fn(self.parse_fn_def(vis, attrs, false, true, None, start)))
                } else if self.peek_kind() == &TokenKind::Kw(Keyword::Unsafe)
                    && self.peek_kind_at(2) == Some(&TokenKind::Kw(Keyword::Fn))
                {
                    self.bump(); // eat `const`
                    self.bump(); // eat `unsafe`
                    Some(Item::Fn(self.parse_fn_def(vis, attrs, true, true, None, start)))
                } else {
                    Some(Item::Const(self.parse_const_def(vis, attrs, start)))
                }
            }
            TokenKind::Kw(Keyword::Static) => {
                Some(Item::Static(self.parse_static_def(vis, attrs, start)))
            }
            TokenKind::Kw(Keyword::Extern) => {
                self.bump();
                if self.at_kw(Keyword::Crate) {
                    self.bump();
                    let name = self.expect_ident_or_self();
                    let alias = if self.at_kw(Keyword::As) {
                        self.bump();
                        Some(self.expect_ident())
                    } else {
                        None
                    };
                    self.expect_exact(&TokenKind::Semi);
                    return Some(Item::ExternCrate(ExternCrateDef {
                        name,
                        alias,
                        span: self.span_from(start),
                    }));
                }
                let abi = if let TokenKind::StringLit(_) = &self.current().kind {
                    if let TokenKind::StringLit(s) = self.bump().kind {
                        Some(s)
                    } else {
                        None
                    }
                } else {
                    None
                };
                if self.at_exact(&TokenKind::LBrace) {
                    Some(Item::ExternBlock(self.parse_extern_block(abi, attrs, start)))
                } else if self.at_kw(Keyword::Fn) {
                    Some(Item::Fn(self.parse_fn_def(vis, attrs, false, false, abi, start)))
                } else {
                    None
                }
            }
            TokenKind::Kw(Keyword::Type) => {
                Some(Item::TypeAlias(self.parse_type_alias(vis, attrs, start)))
            }
            TokenKind::Kw(Keyword::Unsafe) => {
                self.bump();
                if self.at_kw(Keyword::Fn) {
                    Some(Item::Fn(self.parse_fn_def(vis, attrs, true, false, None, start)))
                } else if self.at_kw(Keyword::Trait) {
                    Some(Item::Trait(self.parse_trait_def(vis, attrs, true, start)))
                } else if self.at_kw(Keyword::Impl) {
                    let mut ib = self.parse_impl_block(attrs, start);
                    ib.is_unsafe = true;
                    Some(Item::Impl(ib))
                } else if self.at_kw(Keyword::Extern) {
                    self.bump();
                    let abi = if let TokenKind::StringLit(_) = &self.current().kind {
                        if let TokenKind::StringLit(s) = self.bump().kind {
                            Some(s)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if self.at_exact(&TokenKind::LBrace) {
                        Some(Item::ExternBlock(self.parse_extern_block(abi, attrs, start)))
                    } else if self.at_kw(Keyword::Fn) {
                        Some(Item::Fn(self.parse_fn_def(vis, attrs, true, false, abi, start)))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            // Specialization associated items use `default fn`, `default type`,
            // etc. Keep parsing the associated item normally; type checking can
            // model defaultness once specialization semantics become relevant.
            TokenKind::Ident(sym)
                if self.interner.resolve(*sym) == "default"
                    && Self::can_start_default_item(self.peek_kind()) =>
            {
                self.bump();
                self.parse_item_after_leading(attrs, vis, start)
            }
            // macro_rules! name { ... }
            TokenKind::Ident(sym) if self.interner.resolve(*sym) == "macro_rules" => {
                Some(self.parse_macro_rules_def(attrs, start))
            }
            TokenKind::Ident(sym) if self.interner.resolve(*sym) == "union" => {
                Some(Item::Struct(self.parse_struct_def(vis, attrs, start, true)))
            }
            // Macro invocation at item position: name!(...)
            TokenKind::Ident(_) => {
                let saved = self.pos;
                let path = self.parse_path_expr();
                if self.at_exact(&TokenKind::Not) {
                    self.bump();
                    let tts = self.parse_macro_args();
                    self.eat_exact(&TokenKind::Semi);
                    Some(Item::MacroCall(path, tts, attrs, self.span_from(start)))
                } else {
                    self.pos = saved;
                    None
                }
            }
            _ => None,
        }
    }

    fn can_start_default_item(kind: &TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::Kw(Keyword::Fn)
                | TokenKind::Kw(Keyword::Type)
                | TokenKind::Kw(Keyword::Const)
                | TokenKind::Kw(Keyword::Unsafe)
        )
    }

    fn parse_macro_rules_def(&mut self, attrs: Vec<Attribute>, start: Span) -> Item {
        self.bump(); // eat `macro_rules`
        self.expect_exact(&TokenKind::Not);
        let name = self.expect_ident();
        // { rule ; rule ; ... }
        self.expect_exact(&TokenKind::LBrace);
        let mut rules = Vec::new();
        while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
            // Accept any standard delimiter around macro_rules patterns and bodies.
            let pattern = self.parse_delimited_token_trees();
            self.expect_exact(&TokenKind::FatArrow);
            let body = self.parse_delimited_token_trees();
            rules.push(MacroRule { pattern, body });
            // optional semicolon between rules
            self.eat_exact(&TokenKind::Semi);
        }
        self.expect_exact(&TokenKind::RBrace);
        Item::MacroDef(MacroRulesDef {
            name,
            rules,
            attrs,
            span: self.span_from(start),
        })
    }

    fn parse_fn_def(
        &mut self,
        vis: Visibility,
        attrs: Vec<Attribute>,
        is_unsafe: bool,
        is_const: bool,
        abi: Option<String>,
        start: Span,
    ) -> FnDef {
        self.expect_exact(&TokenKind::Kw(Keyword::Fn));
        let name = self.expect_ident();
        let generics = self.parse_generics();
        self.expect_exact(&TokenKind::LParen);
        let params = self.parse_fn_params();
        self.expect_exact(&TokenKind::RParen);
        let ret_ty = if self.eat_exact(&TokenKind::Arrow) {
            Some(Box::new(self.parse_ty()))
        } else {
            None
        };
        let where_clause = self.parse_where_clause();
        let body = if self.at_exact(&TokenKind::LBrace) {
            Some(self.parse_block())
        } else {
            self.expect_exact(&TokenKind::Semi);
            None
        };
        FnDef {
            name,
            generics,
            params,
            ret_ty,
            where_clause,
            body,
            attrs,
            vis,
            is_unsafe,
            is_const,
            abi,
            span: self.span_from(start),
        }
    }

    fn parse_fn_params(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
            let p_start = self.current().span;
            let attrs = self.parse_attrs();

            // C variadics in extern function declarations: `...`.
            if self.consume_c_variadic_marker() {
                break;
            }

            // &self, &mut self, self, mut self
            if self.at_exact(&TokenKind::Amp)
                && matches!(
                    (
                        self.peek_kind(),
                        self.peek_kind_at(2),
                        self.peek_kind_at(3)
                    ),
                    (TokenKind::Kw(Keyword::SelfValue), _, _)
                        | (TokenKind::Lifetime(_), Some(TokenKind::Kw(Keyword::SelfValue)), _)
                        | (TokenKind::Kw(Keyword::Mut), Some(TokenKind::Kw(Keyword::SelfValue)), _)
                        | (
                            TokenKind::Lifetime(_),
                            Some(TokenKind::Kw(Keyword::Mut)),
                            Some(TokenKind::Kw(Keyword::SelfValue))
                        )
                )
            {
                self.bump(); // &
                let lifetime = if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                    if let TokenKind::Lifetime(sym) = self.bump().kind {
                        Some(sym)
                    } else {
                        None
                    }
                } else {
                    None
                };
                let mutability = if self.eat_exact(&TokenKind::Kw(Keyword::Mut)) {
                    Mutability::Mut
                } else {
                    Mutability::Immutable
                };
                self.expect_exact(&TokenKind::Kw(Keyword::SelfValue));
                let self_sym = self.interner.intern("self");
                let pat = Pattern::Ident(self_sym, Mutability::Immutable, None, self.span_from(p_start));
                // Type is &Self or &mut Self
                let self_type_sym = self.interner.intern("Self");
                let self_ty = Ty::Reference(
                    lifetime,
                    Box::new(Ty::Path(Path {
                        segments: vec![PathSegment {
                            ident: self_type_sym,
                            args: None,
                        }],
                        span: self.prev_span,
                    })),
                    mutability,
                    self.span_from(p_start),
                );
                params.push(Param {
                    pat,
                    ty: self_ty,
                    attrs,
                    span: self.span_from(p_start),
                });
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
                continue;
            }

            if self.at_kw(Keyword::Mut) && self.peek_kind() == &TokenKind::Kw(Keyword::SelfValue) {
                self.bump(); // mut
                self.bump(); // self
                let self_sym = self.interner.intern("self");
                let pat = Pattern::Ident(self_sym, Mutability::Mut, None, self.span_from(p_start));
                let self_type_sym = self.interner.intern("Self");
                let self_ty = Ty::Path(Path {
                    segments: vec![PathSegment {
                        ident: self_type_sym,
                        args: None,
                    }],
                    span: self.prev_span,
                });
                params.push(Param {
                    pat,
                    ty: self_ty,
                    attrs,
                    span: self.span_from(p_start),
                });
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
                continue;
            }

            if self.at_kw(Keyword::SelfValue) && self.peek_kind() == &TokenKind::Colon {
                self.bump(); // self
                self.expect_exact(&TokenKind::Colon);
                let self_sym = self.interner.intern("self");
                let pat = Pattern::Ident(self_sym, Mutability::Immutable, None, self.span_from(p_start));
                let ty = self.parse_ty();
                params.push(Param {
                    pat,
                    ty,
                    attrs,
                    span: self.span_from(p_start),
                });
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
                continue;
            }

            if self.at_kw(Keyword::SelfValue) {
                self.bump();
                let self_sym = self.interner.intern("self");
                let pat = Pattern::Ident(self_sym, Mutability::Immutable, None, self.span_from(p_start));
                let self_type_sym = self.interner.intern("Self");
                let self_ty = Ty::Path(Path {
                    segments: vec![PathSegment {
                        ident: self_type_sym,
                        args: None,
                    }],
                    span: self.prev_span,
                });
                params.push(Param {
                    pat,
                    ty: self_ty,
                    attrs,
                    span: self.span_from(p_start),
                });
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
                continue;
            }

            let pat = self.parse_pattern();
            self.expect_exact(&TokenKind::Colon);
            let ty = self.parse_ty();
            params.push(Param {
                pat,
                ty,
                attrs,
                span: self.span_from(p_start),
            });
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        params
    }

    fn consume_c_variadic_marker(&mut self) -> bool {
        if self.at_exact(&TokenKind::DotDot) && self.peek_kind() == &TokenKind::Dot {
            self.bump(); // ..
            self.bump(); // .
            let _ = self.eat_exact(&TokenKind::Comma);
            true
        } else {
            false
        }
    }

    fn parse_struct_def(
        &mut self,
        vis: Visibility,
        attrs: Vec<Attribute>,
        start: Span,
        is_union: bool,
    ) -> StructDef {
        if is_union {
            let keyword = self.expect_ident();
            debug_assert_eq!(self.interner.resolve(keyword), "union");
        } else {
            self.expect_exact(&TokenKind::Kw(Keyword::Struct));
        }
        let name = self.expect_ident();
        let generics = self.parse_generics();
        let where_clause = self.parse_where_clause();
        let _ = where_clause; // stored in generics implicitly for now
        let mut fields = Vec::new();
        if !is_union && self.at_exact(&TokenKind::LParen) {
            // Tuple struct: struct Foo(pub i32, u64);
            self.bump();
            let mut idx = 0u32;
            while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                let f_start = self.current().span;
                let f_attrs = self.parse_attrs();
                let f_vis = self.parse_visibility();
                let f_ty = self.parse_ty();
                let f_name = self.interner.intern(&idx.to_string());
                idx += 1;
                fields.push(FieldDef {
                    name: f_name,
                    ty: f_ty,
                    vis: f_vis,
                    attrs: f_attrs,
                    span: self.span_from(f_start),
                });
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_exact(&TokenKind::RParen);
            let _where_clause = self.parse_where_clause();
            self.expect_exact(&TokenKind::Semi);
        } else if !is_union && self.eat_exact(&TokenKind::Semi) {
            // Unit struct: struct Foo;
        } else {
            self.expect_exact(&TokenKind::LBrace);
            while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
                let f_start = self.current().span;
                let f_attrs = self.parse_attrs();
                let f_vis = self.parse_visibility();
                let f_name = self.expect_ident();
                self.expect_exact(&TokenKind::Colon);
                let f_ty = self.parse_ty();
                fields.push(FieldDef {
                    name: f_name,
                    ty: f_ty,
                    vis: f_vis,
                    attrs: f_attrs,
                    span: self.span_from(f_start),
                });
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_exact(&TokenKind::RBrace);
        }
        StructDef {
            name,
            generics,
            fields,
            vis,
            attrs,
            is_union,
            span: self.span_from(start),
        }
    }

    fn parse_enum_def(&mut self, vis: Visibility, attrs: Vec<Attribute>, start: Span) -> EnumDef {
        self.expect_exact(&TokenKind::Kw(Keyword::Enum));
        let name = self.expect_ident();
        let generics = self.parse_generics();
        let where_clause = self.parse_where_clause();
        let _ = where_clause;
        self.expect_exact(&TokenKind::LBrace);
        let mut variants = Vec::new();
        while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
            let v_start = self.current().span;
            let v_attrs = self.parse_attrs();
            let v_name = self.expect_ident();
            let fields = if self.at_exact(&TokenKind::LParen) {
                self.bump();
                let mut tys = Vec::new();
                while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                    let _field_attrs = self.parse_attrs();
                    tys.push(self.parse_ty());
                    if !self.eat_exact(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect_exact(&TokenKind::RParen);
                VariantFields::Tuple(tys)
            } else if self.at_exact(&TokenKind::LBrace) {
                self.bump();
                let mut flds = Vec::new();
                while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
                    let f_start = self.current().span;
                    let f_attrs = self.parse_attrs();
                    let f_name = self.expect_ident();
                    self.expect_exact(&TokenKind::Colon);
                    let f_ty = self.parse_ty();
                    flds.push(FieldDef {
                        name: f_name,
                        ty: f_ty,
                        vis: Visibility::Private,
                        attrs: f_attrs,
                        span: self.span_from(f_start),
                    });
                    if !self.eat_exact(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect_exact(&TokenKind::RBrace);
                VariantFields::Struct(flds)
            } else {
                VariantFields::Unit
            };
            let discriminant = if self.eat_exact(&TokenKind::Eq) {
                Some(Box::new(self.parse_expr()))
            } else {
                None
            };
            variants.push(Variant {
                name: v_name,
                fields,
                discriminant,
                attrs: v_attrs,
                span: self.span_from(v_start),
            });
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        self.expect_exact(&TokenKind::RBrace);
        EnumDef {
            name,
            generics,
            variants,
            vis,
            attrs,
            span: self.span_from(start),
        }
    }

    fn parse_impl_block(&mut self, attrs: Vec<Attribute>, start: Span) -> ImplBlock {
        self.expect_exact(&TokenKind::Kw(Keyword::Impl));
        let generics = self.parse_generics();

        // Parse type, then check for `for` (trait impl)
        let first_ty = self.parse_ty();
        let (trait_ref, self_ty) = if self.at_kw(Keyword::For) {
            self.bump();
            // first_ty was actually the trait path
            let trait_path = match first_ty {
                Ty::Path(p) => p,
                _ => panic!("expected trait path in impl"),
            };
            let actual_ty = self.parse_ty();
            (Some(trait_path), actual_ty)
        } else {
            (None, first_ty)
        };

        let where_clause = self.parse_where_clause();
        let _ = where_clause;

        self.expect_exact(&TokenKind::LBrace);
        let mut items = Vec::new();
        while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
            if let Some(item) = self.parse_item() {
                items.push(item);
            } else {
                break;
            }
        }
        self.expect_exact(&TokenKind::RBrace);
        ImplBlock {
            generics,
            trait_ref,
            self_ty,
            items,
            attrs,
            is_unsafe: false,
            span: self.span_from(start),
        }
    }

    fn parse_trait_def(&mut self, vis: Visibility, attrs: Vec<Attribute>, is_unsafe: bool, start: Span) -> TraitDef {
        self.expect_exact(&TokenKind::Kw(Keyword::Trait));
        let name = self.expect_ident();
        let generics = self.parse_generics();

        let mut supertraits = Vec::new();
        if self.eat_exact(&TokenKind::Colon) {
            loop {
                if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                    self.bump();
                    if !self.eat_exact(&TokenKind::Plus) {
                        break;
                    }
                    continue;
                }
                let b_start = self.current().span;
                let path = self.parse_path_ty();
                supertraits.push(TraitBound {
                    path,
                    span: self.span_from(b_start),
                });
                if !self.eat_exact(&TokenKind::Plus) {
                    break;
                }
            }
        }

        let where_clause = self.parse_where_clause();
        let _ = where_clause;

        self.expect_exact(&TokenKind::LBrace);
        let mut items = Vec::new();
        while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
            if let Some(item) = self.parse_item() {
                items.push(item);
            } else {
                break;
            }
        }
        self.expect_exact(&TokenKind::RBrace);
        TraitDef {
            name,
            generics,
            supertraits,
            items,
            vis,
            is_unsafe,
            attrs,
            span: self.span_from(start),
        }
    }

    fn parse_use_tree_item(&mut self, vis: Visibility, attrs: Vec<Attribute>, start: Span) -> UseTree {
        self.expect_exact(&TokenKind::Kw(Keyword::Use));
        let tree = self.parse_use_tree(vis);
        self.expect_exact(&TokenKind::Semi);
        UseTree {
            attrs,
            span: self.span_from(start),
            ..tree
        }
    }

    fn parse_use_tree(&mut self, vis: Visibility) -> UseTree {
        let start = self.current().span;
        let mut path = Vec::new();

        // Absolute use paths: `use ::foo::bar;` or grouped `use ::{a, b};`.
        // The AST currently stores logical segments only; resolution treats an
        // empty prefix with nested entries the same as a crate-root group.
        self.eat_exact(&TokenKind::ColonColon);

        if self.at_exact(&TokenKind::LBrace) {
            self.bump();
            let mut nested = Vec::new();
            while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
                nested.push(self.parse_use_tree(vis));
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_exact(&TokenKind::RBrace);
            return UseTree {
                vis,
                path,
                kind: UseTreeKind::Nested(nested),
                attrs: Vec::new(),
                span: self.span_from(start),
            };
        }

        if self.at_exact(&TokenKind::Star) {
            self.bump();
            return UseTree {
                vis,
                path,
                kind: UseTreeKind::Glob,
                attrs: Vec::new(),
                span: self.span_from(start),
            };
        }

        // Collect path segments
        loop {
            if self.at_ident()
                || self.at_kw(Keyword::SelfValue)
                || self.at_kw(Keyword::Super)
                || self.at_kw(Keyword::Crate)
                || self.at_kw(Keyword::Ref)
                || self.at_exact(&TokenKind::Dollar)
            {
                path.push(self.expect_use_tree_segment());
            } else {
                break;
            }
            if !self.eat_exact(&TokenKind::ColonColon) {
                // Simple use, possibly with alias
                let alias = if self.at_kw(Keyword::As) {
                    self.bump();
                    Some(self.expect_ident_or_self())
                } else {
                    None
                };
                return UseTree {
                    vis,
                    path,
                    kind: UseTreeKind::Simple(alias),
                    attrs: Vec::new(),
                    span: self.span_from(start),
                };
            }
            // After ::, check for *, {, or continue path
            if self.at_exact(&TokenKind::Star) {
                self.bump();
                return UseTree {
                    vis,
                    path,
                    kind: UseTreeKind::Glob,
                    attrs: Vec::new(),
                    span: self.span_from(start),
                };
            }
            if self.at_exact(&TokenKind::LBrace) {
                self.bump();
                let mut nested = Vec::new();
                while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
                    nested.push(self.parse_use_tree(vis));
                    if !self.eat_exact(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect_exact(&TokenKind::RBrace);
                return UseTree {
                    vis,
                    path,
                    kind: UseTreeKind::Nested(nested),
                    attrs: Vec::new(),
                    span: self.span_from(start),
                };
            }
        }

        // Check for `as` alias
        let alias = if self.at_kw(Keyword::As) {
            self.bump();
            Some(self.expect_ident())
        } else {
            None
        };

        UseTree {
            vis,
            path,
            kind: UseTreeKind::Simple(alias),
            attrs: Vec::new(),
            span: self.span_from(start),
        }
    }

    fn expect_use_tree_segment(&mut self) -> Symbol {
        if self.at_kw(Keyword::Ref) {
            self.bump();
            self.interner.intern("ref")
        } else {
            self.expect_ident_or_self()
        }
    }

    fn parse_mod_def(&mut self, vis: Visibility, attrs: Vec<Attribute>, start: Span) -> ModDef {
        self.expect_exact(&TokenKind::Kw(Keyword::Mod));
        let name = self.expect_ident();
        let items = if self.at_exact(&TokenKind::LBrace) {
            self.bump();
            let mut items = Vec::new();
            while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
                if let Some(item) = self.parse_item() {
                    items.push(item);
                } else {
                    break;
                }
            }
            self.expect_exact(&TokenKind::RBrace);
            Some(items)
        } else {
            self.expect_exact(&TokenKind::Semi);
            None
        };
        ModDef {
            name,
            items,
            attrs,
            vis,
            span: self.span_from(start),
        }
    }

    fn parse_const_def(&mut self, vis: Visibility, attrs: Vec<Attribute>, start: Span) -> ConstDef {
        self.expect_exact(&TokenKind::Kw(Keyword::Const));
        let name = self.expect_ident();
        self.expect_exact(&TokenKind::Colon);
        let ty = self.parse_ty();
        let value = if self.eat_exact(&TokenKind::Eq) {
            Some(Box::new(self.parse_expr()))
        } else {
            None
        };
        self.expect_exact(&TokenKind::Semi);
        ConstDef {
            name,
            ty,
            value,
            vis,
            attrs,
            span: self.span_from(start),
        }
    }

    fn parse_static_def(&mut self, vis: Visibility, attrs: Vec<Attribute>, start: Span) -> StaticDef {
        self.expect_exact(&TokenKind::Kw(Keyword::Static));
        let is_mut = self.eat_exact(&TokenKind::Kw(Keyword::Mut));
        let name = self.expect_ident();
        self.expect_exact(&TokenKind::Colon);
        let ty = self.parse_ty();
        let value = if self.eat_exact(&TokenKind::Eq) {
            Some(Box::new(self.parse_expr()))
        } else {
            None
        };
        self.expect_exact(&TokenKind::Semi);
        StaticDef {
            name,
            ty,
            value,
            vis,
            is_mut,
            attrs,
            span: self.span_from(start),
        }
    }

    fn parse_extern_block(&mut self, abi: Option<String>, attrs: Vec<Attribute>, start: Span) -> ExternBlockDef {
        self.expect_exact(&TokenKind::LBrace);
        let mut items = Vec::new();
        while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
            if let Some(item) = self.parse_item() {
                items.push(item);
            } else {
                break;
            }
        }
        self.expect_exact(&TokenKind::RBrace);
        ExternBlockDef {
            abi,
            items,
            attrs,
            span: self.span_from(start),
        }
    }

    fn parse_type_alias(&mut self, vis: Visibility, attrs: Vec<Attribute>, start: Span) -> TypeAliasDef {
        self.expect_exact(&TokenKind::Kw(Keyword::Type));
        let name = self.expect_ident();
        let generics = self.parse_generics();
        self.parse_type_alias_bounds();
        let _where_clause = self.parse_where_clause();
        let ty = if self.eat_exact(&TokenKind::Eq) {
            Some(Box::new(self.parse_ty()))
        } else {
            None
        };
        let _where_clause = self.parse_where_clause();
        self.expect_exact(&TokenKind::Semi);
        TypeAliasDef {
            name,
            generics,
            ty,
            vis,
            attrs,
            span: self.span_from(start),
        }
    }

    fn parse_type_alias_bounds(&mut self) {
        if !self.eat_exact(&TokenKind::Colon) {
            return;
        }
        loop {
            if self.at_kw(Keyword::Where)
                || self.at_exact(&TokenKind::Eq)
                || self.at_exact(&TokenKind::Semi)
                || self.at_exact(&TokenKind::Comma)
                || self.at_exact(&TokenKind::RBrace)
                || self.at_exact(&TokenKind::Eof)
            {
                break;
            }
            if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                self.bump();
            } else {
                let _relaxed_bound = self.eat_exact(&TokenKind::Question);
                let _ = self.parse_path_ty();
                self.parse_callable_trait_suffix();
            }
            if !self.eat_exact(&TokenKind::Plus) {
                break;
            }
        }
    }

    fn parse_visibility(&mut self) -> Visibility {
        if self.at_kw(Keyword::Pub) {
            self.bump();
            if self.at_exact(&TokenKind::LParen) {
                self.bump(); // (
                let vis = if self.at_kw(Keyword::Crate) {
                    self.bump();
                    Visibility::PubCrate
                } else if self.at_kw(Keyword::Super) {
                    self.bump();
                    Visibility::PubSuper
                } else if self.at_kw(Keyword::SelfValue) {
                    self.bump();
                    Visibility::PubSelf
                } else if self.at_kw(Keyword::In) {
                    self.bump();
                    let _path = self.parse_path_ty();
                    Visibility::PubIn
                } else {
                    Visibility::Public
                };
                while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                    self.bump();
                }
                self.expect_exact(&TokenKind::RParen);
                return vis;
            }
            Visibility::Public
        } else {
            Visibility::Private
        }
    }

    fn parse_attrs(&mut self) -> Vec<Attribute> {
        let mut attrs = Vec::new();
        while self.at_exact(&TokenKind::Hash) || matches!(self.current().kind, TokenKind::DocComment(_, false)) {
            if let TokenKind::DocComment(text, false) = self.current().kind.clone() {
                let span = self.bump().span;
                attrs.push(self.doc_comment_attr(text, span));
                continue;
            }
            let start = self.current().span;
            self.bump(); // #
            let _inner = self.eat_exact(&TokenKind::Not);
            self.expect_exact(&TokenKind::LBracket);
            let path = self.parse_path_ty();
            let args = if self.at_exact(&TokenKind::LParen) {
                self.bump();
                let tts = self.collect_token_trees(&TokenKind::RParen);
                self.expect_exact(&TokenKind::RParen);
                AttrArgs::Delimited(tts)
            } else if self.eat_exact(&TokenKind::Eq) {
                AttrArgs::Eq(Box::new(self.parse_expr()))
            } else {
                AttrArgs::Empty
            };
            self.expect_exact(&TokenKind::RBracket);
            attrs.push(Attribute {
                path,
                args,
                span: self.span_from(start),
            });
        }
        attrs
    }

    // ── Types ──

    fn parse_ty(&mut self) -> Ty {
        let start = self.current().span;

        // Reference: &T, &mut T, &'a T, &'a mut T
        if self.at_exact(&TokenKind::Amp) {
            self.bump();
            let lifetime = if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                if let TokenKind::Lifetime(sym) = self.bump().kind {
                    Some(sym)
                } else {
                    None
                }
            } else {
                None
            };
            let mutability = if self.eat_exact(&TokenKind::Kw(Keyword::Mut)) {
                Mutability::Mut
            } else {
                Mutability::Immutable
            };
            let ty = self.parse_ty();
            return Ty::Reference(lifetime, Box::new(ty), mutability, self.span_from(start));
        }

        // `&&T` lexes as a single AndAnd token but is two shared references in type position.
        if self.at_exact(&TokenKind::AndAnd) {
            self.bump();
            let inner_lifetime = if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                if let TokenKind::Lifetime(sym) = self.bump().kind {
                    Some(sym)
                } else {
                    None
                }
            } else {
                None
            };
            let inner_mutability = if self.eat_exact(&TokenKind::Kw(Keyword::Mut)) {
                Mutability::Mut
            } else {
                Mutability::Immutable
            };
            let inner_ty = self.parse_ty();
            let inner_ref = Ty::Reference(
                inner_lifetime,
                Box::new(inner_ty),
                inner_mutability,
                self.span_from(start),
            );
            return Ty::Reference(
                None,
                Box::new(inner_ref),
                Mutability::Immutable,
                self.span_from(start),
            );
        }

        // Raw pointer: *const T, *mut T
        if self.at_exact(&TokenKind::Star) {
            self.bump();
            let mutability = if self.at_kw(Keyword::Const) {
                self.bump();
                Mutability::Immutable
            } else {
                self.expect_exact(&TokenKind::Kw(Keyword::Mut));
                Mutability::Mut
            };
            let ty = self.parse_ty();
            return Ty::RawPtr(Box::new(ty), mutability, self.span_from(start));
        }

        // Tuple: (A, B, C)
        if self.at_exact(&TokenKind::LParen) {
            self.bump();
            if self.at_exact(&TokenKind::RParen) {
                self.bump();
                return Ty::Tuple(vec![], self.span_from(start));
            }
            let first = self.parse_ty();
            if self.at_exact(&TokenKind::Comma) {
                self.bump();
                let mut tys = vec![first];
                while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                    tys.push(self.parse_ty());
                    if !self.eat_exact(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect_exact(&TokenKind::RParen);
                return Ty::Tuple(tys, self.span_from(start));
            }
            self.expect_exact(&TokenKind::RParen);
            // Single-element tuple is just the type in parens
            return first;
        }

        // Array [T; N] or Slice [T]
        if self.at_exact(&TokenKind::LBracket) {
            self.bump();
            let elem_ty = self.parse_ty();
            if self.eat_exact(&TokenKind::Semi) {
                let len = self.parse_expr();
                self.expect_exact(&TokenKind::RBracket);
                return Ty::Array(Box::new(elem_ty), Box::new(len), self.span_from(start));
            }
            self.expect_exact(&TokenKind::RBracket);
            return Ty::Slice(Box::new(elem_ty), self.span_from(start));
        }

        // impl Trait / impl Fn(...)
        if self.at_kw(Keyword::Impl) {
            self.bump();
            if self.at_ident() {
                let name = match &self.current().kind {
                    TokenKind::Ident(sym) => self.interner.resolve(*sym),
                    _ => "",
                };
                if matches!(name, "Fn" | "FnMut" | "FnOnce") && self.peek_kind() == &TokenKind::LParen {
                    self.bump();
                    self.expect_exact(&TokenKind::LParen);
                    let mut param_tys = Vec::new();
                    while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                        if self.consume_c_variadic_marker() {
                            break;
                        }
                        param_tys.push(self.parse_fn_ptr_param_ty());
                        if !self.eat_exact(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect_exact(&TokenKind::RParen);
                    let ret = if self.eat_exact(&TokenKind::Arrow) {
                        Some(Box::new(self.parse_ty()))
                    } else {
                        None
                    };
                    while self.eat_exact(&TokenKind::Plus) {
                        if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                            self.bump();
                        } else {
                            let _ = self.parse_path_ty();
                        }
                    }
                    return Ty::FnPtr(param_tys, ret, self.span_from(start));
                }
            }
            return self.parse_trait_object_ty(start);
        }

        // unsafe extern "C" fn(A, B) -> C / extern "C" fn(A, B) -> C / fn(A, B) -> C
        let saw_unsafe = self.eat_exact(&TokenKind::Kw(Keyword::Unsafe));
        let saw_extern = if self.at_kw(Keyword::Extern) {
            self.bump();
            if let TokenKind::StringLit(_) = &self.current().kind {
                self.bump();
            }
            true
        } else {
            false
        };

        if self.at_kw(Keyword::Fn) {
            self.bump();
            self.expect_exact(&TokenKind::LParen);
            let mut param_tys = Vec::new();
            while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                if self.consume_c_variadic_marker() {
                    break;
                }
                param_tys.push(self.parse_fn_ptr_param_ty());
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_exact(&TokenKind::RParen);
            let ret = if self.eat_exact(&TokenKind::Arrow) {
                Some(Box::new(self.parse_ty()))
            } else {
                None
            };
            return Ty::FnPtr(param_tys, ret, self.span_from(start));
        }
        if saw_unsafe || saw_extern {
            panic!(
                "expected `fn` after function pointer qualifier in type position at {:?}",
                self.current().span
            );
        }

        if self.at_exact(&TokenKind::Lt) || self.at_exact(&TokenKind::Shl) {
            return Ty::QualifiedPath(self.parse_qualified_path(start));
        }

        // Infer: _
        if self.at_ident() {
            let sym = match &self.current().kind {
                TokenKind::Ident(sym) => *sym,
                _ => unreachable!(),
            };
            if self.interner.resolve(sym) == "_" {
                self.bump();
                return Ty::Infer(self.span_from(start));
            }
        }
        if self.at_ident() && self.peek_kind() == &TokenKind::Not {
            return self.parse_type_macro_ty(start);
        }
        if self.at_exact(&TokenKind::Kw(Keyword::Mut)) {
            // This shouldn't happen in type position normally
        }

        // Never: !
        if self.at_exact(&TokenKind::Not) {
            self.bump();
            return Ty::Never(self.span_from(start));
        }

        // dyn Trait
        if self.at_kw(Keyword::Dyn) {
            self.bump();
            return self.parse_trait_object_ty(start);
        }

        // Path type
        let path = self.parse_path_ty();
        Ty::Path(path)
    }

    fn parse_type_macro_ty(&mut self, start: Span) -> Ty {
        let macro_name = if let TokenKind::Ident(sym) = self.bump().kind {
            sym
        } else {
            unreachable!()
        };
        self.expect_exact(&TokenKind::Not);

        let macro_name_str = self.interner.resolve(macro_name);
        if macro_name_str == "to_signed_int" && self.at_exact(&TokenKind::LParen) {
            self.bump();
            let arg_ty = self.parse_ty();
            self.expect_exact(&TokenKind::RParen);
            if let Ty::Path(path) = &arg_ty {
                if path.segments.len() == 1 {
                    let name = self.interner.resolve(path.segments[0].ident);
                    let mapped = match name {
                        "u8" => Some("i8"),
                        "u16" => Some("i16"),
                        "u32" => Some("i32"),
                        "u64" => Some("i64"),
                        "u128" => Some("i128"),
                        "usize" => Some("isize"),
                        _ => None,
                    };
                    if let Some(mapped) = mapped {
                        let ident = self.interner.intern(mapped);
                        return Ty::Path(Path {
                            segments: vec![PathSegment { ident, args: None }],
                            span: self.span_from(start),
                        });
                    }
                }
            }
            let ident = self.interner.intern("isize");
            return Ty::Path(Path {
                segments: vec![PathSegment { ident, args: None }],
                span: self.span_from(start),
            });
        }

        let args = self.parse_macro_args();

        Ty::MacroCall(macro_name, args, self.span_from(start))
    }

    fn parse_trait_object_ty(&mut self, start: Span) -> Ty {
        let first_start = self.current().span;
        self.parse_optional_for_binder();
        let first = self.parse_path_ty();
        let mut bounds = vec![TraitBound {
            path: first,
            span: self.span_from(first_start),
        }];
        while self.eat_exact(&TokenKind::Plus) {
            if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                self.bump();
                continue;
            }
            let bound_start = self.current().span;
            self.parse_optional_for_binder();
            let path = self.parse_path_ty();
            self.parse_callable_trait_suffix();
            bounds.push(TraitBound {
                path,
                span: self.span_from(bound_start),
            });
        }
        Ty::DynTrait(bounds, self.span_from(start))
    }

    fn parse_fn_ptr_param_ty(&mut self) -> Ty {
        if self.at_exact(&TokenKind::Kw(Keyword::Mut))
            && matches!(self.peek_kind_at(1), Some(TokenKind::Ident(_)))
            && self.peek_kind_at(2) == Some(&TokenKind::Colon)
        {
            self.bump(); // mut
            self.bump(); // name
            self.bump(); // :
        } else if matches!(self.current().kind, TokenKind::Ident(_))
            && self.peek_kind() == &TokenKind::Colon
        {
            self.bump(); // name
            self.bump(); // :
        }
        self.parse_ty()
    }

    fn parse_path_ty(&mut self) -> Path {
        let start = self.current().span;
        let mut segments = Vec::new();

        if self.at_exact(&TokenKind::ColonColon) {
            self.bump();
        }

        let ident = self.expect_ident_or_self();
        let args = self.parse_path_segment_type_args(ident);
        segments.push(PathSegment { ident, args });

        while self.at_exact(&TokenKind::ColonColon) {
            self.bump();
            let ident = self.expect_ident_or_self();
            let args = self.parse_path_segment_type_args(ident);
            segments.push(PathSegment { ident, args });
        }

        Path {
            segments,
            span: self.span_from(start),
        }
    }

    fn parse_path_segment_type_args(&mut self, ident: Symbol) -> Option<GenericArgs> {
        if self.at_exact(&TokenKind::Lt) || self.at_exact(&TokenKind::Shl) {
            return Some(self.parse_generic_args());
        }
        if self.at_exact(&TokenKind::LParen) && self.is_callable_trait_ident(ident) {
            return Some(self.parse_callable_trait_args());
        }
        None
    }

    fn is_callable_trait_ident(&self, ident: Symbol) -> bool {
        matches!(self.interner.resolve(ident), "Fn" | "FnMut" | "FnOnce")
    }

    fn parse_callable_trait_args(&mut self) -> GenericArgs {
        let start = self.current().span;
        self.expect_exact(&TokenKind::LParen);
        let mut args = Vec::new();
        while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
            args.push(GenericArg::Type(self.parse_fn_ptr_param_ty()));
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        self.expect_exact(&TokenKind::RParen);
        if self.eat_exact(&TokenKind::Arrow) {
            args.push(GenericArg::Type(self.parse_ty()));
        }
        GenericArgs {
            args,
            span: self.span_from(start),
        }
    }

    fn parse_callable_trait_suffix(&mut self) {
        if !self.at_exact(&TokenKind::LParen) {
            return;
        }
        self.bump();
        while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
            let _ = self.parse_ty();
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        self.expect_exact(&TokenKind::RParen);
        if self.eat_exact(&TokenKind::Arrow) {
            let _ = self.parse_ty();
        }
    }

    // ── Generics ──

    fn parse_generics(&mut self) -> Generics {
        let start = self.current().span;
        if !self.at_exact(&TokenKind::Lt) {
            return Generics {
                params: vec![],
                span: self.span_from(start),
            };
        }
        self.bump(); // <
        let mut params = Vec::new();
        while !self.at_exact(&TokenKind::Gt) && !self.at_exact(&TokenKind::Eof) {
            let p_start = self.current().span;
            let _attrs = self.parse_attrs();
            if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                if let TokenKind::Lifetime(sym) = self.bump().kind {
                    let mut bounds = Vec::new();
                    if self.eat_exact(&TokenKind::Colon) {
                        while matches!(self.current().kind, TokenKind::Lifetime(_)) {
                            if let TokenKind::Lifetime(b) = self.bump().kind {
                                bounds.push(b);
                            }
                            if !self.eat_exact(&TokenKind::Plus) {
                                break;
                            }
                        }
                    }
                    params.push(GenericParam::Lifetime(sym, bounds, self.span_from(p_start)));
                }
            } else if self.at_kw(Keyword::Const) {
                self.bump();
                let name = self.expect_ident();
                self.expect_exact(&TokenKind::Colon);
                let ty = self.parse_ty();
                params.push(GenericParam::Const(name, ty, self.span_from(p_start)));
            } else {
                let name = self.expect_ident();
                let mut bounds = Vec::new();
                let default = if self.eat_exact(&TokenKind::Colon) {
                    loop {
                        // Skip lifetime bounds (e.g. T: Copy + 'static)
                        if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                            self.bump();
                            if !self.eat_exact(&TokenKind::Plus) {
                                break;
                            }
                            continue;
                        }
                        // Stop if we hit >, ,, =, or { (not a bound)
                        if self.at_exact(&TokenKind::Gt)
                            || self.at_exact(&TokenKind::Comma)
                            || self.at_exact(&TokenKind::Eq)
                            || self.at_exact(&TokenKind::LBrace)
                        {
                            break;
                        }
                        let _relaxed_bound = self.eat_exact(&TokenKind::Question);
                        let b_start = self.current().span;
                        let path = self.parse_path_ty();
                        self.parse_callable_trait_suffix();
                        bounds.push(TraitBound {
                            path,
                            span: self.span_from(b_start),
                        });
                        if !self.eat_exact(&TokenKind::Plus) {
                            break;
                        }
                    }
                    if self.eat_exact(&TokenKind::Eq) {
                        Some(self.parse_ty())
                    } else {
                        None
                    }
                } else if self.eat_exact(&TokenKind::Eq) {
                    Some(self.parse_ty())
                } else {
                    None
                };
                params.push(GenericParam::Type(name, bounds, default, self.span_from(p_start)));
            }
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        self.expect_exact(&TokenKind::Gt);
        Generics {
            params,
            span: self.span_from(start),
        }
    }

    fn parse_generic_args(&mut self) -> GenericArgs {
        let start = self.current().span;
        self.expect_type_arg_lt();
        let mut args = Vec::new();
        while !self.at_exact(&TokenKind::Gt) && !self.at_exact(&TokenKind::Eof) {
            if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                if let TokenKind::Lifetime(sym) = self.bump().kind {
                    args.push(GenericArg::Lifetime(sym));
                }
            } else if matches!(self.current().kind, TokenKind::Ident(_))
                && self.peek_kind() == &TokenKind::Eq
            {
                // Associated type binding in generic args, e.g. Iterator<Item = T>.
                let binding_name = match self.bump().kind {
                    TokenKind::Ident(sym) => sym,
                    _ => unreachable!(),
                };
                self.bump(); // =
                args.push(GenericArg::AssocTypeBinding(binding_name, self.parse_ty()));
            } else if self.at_const_generic_arg_start() {
                args.push(GenericArg::Const(self.parse_const_generic_arg()));
            } else {
                args.push(GenericArg::Type(self.parse_ty()));
            }
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        // Handle >> as two > tokens
        if self.at_exact(&TokenKind::Shr) {
            // Split >> into > >. Consume one >.
            // We need to transform the current Shr token into a Gt token for the next read.
            // Easiest: just replace current token.
            let cur = self.pos;
            let span = self.tokens[cur].span;
            self.tokens[cur] = Token {
                kind: TokenKind::Gt,
                span: Span::new(span.start() + 1, span.end()),
            };
            self.prev_span = Span::new(span.start(), span.start() + 1);
        } else {
            self.expect_exact(&TokenKind::Gt);
        }
        GenericArgs {
            args,
            span: self.span_from(start),
        }
    }

    fn expect_type_arg_lt(&mut self) {
        if self.at_exact(&TokenKind::Shl) {
            let cur = self.pos;
            let span = self.tokens[cur].span;
            self.tokens[cur] = Token {
                kind: TokenKind::Lt,
                span: Span::new(span.start() + 1, span.end()),
            };
            self.prev_span = Span::new(span.start(), span.start() + 1);
        } else {
            self.expect_exact(&TokenKind::Lt);
        }
    }

    fn at_const_generic_arg_start(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::IntLit(_, _)
                | TokenKind::FloatLit(_)
                | TokenKind::StringLit(_)
                | TokenKind::CharLit(_)
                | TokenKind::ByteStringLit(_)
                | TokenKind::LBrace
                | TokenKind::Minus
                | TokenKind::Kw(Keyword::True)
                | TokenKind::Kw(Keyword::False)
        )
    }

    fn parse_const_generic_arg(&mut self) -> Expr {
        // Avoid parsing the closing `>` as a greater-than operator for simple
        // const arguments like `foo::<8>()`.
        self.parse_prefix_expr()
    }

    fn parse_where_clause(&mut self) -> WhereClause {
        let start = self.current().span;
        if !self.at_kw(Keyword::Where) {
            return WhereClause {
                predicates: vec![],
                span: self.span_from(start),
            };
        }
        self.bump();
        let mut predicates = Vec::new();
        loop {
            if self.at_exact(&TokenKind::LBrace)
                || self.at_exact(&TokenKind::Semi)
                || self.at_exact(&TokenKind::Eof)
            {
                break;
            }
            let p_start = self.current().span;
            self.parse_optional_for_binder();
            if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                if let TokenKind::Lifetime(sym) = self.bump().kind {
                    self.expect_exact(&TokenKind::Colon);
                    let mut bounds = Vec::new();
                    while matches!(self.current().kind, TokenKind::Lifetime(_)) {
                        if let TokenKind::Lifetime(b) = self.bump().kind {
                            bounds.push(b);
                        }
                        if !self.eat_exact(&TokenKind::Plus) {
                            break;
                        }
                    }
                    predicates.push(WherePredicate::Lifetime(
                        sym,
                        bounds,
                        self.span_from(p_start),
                    ));
                }
            } else {
                let ty = self.parse_ty();
                self.expect_exact(&TokenKind::Colon);
                let mut bounds = Vec::new();
                loop {
                    // Skip lifetime bounds in where clauses
                    if matches!(self.current().kind, TokenKind::Lifetime(_)) {
                        self.bump();
                        if !self.eat_exact(&TokenKind::Plus) {
                            break;
                        }
                        continue;
                    }
                    if self.at_exact(&TokenKind::LBrace)
                        || self.at_exact(&TokenKind::Semi)
                        || self.at_exact(&TokenKind::Comma)
                        || self.at_exact(&TokenKind::Eof)
                    {
                        break;
                    }
                    let _relaxed_bound = self.eat_exact(&TokenKind::Question);
                    let b_start = self.current().span;
                    self.parse_optional_for_binder();
                    let path = self.parse_path_ty();
                    self.parse_callable_trait_suffix();
                    bounds.push(TraitBound {
                        path,
                        span: self.span_from(b_start),
                    });
                    if !self.eat_exact(&TokenKind::Plus) {
                        break;
                    }
                }
                predicates.push(WherePredicate::Type(ty, bounds, self.span_from(p_start)));
            }
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        WhereClause {
            predicates,
            span: self.span_from(start),
        }
    }

    fn parse_optional_for_binder(&mut self) {
        if !self.at_kw(Keyword::For) {
            return;
        }
        self.bump();
        if !self.eat_exact(&TokenKind::Lt) {
            return;
        }
        while !self.at_exact(&TokenKind::Gt) && !self.at_exact(&TokenKind::Eof) {
            self.bump();
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        self.expect_exact(&TokenKind::Gt);
    }

    // ── Patterns ──

    fn parse_pattern(&mut self) -> Pattern {
        self.parse_pattern_with_or(true)
    }

    fn parse_pattern_no_or(&mut self) -> Pattern {
        self.parse_pattern_with_or(false)
    }

    fn parse_pattern_with_or(&mut self, allow_or: bool) -> Pattern {
        let start = self.current().span;
        let mut patterns = vec![self.parse_pattern_atom(allow_or)];
        if allow_or {
            while self.eat_exact(&TokenKind::Pipe) {
                patterns.push(self.parse_pattern_atom(true));
            }
        }
        if patterns.len() == 1 {
            patterns.pop().unwrap()
        } else {
            Pattern::Or(patterns, self.span_from(start))
        }
    }

    fn parse_pattern_atom(&mut self, allow_or: bool) -> Pattern {
        let start = self.current().span;

        if self.at_exact(&TokenKind::Hash) {
            let _attrs = self.parse_attrs();
            return self.parse_pattern_atom(allow_or);
        }

        // Wildcard _
        // We need to check for _ which is an identifier with text "_"
        if self.at_ident() {
            let sym = match &self.current().kind {
                TokenKind::Ident(s) => *s,
                _ => unreachable!(),
            };
            let name = self.interner.resolve(sym);
            if name == "_" {
                self.bump();
                return Pattern::Wildcard(self.span_from(start));
            }
        }

        if self.at_exact(&TokenKind::DotDot) {
            self.bump();
            return Pattern::Rest(self.span_from(start));
        }

        // &pat, &mut pat
        if self.at_exact(&TokenKind::Amp) {
            self.bump();
            let mutability = if self.eat_exact(&TokenKind::Kw(Keyword::Mut)) {
                Mutability::Mut
            } else {
                Mutability::Immutable
            };
            let pat = self.parse_pattern_with_or(allow_or);
            return Pattern::Ref(Box::new(pat), mutability, self.span_from(start));
        }

        // ref pat, ref mut pat
        if self.at_kw(Keyword::Ref) {
            self.bump();
            let mutability = if self.eat_exact(&TokenKind::Kw(Keyword::Mut)) {
                Mutability::Mut
            } else {
                Mutability::Immutable
            };
            let pat = self.parse_pattern_with_or(allow_or);
            return Pattern::RefBinding(Box::new(pat), mutability, self.span_from(start));
        }

        // `&&pat` is tokenized as `AndAnd`, but in pattern position it means
        // nested immutable reference patterns such as `|&&value|`.
        if self.at_exact(&TokenKind::AndAnd) {
            self.bump();
            let inner = self.parse_pattern_with_or(allow_or);
            let inner_ref = Pattern::Ref(
                Box::new(inner),
                Mutability::Immutable,
                self.span_from(start),
            );
            return Pattern::Ref(
                Box::new(inner_ref),
                Mutability::Immutable,
                self.span_from(start),
            );
        }

        // Literal patterns
        match &self.current().kind {
            TokenKind::IntLit(_, _) => {
                if let TokenKind::IntLit(v, _) = self.bump().kind {
                    // Check for range pattern: 0..9 or 0..=9
                    if self.at_exact(&TokenKind::DotDot) || self.at_exact(&TokenKind::DotDotEq) {
                        let inclusive = self.at_exact(&TokenKind::DotDotEq);
                        self.bump();
                        if let TokenKind::IntLit(hi, _) = self.bump().kind {
                            return Pattern::Range(
                                Some(Box::new(Expr::Lit(Literal::Int(v), self.span_from(start)))),
                                Some(Box::new(Expr::Lit(Literal::Int(hi), self.span_from(start)))),
                                inclusive,
                                self.span_from(start),
                            );
                        }
                    }
                    return Pattern::Literal(Literal::Int(v), self.span_from(start));
                }
            }
            TokenKind::StringLit(_) => {
                if let TokenKind::StringLit(s) = self.bump().kind {
                    return Pattern::Literal(Literal::String(s), self.span_from(start));
                }
            }
            TokenKind::CharLit(_) => {
                if let TokenKind::CharLit(c) = self.bump().kind {
                    if self.at_exact(&TokenKind::DotDot) || self.at_exact(&TokenKind::DotDotEq) {
                        let inclusive = self.at_exact(&TokenKind::DotDotEq);
                        self.bump();
                        if let TokenKind::CharLit(hi) = self.bump().kind {
                            return Pattern::Range(
                                Some(Box::new(Expr::Lit(Literal::Char(c), self.span_from(start)))),
                                Some(Box::new(Expr::Lit(Literal::Char(hi), self.span_from(start)))),
                                inclusive,
                                self.span_from(start),
                            );
                        }
                    }
                    return Pattern::Literal(Literal::Char(c), self.span_from(start));
                }
            }
            TokenKind::ByteStringLit(_) => {
                if let TokenKind::ByteStringLit(v) = self.bump().kind {
                    return Pattern::Literal(Literal::ByteString(v), self.span_from(start));
                }
            }
            TokenKind::Kw(Keyword::True) => {
                self.bump();
                return Pattern::Literal(Literal::Bool(true), self.span_from(start));
            }
            TokenKind::Kw(Keyword::False) => {
                self.bump();
                return Pattern::Literal(Literal::Bool(false), self.span_from(start));
            }
            _ => {}
        }

        // Negative literal pattern: -42
        if self.at_exact(&TokenKind::Minus) {
            self.bump();
            if let TokenKind::IntLit(v, _) = self.bump().kind {
                // Store as literal; the negation is implicit
                return Pattern::Literal(Literal::Int(v), self.span_from(start));
            }
        }

        // Tuple pattern: (a, b)
        if self.at_exact(&TokenKind::LParen) {
            self.bump();
            let mut pats = Vec::new();
            while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                pats.push(self.parse_pattern_with_or(allow_or));
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_exact(&TokenKind::RParen);
            return Pattern::Tuple(pats, self.span_from(start));
        }

        // Slice/array pattern: [a, b, ..]
        if self.at_exact(&TokenKind::LBracket) {
            self.bump();
            let mut pats = Vec::new();
            while !self.at_exact(&TokenKind::RBracket) && !self.at_exact(&TokenKind::Eof) {
                pats.push(self.parse_pattern_with_or(allow_or));
                if !self.eat_exact(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_exact(&TokenKind::RBracket);
            return Pattern::Slice(pats, self.span_from(start));
        }

        // mut ident
        if self.at_kw(Keyword::Mut) {
            self.bump();
            let name = self.expect_ident();
            return Pattern::Ident(name, Mutability::Mut, None, self.span_from(start));
        }

        // Ident pattern (possibly path, tuple struct, struct pattern)
        if self.at_ident()
            || self.at_kw(Keyword::SelfValue)
            || self.at_kw(Keyword::SelfType)
            || self.at_kw(Keyword::Super)
            || self.at_kw(Keyword::Crate)
        {
            let first = self.expect_ident_or_self();

            // Path pattern
            if self.at_exact(&TokenKind::ColonColon) {
                let mut segments = vec![PathSegment {
                    ident: first,
                    args: None,
                }];
                while self.eat_exact(&TokenKind::ColonColon) {
                    let seg = self.expect_ident_or_self();
                    segments.push(PathSegment {
                        ident: seg,
                        args: None,
                    });
                }
                let path = Path {
                    segments,
                    span: self.span_from(start),
                };
                // Tuple struct pattern
                if self.at_exact(&TokenKind::LParen) {
                    self.bump();
                    let mut pats = Vec::new();
                    while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                        pats.push(self.parse_pattern_with_or(allow_or));
                        if !self.eat_exact(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect_exact(&TokenKind::RParen);
                    return Pattern::TupleStruct(path, pats, self.span_from(start));
                }
                // Struct pattern
                if self.at_exact(&TokenKind::LBrace) {
                    return self.parse_struct_pattern(path, start, allow_or);
                }
                return Pattern::Path(path);
            }

            // Tuple struct with single-segment path
            if self.at_exact(&TokenKind::LParen) {
                let path = Path {
                    segments: vec![PathSegment {
                        ident: first,
                        args: None,
                    }],
                    span: self.span_from(start),
                };
                self.bump();
                let mut pats = Vec::new();
                while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                    pats.push(self.parse_pattern_with_or(allow_or));
                    if !self.eat_exact(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect_exact(&TokenKind::RParen);
                return Pattern::TupleStruct(path, pats, self.span_from(start));
            }

            // Struct pattern with single-segment path
            if self.at_exact(&TokenKind::LBrace) {
                let path = Path {
                    segments: vec![PathSegment {
                        ident: first,
                        args: None,
                    }],
                    span: self.span_from(start),
                };
                return self.parse_struct_pattern(path, start, allow_or);
            }

            // @ binding
            let sub_pat = if self.eat_exact(&TokenKind::At) {
                Some(Box::new(self.parse_pattern_with_or(allow_or)))
            } else {
                None
            };

            return Pattern::Ident(first, Mutability::Immutable, sub_pat, self.span_from(start));
        }

        panic!(
            "unexpected token in pattern: {:?} at {:?}; near {}",
            self.current().kind,
            self.current().span,
            self.token_window()
        );
    }

    fn parse_struct_pattern(&mut self, path: Path, start: Span, allow_or: bool) -> Pattern {
        self.expect_exact(&TokenKind::LBrace);
        let mut fields = Vec::new();
        let mut has_rest = false;
        while !self.at_exact(&TokenKind::RBrace) && !self.at_exact(&TokenKind::Eof) {
            let attrs = self.parse_attrs();
            if self.at_exact(&TokenKind::DotDot) {
                self.bump();
                has_rest = true;
                break;
            }
            let f_start = self.current().span;
            let (name, shorthand_pat) = if self.at_kw(Keyword::Ref) {
                self.bump();
                let mutability = if self.eat_exact(&TokenKind::Kw(Keyword::Mut)) {
                    Mutability::Mut
                } else {
                    Mutability::Immutable
                };
                let name = self.expect_ident();
                let ident = Pattern::Ident(name, Mutability::Immutable, None, self.span_from(f_start));
                (name, Some(Pattern::RefBinding(Box::new(ident), mutability, self.span_from(f_start))))
            } else if self.at_kw(Keyword::Mut) {
                self.bump();
                let name = self.expect_ident();
                (name, Some(Pattern::Ident(name, Mutability::Mut, None, self.span_from(f_start))))
            } else {
                (self.expect_ident(), None)
            };
            let pat = if self.eat_exact(&TokenKind::Colon) {
                self.parse_pattern_with_or(allow_or)
            } else if let Some(pat) = shorthand_pat {
                pat
            } else {
                // Shorthand: `name` means `name: name`
                Pattern::Ident(name, Mutability::Immutable, None, self.span_from(f_start))
            };
            fields.push(FieldPat {
                name,
                pat,
                attrs,
                span: self.span_from(f_start),
            });
            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }
        self.expect_exact(&TokenKind::RBrace);
        Pattern::Struct(path, fields, has_rest, self.span_from(start))
    }

    fn parse_inline_asm(&mut self, start: Span) -> Expr {
        self.expect_exact(&TokenKind::LParen);

        let mut template = Vec::new();
        let mut operands = Vec::new();
        let mut options = Vec::new();

        // Parse template strings first
        while let TokenKind::StringLit(s) = &self.current().kind {
            template.push(s.clone());
            self.bump();
            if !self.eat_exact(&TokenKind::Comma) {
                self.expect_exact(&TokenKind::RParen);
                return Expr::InlineAsm(InlineAsm {
                    template, operands, options,
                    span: self.span_from(start),
                });
            }
        }

        // Parse operands and options
        while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
            // Check for options(...)
            if self.at_ident() {
                let name = self.interner.resolve(self.peek_ident()).to_string();
                if name == "options" {
                    self.bump(); // options
                    self.expect_exact(&TokenKind::LParen);
                    while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                        if self.at_ident() {
                            let opt = self.interner.resolve(self.peek_ident()).to_string();
                            self.bump();
                            options.push(opt);
                        }
                        if !self.eat_exact(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect_exact(&TokenKind::RParen);
                    if !self.eat_exact(&TokenKind::Comma) {
                        break;
                    }
                    continue;
                } else if name == "clobber_abi" {
                    self.bump(); // clobber_abi
                    self.expect_exact(&TokenKind::LParen);
                    while !self.at_exact(&TokenKind::RParen) && !self.at_exact(&TokenKind::Eof) {
                        if let TokenKind::StringLit(abi) = &self.current().kind {
                            options.push(format!("clobber_abi:{}", abi));
                            self.bump();
                        } else {
                            self.bump();
                        }
                        if !self.eat_exact(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect_exact(&TokenKind::RParen);
                    if !self.eat_exact(&TokenKind::Comma) {
                        break;
                    }
                    continue;
                }
            }

            // Parse operand: [name =] direction(reg) expr

            // Check for "name =" prefix
            if self.at_ident() {
                let saved = self.pos;
                let _ident = self.peek_ident();
                self.bump();
                if self.eat_exact(&TokenKind::Eq) {
                    // consumed "name ="
                } else {
                    self.pos = saved;
                }
            }

            // Parse direction keyword: in, out, inout, const, sym
            let dir = if self.at_kw(Keyword::In) {
                self.bump();
                "in"
            } else if self.at_ident() {
                let d = self.interner.resolve(self.peek_ident()).to_string();
                if d == "out" || d == "inout" || d == "lateout" || d == "inlateout" {
                    self.bump();
                    if d == "lateout" { "out" } else if d == "inlateout" { "inout" } else if d == "out" { "out" } else { "inout" }
                } else if d == "const" {
                    self.bump();
                    let expr = self.parse_expr();
                    operands.push(AsmOperand::Const { expr: Box::new(expr) });
                    if !self.eat_exact(&TokenKind::Comma) { break; }
                    continue;
                } else if d == "sym" {
                    self.bump();
                    let path = self.parse_path_expr();
                    operands.push(AsmOperand::Sym { path });
                    if !self.eat_exact(&TokenKind::Comma) { break; }
                    continue;
                } else {
                    panic!("unexpected asm operand direction: {}", d);
                }
            } else {
                break;
            };

            {

                // Parse register spec: (reg) or ("specific_reg")
                self.expect_exact(&TokenKind::LParen);
                let reg = if let TokenKind::StringLit(s) = &self.current().kind {
                    let r = AsmReg::Named(s.clone());
                    self.bump();
                    r
                } else if self.at_ident() {
                    let r = AsmReg::Class(self.interner.resolve(self.peek_ident()).to_string());
                    self.bump();
                    r
                } else {
                    panic!("expected register spec in asm! at {:?}", self.current().span);
                };
                self.expect_exact(&TokenKind::RParen);

                // Parse expression (for in/inout, required; for out, optional with _)
                match dir {
                    "in" => {
                        let expr = self.parse_expr();
                        operands.push(AsmOperand::In { reg, expr: Box::new(expr) });
                    }
                    "out" => {
                        if self.at_ident() && self.interner.resolve(self.peek_ident()) == "_" {
                            self.bump();
                            operands.push(AsmOperand::Out { reg, expr: None });
                        } else {
                            let expr = self.parse_expr();
                            operands.push(AsmOperand::Out { reg, expr: Some(Box::new(expr)) });
                        }
                    }
                    "inout" => {
                        let expr = self.parse_expr();
                        let out_expr = if self.eat_exact(&TokenKind::FatArrow) {
                            if self.at_ident() && self.interner.resolve(self.peek_ident()) == "_" {
                                self.bump();
                                None
                            } else {
                                Some(Box::new(self.parse_expr()))
                            }
                        } else {
                            None
                        };
                        operands.push(AsmOperand::InOut {
                            reg,
                            expr: Box::new(expr),
                            out_expr,
                        });
                    }
                    _ => unreachable!(),
                }
            }

            if !self.eat_exact(&TokenKind::Comma) {
                break;
            }
        }

        self.expect_exact(&TokenKind::RParen);
        Expr::InlineAsm(InlineAsm {
            template, operands, options,
            span: self.span_from(start),
        })
    }

    fn peek_ident(&self) -> Symbol {
        match self.current().kind {
            TokenKind::Ident(sym) => sym,
            _ => panic!("expected ident"),
        }
    }
}

fn collect_pattern_bindings(pat: &Pattern, out: &mut Vec<(Symbol, Mutability, Span)>) {
    match pat {
        Pattern::Ident(name, mutability, sub, span) => {
            out.push((*name, *mutability, *span));
            if let Some(sub) = sub {
                collect_pattern_bindings(sub, out);
            }
        }
        Pattern::Tuple(pats, _)
        | Pattern::Slice(pats, _)
        | Pattern::TupleStruct(_, pats, _)
        | Pattern::Or(pats, _) => {
            for pat in pats {
                collect_pattern_bindings(pat, out);
            }
        }
        Pattern::Struct(_, fields, _, _) => {
            for field in fields {
                collect_pattern_bindings(&field.pat, out);
            }
        }
        Pattern::Ref(inner, _, _) | Pattern::RefBinding(inner, _, _) => collect_pattern_bindings(inner, out),
        Pattern::Literal(_, _)
        | Pattern::Wildcard(_)
        | Pattern::Rest(_)
        | Pattern::Range(_, _, _, _)
        | Pattern::Path(_) => {}
    }
}

enum InfixOp {
    Binary(BinOp),
    Assign,
    AssignOp(BinOp),
    Range(bool),
}
