use anyrc::lexer::{Lexer, TokenKind, Keyword};
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
    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[0], TokenKind::Kw(Keyword::Fn));
    assert!(matches!(tokens[1], TokenKind::Ident(_)));
    assert_eq!(tokens[2], TokenKind::LParen);
    assert_eq!(tokens[3], TokenKind::RParen);
}

#[test]
fn lex_let_binding() {
    let tokens = lex("let mut x: u32 = 42;");
    assert_eq!(tokens[0], TokenKind::Kw(Keyword::Let));
    assert_eq!(tokens[1], TokenKind::Kw(Keyword::Mut));
    assert!(matches!(tokens[2], TokenKind::Ident(_)));
    assert_eq!(tokens[3], TokenKind::Colon);
    assert!(matches!(tokens[4], TokenKind::Ident(_)));
    assert_eq!(tokens[5], TokenKind::Eq);
    assert_eq!(tokens[6], TokenKind::IntLit(42, None));
    assert_eq!(tokens[7], TokenKind::Semi);
    assert_eq!(tokens.len(), 8);
}

#[test]
fn lex_operators() {
    let tokens = lex("a + b == c && d != e");
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
fn lex_string_escapes() {
    let tokens = lex(r#""hello\nworld\t!""#);
    match &tokens[0] {
        TokenKind::StringLit(s) => assert_eq!(s, "hello\nworld\t!"),
        _ => panic!("expected string literal"),
    }
}

#[test]
fn lex_integer_literals() {
    let tokens = lex("42 0xFF 0b1010 1_000_000 0o77");
    assert_eq!(tokens.len(), 5);
    assert_eq!(tokens[0], TokenKind::IntLit(42, None));
    assert_eq!(tokens[1], TokenKind::IntLit(255, None));
    assert_eq!(tokens[2], TokenKind::IntLit(10, None));
    assert_eq!(tokens[3], TokenKind::IntLit(1_000_000, None));
    assert_eq!(tokens[4], TokenKind::IntLit(63, None));
}

#[test]
fn lex_float_literal() {
    let tokens = lex("3.14 1e10 2.5e-3");
    assert_eq!(tokens.len(), 3);
    assert!(matches!(tokens[0], TokenKind::FloatLit(f) if (f - 3.14).abs() < 1e-10));
    assert!(matches!(tokens[1], TokenKind::FloatLit(f) if (f - 1e10).abs() < 1.0));
    assert!(matches!(tokens[2], TokenKind::FloatLit(f) if (f - 2.5e-3).abs() < 1e-10));
}

#[test]
fn lex_lifetime() {
    let tokens = lex("&'a mut T");
    assert_eq!(tokens[0], TokenKind::Amp);
    assert!(matches!(tokens[1], TokenKind::Lifetime(_)));
    assert_eq!(tokens[2], TokenKind::Kw(Keyword::Mut));
    assert!(matches!(tokens[3], TokenKind::Ident(_)));
}

#[test]
fn lex_char_literal() {
    let tokens = lex("'a' '\\n' '\\x41'");
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[0], TokenKind::CharLit('a'));
    assert_eq!(tokens[1], TokenKind::CharLit('\n'));
    assert_eq!(tokens[2], TokenKind::CharLit('\x41'));
}

#[test]
fn lex_comments_skipped() {
    let tokens = lex("a // comment\nb");
    assert_eq!(tokens.len(), 2);
}

#[test]
fn lex_block_comment() {
    let tokens = lex("a /* block */ b");
    assert_eq!(tokens.len(), 2);
}

#[test]
fn lex_nested_block_comment() {
    let tokens = lex("a /* outer /* inner */ still comment */ b");
    assert_eq!(tokens.len(), 2);
}

#[test]
fn lex_arrows() {
    let tokens = lex("-> =>");
    assert_eq!(tokens[0], TokenKind::Arrow);
    assert_eq!(tokens[1], TokenKind::FatArrow);
}

#[test]
fn lex_double_colon() {
    let tokens = lex("std::mem");
    assert_eq!(tokens.len(), 3);
    assert_eq!(tokens[1], TokenKind::ColonColon);
}

#[test]
fn lex_ranges() {
    let tokens = lex("0..10 0..=9");
    assert_eq!(tokens[1], TokenKind::DotDot);
    assert_eq!(tokens[4], TokenKind::DotDotEq);
}

#[test]
fn lex_compound_assign() {
    let tokens = lex("+= -= *= /= %= &= |= ^= <<= >>=");
    assert_eq!(tokens[0], TokenKind::PlusEq);
    assert_eq!(tokens[1], TokenKind::MinusEq);
    assert_eq!(tokens[2], TokenKind::StarEq);
    assert_eq!(tokens[3], TokenKind::SlashEq);
    assert_eq!(tokens[4], TokenKind::PercentEq);
    assert_eq!(tokens[5], TokenKind::AmpEq);
    assert_eq!(tokens[6], TokenKind::PipeEq);
    assert_eq!(tokens[7], TokenKind::CaretEq);
    assert_eq!(tokens[8], TokenKind::ShlEq);
    assert_eq!(tokens[9], TokenKind::ShrEq);
}

#[test]
fn lex_shifts() {
    let tokens = lex("<< >>");
    assert_eq!(tokens[0], TokenKind::Shl);
    assert_eq!(tokens[1], TokenKind::Shr);
}

#[test]
fn lex_raw_string() {
    let tokens = lex(r###"r#"hello "world""#"###);
    match &tokens[0] {
        TokenKind::StringLit(s) => assert_eq!(s, r#"hello "world""#),
        _ => panic!("expected string literal"),
    }
}

#[test]
fn lex_all_keywords() {
    let tokens = lex("fn let mut pub struct enum impl trait type use mod crate self Self super as in for while loop if else match return break continue where const static unsafe extern ref move true false");
    for tok in &tokens {
        assert!(matches!(tok, TokenKind::Kw(_)), "expected keyword, got {:?}", tok);
    }
}

#[test]
fn lex_braces_brackets() {
    let tokens = lex("{ } [ ] ( )");
    assert_eq!(tokens[0], TokenKind::LBrace);
    assert_eq!(tokens[1], TokenKind::RBrace);
    assert_eq!(tokens[2], TokenKind::LBracket);
    assert_eq!(tokens[3], TokenKind::RBracket);
    assert_eq!(tokens[4], TokenKind::LParen);
    assert_eq!(tokens[5], TokenKind::RParen);
}

#[test]
fn lex_question_hash_at() {
    let tokens = lex("? # @");
    assert_eq!(tokens[0], TokenKind::Question);
    assert_eq!(tokens[1], TokenKind::Hash);
    assert_eq!(tokens[2], TokenKind::At);
}
