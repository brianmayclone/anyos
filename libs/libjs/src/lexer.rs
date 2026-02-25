//! JavaScript lexer / tokenizer.
//!
//! Converts JavaScript source text into a sequence of tokens.

use alloc::string::String;
use alloc::vec::Vec;
use crate::token::{Token, TokenKind, Span};
use crate::value::parse_js_float;

pub struct Lexer<'a> {
    src: &'a [u8],
    pos: usize,
    line: u32,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        Lexer {
            src: source.as_bytes(),
            pos: 0,
            line: 1,
        }
    }

    /// Tokenize the entire source into a Vec of tokens.
    pub fn tokenize(source: &str) -> Vec<Token> {
        let mut lexer = Lexer::new(source);
        let mut tokens = Vec::new();
        loop {
            let tok = lexer.next_token();
            let is_eof = tok.kind == TokenKind::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        tokens
    }

    fn peek(&self) -> u8 {
        if self.pos < self.src.len() {
            self.src[self.pos]
        } else {
            0
        }
    }

    fn peek2(&self) -> u8 {
        if self.pos + 1 < self.src.len() {
            self.src[self.pos + 1]
        } else {
            0
        }
    }

    fn advance(&mut self) -> u8 {
        let ch = self.peek();
        if ch == b'\n' {
            self.line += 1;
        }
        self.pos += 1;
        ch
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            // Whitespace
            while self.pos < self.src.len() {
                let ch = self.src[self.pos];
                if ch == b' ' || ch == b'\t' || ch == b'\r' || ch == b'\n' {
                    if ch == b'\n' {
                        self.line += 1;
                    }
                    self.pos += 1;
                } else {
                    break;
                }
            }

            // Single-line comment
            if self.pos + 1 < self.src.len()
                && self.src[self.pos] == b'/'
                && self.src[self.pos + 1] == b'/'
            {
                self.pos += 2;
                while self.pos < self.src.len() && self.src[self.pos] != b'\n' {
                    self.pos += 1;
                }
                continue;
            }

            // Multi-line comment
            if self.pos + 1 < self.src.len()
                && self.src[self.pos] == b'/'
                && self.src[self.pos + 1] == b'*'
            {
                self.pos += 2;
                while self.pos + 1 < self.src.len() {
                    if self.src[self.pos] == b'\n' {
                        self.line += 1;
                    }
                    if self.src[self.pos] == b'*' && self.src[self.pos + 1] == b'/' {
                        self.pos += 2;
                        break;
                    }
                    self.pos += 1;
                }
                continue;
            }

            break;
        }
    }

    pub fn next_token(&mut self) -> Token {
        self.skip_whitespace_and_comments();

        let start = self.pos as u32;
        let line = self.line;

        if self.pos >= self.src.len() {
            return Token {
                kind: TokenKind::Eof,
                span: Span { start, end: start, line },
            };
        }

        let ch = self.src[self.pos];

        // String literals
        if ch == b'"' || ch == b'\'' {
            return self.read_string(ch);
        }

        // Template literals
        if ch == b'`' {
            return self.read_template();
        }

        // Numbers
        if ch.is_ascii_digit() || (ch == b'.' && self.peek2().is_ascii_digit()) {
            return self.read_number();
        }

        // Identifiers and keywords
        if is_ident_start(ch) {
            return self.read_ident();
        }

        // Punctuation and operators
        self.read_punct()
    }

    fn read_string(&mut self, quote: u8) -> Token {
        let start = self.pos as u32;
        let line = self.line;
        self.pos += 1; // skip opening quote
        let mut s = String::new();

        while self.pos < self.src.len() && self.src[self.pos] != quote {
            if self.src[self.pos] == b'\\' {
                self.pos += 1;
                if self.pos >= self.src.len() {
                    break;
                }
                match self.src[self.pos] {
                    b'n' => { s.push('\n'); self.pos += 1; }
                    b'r' => { s.push('\r'); self.pos += 1; }
                    b't' => { s.push('\t'); self.pos += 1; }
                    b'\\' => { s.push('\\'); self.pos += 1; }
                    b'\'' => { s.push('\''); self.pos += 1; }
                    b'"' => { s.push('"'); self.pos += 1; }
                    b'`' => { s.push('`'); self.pos += 1; }
                    b'0' => { s.push('\0'); self.pos += 1; }
                    b'b' => { s.push('\u{0008}'); self.pos += 1; }
                    b'f' => { s.push('\u{000C}'); self.pos += 1; }
                    b'v' => { s.push('\u{000B}'); self.pos += 1; }
                    b'\n' => { self.line += 1; self.pos += 1; }
                    b'u' => {
                        self.pos += 1;
                        let ch = self.read_unicode_escape();
                        s.push(ch);
                    }
                    b'x' => {
                        self.pos += 1;
                        let ch = self.read_hex_escape();
                        s.push(ch);
                    }
                    other => {
                        s.push(other as char);
                        self.pos += 1;
                    }
                }
            } else {
                if self.src[self.pos] == b'\n' {
                    self.line += 1;
                }
                // UTF-8 safe: read full character
                let ch = self.read_utf8_char();
                s.push(ch);
            }
        }

        if self.pos < self.src.len() {
            self.pos += 1; // skip closing quote
        }

        Token {
            kind: TokenKind::String(s),
            span: Span { start, end: self.pos as u32, line },
        }
    }

    fn read_template(&mut self) -> Token {
        let start = self.pos as u32;
        let line = self.line;
        self.pos += 1; // skip `
        let mut s = String::new();

        while self.pos < self.src.len() && self.src[self.pos] != b'`' {
            if self.src[self.pos] == b'\\' {
                self.pos += 1;
                if self.pos < self.src.len() {
                    match self.src[self.pos] {
                        b'n' => { s.push('\n'); self.pos += 1; }
                        b'r' => { s.push('\r'); self.pos += 1; }
                        b't' => { s.push('\t'); self.pos += 1; }
                        b'\\' => { s.push('\\'); self.pos += 1; }
                        b'`' => { s.push('`'); self.pos += 1; }
                        b'$' => { s.push('$'); self.pos += 1; }
                        other => { s.push(other as char); self.pos += 1; }
                    }
                }
            } else if self.src[self.pos] == b'$'
                && self.pos + 1 < self.src.len()
                && self.src[self.pos + 1] == b'{'
            {
                // Template expression — for now just include as literal
                // A full implementation would tokenize expressions separately
                s.push('$');
                s.push('{');
                self.pos += 2;
                let mut depth = 1u32;
                while self.pos < self.src.len() && depth > 0 {
                    match self.src[self.pos] {
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        b'\n' => self.line += 1,
                        _ => {}
                    }
                    if depth > 0 {
                        let ch = self.read_utf8_char();
                        s.push(ch);
                    } else {
                        s.push('}');
                        self.pos += 1;
                    }
                }
            } else {
                if self.src[self.pos] == b'\n' {
                    self.line += 1;
                }
                let ch = self.read_utf8_char();
                s.push(ch);
            }
        }

        if self.pos < self.src.len() {
            self.pos += 1; // skip closing `
        }

        Token {
            kind: TokenKind::Template(s),
            span: Span { start, end: self.pos as u32, line },
        }
    }

    fn read_number(&mut self) -> Token {
        let start = self.pos as u32;
        let line = self.line;
        let mut s = String::new();

        // Hex, octal, binary prefix
        if self.peek() == b'0' {
            let next = self.peek2();
            if next == b'x' || next == b'X' {
                s.push('0');
                s.push('x');
                self.pos += 2;
                while self.pos < self.src.len()
                    && (self.src[self.pos].is_ascii_hexdigit() || self.src[self.pos] == b'_')
                {
                    if self.src[self.pos] != b'_' {
                        s.push(self.src[self.pos] as char);
                    }
                    self.pos += 1;
                }
                let val = parse_js_float(&s);
                return Token {
                    kind: TokenKind::Number(val),
                    span: Span { start, end: self.pos as u32, line },
                };
            }
            if next == b'o' || next == b'O' {
                self.pos += 2;
                let mut val: f64 = 0.0;
                while self.pos < self.src.len()
                    && (self.src[self.pos] >= b'0' && self.src[self.pos] <= b'7'
                        || self.src[self.pos] == b'_')
                {
                    if self.src[self.pos] != b'_' {
                        val = val * 8.0 + (self.src[self.pos] - b'0') as f64;
                    }
                    self.pos += 1;
                }
                return Token {
                    kind: TokenKind::Number(val),
                    span: Span { start, end: self.pos as u32, line },
                };
            }
            if next == b'b' || next == b'B' {
                self.pos += 2;
                let mut val: f64 = 0.0;
                while self.pos < self.src.len()
                    && (self.src[self.pos] == b'0' || self.src[self.pos] == b'1'
                        || self.src[self.pos] == b'_')
                {
                    if self.src[self.pos] != b'_' {
                        val = val * 2.0 + (self.src[self.pos] - b'0') as f64;
                    }
                    self.pos += 1;
                }
                return Token {
                    kind: TokenKind::Number(val),
                    span: Span { start, end: self.pos as u32, line },
                };
            }
        }

        // Decimal number
        while self.pos < self.src.len()
            && (self.src[self.pos].is_ascii_digit() || self.src[self.pos] == b'_')
        {
            if self.src[self.pos] != b'_' {
                s.push(self.src[self.pos] as char);
            }
            self.pos += 1;
        }

        // Fractional part
        if self.pos < self.src.len() && self.src[self.pos] == b'.' {
            s.push('.');
            self.pos += 1;
            while self.pos < self.src.len()
                && (self.src[self.pos].is_ascii_digit() || self.src[self.pos] == b'_')
            {
                if self.src[self.pos] != b'_' {
                    s.push(self.src[self.pos] as char);
                }
                self.pos += 1;
            }
        }

        // Exponent
        if self.pos < self.src.len() && (self.src[self.pos] == b'e' || self.src[self.pos] == b'E')
        {
            s.push('e');
            self.pos += 1;
            if self.pos < self.src.len()
                && (self.src[self.pos] == b'+' || self.src[self.pos] == b'-')
            {
                s.push(self.src[self.pos] as char);
                self.pos += 1;
            }
            while self.pos < self.src.len() && self.src[self.pos].is_ascii_digit() {
                s.push(self.src[self.pos] as char);
                self.pos += 1;
            }
        }

        let val = parse_js_float(&s);
        Token {
            kind: TokenKind::Number(val),
            span: Span { start, end: self.pos as u32, line },
        }
    }

    fn read_ident(&mut self) -> Token {
        let start = self.pos as u32;
        let line = self.line;
        let mut name = String::new();

        while self.pos < self.src.len() && is_ident_continue(self.src[self.pos]) {
            name.push(self.src[self.pos] as char);
            self.pos += 1;
        }

        let kind = match name.as_str() {
            "var" => TokenKind::Var,
            "let" => TokenKind::Let,
            "const" => TokenKind::Const,
            "function" => TokenKind::Function,
            "return" => TokenKind::Return,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "do" => TokenKind::Do,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "switch" => TokenKind::Switch,
            "case" => TokenKind::Case,
            "default" => TokenKind::Default,
            "new" => TokenKind::New,
            "delete" => TokenKind::Delete,
            "typeof" => TokenKind::Typeof,
            "void" => TokenKind::Void,
            "in" => TokenKind::In,
            "instanceof" => TokenKind::Instanceof,
            "this" => TokenKind::This,
            "true" => TokenKind::Bool(true),
            "false" => TokenKind::Bool(false),
            "null" => TokenKind::Null,
            "undefined" => TokenKind::Undefined,
            "class" => TokenKind::Class,
            "extends" => TokenKind::Extends,
            "super" => TokenKind::Super,
            "import" => TokenKind::Import,
            "export" => TokenKind::Export,
            "from" => TokenKind::From,
            "as" => TokenKind::As,
            "try" => TokenKind::Try,
            "catch" => TokenKind::Catch,
            "finally" => TokenKind::Finally,
            "throw" => TokenKind::Throw,
            "yield" => TokenKind::Yield,
            "async" => TokenKind::Async,
            "await" => TokenKind::Await,
            "of" => TokenKind::Of,
            "with" => TokenKind::With,
            "debugger" => TokenKind::Debugger,
            _ => TokenKind::Ident(name),
        };

        Token {
            kind,
            span: Span { start, end: self.pos as u32, line },
        }
    }

    fn read_punct(&mut self) -> Token {
        let start = self.pos as u32;
        let line = self.line;
        let ch = self.advance();

        let kind = match ch {
            b'(' => TokenKind::LParen,
            b')' => TokenKind::RParen,
            b'{' => TokenKind::LBrace,
            b'}' => TokenKind::RBrace,
            b'[' => TokenKind::LBracket,
            b']' => TokenKind::RBracket,
            b';' => TokenKind::Semicolon,
            b',' => TokenKind::Comma,
            b':' => TokenKind::Colon,
            b'~' => TokenKind::Tilde,
            b'.' => {
                if self.peek() == b'.' && self.peek2() == b'.' {
                    self.pos += 2;
                    TokenKind::DotDotDot
                } else {
                    TokenKind::Dot
                }
            }
            b'?' => {
                if self.peek() == b'.' {
                    self.pos += 1;
                    TokenKind::QuestionDot
                } else if self.peek() == b'?' {
                    self.pos += 1;
                    if self.peek() == b'=' {
                        self.pos += 1;
                        TokenKind::QuestionQuestionEq
                    } else {
                        TokenKind::QuestionQuestion
                    }
                } else {
                    TokenKind::Question
                }
            }
            b'+' => {
                if self.peek() == b'+' {
                    self.pos += 1;
                    TokenKind::PlusPlus
                } else if self.peek() == b'=' {
                    self.pos += 1;
                    TokenKind::PlusEq
                } else {
                    TokenKind::Plus
                }
            }
            b'-' => {
                if self.peek() == b'-' {
                    self.pos += 1;
                    TokenKind::MinusMinus
                } else if self.peek() == b'=' {
                    self.pos += 1;
                    TokenKind::MinusEq
                } else {
                    TokenKind::Minus
                }
            }
            b'*' => {
                if self.peek() == b'*' {
                    self.pos += 1;
                    if self.peek() == b'=' {
                        self.pos += 1;
                        TokenKind::StarStarEq
                    } else {
                        TokenKind::StarStar
                    }
                } else if self.peek() == b'=' {
                    self.pos += 1;
                    TokenKind::StarEq
                } else {
                    TokenKind::Star
                }
            }
            b'/' => {
                if self.peek() == b'=' {
                    self.pos += 1;
                    TokenKind::SlashEq
                } else {
                    TokenKind::Slash
                }
            }
            b'%' => {
                if self.peek() == b'=' {
                    self.pos += 1;
                    TokenKind::PercentEq
                } else {
                    TokenKind::Percent
                }
            }
            b'=' => {
                if self.peek() == b'=' {
                    self.pos += 1;
                    if self.peek() == b'=' {
                        self.pos += 1;
                        TokenKind::EqEqEq
                    } else {
                        TokenKind::EqEq
                    }
                } else if self.peek() == b'>' {
                    self.pos += 1;
                    TokenKind::Arrow
                } else {
                    TokenKind::Eq
                }
            }
            b'!' => {
                if self.peek() == b'=' {
                    self.pos += 1;
                    if self.peek() == b'=' {
                        self.pos += 1;
                        TokenKind::BangEqEq
                    } else {
                        TokenKind::BangEq
                    }
                } else {
                    TokenKind::Bang
                }
            }
            b'<' => {
                if self.peek() == b'<' {
                    self.pos += 1;
                    if self.peek() == b'=' {
                        self.pos += 1;
                        TokenKind::LtLtEq
                    } else {
                        TokenKind::LtLt
                    }
                } else if self.peek() == b'=' {
                    self.pos += 1;
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            b'>' => {
                if self.peek() == b'>' {
                    self.pos += 1;
                    if self.peek() == b'>' {
                        self.pos += 1;
                        if self.peek() == b'=' {
                            self.pos += 1;
                            TokenKind::GtGtGtEq
                        } else {
                            TokenKind::GtGtGt
                        }
                    } else if self.peek() == b'=' {
                        self.pos += 1;
                        TokenKind::GtGtEq
                    } else {
                        TokenKind::GtGt
                    }
                } else if self.peek() == b'=' {
                    self.pos += 1;
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            b'&' => {
                if self.peek() == b'&' {
                    self.pos += 1;
                    if self.peek() == b'=' {
                        self.pos += 1;
                        TokenKind::AmpAmpEq
                    } else {
                        TokenKind::AmpAmp
                    }
                } else if self.peek() == b'=' {
                    self.pos += 1;
                    TokenKind::AmpEq
                } else {
                    TokenKind::Amp
                }
            }
            b'|' => {
                if self.peek() == b'|' {
                    self.pos += 1;
                    if self.peek() == b'=' {
                        self.pos += 1;
                        TokenKind::PipePipeEq
                    } else {
                        TokenKind::PipePipe
                    }
                } else if self.peek() == b'=' {
                    self.pos += 1;
                    TokenKind::PipeEq
                } else {
                    TokenKind::Pipe
                }
            }
            b'^' => {
                if self.peek() == b'=' {
                    self.pos += 1;
                    TokenKind::CaretEq
                } else {
                    TokenKind::Caret
                }
            }
            _ => {
                // Unknown character — emit as EOF to avoid infinite loop
                TokenKind::Eof
            }
        };

        Token {
            kind,
            span: Span { start, end: self.pos as u32, line },
        }
    }

    fn read_unicode_escape(&mut self) -> char {
        if self.pos < self.src.len() && self.src[self.pos] == b'{' {
            // \u{XXXX}
            self.pos += 1;
            let mut val: u32 = 0;
            while self.pos < self.src.len() && self.src[self.pos] != b'}' {
                val = val * 16 + hex_digit(self.src[self.pos]) as u32;
                self.pos += 1;
            }
            if self.pos < self.src.len() {
                self.pos += 1; // skip }
            }
            char::from_u32(val).unwrap_or('\u{FFFD}')
        } else {
            // \uXXXX (exactly 4 hex digits)
            let mut val: u32 = 0;
            for _ in 0..4 {
                if self.pos < self.src.len() {
                    val = val * 16 + hex_digit(self.src[self.pos]) as u32;
                    self.pos += 1;
                }
            }
            char::from_u32(val).unwrap_or('\u{FFFD}')
        }
    }

    fn read_hex_escape(&mut self) -> char {
        let mut val: u32 = 0;
        for _ in 0..2 {
            if self.pos < self.src.len() {
                val = val * 16 + hex_digit(self.src[self.pos]) as u32;
                self.pos += 1;
            }
        }
        char::from_u32(val).unwrap_or('\u{FFFD}')
    }

    fn read_utf8_char(&mut self) -> char {
        let b0 = self.src[self.pos];
        if b0 < 0x80 {
            self.pos += 1;
            return b0 as char;
        }
        let len = if b0 < 0xE0 {
            2
        } else if b0 < 0xF0 {
            3
        } else {
            4
        };
        let end = (self.pos + len).min(self.src.len());
        let s = core::str::from_utf8(&self.src[self.pos..end]).unwrap_or("\u{FFFD}");
        let ch = s.chars().next().unwrap_or('\u{FFFD}');
        self.pos = end;
        ch
    }
}

fn is_ident_start(ch: u8) -> bool {
    ch.is_ascii_alphabetic() || ch == b'_' || ch == b'$'
}

fn is_ident_continue(ch: u8) -> bool {
    ch.is_ascii_alphanumeric() || ch == b'_' || ch == b'$'
}

fn hex_digit(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}
