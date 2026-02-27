//! GLSL ES 1.00 lexer (tokenizer).
//!
//! Converts GLSL source text into a flat vector of [`Token`]s. Handles keywords,
//! identifiers, numeric literals, operators, and punctuation.

use alloc::string::String;
use alloc::vec::Vec;

/// Token types produced by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // ── Keywords ────────────────────────────────────────────────────────
    Attribute,
    Varying,
    Uniform,
    Const,
    Void,
    Float,
    Int,
    Bool,
    Vec2,
    Vec3,
    Vec4,
    Mat3,
    Mat4,
    Sampler2D,
    Precision,
    LowP,
    MediumP,
    HighP,
    If,
    Else,
    For,
    While,
    Return,
    Discard,
    Struct,
    In,
    Out,

    // ── Literals ────────────────────────────────────────────────────────
    IntLiteral(i32),
    FloatLiteral(f32),
    BoolLiteral(bool),

    // ── Identifier ──────────────────────────────────────────────────────
    Ident(String),

    // ── Operators ───────────────────────────────────────────────────────
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    EqEq,
    NotEq,
    Less,
    LessEq,
    Greater,
    GreaterEq,
    And,
    Or,
    Not,
    Question,
    Colon,

    // ── Punctuation ─────────────────────────────────────────────────────
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Dot,

    // ── Assignment Operators ────────────────────────────────────────────
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PlusPlus,
    MinusMinus,
}

/// Tokenize GLSL source code into a vector of tokens.
pub fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Skip whitespace
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            i += 1;
            continue;
        }

        // Skip line comments
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Skip block comments
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < len { i += 2; }
            continue;
        }

        // Skip preprocessor lines (#version, #define, etc.)
        if b == b'#' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Numbers
        if b.is_ascii_digit() || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            let mut is_float = b == b'.';
            i += 1;
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                if bytes[i] == b'.' { is_float = true; }
                i += 1;
            }
            // Handle 'f' suffix
            if i < len && (bytes[i] == b'f' || bytes[i] == b'F') {
                is_float = true;
                i += 1;
            }
            // Handle 'e'/'E' exponent
            if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
                is_float = true;
                i += 1;
                if i < len && (bytes[i] == b'+' || bytes[i] == b'-') {
                    i += 1;
                }
                while i < len && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if i < len && (bytes[i] == b'f' || bytes[i] == b'F') {
                    i += 1;
                }
            }
            let num_str = core::str::from_utf8(&bytes[start..i]).unwrap_or("0");
            // Strip trailing 'f'/'F'
            let clean = num_str.trim_end_matches(|c| c == 'f' || c == 'F');
            if is_float {
                let val = parse_f32(clean);
                tokens.push(Token::FloatLiteral(val));
            } else {
                let val = parse_i32(clean);
                tokens.push(Token::IntLiteral(val));
            }
            continue;
        }

        // Identifiers and keywords
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = core::str::from_utf8(&bytes[start..i]).unwrap_or("");
            let tok = match word {
                "attribute" => Token::Attribute,
                "varying" => Token::Varying,
                "uniform" => Token::Uniform,
                "const" => Token::Const,
                "void" => Token::Void,
                "float" => Token::Float,
                "int" => Token::Int,
                "bool" => Token::Bool,
                "vec2" => Token::Vec2,
                "vec3" => Token::Vec3,
                "vec4" => Token::Vec4,
                "mat3" => Token::Mat3,
                "mat4" => Token::Mat4,
                "sampler2D" => Token::Sampler2D,
                "precision" => Token::Precision,
                "lowp" => Token::LowP,
                "mediump" => Token::MediumP,
                "highp" => Token::HighP,
                "if" => Token::If,
                "else" => Token::Else,
                "for" => Token::For,
                "while" => Token::While,
                "return" => Token::Return,
                "discard" => Token::Discard,
                "struct" => Token::Struct,
                "in" => Token::In,
                "out" => Token::Out,
                "true" => Token::BoolLiteral(true),
                "false" => Token::BoolLiteral(false),
                _ => Token::Ident(String::from(word)),
            };
            tokens.push(tok);
            continue;
        }

        // Two-character operators
        if i + 1 < len {
            let next = bytes[i + 1];
            match (b, next) {
                (b'=', b'=') => { tokens.push(Token::EqEq); i += 2; continue; }
                (b'!', b'=') => { tokens.push(Token::NotEq); i += 2; continue; }
                (b'<', b'=') => { tokens.push(Token::LessEq); i += 2; continue; }
                (b'>', b'=') => { tokens.push(Token::GreaterEq); i += 2; continue; }
                (b'&', b'&') => { tokens.push(Token::And); i += 2; continue; }
                (b'|', b'|') => { tokens.push(Token::Or); i += 2; continue; }
                (b'+', b'=') => { tokens.push(Token::PlusEq); i += 2; continue; }
                (b'-', b'=') => { tokens.push(Token::MinusEq); i += 2; continue; }
                (b'*', b'=') => { tokens.push(Token::StarEq); i += 2; continue; }
                (b'/', b'=') => { tokens.push(Token::SlashEq); i += 2; continue; }
                (b'+', b'+') => { tokens.push(Token::PlusPlus); i += 2; continue; }
                (b'-', b'-') => { tokens.push(Token::MinusMinus); i += 2; continue; }
                _ => {}
            }
        }

        // Single-character tokens
        let tok = match b {
            b'+' => Token::Plus,
            b'-' => Token::Minus,
            b'*' => Token::Star,
            b'/' => Token::Slash,
            b'=' => Token::Eq,
            b'<' => Token::Less,
            b'>' => Token::Greater,
            b'!' => Token::Not,
            b'?' => Token::Question,
            b':' => Token::Colon,
            b'(' => Token::LParen,
            b')' => Token::RParen,
            b'{' => Token::LBrace,
            b'}' => Token::RBrace,
            b'[' => Token::LBracket,
            b']' => Token::RBracket,
            b',' => Token::Comma,
            b';' => Token::Semicolon,
            b'.' => Token::Dot,
            _ => {
                i += 1;
                continue;
            }
        };
        tokens.push(tok);
        i += 1;
    }

    Ok(tokens)
}

/// Simple f32 parser for no_std.
fn parse_f32(s: &str) -> f32 {
    let bytes = s.as_bytes();
    let mut result: f64 = 0.0;
    let mut frac_div: f64 = 0.0;
    let mut negative = false;
    let mut i = 0;

    if i < bytes.len() && bytes[i] == b'-' {
        negative = true;
        i += 1;
    }

    while i < bytes.len() && bytes[i].is_ascii_digit() {
        result = result * 10.0 + (bytes[i] - b'0') as f64;
        i += 1;
    }

    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        frac_div = 10.0;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            result += (bytes[i] - b'0') as f64 / frac_div;
            frac_div *= 10.0;
            i += 1;
        }
    }

    let _ = frac_div;
    if negative { result = -result; }
    result as f32
}

/// Simple i32 parser for no_std.
fn parse_i32(s: &str) -> i32 {
    let bytes = s.as_bytes();
    let mut result: i32 = 0;
    let mut negative = false;
    let mut i = 0;

    if i < bytes.len() && bytes[i] == b'-' {
        negative = true;
        i += 1;
    }
    // Hex
    if i + 1 < bytes.len() && bytes[i] == b'0' && (bytes[i + 1] == b'x' || bytes[i + 1] == b'X') {
        i += 2;
        while i < bytes.len() {
            let d = match bytes[i] {
                b'0'..=b'9' => bytes[i] - b'0',
                b'a'..=b'f' => bytes[i] - b'a' + 10,
                b'A'..=b'F' => bytes[i] - b'A' + 10,
                _ => break,
            };
            result = result * 16 + d as i32;
            i += 1;
        }
    } else {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            result = result * 10 + (bytes[i] - b'0') as i32;
            i += 1;
        }
    }

    if negative { -result } else { result }
}
