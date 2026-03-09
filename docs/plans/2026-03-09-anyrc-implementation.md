# anyrc (Rust Subset Compiler) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a self-hosting Rust subset compiler with native x86_64 codegen that can compile anyOS within anyOS.

**Architecture:** Multi-pass pipeline (Lexer → Parser → AST → HIR → Name Resolution → Type Check → MIR → Borrow Check → Codegen → ELF). Compiler is a Rust library (`libs/anyrc`) with CLI frontend (`bin/anyrc`). Dual-target: runs on Linux host and anyOS.

**Tech Stack:** Rust (no external dependencies), custom x86_64 assembler, ELF generator, NLL borrow checker.

**Design doc:** `docs/plans/2026-03-09-anyrc-design.md`

**Test strategy:** Compiler tests follow the `libjs_tests` pattern — a separate `libs/anyrc_tests` crate with its own `[workspace]` that runs on the host. Tests compile Rust source strings and assert on tokens, AST, HIR, MIR, types, borrow-check errors, and generated machine code.

---

## Phase 1: Project Scaffolding + Lexer (M1 foundation)

### Task 1.1: Create project structure

**Files:**
- Create: `libs/anyrc/Cargo.toml`
- Create: `libs/anyrc/src/lib.rs`
- Create: `bin/anyrc/Cargo.toml`
- Create: `bin/anyrc/src/main.rs`
- Create: `libs/anyrc_tests/Cargo.toml`
- Create: `libs/anyrc_tests/src/lib.rs`
- Create: `libs/anyrc_tests/tests/01_lexer.rs`

**Step 1: Create libs/anyrc/Cargo.toml**

```toml
[package]
name = "anyrc"
version = "0.1.0"
edition = "2021"

[workspace]

[lib]
name = "anyrc"

[features]
default = ["host"]
host = []          # Uses std::fs for I/O
anyos = []         # Uses anyos_std for I/O
```

**Step 2: Create libs/anyrc/src/lib.rs**

```rust
pub mod lexer;
pub mod diagnostics;
```

**Step 3: Create bin/anyrc/Cargo.toml**

```toml
[package]
name = "anyrc-cli"
version = "0.1.0"
edition = "2021"

[workspace]

[[bin]]
name = "anyrc"

[dependencies]
anyrc = { path = "../../libs/anyrc" }
```

**Step 4: Create bin/anyrc/src/main.rs**

```rust
fn main() {
    eprintln!("anyrc 0.1.0");
    std::process::exit(1);
}
```

**Step 5: Create libs/anyrc_tests/Cargo.toml**

```toml
[package]
name = "anyrc_tests"
version = "0.1.0"
edition = "2021"

[workspace]

[lib]
name = "anyrc_tests"
path = "src/lib.rs"

[dependencies]
anyrc = { path = "../anyrc" }
```

**Step 6: Create libs/anyrc_tests/src/lib.rs**

```rust
// Test harness for anyrc compiler
```

**Step 7: Verify builds**

Run: `cd libs/anyrc && cargo build`
Expected: Compiles (with warnings about missing modules)

Run: `cd libs/anyrc_tests && cargo test`
Expected: 0 tests pass

**Step 8: Commit**

```bash
git add libs/anyrc/ bin/anyrc/ libs/anyrc_tests/
git commit -m "feat(anyrc): scaffold project structure"
```

---

### Task 1.2: Symbol interning

**Files:**
- Create: `libs/anyrc/src/intern.rs`
- Modify: `libs/anyrc/src/lib.rs` — add `pub mod intern;`
- Create: `libs/anyrc_tests/tests/00_intern.rs`

**Step 1: Write failing test**

File: `libs/anyrc_tests/tests/00_intern.rs`

```rust
use anyrc::intern::Interner;

#[test]
fn intern_returns_same_symbol_for_same_string() {
    let mut interner = Interner::new();
    let s1 = interner.intern("hello");
    let s2 = interner.intern("hello");
    assert_eq!(s1, s2);
}

#[test]
fn intern_returns_different_symbols_for_different_strings() {
    let mut interner = Interner::new();
    let s1 = interner.intern("hello");
    let s2 = interner.intern("world");
    assert_ne!(s1, s2);
}

#[test]
fn resolve_returns_original_string() {
    let mut interner = Interner::new();
    let sym = interner.intern("test_string");
    assert_eq!(interner.resolve(sym), "test_string");
}
```

**Step 2: Run test to verify it fails**

Run: `cd libs/anyrc_tests && cargo test --test 00_intern`
Expected: FAIL — module `intern` not found

**Step 3: Implement**

File: `libs/anyrc/src/intern.rs`

```rust
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol(u32);

pub struct Interner {
    map: HashMap<String, Symbol>,
    strings: Vec<String>,
}

impl Interner {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            strings: Vec::new(),
        }
    }

    pub fn intern(&mut self, s: &str) -> Symbol {
        if let Some(&sym) = self.map.get(s) {
            return sym;
        }
        let sym = Symbol(self.strings.len() as u32);
        self.strings.push(s.to_string());
        self.map.insert(s.to_string(), sym);
        sym
    }

    pub fn resolve(&self, sym: Symbol) -> &str {
        &self.strings[sym.0 as usize]
    }
}
```

**Step 4: Add module to lib.rs**

```rust
pub mod intern;
pub mod lexer;
pub mod diagnostics;
```

**Step 5: Run test to verify it passes**

Run: `cd libs/anyrc_tests && cargo test --test 00_intern`
Expected: 3 tests PASS

**Step 6: Commit**

```bash
git add libs/anyrc/src/intern.rs libs/anyrc/src/lib.rs libs/anyrc_tests/tests/00_intern.rs
git commit -m "feat(anyrc): add symbol interning"
```

---

### Task 1.3: Span and diagnostics

**Files:**
- Create: `libs/anyrc/src/diagnostics.rs`
- Create: `libs/anyrc_tests/tests/00_diagnostics.rs`

**Step 1: Write failing test**

File: `libs/anyrc_tests/tests/00_diagnostics.rs`

```rust
use anyrc::diagnostics::{Span, SourceMap, Diagnostic, Level};

#[test]
fn span_contains_offset() {
    let span = Span::new(5, 10);
    assert_eq!(span.start(), 5);
    assert_eq!(span.end(), 10);
    assert_eq!(span.len(), 5);
}

#[test]
fn source_map_resolves_line_col() {
    let src = "fn main() {\n    let x = 5;\n}";
    let sm = SourceMap::new("test.rs".to_string(), src.to_string());
    let (line, col) = sm.line_col(Span::new(16, 17)); // 'x' on line 2
    assert_eq!(line, 2);
    assert_eq!(col, 9);
}

#[test]
fn diagnostic_formats_with_span() {
    let src = "fn main() {\n    let x = 5;\n}";
    let sm = SourceMap::new("test.rs".to_string(), src.to_string());
    let diag = Diagnostic::new(Level::Error, "test error", Span::new(16, 17));
    let output = diag.render(&sm);
    assert!(output.contains("error: test error"));
    assert!(output.contains("test.rs:2:9"));
}
```

**Step 2: Implement**

File: `libs/anyrc/src/diagnostics.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    start: u32,
    end: u32,
}

impl Span {
    pub fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }

    pub fn start(&self) -> u32 { self.start }
    pub fn end(&self) -> u32 { self.end }
    pub fn len(&self) -> u32 { self.end - self.start }
    pub fn dummy() -> Self { Self { start: 0, end: 0 } }
}

pub struct SourceMap {
    filename: String,
    source: String,
    line_starts: Vec<u32>,
}

impl SourceMap {
    pub fn new(filename: String, source: String) -> Self {
        let mut line_starts = vec![0];
        for (i, c) in source.char_indices() {
            if c == '\n' {
                line_starts.push(i as u32 + 1);
            }
        }
        Self { filename, source, line_starts }
    }

    pub fn filename(&self) -> &str { &self.filename }
    pub fn source(&self) -> &str { &self.source }

    /// Returns (1-based line, 1-based column)
    pub fn line_col(&self, span: Span) -> (u32, u32) {
        let offset = span.start();
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let col = offset - self.line_starts[line];
        (line as u32 + 1, col + 1)
    }

    pub fn line_text(&self, line: u32) -> &str {
        let idx = (line - 1) as usize;
        let start = self.line_starts[idx] as usize;
        let end = self.line_starts.get(idx + 1)
            .map(|&s| s as usize - 1)
            .unwrap_or(self.source.len());
        &self.source[start..end]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
    Note,
}

pub struct Diagnostic {
    pub level: Level,
    pub message: String,
    pub span: Span,
}

impl Diagnostic {
    pub fn new(level: Level, message: &str, span: Span) -> Self {
        Self { level, message: message.to_string(), span }
    }

    pub fn render(&self, sm: &SourceMap) -> String {
        let (line, col) = sm.line_col(self.span);
        let level_str = match self.level {
            Level::Error => "error",
            Level::Warning => "warning",
            Level::Note => "note",
        };
        let line_text = sm.line_text(line);
        let underline = " ".repeat((col - 1) as usize) + &"^".repeat(self.span.len().max(1) as usize);
        format!(
            "{}: {}\n  --> {}:{}:{}\n   |\n{:>3}| {}\n   | {}",
            level_str, self.message,
            sm.filename(), line, col,
            line, line_text,
            underline,
        )
    }
}
```

**Step 3: Run tests**

Run: `cd libs/anyrc_tests && cargo test --test 00_diagnostics`
Expected: 3 tests PASS

**Step 4: Commit**

```bash
git add libs/anyrc/src/diagnostics.rs libs/anyrc_tests/tests/00_diagnostics.rs
git commit -m "feat(anyrc): add span tracking and diagnostic rendering"
```

---

### Task 1.4: Lexer — keywords and simple tokens

**Files:**
- Create: `libs/anyrc/src/lexer.rs`
- Create: `libs/anyrc_tests/tests/01_lexer.rs`

**Step 1: Write failing test**

File: `libs/anyrc_tests/tests/01_lexer.rs`

```rust
use anyrc::lexer::{Lexer, TokenKind};
use anyrc::intern::Interner;

fn lex(src: &str) -> Vec<TokenKind> {
    let mut interner = Interner::new();
    let mut lexer = Lexer::new(src, &mut interner);
    let mut tokens = Vec::new();
    loop {
        let tok = lexer.next_token();
        if tok.kind == TokenKind::Eof { break; }
        tokens.push(tok.kind);
    }
    tokens
}

#[test]
fn lex_fn_main() {
    let tokens = lex("fn main()");
    assert_eq!(tokens, vec![
        TokenKind::Kw(anyrc::lexer::Keyword::Fn),
        TokenKind::Ident(anyrc::intern::Symbol::from_raw(0)), // "main"
        TokenKind::LParen,
        TokenKind::RParen,
    ]);
}

#[test]
fn lex_let_binding() {
    let tokens = lex("let mut x: u32 = 42;");
    assert_eq!(tokens[0], TokenKind::Kw(anyrc::lexer::Keyword::Let));
    assert_eq!(tokens[1], TokenKind::Kw(anyrc::lexer::Keyword::Mut));
    // x = Ident, : = Colon, u32 = Ident, = = Eq, 42 = IntLit, ; = Semi
    assert_eq!(tokens.len(), 8);
}

#[test]
fn lex_operators() {
    let tokens = lex("a + b == c && d != e");
    // Ident Plus Ident EqEq Ident AndAnd Ident Ne Ident
    assert_eq!(tokens.len(), 9);
    assert_eq!(tokens[1], TokenKind::Plus);
    assert_eq!(tokens[3], TokenKind::EqEq);
    assert_eq!(tokens[5], TokenKind::AndAnd);
    assert_eq!(tokens[7], TokenKind::Ne);
}

#[test]
fn lex_string_literal() {
    let tokens = lex(r#""hello world""#);
    assert_eq!(tokens.len(), 1);
    match &tokens[0] {
        TokenKind::StringLit(s) => assert_eq!(s, "hello world"),
        _ => panic!("expected string literal"),
    }
}

#[test]
fn lex_integer_literals() {
    let tokens = lex("42 0xFF 0b1010 1_000_000");
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[0], TokenKind::IntLit(42));
    assert_eq!(tokens[1], TokenKind::IntLit(255));
    assert_eq!(tokens[2], TokenKind::IntLit(10));
    assert_eq!(tokens[3], TokenKind::IntLit(1_000_000));
}

#[test]
fn lex_lifetime() {
    let tokens = lex("&'a mut T");
    assert_eq!(tokens[0], TokenKind::Amp);
    assert!(matches!(tokens[1], TokenKind::Lifetime(_)));
    assert_eq!(tokens[2], TokenKind::Kw(anyrc::lexer::Keyword::Mut));
}

#[test]
fn lex_comments_skipped() {
    let tokens = lex("a // comment\nb");
    assert_eq!(tokens.len(), 2); // just two idents
}

#[test]
fn lex_arrows() {
    let tokens = lex("-> =>");
    assert_eq!(tokens[0], TokenKind::Arrow);
    assert_eq!(tokens[1], TokenKind::FatArrow);
}
```

**Step 2: Implement lexer**

File: `libs/anyrc/src/lexer.rs`

This is a large file (~400 lines). Key structure:

```rust
use crate::intern::{Interner, Symbol};
use crate::diagnostics::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keyword {
    Fn, Let, Mut, Pub, Struct, Enum, Impl, Trait, Type,
    Use, Mod, Crate, Self_, Super, As, In, For, While,
    Loop, If, Else, Match, Return, Break, Continue,
    Where, Const, Static, Unsafe, Extern, Ref, Move,
    True, False,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literals
    IntLit(u128),
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
    Semi, Colon, Comma, Dot, At, Hash, Question,
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
    pub fn new(src: &'a str, interner: &'a mut Interner) -> Self { ... }
    pub fn next_token(&mut self) -> Token { ... }
    fn skip_whitespace_and_comments(&mut self) { ... }
    fn lex_ident_or_keyword(&mut self) -> Token { ... }
    fn lex_number(&mut self) -> Token { ... }
    fn lex_string(&mut self) -> Token { ... }
    fn lex_char(&mut self) -> Token { ... }
    fn lex_lifetime(&mut self) -> Token { ... }
    fn keyword_or_ident(&mut self, s: &str) -> TokenKind { ... }
}
```

Implement the full lexer with all the token types listed above. Handle:
- Line comments (`//`) and block comments (`/* */`)
- String escape sequences (`\n`, `\t`, `\\`, `\"`, `\0`, `\x..`, `\u{...}`)
- Raw strings (`r#"..."#`)
- Integer suffixes (`42u32`, `0xFFi64`)
- Float literals (`3.14`, `1e10`, `1.0f32`)
- All multi-character operators (`==`, `!=`, `<=`, `>=`, `&&`, `||`, `->`, `=>`, `::`, `..`, `..=`, `<<`, `>>`, `+=`, etc.)
- Lifetimes (`'a`, `'static`)

**Step 3: Run tests**

Run: `cd libs/anyrc_tests && cargo test --test 01_lexer`
Expected: 8 tests PASS

**Step 4: Commit**

```bash
git add libs/anyrc/src/lexer.rs libs/anyrc_tests/tests/01_lexer.rs
git commit -m "feat(anyrc): implement lexer with full token set"
```

---

## Phase 2: Parser + AST

### Task 2.1: AST data structures

**Files:**
- Create: `libs/anyrc/src/ast.rs`
- Modify: `libs/anyrc/src/lib.rs` — add `pub mod ast;`

**Step 1: Define all AST types**

File: `libs/anyrc/src/ast.rs`

Define the complete AST as described in the design doc. Key types:
- `Crate`, `Item` (Fn, Struct, Enum, Impl, Trait, TypeAlias, Const, Static, Use, Mod, MacroDef, ExternBlock, GlobalAsm)
- `FnDef`, `StructDef`, `EnumDef`, `ImplBlock`, `TraitDef`
- `Expr` (all ~30 variants from design)
- `Pattern` (Ident, Literal, Tuple, Struct, Enum, Wildcard, Ref, Or)
- `Ty` (Path, Reference, RawPtr, Tuple, Array, Slice, FnPtr, Infer, Never)
- `Generics`, `GenericParam`, `WhereClause`, `WherePredicate`
- `Visibility`, `Mutability`, `BinOp`, `UnOp`
- `Block`, `Statement`, `MatchArm`, `FieldDef`, `FieldExpr`

All nodes carry a `Span`. All identifiers are `Symbol`.

This is a data-only module — no logic, just structs and enums.

**Step 2: Verify it compiles**

Run: `cd libs/anyrc && cargo build`
Expected: Compiles

**Step 3: Commit**

```bash
git add libs/anyrc/src/ast.rs libs/anyrc/src/lib.rs
git commit -m "feat(anyrc): define AST data structures"
```

---

### Task 2.2: Expression parser (Pratt parsing)

**Files:**
- Create: `libs/anyrc/src/parser.rs`
- Modify: `libs/anyrc/src/lib.rs` — add `pub mod parser;`
- Create: `libs/anyrc_tests/tests/02_parser_expr.rs`

**Step 1: Write failing tests**

File: `libs/anyrc_tests/tests/02_parser_expr.rs`

```rust
use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::ast::*;

fn parse_expr(src: &str) -> Expr {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    parser.parse_expr()
}

#[test]
fn parse_integer_literal() {
    let expr = parse_expr("42");
    assert!(matches!(expr, Expr::Lit(Literal::Int(42))));
}

#[test]
fn parse_binary_op_precedence() {
    // 1 + 2 * 3 should parse as 1 + (2 * 3)
    let expr = parse_expr("1 + 2 * 3");
    match expr {
        Expr::Binary(BinOp::Add, lhs, rhs) => {
            assert!(matches!(*lhs, Expr::Lit(Literal::Int(1))));
            match *rhs {
                Expr::Binary(BinOp::Mul, l, r) => {
                    assert!(matches!(*l, Expr::Lit(Literal::Int(2))));
                    assert!(matches!(*r, Expr::Lit(Literal::Int(3))));
                }
                _ => panic!("expected mul"),
            }
        }
        _ => panic!("expected add"),
    }
}

#[test]
fn parse_function_call() {
    let expr = parse_expr("foo(1, 2)");
    match expr {
        Expr::Call(func, args) => {
            assert!(matches!(*func, Expr::Path(_)));
            assert_eq!(args.len(), 2);
        }
        _ => panic!("expected call"),
    }
}

#[test]
fn parse_method_call() {
    let expr = parse_expr("x.push(5)");
    match expr {
        Expr::MethodCall(_, _, _, args) => {
            assert_eq!(args.len(), 1);
        }
        _ => panic!("expected method call"),
    }
}

#[test]
fn parse_if_else() {
    let expr = parse_expr("if x > 0 { x } else { -x }");
    assert!(matches!(expr, Expr::If(_, _, Some(_))));
}

#[test]
fn parse_match() {
    let expr = parse_expr("match x { 0 => 1, _ => 2 }");
    match expr {
        Expr::Match(_, arms) => assert_eq!(arms.len(), 2),
        _ => panic!("expected match"),
    }
}

#[test]
fn parse_closure() {
    let expr = parse_expr("|x, y| x + y");
    assert!(matches!(expr, Expr::Closure(_, _, _, false)));
}

#[test]
fn parse_reference() {
    let expr = parse_expr("&mut x");
    match expr {
        Expr::Ref(_, Mutability::Mut) => {}
        _ => panic!("expected &mut"),
    }
}

#[test]
fn parse_struct_literal() {
    let expr = parse_expr("Point { x: 1, y: 2 }");
    match expr {
        Expr::Struct(_, fields) => assert_eq!(fields.len(), 2),
        _ => panic!("expected struct literal"),
    }
}

#[test]
fn parse_index() {
    let expr = parse_expr("arr[0]");
    assert!(matches!(expr, Expr::Index(_, _)));
}
```

**Step 2: Implement parser**

File: `libs/anyrc/src/parser.rs`

Key structure:

```rust
use crate::lexer::{Lexer, Token, TokenKind, Keyword};
use crate::ast::*;
use crate::intern::{Interner, Symbol};
use crate::diagnostics::Span;

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Token,
    interner: &'a mut Interner,
}

impl<'a> Parser<'a> {
    pub fn new(src: &'a str, interner: &'a mut Interner) -> Self { ... }

    // Expressions (Pratt parsing)
    pub fn parse_expr(&mut self) -> Expr { self.parse_expr_bp(0) }
    fn parse_expr_bp(&mut self, min_bp: u8) -> Expr { ... }
    fn prefix_binding_power(op: &TokenKind) -> Option<((), u8)> { ... }
    fn infix_binding_power(op: &TokenKind) -> Option<(u8, u8)> { ... }
    fn postfix_binding_power(op: &TokenKind) -> Option<(u8, ())> { ... }

    // Items
    pub fn parse_crate(&mut self) -> Crate { ... }
    fn parse_item(&mut self) -> Item { ... }
    fn parse_fn(&mut self) -> FnDef { ... }
    fn parse_struct(&mut self) -> StructDef { ... }
    fn parse_enum(&mut self) -> EnumDef { ... }
    fn parse_impl(&mut self) -> ImplBlock { ... }
    fn parse_trait(&mut self) -> TraitDef { ... }
    fn parse_use(&mut self) -> UseTree { ... }
    fn parse_mod(&mut self) -> ModDef { ... }

    // Types
    fn parse_ty(&mut self) -> Ty { ... }

    // Patterns
    fn parse_pattern(&mut self) -> Pattern { ... }

    // Generics
    fn parse_generics(&mut self) -> Generics { ... }
    fn parse_where_clause(&mut self) -> WhereClause { ... }

    // Statements
    fn parse_block(&mut self) -> Block { ... }
    fn parse_statement(&mut self) -> Statement { ... }

    // Helpers
    fn bump(&mut self) -> Token { ... }
    fn expect(&mut self, kind: TokenKind) -> Token { ... }
    fn eat(&mut self, kind: TokenKind) -> bool { ... }
    fn at(&self, kind: &TokenKind) -> bool { ... }
}
```

Implement the full parser. Pratt parsing operator precedence table (binding power, left-to-right):
- Assignment: `=`, `+=`, `-=`, etc. → bp 2 (right-associative)
- Range: `..`, `..=` → bp 4
- LogicalOr: `||` → bp 6
- LogicalAnd: `&&` → bp 8
- Comparison: `==`, `!=`, `<`, `>`, `<=`, `>=` → bp 10
- BitwiseOr: `|` → bp 12
- BitwiseXor: `^` → bp 14
- BitwiseAnd: `&` → bp 16
- Shift: `<<`, `>>` → bp 18
- Additive: `+`, `-` → bp 20
- Multiplicative: `*`, `/`, `%` → bp 22
- Cast: `as` → bp 24
- Prefix: `-`, `!`, `*`, `&`, `&mut` → bp 26
- Postfix: `?`, `.field`, `.method()`, `[index]`, `(args)` → bp 28

**Step 3: Run tests**

Run: `cd libs/anyrc_tests && cargo test --test 02_parser_expr`
Expected: 10 tests PASS

**Step 4: Commit**

```bash
git add libs/anyrc/src/parser.rs libs/anyrc/src/lib.rs libs/anyrc_tests/tests/02_parser_expr.rs
git commit -m "feat(anyrc): implement expression parser with Pratt parsing"
```

---

### Task 2.3: Item parser (fn, struct, enum, impl, trait, use, mod)

**Files:**
- Modify: `libs/anyrc/src/parser.rs` — item parsing methods already stubbed
- Create: `libs/anyrc_tests/tests/02_parser_items.rs`

**Step 1: Write failing tests**

File: `libs/anyrc_tests/tests/02_parser_items.rs`

```rust
use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::ast::*;

fn parse(src: &str) -> Crate {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    parser.parse_crate()
}

#[test]
fn parse_simple_fn() {
    let krate = parse("fn add(a: i32, b: i32) -> i32 { a + b }");
    assert_eq!(krate.items.len(), 1);
    match &krate.items[0] {
        Item::Fn(f) => {
            assert_eq!(f.params.len(), 2);
            assert!(f.ret_ty.is_some());
            assert!(f.body.is_some());
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_generic_fn() {
    let krate = parse("fn max<T: Ord>(a: T, b: T) -> T { if a > b { a } else { b } }");
    match &krate.items[0] {
        Item::Fn(f) => {
            assert_eq!(f.generics.params.len(), 1);
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_struct() {
    let krate = parse("pub struct Point<T> { pub x: T, pub y: T }");
    match &krate.items[0] {
        Item::Struct(s) => {
            assert_eq!(s.fields.len(), 2);
            assert_eq!(s.generics.params.len(), 1);
        }
        _ => panic!("expected struct"),
    }
}

#[test]
fn parse_enum_with_data() {
    let krate = parse("enum Option<T> { Some(T), None }");
    match &krate.items[0] {
        Item::Enum(e) => {
            assert_eq!(e.variants.len(), 2);
        }
        _ => panic!("expected enum"),
    }
}

#[test]
fn parse_impl_block() {
    let krate = parse("impl Point { fn new() -> Self { Point { x: 0, y: 0 } } }");
    match &krate.items[0] {
        Item::Impl(i) => {
            assert_eq!(i.items.len(), 1);
            assert!(i.trait_ref.is_none());
        }
        _ => panic!("expected impl"),
    }
}

#[test]
fn parse_trait_impl() {
    let krate = parse("impl Display for Point { fn fmt(&self) -> String { todo!() } }");
    match &krate.items[0] {
        Item::Impl(i) => {
            assert!(i.trait_ref.is_some());
        }
        _ => panic!("expected impl"),
    }
}

#[test]
fn parse_trait_def() {
    let krate = parse("trait Drawable { fn draw(&self); fn color(&self) -> u32; }");
    match &krate.items[0] {
        Item::Trait(t) => {
            assert_eq!(t.items.len(), 2);
        }
        _ => panic!("expected trait"),
    }
}

#[test]
fn parse_use_tree() {
    let krate = parse("use std::collections::HashMap;");
    assert!(matches!(&krate.items[0], Item::Use(_)));
}

#[test]
fn parse_where_clause() {
    let krate = parse("fn foo<T>(x: T) where T: Clone + Debug { }");
    match &krate.items[0] {
        Item::Fn(f) => {
            assert!(!f.where_clause.predicates.is_empty());
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_unsafe_fn() {
    let krate = parse("unsafe fn dangerous() { }");
    match &krate.items[0] {
        Item::Fn(f) => assert!(f.is_unsafe),
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_extern_block() {
    let krate = parse(r#"extern "C" { fn malloc(size: usize) -> *mut u8; }"#);
    assert!(matches!(&krate.items[0], Item::ExternBlock(_)));
}
```

**Step 2: Implement item parsing in parser.rs**

Fill in the stubbed methods: `parse_fn`, `parse_struct`, `parse_enum`, `parse_impl`, `parse_trait`, `parse_use`, `parse_mod`, `parse_item`.

**Step 3: Run tests**

Run: `cd libs/anyrc_tests && cargo test --test 02_parser_items`
Expected: 11 tests PASS

**Step 4: Commit**

```bash
git add libs/anyrc/src/parser.rs libs/anyrc_tests/tests/02_parser_items.rs
git commit -m "feat(anyrc): implement item parser (fn, struct, enum, impl, trait, use)"
```

---

### Task 2.4: Pattern parser

**Files:**
- Modify: `libs/anyrc/src/parser.rs`
- Create: `libs/anyrc_tests/tests/02_parser_patterns.rs`

**Step 1: Write tests for pattern parsing**

Test `match` arms with: literal patterns, ident binding, wildcard `_`, tuple patterns `(a, b)`, struct patterns `Point { x, y }`, enum patterns `Some(x)`, reference patterns `&x`, or-patterns `A | B`, range patterns `0..=9`.

**Step 2: Implement `parse_pattern` in parser.rs**

**Step 3: Run tests, commit**

---

### Task 2.5: Type parser

**Files:**
- Modify: `libs/anyrc/src/parser.rs`
- Create: `libs/anyrc_tests/tests/02_parser_types.rs`

**Step 1: Write tests for type parsing**

Test: `i32`, `&'a mut T`, `*const u8`, `[T; 4]`, `&[u8]`, `(A, B, C)`, `fn(i32) -> bool`, `Vec<T>`, `Option<&'a str>`, `Self`.

**Step 2: Implement `parse_ty` in parser.rs**

**Step 3: Run tests, commit**

---

## Phase 3: Macro Expansion + HIR

### Task 3.1: macro_rules! expansion

**Files:**
- Create: `libs/anyrc/src/macros.rs`
- Create: `libs/anyrc_tests/tests/03_macros.rs`

**Step 1: Write tests**

```rust
#[test]
fn expand_simple_macro() {
    let src = r#"
        macro_rules! my_vec {
            ($($x:expr),*) => { { let mut v = Vec::new(); $(v.push($x);)* v } }
        }
        fn main() { let v = my_vec![1, 2, 3]; }
    "#;
    let krate = parse_and_expand(src);
    // After expansion, my_vec![1,2,3] should be replaced with the block
    // Verify the fn body contains the expanded code
}
```

Test matchers: `$x:expr`, `$x:ident`, `$x:ty`, `$x:pat`, `$x:tt`, `$x:literal`.
Test repetitions: `$(...)*`, `$(...)+`, `$(...),*`.
Test nested macros and multiple expansion rounds.

**Step 2: Implement macro expander**

- Match token trees against macro rules
- Substitute captures into the template
- Re-parse expanded token stream
- Iterate until no more macro calls (fixpoint)

**Step 3: Run tests, commit**

---

### Task 3.2: AST → HIR lowering

**Files:**
- Create: `libs/anyrc/src/hir.rs` (HIR data structures)
- Create: `libs/anyrc/src/hir_lower.rs` (lowering pass)
- Create: `libs/anyrc_tests/tests/03_hir.rs`

**Step 1: Write tests**

Test desugaring:
- `for` loops → `loop` + `match` + `Iterator::next`
- `?` operator → `match` + `return Err`
- `if let` → `match`
- `while let` → `loop` + `match`
- `x += 1` → `x = x + 1` (later resolved to trait call)
- `#[derive(Clone)]` → generated `impl Clone`

**Step 2: Implement HIR types + lowering**

HIR mirrors AST but with fewer variants. Each node gets `HirId(u32)`.

**Step 3: Run tests, commit**

---

## Phase 4: Name Resolution

### Task 4.1: Scope building and name resolution

**Files:**
- Create: `libs/anyrc/src/resolve.rs`
- Create: `libs/anyrc_tests/tests/04_resolve.rs`

**Step 1: Write tests**

```rust
#[test]
fn resolve_local_variable() {
    assert_resolves("fn main() { let x = 5; let y = x; }");
}

#[test]
fn resolve_fn_call() {
    assert_resolves("fn foo() {} fn main() { foo(); }");
}

#[test]
fn error_undefined_variable() {
    assert_resolve_error("fn main() { let x = y; }", "not found");
}

#[test]
fn resolve_use_import() {
    // Multi-module resolution
    assert_resolves("mod a { pub fn foo() {} } use a::foo; fn main() { foo(); }");
}

#[test]
fn resolve_impl_methods() {
    assert_resolves("struct S; impl S { fn new() -> S { S } } fn main() { S::new(); }");
}

#[test]
fn resolve_generic_params() {
    assert_resolves("fn id<T>(x: T) -> T { x }");
}

#[test]
fn resolve_enum_variants() {
    assert_resolves("enum E { A, B(i32) } fn main() { let x = E::A; let y = E::B(1); }");
}
```

**Step 2: Implement resolver**

- Build scope tree (nested scopes for blocks, functions, modules)
- Three namespace maps per scope (type, value, macro)
- Walk HIR, resolve all paths to `DefId`
- Fixpoint for `use` imports
- Report errors for unresolved names

**Step 3: Run tests, commit**

---

## Phase 5: Type Inference + Checking

### Task 5.1: Basic type inference (primitives, locals, arithmetic)

**Files:**
- Create: `libs/anyrc/src/typeck.rs`
- Create: `libs/anyrc/src/typeck/infer.rs` (inference engine)
- Create: `libs/anyrc/src/typeck/unify.rs` (unification)
- Create: `libs/anyrc_tests/tests/05_typeck.rs`

**Step 1: Write tests**

```rust
#[test]
fn infer_integer_literal() {
    assert_type_of("fn main() { let x = 42; }", "x", "i32"); // default
}

#[test]
fn infer_from_annotation() {
    assert_type_of("fn main() { let x: u64 = 42; }", "x", "u64");
}

#[test]
fn infer_binary_op() {
    assert_type_of("fn main() { let x = 1u32 + 2; }", "x", "u32");
}

#[test]
fn infer_fn_return() {
    assert_type_of("fn foo() -> bool { true } fn main() { let x = foo(); }", "x", "bool");
}

#[test]
fn error_type_mismatch() {
    assert_type_error("fn main() { let x: u32 = true; }", "mismatch");
}

#[test]
fn infer_reference() {
    assert_type_of("fn main() { let x = 5; let y = &x; }", "y", "&i32");
}

#[test]
fn infer_struct_fields() {
    assert_type_of(
        "struct S { x: i32 } fn main() { let s = S { x: 1 }; let v = s.x; }",
        "v", "i32"
    );
}
```

**Step 2: Implement type inference**

- Constraint-based: each expression generates type equations
- Unification algorithm: solve equations, substitute type variables
- Integer/float literal fallback: unresolved integer → `i32`, float → `f64`
- Function signature checking: args must match param types

**Step 3: Run tests, commit**

---

### Task 5.2: Generics + trait resolution

**Files:**
- Modify: `libs/anyrc/src/typeck/`
- Create: `libs/anyrc_tests/tests/05_typeck_generics.rs`

**Step 1: Write tests**

```rust
#[test]
fn infer_generic_fn() {
    assert_type_of(
        "fn id<T>(x: T) -> T { x } fn main() { let x = id(42u32); }",
        "x", "u32"
    );
}

#[test]
fn check_trait_bound() {
    assert_type_ok(
        "trait Foo { fn foo(&self); } fn bar<T: Foo>(x: T) { x.foo(); }"
    );
}

#[test]
fn error_missing_trait_bound() {
    assert_type_error(
        "trait Foo { fn foo(&self); } fn bar<T>(x: T) { x.foo(); }",
        "no method"
    );
}

#[test]
fn resolve_trait_impl() {
    assert_type_ok(
        "trait Foo { fn foo(&self) -> i32; }
         struct S;
         impl Foo for S { fn foo(&self) -> i32 { 42 } }
         fn main() { let s = S; s.foo(); }"
    );
}

#[test]
fn infer_generic_struct() {
    assert_type_of(
        "struct Pair<A, B> { first: A, second: B }
         fn main() { let p = Pair { first: 1u32, second: true }; let x = p.first; }",
        "x", "u32"
    );
}
```

**Step 2: Implement generics + traits**

- Generic instantiation: fresh type variables per call site
- Trait resolution: search impl blocks, check bounds
- Method resolution: self type → direct impls → trait impls → auto-deref
- Where clause checking

**Step 3: Run tests, commit**

---

### Task 5.3: Lifetime inference (pre-borrow-check)

**Files:**
- Modify: `libs/anyrc/src/typeck/`
- Create: `libs/anyrc_tests/tests/05_typeck_lifetimes.rs`

**Step 1: Write tests**

Test lifetime elision rules:
- `fn foo(x: &i32) -> &i32` → output lifetime = input lifetime
- `fn foo(&self, x: &i32) -> &i32` → output lifetime = `&self` lifetime
- Multiple inputs without `&self` → must annotate explicitly

**Step 2: Implement lifetime elision + basic lifetime tracking**

**Step 3: Run tests, commit**

---

## Phase 6: MIR + Borrow Checker

### Task 6.1: HIR → MIR lowering

**Files:**
- Create: `libs/anyrc/src/mir.rs` (MIR data structures)
- Create: `libs/anyrc/src/mir_build.rs` (HIR → MIR)
- Create: `libs/anyrc_tests/tests/06_mir.rs`

**Step 1: Write tests**

```rust
#[test]
fn mir_simple_fn() {
    let mir = build_mir("fn foo(x: i32) -> i32 { x + 1 }");
    assert_eq!(mir.basic_blocks.len(), 1); // single block, no branches
    assert!(matches!(mir.basic_blocks[0].terminator, Terminator::Return));
}

#[test]
fn mir_if_else() {
    let mir = build_mir("fn foo(x: bool) -> i32 { if x { 1 } else { 2 } }");
    assert!(mir.basic_blocks.len() >= 3); // condition, then, else (+ merge)
}

#[test]
fn mir_loop() {
    let mir = build_mir("fn foo() { let mut i = 0; while i < 10 { i = i + 1; } }");
    // Should have back-edge in CFG
    let has_back_edge = mir.basic_blocks.iter().any(|bb|
        matches!(&bb.terminator, Terminator::Goto(id) if id.0 < mir.basic_blocks.len())
    );
    assert!(has_back_edge);
}

#[test]
fn mir_match() {
    let mir = build_mir("enum E { A, B } fn foo(e: E) -> i32 { match e { E::A => 1, E::B => 2 } }");
    // SwitchInt on discriminant
    assert!(mir.basic_blocks.iter().any(|bb|
        matches!(&bb.terminator, Terminator::SwitchInt(_, _, _))
    ));
}

#[test]
fn mir_explicit_drops() {
    let mir = build_mir("fn foo() { let v = Vec::new(); }");
    // Should have Drop terminator for v at end of scope
    assert!(mir.basic_blocks.iter().any(|bb|
        matches!(&bb.terminator, Terminator::Drop { .. })
    ));
}
```

**Step 2: Implement MIR builder**

- Create basic blocks from control flow
- Lower expressions to statements with temporaries
- Insert StorageLive/StorageDead at scope boundaries
- Insert Drop terminators for types that implement Drop

**Step 3: Run tests, commit**

---

### Task 6.2: Borrow checker — NLL lifetime inference

**Files:**
- Create: `libs/anyrc/src/borrowck.rs`
- Create: `libs/anyrc/src/borrowck/regions.rs`
- Create: `libs/anyrc/src/borrowck/liveness.rs`
- Create: `libs/anyrc_tests/tests/06_borrowck.rs`

**Step 1: Write tests**

```rust
// These should PASS (no borrow errors)
#[test]
fn borrowck_simple_ref() {
    assert_borrowck_ok("fn foo() { let x = 5; let y = &x; let z = *y; }");
}

#[test]
fn borrowck_nll_early_drop() {
    // NLL: r's borrow ends before the mutation
    assert_borrowck_ok("fn foo() { let mut x = 5; let r = &x; let _ = *r; x = 6; }");
}

// These should FAIL
#[test]
fn borrowck_use_after_move() {
    assert_borrowck_error(
        "fn foo() { let s = String::new(); let t = s; let u = s; }",
        "use of moved value"
    );
}

#[test]
fn borrowck_mut_while_shared() {
    assert_borrowck_error(
        "fn foo() { let mut x = 5; let r = &x; x = 6; let _ = *r; }",
        "cannot assign.*also borrowed"
    );
}

#[test]
fn borrowck_two_mut_borrows() {
    assert_borrowck_error(
        "fn foo() { let mut x = 5; let a = &mut x; let b = &mut x; *a = 1; }",
        "cannot borrow.*mutably.*already borrowed"
    );
}

#[test]
fn borrowck_return_local_ref() {
    assert_borrowck_error(
        "fn foo() -> &i32 { let x = 5; &x }",
        "returns a reference to.*local"
    );
}
```

**Step 2: Implement NLL borrow checker**

1. Liveness analysis on MIR (which locals are live at each point)
2. Region inference: assign region variables, collect constraints, propagate
3. Borrow checking: at each point, verify active borrows are compatible
4. Move checking: dataflow for init/move state

**Step 3: Run tests, commit**

---

## Phase 7: MIR Optimizations

### Task 7.1: Optimization passes

**Files:**
- Create: `libs/anyrc/src/mir_opt.rs`
- Create: `libs/anyrc_tests/tests/07_mir_opt.rs`

**Step 1: Write tests**

```rust
#[test]
fn const_prop_folds_arithmetic() {
    let mir = build_and_optimize("fn foo() -> i32 { 3 + 4 }");
    // Should be a single return of constant 7
    assert_contains_constant(&mir, 7);
}

#[test]
fn dce_removes_unused_assignment() {
    let mir = build_and_optimize("fn foo() -> i32 { let x = 5; let y = 10; x }");
    // y=10 should be eliminated
    assert_no_assignment_to(&mir, "y");
}

#[test]
fn simplify_cfg_removes_empty_blocks() {
    let mir_before = build_mir("fn foo(x: bool) -> i32 { if x { 1 } else { 1 } }");
    let mir_after = optimize(mir_before);
    assert!(mir_after.basic_blocks.len() < mir_before.basic_blocks.len());
}
```

**Step 2: Implement passes**

- ConstProp: evaluate constant expressions at compile time
- SimplifyCFG: merge blocks, eliminate empty blocks, fold goto chains
- DeadCodeElim: remove assignments whose result is never read
- CopyProp: propagate copies (`_2 = _1` → replace `_2` with `_1`)
- InstCombine: algebraic simplifications (`x * 1` → `x`, etc.)
- Inlining: inline functions with body < 30 statements

**Step 3: Run tests, commit**

---

## Phase 8: x86_64 Codegen

### Task 8.1: x86_64 assembler (instruction encoding)

**Files:**
- Create: `libs/anyrc/src/codegen/mod.rs`
- Create: `libs/anyrc/src/codegen/x86asm.rs`
- Create: `libs/anyrc_tests/tests/08_x86asm.rs`

**Step 1: Write tests**

```rust
use anyrc::codegen::x86asm::{X86Assembler, Reg};

#[test]
fn encode_mov_reg_reg() {
    let mut asm = X86Assembler::new();
    asm.mov_rr(Reg::RAX, Reg::RBX);
    // REX.W + MOV r64, r/m64: 48 89 D8
    assert_eq!(asm.code(), &[0x48, 0x89, 0xD8]);
}

#[test]
fn encode_mov_reg_imm64() {
    let mut asm = X86Assembler::new();
    asm.mov_ri(Reg::RAX, 0x1234);
    // REX.W + MOV r64, imm64: 48 B8 34 12 00 00 00 00 00 00
    assert_eq!(asm.code()[0..2], [0x48, 0xB8]);
}

#[test]
fn encode_add_reg_reg() {
    let mut asm = X86Assembler::new();
    asm.add_rr(Reg::RAX, Reg::RCX);
    // REX.W + ADD r/m64, r64: 48 01 C8
    assert_eq!(asm.code(), &[0x48, 0x01, 0xC8]);
}

#[test]
fn encode_push_pop() {
    let mut asm = X86Assembler::new();
    asm.push(Reg::RBP);
    asm.pop(Reg::RBP);
    assert_eq!(asm.code(), &[0x55, 0x5D]);
}

#[test]
fn encode_ret() {
    let mut asm = X86Assembler::new();
    asm.ret();
    assert_eq!(asm.code(), &[0xC3]);
}

#[test]
fn encode_jmp_forward_label() {
    let mut asm = X86Assembler::new();
    let label = asm.new_label();
    asm.jmp(label);
    asm.nop(); // 1 byte
    asm.bind_label(label);
    asm.resolve_fixups();
    // JMP rel32 should jump over the NOP
    assert_eq!(asm.code()[0], 0xE9); // JMP rel32
}

#[test]
fn encode_call_rel() {
    let mut asm = X86Assembler::new();
    let label = asm.new_label();
    asm.bind_label(label);
    asm.nop();
    asm.call_rel(label);
    asm.resolve_fixups();
    assert_eq!(asm.code()[1], 0xE8); // CALL rel32
}

#[test]
fn encode_cmp_jcc() {
    let mut asm = X86Assembler::new();
    let label = asm.new_label();
    asm.cmp_rr(Reg::RAX, Reg::RBX);
    asm.jcc(anyrc::codegen::x86asm::CondCode::Equal, label);
    asm.bind_label(label);
    asm.resolve_fixups();
    // CMP: 48 39 D8, JE rel32: 0F 84 xx xx xx xx
    assert_eq!(asm.code()[0..3], [0x48, 0x39, 0xD8]);
    assert_eq!(asm.code()[3..5], [0x0F, 0x84]);
}

#[test]
fn encode_mov_mem() {
    let mut asm = X86Assembler::new();
    asm.mov_rm(Reg::RAX, Reg::RBP, -8); // mov rax, [rbp-8]
    // REX.W + MOV r64, r/m64 with disp8: 48 8B 45 F8
    assert_eq!(asm.code(), &[0x48, 0x8B, 0x45, 0xF8]);
}
```

**Step 2: Implement x86_64 assembler**

Cover:
- REX prefix generation (W, R, X, B bits)
- ModR/M byte encoding (reg-reg, reg-mem with displacement)
- SIB byte for complex addressing
- All registers: RAX-R15, XMM0-XMM15
- Instructions: MOV, ADD, SUB, IMUL, IDIV, AND, OR, XOR, SHL, SHR, SAR, CMP, TEST, LEA, PUSH, POP, CALL, RET, JMP, Jcc, NOP, SYSCALL, CDQE, CQO
- SSE: MOVSD, ADDSD, SUBSD, MULSD, DIVSD, UCOMISD, CVTSI2SD, CVTTSD2SI
- Label/fixup system for forward references
- Relocation entries for external symbols

Note: reference `libs/libcorevm/src/` for x86 encoding patterns — the emulator already decodes these instructions, encoding is the reverse.

**Step 3: Run tests, commit**

---

### Task 8.2: Register allocator (linear scan)

**Files:**
- Create: `libs/anyrc/src/codegen/regalloc.rs`
- Create: `libs/anyrc_tests/tests/08_regalloc.rs`

**Step 1: Write tests**

```rust
#[test]
fn alloc_simple_no_spill() {
    // 3 locals, plenty of registers
    let alloc = allocate_registers(&mir_with_n_locals(3));
    assert!(alloc.spills.is_empty());
}

#[test]
fn alloc_spills_when_registers_exhausted() {
    // More locals than available registers
    let alloc = allocate_registers(&mir_with_n_locals(20));
    assert!(!alloc.spills.is_empty());
}

#[test]
fn alloc_respects_calling_convention() {
    // Function args should be in RDI, RSI, RDX, RCX, R8, R9
    let alloc = allocate_registers(&mir_with_args(3));
    assert_eq!(alloc.reg_for(0), Some(Reg::RDI)); // arg 0
    assert_eq!(alloc.reg_for(1), Some(Reg::RSI)); // arg 1
    assert_eq!(alloc.reg_for(2), Some(Reg::RDX)); // arg 2
}
```

**Step 2: Implement linear scan**

1. Compute live ranges for all MIR locals
2. Sort by start point
3. Greedy register assignment (prefer caller-saved for short-lived)
4. Spill furthest-next-use when no register free
5. Track which callee-saved registers are used (for prologue/epilogue)

**Step 3: Run tests, commit**

---

### Task 8.3: MIR → x86_64 code generation

**Files:**
- Create: `libs/anyrc/src/codegen/emit.rs`
- Create: `libs/anyrc_tests/tests/08_codegen.rs`

**Step 1: Write tests**

```rust
#[test]
fn codegen_return_constant() {
    let code = compile_fn("fn foo() -> i32 { 42 }");
    // Execute the generated code and check return value
    let result = execute_code::<i32>(&code);
    assert_eq!(result, 42);
}

#[test]
fn codegen_add() {
    let code = compile_fn("fn add(a: i32, b: i32) -> i32 { a + b }");
    let result = execute_code_with_args::<i32>(&code, &[5i64 as u64, 3i64 as u64]);
    assert_eq!(result, 8);
}

#[test]
fn codegen_if_else() {
    let code = compile_fn("fn abs(x: i32) -> i32 { if x < 0 { -x } else { x } }");
    let r1 = execute_code_with_args::<i32>(&code, &[(-5i32) as u64]);
    let r2 = execute_code_with_args::<i32>(&code, &[5u64]);
    assert_eq!(r1, 5);
    assert_eq!(r2, 5);
}

#[test]
fn codegen_loop() {
    let code = compile_fn(
        "fn sum(n: i32) -> i32 { let mut s = 0; let mut i = 1; while i <= n { s = s + i; i = i + 1; } s }"
    );
    let result = execute_code_with_args::<i32>(&code, &[10u64]);
    assert_eq!(result, 55);
}
```

**Test execution helper:** Use `mmap` with `PROT_READ|PROT_WRITE|PROT_EXEC` to create executable memory, copy generated code, cast to fn pointer, and call. This works on the Linux host.

**Step 2: Implement MIR → x86_64 translation**

- Function prologue/epilogue (push callee-saved, allocate stack frame)
- Statement lowering (Assign, StorageLive/Dead)
- Terminator lowering (Goto, SwitchInt, Call, Return, Drop)
- Operand loading (Copy/Move from register or stack, Constant)
- Rvalue codegen (BinaryOp, UnaryOp, Ref, Cast, Aggregate)

**Step 3: Run tests, commit**

---

### Task 8.4: Monomorphization

**Files:**
- Create: `libs/anyrc/src/mono.rs`
- Create: `libs/anyrc_tests/tests/08_mono.rs`

**Step 1: Write tests**

```rust
#[test]
fn mono_generic_fn() {
    let instances = monomorphize(
        "fn id<T>(x: T) -> T { x } fn main() { id(1u32); id(true); }"
    );
    // Should produce: id__u32, id__bool, main
    assert_eq!(instances.len(), 3);
}

#[test]
fn mono_generic_struct() {
    let instances = monomorphize(
        "struct Pair<T> { a: T, b: T }
         fn make<T>(x: T) -> Pair<T> { Pair { a: x, b: x } }
         fn main() { make(1i32); }"
    );
    assert!(instances.iter().any(|i| i.name.contains("make") && i.name.contains("i32")));
}

#[test]
fn mono_transitive() {
    let instances = monomorphize(
        "fn a<T>(x: T) -> T { b(x) }
         fn b<T>(x: T) -> T { x }
         fn main() { a(1u32); }"
    );
    // a__u32 calls b__u32, both must be instantiated
    assert!(instances.len() >= 3);
}
```

**Step 2: Implement monomorphization**

- Worklist algorithm starting from `main()`
- For each function call with generic args, create concrete instance
- Substitute type parameters throughout MIR body
- Track seen instances to avoid duplicates
- Generate mangled symbol names

**Step 3: Run tests, commit**

---

## Phase 9: ELF Output + Linker

### Task 9.1: ELF object file generation

**Files:**
- Create: `libs/anyrc/src/linker/mod.rs`
- Create: `libs/anyrc/src/linker/elf.rs`
- Create: `libs/anyrc_tests/tests/09_elf.rs`

**Step 1: Write tests**

```rust
#[test]
fn generate_valid_elf_object() {
    let obj = compile_to_object("fn foo() -> i32 { 42 }");
    // Check ELF magic
    assert_eq!(&obj[0..4], b"\x7fELF");
    // Check it's 64-bit, little-endian, relocatable
    assert_eq!(obj[4], 2);  // ELFCLASS64
    assert_eq!(obj[5], 1);  // ELFDATA2LSB
    assert_eq!(obj[16], 1); // ET_REL
}

#[test]
fn elf_has_text_section() {
    let obj = compile_to_object("fn foo() -> i32 { 42 }");
    let sections = parse_elf_sections(&obj);
    assert!(sections.iter().any(|s| s.name == ".text"));
}

#[test]
fn elf_has_symbol_for_fn() {
    let obj = compile_to_object("fn foo() -> i32 { 42 }");
    let symbols = parse_elf_symbols(&obj);
    assert!(symbols.iter().any(|s| s.name.contains("foo")));
}
```

**Step 2: Implement ELF writer**

Write ELF64 object files with sections: `.text`, `.rodata`, `.data`, `.bss`, `.symtab`, `.strtab`, `.rela.text`, `.shstrtab`.

**Step 3: Run tests, commit**

---

### Task 9.2: Linker

**Files:**
- Create: `libs/anyrc/src/linker/link.rs`
- Create: `libs/anyrc_tests/tests/09_linker.rs`

**Step 1: Write tests**

```rust
#[test]
fn link_single_object_to_executable() {
    let exe = compile_and_link("fn main() -> i32 { 0 }");
    assert_eq!(&exe[0..4], b"\x7fELF");
    assert_eq!(exe[16], 2); // ET_EXEC
    // Should be executable on Linux
}

#[test]
fn link_resolves_cross_object_calls() {
    let obj1 = compile_to_object("pub fn helper() -> i32 { 42 }");
    let obj2 = compile_to_object("extern { fn helper() -> i32; } fn main() -> i32 { unsafe { helper() } }");
    let exe = link(&[obj1, obj2]);
    // Execute and verify
    let status = run_elf(&exe);
    assert_eq!(status, 42);
}

#[test]
fn link_with_entry_point() {
    let exe = compile_and_link("fn main() {}");
    let entry = parse_elf_entry(&exe);
    assert_ne!(entry, 0); // has valid entry point
}
```

**Step 2: Implement linker**

1. Parse input .o files
2. Symbol resolution (match undefined with defined)
3. Section merging (concatenate .text, .rodata, etc.)
4. Relocation (R_X86_64_PC32, R_X86_64_PLT32, R_X86_64_64)
5. Program headers (PT_LOAD segments)
6. Generate `_start` wrapper that calls `main()` and does `exit(retval)`

**Step 3: Run tests, commit**

---

## Phase 10: Driver + CLI

### Task 10.1: Compilation driver

**Files:**
- Create: `libs/anyrc/src/driver.rs`
- Modify: `bin/anyrc/src/main.rs`

**Step 1: Write integration test**

File: `libs/anyrc_tests/tests/10_integration.rs`

```rust
#[test]
fn compile_hello_world() {
    let src = r#"
        extern "C" {
            fn write(fd: i32, buf: *const u8, len: usize) -> isize;
            fn exit(code: i32) -> !;
        }

        fn main() {
            unsafe {
                let msg = "Hello, World!\n";
                write(1, msg.as_ptr(), msg.len());
                exit(0);
            }
        }
    "#;
    let exe = compile_full(src, Target::LinuxX86_64);
    let output = run_and_capture(&exe);
    assert_eq!(output.stdout, "Hello, World!\n");
    assert_eq!(output.status, 0);
}
```

**Step 2: Implement driver**

Orchestrate the full pipeline:
1. Read source file
2. Lex → Parse → Expand macros → Lower to HIR
3. Resolve names → Type check → Lower to MIR
4. Borrow check → Optimize MIR → Monomorphize
5. Codegen → ELF output → Link

**Step 3: Implement CLI argument parsing in main.rs**

Parse all flags from the design doc (`-o`, `--target`, `--emit`, `--crate-type`, etc.)

**Step 4: Run tests, commit**

---

### Task 10.2: Crate metadata

**Files:**
- Create: `libs/anyrc/src/metadata.rs`
- Create: `libs/anyrc_tests/tests/10_metadata.rs`

**Step 1: Write tests**

```rust
#[test]
fn write_and_read_crate_metadata() {
    let meta = compile_crate_metadata(
        "pub fn add(a: i32, b: i32) -> i32 { a + b }
         pub struct Point { pub x: i32, pub y: i32 }"
    );
    // Read it back
    let loaded = CrateMetadata::deserialize(&meta);
    assert!(loaded.has_fn("add"));
    assert!(loaded.has_struct("Point"));
}

#[test]
fn cross_crate_generic_instantiation() {
    let lib_obj = compile_crate(
        "pub fn id<T>(x: T) -> T { x }",
        "--crate-type lib --crate-name mylib"
    );
    let exe = compile_and_link_with_extern(
        "fn main() { let x = mylib::id(42i32); }",
        &[("mylib", &lib_obj)]
    );
    let status = run_elf(&exe);
    assert_eq!(status, 0);
}
```

**Step 2: Implement metadata serialization/deserialization**

Binary format in `.anyrc_meta` ELF section. Contains:
- Function signatures + generic bodies (for monomorphization)
- Struct/enum/trait definitions
- Impl blocks
- Macro definitions

**Step 3: Run tests, commit**

---

## Phase 11: core/alloc Compilation

### Task 11.1: Lang items + compiler builtins

**Files:**
- Create: `libs/anyrc/src/lang_items.rs`
- Create: `libs/anyrc/src/compiler_rt.rs`

**Step 1: Implement lang item recognition**

anyrc must recognize `#[lang = "..."]` attributes for:
- `sized`, `copy`, `clone`, `drop`, `sync`, `send`
- `fn`, `fn_mut`, `fn_once`
- `eq`, `partial_eq`, `ord`, `partial_ord`
- `add`, `sub`, `mul`, `div`, `rem`, `neg`, `not` (and assign variants)
- `index`, `index_mut`
- `deref`, `deref_mut`
- `iterator`, `into_iterator`
- `option`, `result`
- `box_free`, `exchange_malloc`

**Step 2: Implement compiler_rt**

Provide: `memcpy`, `memset`, `memmove`, `__muloti4`, `__divti3`, etc.

**Step 3: Test with minimal core subset, commit**

---

### Task 11.2: Compile official core + alloc

**Step 1: Get Rust source**

Download matching `library/core/` and `library/alloc/` source. Place in sysroot.

**Step 2: Iteratively fix compilation**

This is an iterative process:
1. Run `anyrc --crate-type lib --crate-name core library/core/lib.rs`
2. Fix errors (missing features, unsupported syntax)
3. Add `#[cfg(anyrc)]` gates for problematic code
4. Repeat until core compiles

Same for alloc (depends on core).

**Step 3: Verify by compiling a program that uses Vec, Box, String**

**Step 4: Commit**

---

## Phase 12: Self-Hosting + anyOS Integration

### Task 12.1: anyrc compiles itself on Linux

**Step 1: Compile anyrc with anyrc**

```bash
# Stage 1: build anyrc with rustc (already done)
cargo build -p anyrc-cli --release

# Stage 2: build anyrc with anyrc
./target/release/anyrc --crate-type lib libs/anyrc/src/lib.rs -o anyrc_stage2.o \
    --extern core=libcore.o --extern alloc=liballoc.o
./target/release/anyrc bin/anyrc/src/main.rs -o anyrc_stage2 \
    --extern anyrc=anyrc_stage2.o --extern core=libcore.o --extern alloc=liballoc.o

# Stage 3: build anyrc with stage2 anyrc
./anyrc_stage2 --crate-type lib libs/anyrc/src/lib.rs -o anyrc_stage3.o ...
./anyrc_stage2 bin/anyrc/src/main.rs -o anyrc_stage3 ...

# Verify: stage2 and stage3 should produce identical output
diff anyrc_stage2 anyrc_stage3
```

**Step 2: Fix any differences until stage2 == stage3**

**Step 3: Commit**

---

### Task 12.2: anyrc on anyOS

**Step 1: Cross-compile anyrc for anyOS**

```bash
cargo build -p anyrc-cli --target x86_64-anyos-user.json --release
```

**Step 2: Package into sysroot**

- Binary: `sysroot/usr/bin/anyrc`
- core/alloc source: `sysroot/usr/lib/anyrc/src/`
- Pre-compiled libcore.o + liballoc.o: `sysroot/usr/lib/anyrc/lib/`

**Step 3: Test on anyOS**

Boot anyOS, open terminal:
```
$ anyrc hello.rs -o hello
$ ./hello
Hello from anyOS!
```

**Step 4: Self-hosting test on anyOS**

```
$ anyrc --crate-type lib /usr/lib/anyrc/src/anyrc/lib.rs -o anyrc.o ...
$ anyrc /usr/lib/anyrc/src/anyrc-cli/main.rs -o anyrc2 ...
$ ./anyrc2 hello.rs -o hello2
$ ./hello2
Hello from anyOS!
```

**Step 5: Commit + celebrate**

---

### Task 12.3: Compile anyOS kernel with anyrc

**Step 1: Build kernel with anyrc**

```
anyrc --crate-type lib --crate-name core ... -o libcore.o
anyrc --crate-type lib --crate-name alloc ... -o liballoc.o
anyrc --crate-type bin --crate-name anyos_kernel kernel/src/main.rs \
    --extern core=libcore.o --extern alloc=liballoc.o \
    --target x86_64-anyos --cfg 'feature="x86_64"' \
    -o kernel.o
anyld -T kernel/link.ld kernel.o libcore.o liballoc.o -o anyos_kernel.elf
```

**Step 2: Boot the kernel compiled by anyrc**

Verify it boots and runs correctly in QEMU/corevm.

**Step 3: Commit — full circle achieved**

---

## Summary of Phases

| Phase | Tasks | Milestone |
|-------|-------|-----------|
| 1. Scaffolding + Lexer | 1.1–1.4 | Foundation |
| 2. Parser + AST | 2.1–2.5 | Full Rust subset parsing |
| 3. Macros + HIR | 3.1–3.2 | Desugared IR |
| 4. Name Resolution | 4.1 | All paths resolved |
| 5. Type System | 5.1–5.3 | Full type inference + generics + lifetimes |
| 6. MIR + Borrow Check | 6.1–6.2 | Memory safety verified |
| 7. MIR Optimizations | 7.1 | Optimized IR |
| 8. Codegen | 8.1–8.4 | Working x86_64 machine code |
| 9. ELF + Linker | 9.1–9.2 | Standalone executables |
| 10. Driver + CLI | 10.1–10.2 | **M1: First "Hello World" ELF** |
| 11. core/alloc | 11.1–11.2 | **M3+M4: core + alloc compiled** |
| 12. Self-Hosting | 12.1–12.3 | **M5–M7: Self-hosting + kernel compilation** |
