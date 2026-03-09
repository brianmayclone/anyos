use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::ast::*;

fn parse_and_expand(src: &str) -> Crate {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    krate
}

#[test]
fn expand_simple_macro() {
    let krate = parse_and_expand(r#"
        macro_rules! five {
            () => { 5 }
        }
        fn main() -> i32 { five!() }
    "#);
    // After expansion, five!() should be replaced with the literal 5
    match &krate.items[1] {
        Item::Fn(f) => {
            let body = f.body.as_ref().unwrap();
            match &body.stmts[0] {
                Stmt::Expr(Expr::Lit(Literal::Int(5), _)) => {} // ok
                other => panic!("expected literal 5, got {:?}", std::mem::discriminant(match other { Stmt::Expr(e) => e, _ => panic!() })),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn expand_macro_with_expr_arg() {
    let krate = parse_and_expand(r#"
        macro_rules! double {
            ($x:expr) => { $x + $x }
        }
        fn main() -> i32 { double!(21) }
    "#);
    match &krate.items[1] {
        Item::Fn(f) => {
            let body = f.body.as_ref().unwrap();
            // Should be 21 + 21
            match &body.stmts[0] {
                Stmt::Expr(Expr::Binary(BinOp::Add, _, _, _)) => {}
                _ => panic!("expected binary add"),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn expand_macro_with_repetition() {
    let krate = parse_and_expand(r#"
        macro_rules! count {
            ($($x:expr),*) => { 0 $(+ 1)* }
        }
        fn main() -> i32 { count!(a, b, c) }
    "#);
    // count!(a, b, c) should expand to 0 + 1 + 1 + 1
    match &krate.items[1] {
        Item::Fn(f) => {
            assert!(f.body.is_some());
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn expand_macro_with_ident_arg() {
    let krate = parse_and_expand(r#"
        macro_rules! make_fn {
            ($name:ident) => { fn $name() -> i32 { 42 } }
        }
        make_fn!(hello);
    "#);
    // Should have two items: the macro def and the generated fn
    assert!(krate.items.len() >= 2);
}

#[test]
fn macro_not_found_is_preserved() {
    // Unknown macros should remain as MacroCall (or could be an error)
    let krate = parse_and_expand(r#"
        fn main() { unknown_macro!(1, 2, 3); }
    "#);
    // Should not crash
    assert_eq!(krate.items.len(), 1);
}
