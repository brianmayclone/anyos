use crate::prelude::*;
use crate::diagnostics::Span;
use crate::intern::{Interner, Symbol};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Fn, Let, Mut, Pub, Struct, Enum, Impl, Trait, Type,
    Use, Mod, Crate, SelfValue, SelfType, Super, As, In, For, While,
    Loop, If, Else, Match, Return, Break, Continue,
    Where, Const, Static, Unsafe, Extern, Ref, Move,
    True, False, Dyn,
}

fn keyword_from_str(s: &str) -> Option<Keyword> {
    match s {
        "fn" => Some(Keyword::Fn),
        "let" => Some(Keyword::Let),
        "mut" => Some(Keyword::Mut),
        "pub" => Some(Keyword::Pub),
        "struct" => Some(Keyword::Struct),
        "enum" => Some(Keyword::Enum),
        "impl" => Some(Keyword::Impl),
        "trait" => Some(Keyword::Trait),
        "type" => Some(Keyword::Type),
        "use" => Some(Keyword::Use),
        "mod" => Some(Keyword::Mod),
        "crate" => Some(Keyword::Crate),
        "self" => Some(Keyword::SelfValue),
        "Self" => Some(Keyword::SelfType),
        "super" => Some(Keyword::Super),
        "as" => Some(Keyword::As),
        "in" => Some(Keyword::In),
        "for" => Some(Keyword::For),
        "while" => Some(Keyword::While),
        "loop" => Some(Keyword::Loop),
        "if" => Some(Keyword::If),
        "else" => Some(Keyword::Else),
        "match" => Some(Keyword::Match),
        "return" => Some(Keyword::Return),
        "break" => Some(Keyword::Break),
        "continue" => Some(Keyword::Continue),
        "where" => Some(Keyword::Where),
        "const" => Some(Keyword::Const),
        "static" => Some(Keyword::Static),
        "unsafe" => Some(Keyword::Unsafe),
        "extern" => Some(Keyword::Extern),
        "ref" => Some(Keyword::Ref),
        "move" => Some(Keyword::Move),
        "true" => Some(Keyword::True),
        "false" => Some(Keyword::False),
        "dyn" => Some(Keyword::Dyn),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum IntSuffix {
    I8,
    I16,
    I32,
    I64,
    I128,
    Isize,
    U8,
    U16,
    U32,
    U64,
    U128,
    Usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    IntLit(u128, Option<IntSuffix>),
    FloatLit(f64),
    StringLit(String),
    CharLit(char),
    ByteStringLit(Vec<u8>),
    // Identifiers
    Ident(Symbol),
    Lifetime(Symbol),
    // Keywords
    Kw(Keyword),
    // Operators
    Plus, Minus, Star, Slash, Percent,
    Amp, Pipe, Caret, Tilde, Not,
    Eq, EqEq, Ne, Lt, Le, Gt, Ge,
    AndAnd, OrOr,
    Shl, Shr,
    PlusEq, MinusEq, StarEq, SlashEq, PercentEq,
    AmpEq, PipeEq, CaretEq, ShlEq, ShrEq,
    // Punctuation
    Arrow, FatArrow, ColonColon, DotDot, DotDotEq,
    LParen, RParen, LBrace, RBrace, LBracket, RBracket,
    Semi, Colon, Comma, Dot, At, Hash, Question, Dollar,
    // Special
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    interner: &'a mut Interner,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, interner: &'a mut Interner) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
            interner,
        }
    }

    fn peek(&self) -> u8 {
        if self.pos < self.src.len() { self.src[self.pos] } else { 0 }
    }

    fn peek_at(&self, offset: usize) -> u8 {
        let i = self.pos + offset;
        if i < self.src.len() { self.src[i] } else { 0 }
    }

    fn advance(&mut self) -> u8 {
        let b = self.peek();
        self.pos += 1;
        b
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // whitespace
            while self.pos < self.src.len() && matches!(self.peek(), b' ' | b'\t' | b'\n' | b'\r') {
                self.pos += 1;
            }
            // line comment
            if self.peek() == b'/' && self.peek_at(1) == b'/' {
                self.pos += 2;
                while self.pos < self.src.len() && self.peek() != b'\n' {
                    self.pos += 1;
                }
                continue;
            }
            // block comment (nested)
            if self.peek() == b'/' && self.peek_at(1) == b'*' {
                self.pos += 2;
                let mut depth = 1u32;
                while self.pos < self.src.len() && depth > 0 {
                    if self.peek() == b'/' && self.peek_at(1) == b'*' {
                        self.pos += 2;
                        depth += 1;
                    } else if self.peek() == b'*' && self.peek_at(1) == b'/' {
                        self.pos += 2;
                        depth -= 1;
                    } else {
                        self.pos += 1;
                    }
                }
                continue;
            }
            break;
        }
    }

    fn lex_string(&mut self) -> String {
        self.pos += 1; // skip opening "
        let mut s = String::new();
        loop {
            if self.pos >= self.src.len() { break; }
            let b = self.advance();
            match b {
                b'"' => break,
                b'\\' => s.push(self.lex_char_escape()),
                _ => s.push(b as char),
            }
        }
        s
    }

    fn lex_char_escape(&mut self) -> char {
        let b = self.advance();
        match b {
            b'n' => '\n',
            b't' => '\t',
            b'r' => '\r',
            b'\\' => '\\',
            b'"' => '"',
            b'\'' => '\'',
            b'0' => '\0',
            b'x' => {
                let hi = self.advance();
                let lo = self.advance();
                let val = (hex_val(hi) << 4) | hex_val(lo);
                val as u8 as char
            }
            b'u' => {
                self.advance(); // {
                let mut val = 0u32;
                loop {
                    let c = self.peek();
                    if c == b'}' { self.pos += 1; break; }
                    val = val * 16 + hex_val(self.advance()) as u32;
                }
                char::from_u32(val).unwrap_or('\u{FFFD}')
            }
            _ => b as char,
        }
    }

    fn lex_raw_string(&mut self) -> String {
        // pos is after 'r', count '#'s
        let mut hashes = 0usize;
        while self.peek() == b'#' {
            self.pos += 1;
            hashes += 1;
        }
        self.pos += 1; // skip "
        let mut s = String::new();
        'outer: loop {
            if self.pos >= self.src.len() { break; }
            let b = self.advance();
            if b == b'"' {
                // check for matching hashes
                let mut count = 0;
                while count < hashes && self.peek() == b'#' {
                    self.pos += 1;
                    count += 1;
                }
                if count == hashes {
                    break 'outer;
                }
                s.push('"');
                for _ in 0..count { s.push('#'); }
            } else {
                s.push(b as char);
            }
        }
        s
    }

    fn lex_byte_string(&mut self) -> Vec<u8> {
        self.pos += 1; // skip "
        let mut v = Vec::new();
        loop {
            if self.pos >= self.src.len() { break; }
            let b = self.advance();
            match b {
                b'"' => break,
                b'\\' => v.push(self.lex_char_escape() as u8),
                _ => v.push(b),
            }
        }
        v
    }

    fn lex_byte_char(&mut self) -> u8 {
        self.pos += 1; // skip opening '
        if self.pos >= self.src.len() {
            return 0;
        }
        let value = if self.peek() == b'\\' {
            self.pos += 1; // skip backslash
            self.lex_char_escape() as u8
        } else {
            self.advance()
        };
        if self.peek() == b'\'' {
            self.pos += 1; // skip closing '
        }
        value
    }

    fn lex_number(&mut self, start: usize) -> TokenKind {
        let first = self.src[start];
        if first == b'0' && self.pos < self.src.len() {
            match self.peek() {
                b'x' | b'X' => {
                    self.pos += 1;
                    return self.lex_int_radix(16);
                }
                b'b' | b'B' => {
                    self.pos += 1;
                    return self.lex_int_radix(2);
                }
                b'o' | b'O' => {
                    self.pos += 1;
                    return self.lex_int_radix(8);
                }
                _ => {}
            }
        }
        // decimal digits
        self.eat_decimal_digits();
        // check for dot (but not .. or ..= or method call on non-digit)
        if self.peek() == b'.' && self.peek_at(1) != b'.' && self.peek_at(1).is_ascii_digit() {
            self.pos += 1; // skip .
            self.eat_decimal_digits();
            self.eat_exponent();
            self.eat_float_suffix();
            let text = self.num_text(start);
            return TokenKind::FloatLit(parse_float(&text));
        }
        // check for exponent without dot
        if self.peek() == b'e' || self.peek() == b'E' {
            let next = self.peek_at(1);
            if next.is_ascii_digit() || next == b'+' || next == b'-' {
                self.eat_exponent();
                self.eat_float_suffix();
                let text = self.num_text(start);
                return TokenKind::FloatLit(parse_float(&text));
            }
        }
        // integer - skip suffix
        let suffix = self.eat_int_suffix();
        let text = self.num_text(start);
        TokenKind::IntLit(parse_int_dec(&text), suffix)
    }

    fn lex_int_radix(&mut self, radix: u32) -> TokenKind {
        let mut val: u128 = 0;
        loop {
            let b = self.peek();
            if b == b'_' { self.pos += 1; continue; }
            let d = match b {
                b'0'..=b'9' => (b - b'0') as u128,
                b'a'..=b'f' => (b - b'a' + 10) as u128,
                b'A'..=b'F' => (b - b'A' + 10) as u128,
                _ => break,
            };
            if d >= radix as u128 { break; }
            val = val * radix as u128 + d;
            self.pos += 1;
        }
        let suffix = self.eat_int_suffix();
        TokenKind::IntLit(val, suffix)
    }

    fn eat_decimal_digits(&mut self) {
        while self.pos < self.src.len() {
            match self.peek() {
                b'0'..=b'9' | b'_' => { self.pos += 1; }
                _ => break,
            }
        }
    }

    fn eat_exponent(&mut self) {
        if self.peek() == b'e' || self.peek() == b'E' {
            self.pos += 1;
            if self.peek() == b'+' || self.peek() == b'-' {
                self.pos += 1;
            }
            self.eat_decimal_digits();
        }
    }

    fn eat_float_suffix(&mut self) {
        if self.peek() == b'f' {
            let n = self.peek_at(1);
            if n == b'3' || n == b'6' { self.pos += 3; }
        }
    }

    fn eat_int_suffix(&mut self) -> Option<IntSuffix> {
        let start = self.pos;
        let b = self.peek();
        if b == b'u' || b == b'i' {
            let next = self.peek_at(1);
            if next.is_ascii_digit() || next.is_ascii_alphabetic() {
                self.pos += 1;
                while self.pos < self.src.len() && self.peek().is_ascii_alphanumeric() {
                    self.pos += 1;
                }
                let suffix = core::str::from_utf8(&self.src[start..self.pos]).ok();
                return match suffix {
                    Some("i8") => Some(IntSuffix::I8),
                    Some("i16") => Some(IntSuffix::I16),
                    Some("i32") => Some(IntSuffix::I32),
                    Some("i64") => Some(IntSuffix::I64),
                    Some("i128") => Some(IntSuffix::I128),
                    Some("isize") => Some(IntSuffix::Isize),
                    Some("u8") => Some(IntSuffix::U8),
                    Some("u16") => Some(IntSuffix::U16),
                    Some("u32") => Some(IntSuffix::U32),
                    Some("u64") => Some(IntSuffix::U64),
                    Some("u128") => Some(IntSuffix::U128),
                    Some("usize") => Some(IntSuffix::Usize),
                    _ => None,
                };
            }
        }
        None
    }

    fn num_text(&self, start: usize) -> String {
        let raw = &self.src[start..self.pos];
        let s: String = raw.iter().filter(|&&b| b != b'_').map(|&b| b as char).collect();
        // strip suffix
        if let Some(idx) = s.rfind(|c: char| !c.is_ascii_alphabetic()) {
            s[..=idx].to_string()
        } else {
            s
        }
    }

    fn is_ident_start(b: u8) -> bool {
        b.is_ascii_alphabetic() || b == b'_'
    }

    fn is_ident_cont(b: u8) -> bool {
        b.is_ascii_alphanumeric() || b == b'_'
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace_and_comments();
        let start = self.pos;
        if self.pos >= self.src.len() {
            return Token { kind: TokenKind::Eof, span: Span::new(start as u32, start as u32) };
        }

        let b = self.advance();
        let kind = match b {
            // Identifiers and keywords
            _ if Self::is_ident_start(b) => {
                if b == b'r' && self.peek() == b'#' && Self::is_ident_start(self.peek_at(1)) {
                    self.pos += 1; // skip '#'
                    let ident_start = self.pos;
                    while self.pos < self.src.len() && Self::is_ident_cont(self.peek()) {
                        self.pos += 1;
                    }
                    let text = core::str::from_utf8(&self.src[ident_start..self.pos]).unwrap();
                    let sym = self.interner.intern(text);
                    return Token {
                        kind: TokenKind::Ident(sym),
                        span: Span::new(start as u32, self.pos as u32),
                    };
                }
                // check for raw string r"..." or r#"..."#
                if b == b'r' && (self.peek() == b'"' || self.peek() == b'#') {
                    let s = self.lex_raw_string();
                    TokenKind::StringLit(s)
                } else if b == b'b' {
                    if self.peek() == b'"' {
                        let v = self.lex_byte_string();
                        TokenKind::ByteStringLit(v)
                    } else if self.peek() == b'\'' {
                        TokenKind::IntLit(self.lex_byte_char() as u128, Some(IntSuffix::U8))
                    } else {
                        while self.pos < self.src.len() && Self::is_ident_cont(self.peek()) {
                            self.pos += 1;
                        }
                        let text = core::str::from_utf8(&self.src[start..self.pos]).unwrap();
                        if let Some(kw) = keyword_from_str(text) {
                            TokenKind::Kw(kw)
                        } else {
                            let sym = self.interner.intern(text);
                            TokenKind::Ident(sym)
                        }
                    }
                } else {
                    while self.pos < self.src.len() && Self::is_ident_cont(self.peek()) {
                        self.pos += 1;
                    }
                    let text = core::str::from_utf8(&self.src[start..self.pos]).unwrap();
                    if let Some(kw) = keyword_from_str(text) {
                        TokenKind::Kw(kw)
                    } else {
                        let sym = self.interner.intern(text);
                        TokenKind::Ident(sym)
                    }
                }
            }

            // Numbers
            b'0'..=b'9' => self.lex_number(start),

            // String
            b'"' => {
                self.pos -= 1;
                let s = self.lex_string();
                TokenKind::StringLit(s)
            }

            // Char literal or lifetime
            b'\'' => {
                // Try char literal: 'X' or '\...'
                if self.peek() == b'\\' {
                    // escaped char literal
                    self.pos += 1; // skip backslash
                    let ch = self.lex_char_escape();
                    self.pos += 1; // skip closing '
                    TokenKind::CharLit(ch)
                } else if self.pos + 1 < self.src.len() && self.src[self.pos + 1] == b'\'' {
                    // simple char literal 'X'
                    let ch = self.advance() as char;
                    self.pos += 1; // skip closing '
                    TokenKind::CharLit(ch)
                } else if Self::is_ident_start(self.peek()) {
                    // lifetime
                    let lt_start = self.pos;
                    while self.pos < self.src.len() && Self::is_ident_cont(self.peek()) {
                        self.pos += 1;
                    }
                    let text = core::str::from_utf8(&self.src[lt_start..self.pos]).unwrap();
                    let sym = self.interner.intern(text);
                    TokenKind::Lifetime(sym)
                } else {
                    // single quote - treat as unknown, but shouldn't happen in valid Rust
                    TokenKind::Question // placeholder
                }
            }

            // Operators and punctuation
            b'+' => if self.peek() == b'=' { self.pos += 1; TokenKind::PlusEq } else { TokenKind::Plus },
            b'-' => match self.peek() {
                b'=' => { self.pos += 1; TokenKind::MinusEq }
                b'>' => { self.pos += 1; TokenKind::Arrow }
                _ => TokenKind::Minus,
            },
            b'*' => if self.peek() == b'=' { self.pos += 1; TokenKind::StarEq } else { TokenKind::Star },
            b'/' => if self.peek() == b'=' { self.pos += 1; TokenKind::SlashEq } else { TokenKind::Slash },
            b'%' => if self.peek() == b'=' { self.pos += 1; TokenKind::PercentEq } else { TokenKind::Percent },
            b'&' => match self.peek() {
                b'&' => { self.pos += 1; TokenKind::AndAnd }
                b'=' => { self.pos += 1; TokenKind::AmpEq }
                _ => TokenKind::Amp,
            },
            b'|' => match self.peek() {
                b'|' => { self.pos += 1; TokenKind::OrOr }
                b'=' => { self.pos += 1; TokenKind::PipeEq }
                _ => TokenKind::Pipe,
            },
            b'^' => if self.peek() == b'=' { self.pos += 1; TokenKind::CaretEq } else { TokenKind::Caret },
            b'~' => TokenKind::Tilde,
            b'!' => if self.peek() == b'=' { self.pos += 1; TokenKind::Ne } else { TokenKind::Not },
            b'=' => match self.peek() {
                b'=' => { self.pos += 1; TokenKind::EqEq }
                b'>' => { self.pos += 1; TokenKind::FatArrow }
                _ => TokenKind::Eq,
            },
            b'<' => match self.peek() {
                b'=' => { self.pos += 1; TokenKind::Le }
                b'<' => {
                    self.pos += 1;
                    if self.peek() == b'=' { self.pos += 1; TokenKind::ShlEq } else { TokenKind::Shl }
                }
                _ => TokenKind::Lt,
            },
            b'>' => match self.peek() {
                b'=' => { self.pos += 1; TokenKind::Ge }
                b'>' => {
                    self.pos += 1;
                    if self.peek() == b'=' { self.pos += 1; TokenKind::ShrEq } else { TokenKind::Shr }
                }
                _ => TokenKind::Gt,
            },
            b'.' => {
                if self.peek() == b'.' {
                    self.pos += 1;
                    if self.peek() == b'=' { self.pos += 1; TokenKind::DotDotEq } else { TokenKind::DotDot }
                } else {
                    TokenKind::Dot
                }
            }
            b':' => if self.peek() == b':' { self.pos += 1; TokenKind::ColonColon } else { TokenKind::Colon },
            b';' => TokenKind::Semi,
            b',' => TokenKind::Comma,
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'{' => TokenKind::LBrace,
            b'}' => TokenKind::RBrace,
            b'[' => TokenKind::LBracket,
            b']' => TokenKind::RBracket,
            b'@' => TokenKind::At,
            b'#' => TokenKind::Hash,
            b'?' => TokenKind::Question,
            b'$' => TokenKind::Dollar,

            _ => {
                // Unknown byte, skip
                TokenKind::Eof
            }
        };

        Token { kind, span: Span::new(start as u32, self.pos as u32) }
    }
}

fn hex_val(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

fn parse_float(s: &str) -> f64 {
    // strip suffix like f32/f64
    let s = s.trim_end_matches("f32").trim_end_matches("f64");
    s.parse().unwrap_or(0.0)
}

fn parse_int_dec(s: &str) -> u128 {
    s.parse().unwrap_or(0)
}
