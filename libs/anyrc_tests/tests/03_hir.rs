use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::hir::*;

fn lower(src: &str) -> HirCrate {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut ctx = LoweringContext::new(&mut interner);
    ctx.lower_crate(&krate)
}

#[test]
fn lower_simple_fn() {
    let hir = lower("fn main() { let x = 5; }");
    assert_eq!(hir.items.len(), 1);
    assert!(matches!(&hir.items[0].kind, HirItemKind::Fn(_)));
}

#[test]
fn hir_nodes_have_unique_ids() {
    let hir = lower("fn a() {} fn b() {}");
    assert_ne!(hir.items[0].id, hir.items[1].id);
}

#[test]
fn lower_while_to_loop() {
    let hir = lower("fn foo() { while true { } }");
    match &hir.items[0].kind {
        HirItemKind::Fn(f) => {
            let body = f.body.as_ref().unwrap();
            // while should be lowered to loop
            assert!(body.stmts.iter().any(|s| matches!(s, HirStmt::Expr(HirExpr { kind: HirExprKind::Loop(_, _), .. })
                                                        | HirStmt::Semi(HirExpr { kind: HirExprKind::Loop(_, _), .. }, _))));
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn lower_struct() {
    let hir = lower("struct Point { x: i32, y: i32 }");
    assert!(matches!(&hir.items[0].kind, HirItemKind::Struct(_)));
}

#[test]
fn lower_enum() {
    let hir = lower("enum Color { Red, Green, Blue }");
    assert!(matches!(&hir.items[0].kind, HirItemKind::Enum(_)));
}

#[test]
fn lower_impl() {
    let hir = lower("impl Point { fn new() -> Point { Point { x: 0, y: 0 } } }");
    assert!(matches!(&hir.items[0].kind, HirItemKind::Impl(_)));
}

#[test]
fn lower_trait() {
    let hir = lower("trait Foo { fn foo(&self); }");
    assert!(matches!(&hir.items[0].kind, HirItemKind::Trait(_)));
}
