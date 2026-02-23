//! SQL tokenizer and recursive-descent parser.
//!
//! Parses a subset of SQL into an AST ([`Statement`]) for execution by the
//! query executor. Supports CREATE TABLE, DROP TABLE, INSERT, SELECT, UPDATE,
//! and DELETE statements with WHERE clauses.

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use crate::types::*;

// ── Token ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum Token {
    // Keywords
    Create,
    Table,
    Drop,
    Insert,
    Into,
    Values,
    Select,
    From,
    Where,
    Update,
    Set,
    Delete,
    And,
    Or,
    Null,
    Integer,  // type keyword
    Text,     // type keyword

    // Identifiers and literals
    Ident(String),
    IntLit(i64),
    StrLit(String),

    // Operators
    Eq,       // =
    Ne,       // != or <>
    Lt,       // <
    Gt,       // >
    Le,       // <=
    Ge,       // >=
    Star,     // *

    // Punctuation
    LParen,
    RParen,
    Comma,
    Semi,

    Eof,
}

// ── Tokenizer ────────────────────────────────────────────────────────────────

/// Tokenizes a SQL string into a sequence of tokens.
struct Tokenizer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(sql: &'a str) -> Self {
        Tokenizer { input: sql.as_bytes(), pos: 0 }
    }

    fn peek_byte(&self) -> Option<u8> {
        if self.pos < self.input.len() { Some(self.input[self.pos]) } else { None }
    }

    fn advance(&mut self) -> u8 {
        let b = self.input[self.pos];
        self.pos += 1;
        b
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                b'-' if self.pos + 1 < self.input.len() && self.input[self.pos + 1] == b'-' => {
                    // Line comment: skip to end of line
                    self.pos += 2;
                    while self.pos < self.input.len() && self.input[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                _ => break,
            }
        }
    }

    /// Produce the next token.
    fn next_token(&mut self) -> DbResult<Token> {
        self.skip_whitespace();

        let b = match self.peek_byte() {
            None => return Ok(Token::Eof),
            Some(b) => b,
        };

        match b {
            b'(' => { self.advance(); Ok(Token::LParen) }
            b')' => { self.advance(); Ok(Token::RParen) }
            b',' => { self.advance(); Ok(Token::Comma) }
            b';' => { self.advance(); Ok(Token::Semi) }
            b'*' => { self.advance(); Ok(Token::Star) }
            b'=' => { self.advance(); Ok(Token::Eq) }
            b'<' => {
                self.advance();
                match self.peek_byte() {
                    Some(b'=') => { self.advance(); Ok(Token::Le) }
                    Some(b'>') => { self.advance(); Ok(Token::Ne) }
                    _ => Ok(Token::Lt),
                }
            }
            b'>' => {
                self.advance();
                if self.peek_byte() == Some(b'=') {
                    self.advance();
                    Ok(Token::Ge)
                } else {
                    Ok(Token::Gt)
                }
            }
            b'!' => {
                self.advance();
                if self.peek_byte() == Some(b'=') {
                    self.advance();
                    Ok(Token::Ne)
                } else {
                    Err(DbError::Parse(String::from("Expected '=' after '!'")))
                }
            }
            b'\'' => self.read_string(),
            b'0'..=b'9' => self.read_number(false),
            b'-' => {
                // Could be negative number
                self.advance();
                if matches!(self.peek_byte(), Some(b'0'..=b'9')) {
                    self.read_number(true)
                } else {
                    Err(DbError::Parse(String::from("Unexpected '-'")))
                }
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.read_ident_or_keyword(),
            _ => {
                let mut msg = String::from("Unexpected character: ");
                msg.push(b as char);
                Err(DbError::Parse(msg))
            }
        }
    }

    /// Read a single-quoted string literal.
    fn read_string(&mut self) -> DbResult<Token> {
        self.advance(); // skip opening '
        let start = self.pos;
        while self.pos < self.input.len() && self.input[self.pos] != b'\'' {
            // Handle escaped quotes ('')
            if self.input[self.pos] == b'\'' && self.pos + 1 < self.input.len()
                && self.input[self.pos + 1] == b'\''
            {
                self.pos += 2;
                continue;
            }
            self.pos += 1;
        }
        if self.pos >= self.input.len() {
            return Err(DbError::Parse(String::from("Unterminated string literal")));
        }
        let raw = &self.input[start..self.pos];
        self.advance(); // skip closing '

        // Handle escaped quotes: replace '' with '
        let s = core::str::from_utf8(raw).unwrap_or("");
        let result = if s.contains("''") {
            let mut out = String::new();
            let mut chars = s.chars();
            while let Some(c) = chars.next() {
                if c == '\'' {
                    // Skip the second quote of an escaped pair
                    let _ = chars.next();
                }
                out.push(c);
            }
            out
        } else {
            String::from(s)
        };
        Ok(Token::StrLit(result))
    }

    /// Read a numeric literal.
    fn read_number(&mut self, negative: bool) -> DbResult<Token> {
        let start = self.pos;
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        let digits = core::str::from_utf8(&self.input[start..self.pos]).unwrap_or("0");
        let mut val: i64 = 0;
        for &b in digits.as_bytes() {
            val = val.wrapping_mul(10).wrapping_add((b - b'0') as i64);
        }
        if negative { val = -val; }
        Ok(Token::IntLit(val))
    }

    /// Read an identifier or keyword.
    fn read_ident_or_keyword(&mut self) -> DbResult<Token> {
        let start = self.pos;
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' => self.pos += 1,
                _ => break,
            }
        }
        let word = core::str::from_utf8(&self.input[start..self.pos]).unwrap_or("");

        // Case-insensitive keyword matching
        let mut upper = [0u8; 32];
        let len = word.len().min(32);
        for i in 0..len {
            upper[i] = word.as_bytes()[i].to_ascii_uppercase();
        }
        let upper_str = core::str::from_utf8(&upper[..len]).unwrap_or("");

        let tok = match upper_str {
            "CREATE" => Token::Create,
            "TABLE" => Token::Table,
            "DROP" => Token::Drop,
            "INSERT" => Token::Insert,
            "INTO" => Token::Into,
            "VALUES" => Token::Values,
            "SELECT" => Token::Select,
            "FROM" => Token::From,
            "WHERE" => Token::Where,
            "UPDATE" => Token::Update,
            "SET" => Token::Set,
            "DELETE" => Token::Delete,
            "AND" => Token::And,
            "OR" => Token::Or,
            "NULL" => Token::Null,
            "INTEGER" | "INT" => Token::Integer,
            "TEXT" | "VARCHAR" => Token::Text,
            _ => Token::Ident(String::from(word)),
        };
        Ok(tok)
    }

    /// Tokenize entire input into a vector.
    fn tokenize_all(&mut self) -> DbResult<Vec<Token>> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            if tok == Token::Eof { break; }
            tokens.push(tok);
        }
        Ok(tokens)
    }
}

// ── Parser ───────────────────────────────────────────────────────────────────

/// Recursive-descent SQL parser operating on a token stream.
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
        if self.pos < self.tokens.len() {
            let tok = self.tokens[self.pos].clone();
            self.pos += 1;
            tok
        } else {
            Token::Eof
        }
    }

    fn expect(&mut self, expected: &Token) -> DbResult<()> {
        let tok = self.advance();
        if &tok == expected {
            Ok(())
        } else {
            let mut msg = String::from("Expected ");
            msg.push_str(&format_token(expected));
            msg.push_str(", got ");
            msg.push_str(&format_token(&tok));
            Err(DbError::Parse(msg))
        }
    }

    fn expect_ident(&mut self) -> DbResult<String> {
        match self.advance() {
            Token::Ident(s) => Ok(s),
            other => {
                let mut msg = String::from("Expected identifier, got ");
                msg.push_str(&format_token(&other));
                Err(DbError::Parse(msg))
            }
        }
    }

    /// Parse the top-level statement.
    fn parse_statement(&mut self) -> DbResult<Statement> {
        match self.peek().clone() {
            Token::Create => self.parse_create_table(),
            Token::Drop => self.parse_drop_table(),
            Token::Insert => self.parse_insert(),
            Token::Select => self.parse_select(),
            Token::Update => self.parse_update(),
            Token::Delete => self.parse_delete(),
            ref other => {
                let mut msg = String::from("Expected SQL statement, got ");
                msg.push_str(&format_token(other));
                Err(DbError::Parse(msg))
            }
        }
    }

    // ── CREATE TABLE name (col1 TYPE, col2 TYPE, ...) ───────────────────

    fn parse_create_table(&mut self) -> DbResult<Statement> {
        self.advance(); // CREATE
        self.expect(&Token::Table)?;
        let name = self.expect_ident()?;
        self.expect(&Token::LParen)?;

        let mut columns = Vec::new();
        loop {
            let col_name = self.expect_ident()?;
            let col_type = match self.advance() {
                Token::Integer => ColumnType::Integer,
                Token::Text => ColumnType::Text,
                other => {
                    let mut msg = String::from("Expected column type (INTEGER or TEXT), got ");
                    msg.push_str(&format_token(&other));
                    return Err(DbError::Parse(msg));
                }
            };
            columns.push(ColumnDef { name: col_name, col_type });

            match self.peek() {
                Token::Comma => { self.advance(); }
                Token::RParen => break,
                other => {
                    let mut msg = String::from("Expected ',' or ')', got ");
                    msg.push_str(&format_token(other));
                    return Err(DbError::Parse(msg));
                }
            }
        }
        self.expect(&Token::RParen)?;
        // Optional semicolon
        if self.peek() == &Token::Semi { self.advance(); }

        Ok(Statement::CreateTable { name, columns })
    }

    // ── DROP TABLE name ─────────────────────────────────────────────────

    fn parse_drop_table(&mut self) -> DbResult<Statement> {
        self.advance(); // DROP
        self.expect(&Token::Table)?;
        let name = self.expect_ident()?;
        if self.peek() == &Token::Semi { self.advance(); }
        Ok(Statement::DropTable { name })
    }

    // ── INSERT INTO name (cols) VALUES (vals) ───────────────────────────

    fn parse_insert(&mut self) -> DbResult<Statement> {
        self.advance(); // INSERT
        self.expect(&Token::Into)?;
        let table = self.expect_ident()?;

        // Optional column list
        let columns = if self.peek() == &Token::LParen {
            self.advance(); // (
            let mut cols = Vec::new();
            loop {
                cols.push(self.expect_ident()?);
                match self.peek() {
                    Token::Comma => { self.advance(); }
                    Token::RParen => break,
                    other => {
                        let mut msg = String::from("Expected ',' or ')', got ");
                        msg.push_str(&format_token(other));
                        return Err(DbError::Parse(msg));
                    }
                }
            }
            self.expect(&Token::RParen)?;
            cols
        } else {
            Vec::new()
        };

        self.expect(&Token::Values)?;
        self.expect(&Token::LParen)?;

        let mut values = Vec::new();
        loop {
            let val = self.parse_value()?;
            values.push(val);
            match self.peek() {
                Token::Comma => { self.advance(); }
                Token::RParen => break,
                other => {
                    let mut msg = String::from("Expected ',' or ')', got ");
                    msg.push_str(&format_token(other));
                    return Err(DbError::Parse(msg));
                }
            }
        }
        self.expect(&Token::RParen)?;
        if self.peek() == &Token::Semi { self.advance(); }

        Ok(Statement::Insert { table, columns, values })
    }

    // ── SELECT cols FROM name [WHERE ...] ───────────────────────────────

    fn parse_select(&mut self) -> DbResult<Statement> {
        self.advance(); // SELECT

        let columns = if self.peek() == &Token::Star {
            self.advance();
            SelectColumns::All
        } else {
            let mut cols = Vec::new();
            loop {
                cols.push(self.expect_ident()?);
                if self.peek() == &Token::Comma {
                    self.advance();
                } else {
                    break;
                }
            }
            SelectColumns::Named(cols)
        };

        self.expect(&Token::From)?;
        let table = self.expect_ident()?;

        let where_clause = if self.peek() == &Token::Where {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        if self.peek() == &Token::Semi { self.advance(); }

        Ok(Statement::Select { table, columns, where_clause })
    }

    // ── UPDATE name SET col=val [, col=val] [WHERE ...] ─────────────────

    fn parse_update(&mut self) -> DbResult<Statement> {
        self.advance(); // UPDATE
        let table = self.expect_ident()?;
        self.expect(&Token::Set)?;

        let mut assignments = Vec::new();
        loop {
            let col = self.expect_ident()?;
            self.expect(&Token::Eq)?;
            let val = self.parse_value()?;
            assignments.push((col, val));
            if self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }

        let where_clause = if self.peek() == &Token::Where {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        if self.peek() == &Token::Semi { self.advance(); }

        Ok(Statement::Update { table, assignments, where_clause })
    }

    // ── DELETE FROM name [WHERE ...] ────────────────────────────────────

    fn parse_delete(&mut self) -> DbResult<Statement> {
        self.advance(); // DELETE
        self.expect(&Token::From)?;
        let table = self.expect_ident()?;

        let where_clause = if self.peek() == &Token::Where {
            self.advance();
            Some(self.parse_expr()?)
        } else {
            None
        };
        if self.peek() == &Token::Semi { self.advance(); }

        Ok(Statement::Delete { table, where_clause })
    }

    // ── Expression parsing (WHERE clause) ───────────────────────────────

    /// Parse an expression: handles OR at the lowest precedence.
    fn parse_expr(&mut self) -> DbResult<Expr> {
        let mut left = self.parse_and_expr()?;
        while self.peek() == &Token::Or {
            self.advance();
            let right = self.parse_and_expr()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Parse AND expressions (higher precedence than OR).
    fn parse_and_expr(&mut self) -> DbResult<Expr> {
        let mut left = self.parse_comparison()?;
        while self.peek() == &Token::And {
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Parse a comparison: term op term.
    fn parse_comparison(&mut self) -> DbResult<Expr> {
        let left = self.parse_term()?;

        let op = match self.peek() {
            Token::Eq => CmpOp::Eq,
            Token::Ne => CmpOp::Ne,
            Token::Lt => CmpOp::Lt,
            Token::Gt => CmpOp::Gt,
            Token::Le => CmpOp::Le,
            Token::Ge => CmpOp::Ge,
            _ => return Ok(left),
        };
        self.advance();
        let right = self.parse_term()?;
        Ok(Expr::BinOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    /// Parse a primary term: literal, column, or parenthesized expression.
    fn parse_term(&mut self) -> DbResult<Expr> {
        match self.peek().clone() {
            Token::IntLit(v) => { self.advance(); Ok(Expr::Literal(Value::Integer(v))) }
            Token::StrLit(s) => { self.advance(); Ok(Expr::Literal(Value::Text(s))) }
            Token::Null => { self.advance(); Ok(Expr::Literal(Value::Null)) }
            Token::Ident(s) => { self.advance(); Ok(Expr::Column(s)) }
            Token::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            ref other => {
                let mut msg = String::from("Expected expression, got ");
                msg.push_str(&format_token(other));
                Err(DbError::Parse(msg))
            }
        }
    }

    /// Parse a literal value (for INSERT VALUES and UPDATE SET).
    fn parse_value(&mut self) -> DbResult<Value> {
        match self.advance() {
            Token::IntLit(v) => Ok(Value::Integer(v)),
            Token::StrLit(s) => Ok(Value::Text(s)),
            Token::Null => Ok(Value::Null),
            other => {
                let mut msg = String::from("Expected value (integer, string, or NULL), got ");
                msg.push_str(&format_token(&other));
                Err(DbError::Parse(msg))
            }
        }
    }
}

// ── Token formatting (for error messages) ────────────────────────────────────

fn format_token(tok: &Token) -> String {
    match tok {
        Token::Create => String::from("CREATE"),
        Token::Table => String::from("TABLE"),
        Token::Drop => String::from("DROP"),
        Token::Insert => String::from("INSERT"),
        Token::Into => String::from("INTO"),
        Token::Values => String::from("VALUES"),
        Token::Select => String::from("SELECT"),
        Token::From => String::from("FROM"),
        Token::Where => String::from("WHERE"),
        Token::Update => String::from("UPDATE"),
        Token::Set => String::from("SET"),
        Token::Delete => String::from("DELETE"),
        Token::And => String::from("AND"),
        Token::Or => String::from("OR"),
        Token::Null => String::from("NULL"),
        Token::Integer => String::from("INTEGER"),
        Token::Text => String::from("TEXT"),
        Token::Ident(s) => {
            let mut m = String::from("'");
            m.push_str(s);
            m.push('\'');
            m
        }
        Token::IntLit(v) => {
            let mut m = String::new();
            fmt_i64(&mut m, *v);
            m
        }
        Token::StrLit(s) => {
            let mut m = String::from("'");
            m.push_str(s);
            m.push('\'');
            m
        }
        Token::Eq => String::from("'='"),
        Token::Ne => String::from("'!='"),
        Token::Lt => String::from("'<'"),
        Token::Gt => String::from("'>'"),
        Token::Le => String::from("'<='"),
        Token::Ge => String::from("'>='"),
        Token::Star => String::from("'*'"),
        Token::LParen => String::from("'('"),
        Token::RParen => String::from("')'"),
        Token::Comma => String::from("','"),
        Token::Semi => String::from("';'"),
        Token::Eof => String::from("end of input"),
    }
}

/// Format an i64 into a String (no_std compatible).
fn fmt_i64(out: &mut String, v: i64) {
    if v == 0 {
        out.push('0');
        return;
    }
    let (neg, abs) = if v < 0 { (true, (-(v + 1)) as u64 + 1) } else { (false, v as u64) };
    if neg { out.push('-'); }
    let mut buf = [0u8; 20];
    let mut n = 0;
    let mut val = abs;
    while val > 0 {
        buf[n] = b'0' + (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    for i in (0..n).rev() {
        out.push(buf[i] as char);
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Parse a SQL string into a [`Statement`] AST node.
pub fn parse_sql(sql: &str) -> DbResult<Statement> {
    let mut tokenizer = Tokenizer::new(sql);
    let tokens = tokenizer.tokenize_all()?;
    if tokens.is_empty() {
        return Err(DbError::Parse(String::from("Empty SQL statement")));
    }
    let mut parser = Parser::new(tokens);
    parser.parse_statement()
}
