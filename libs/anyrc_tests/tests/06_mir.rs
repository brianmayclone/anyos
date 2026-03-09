use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::TypeChecker;
use anyrc::mir_build::MirBuilder;
use anyrc::mir::*;

fn build_mir(src: &str) -> Vec<MirBody> {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let typeck_result = checker.check_crate(&hir);

    MirBuilder::build_crate(&interner, &resolve_result, &typeck_result, &hir)
}

fn build_fn_mir(src: &str) -> MirBody {
    build_mir(src).into_iter().next().unwrap()
}

#[test]
fn mir_simple_return() {
    let mir = build_fn_mir("fn foo() -> i32 { 42 }");
    assert!(!mir.basic_blocks.is_empty());
    assert!(matches!(&mir.basic_blocks.last().unwrap().terminator, Terminator::Return));
}

#[test]
fn mir_binary_op() {
    let mir = build_fn_mir("fn foo(a: i32, b: i32) -> i32 { a + b }");
    let has_binop = mir.basic_blocks.iter().any(|bb|
        bb.statements.iter().any(|s| matches!(&s.kind, StatementKind::Assign(_, Rvalue::BinaryOp(..))))
    );
    assert!(has_binop);
}

#[test]
fn mir_if_else_creates_branches() {
    let mir = build_fn_mir("fn foo(x: bool) -> i32 { if x { 1 } else { 2 } }");
    assert!(mir.basic_blocks.len() >= 3, "got {} blocks", mir.basic_blocks.len());
    let has_switch = mir.basic_blocks.iter().any(|bb|
        matches!(&bb.terminator, Terminator::SwitchInt { .. })
    );
    assert!(has_switch);
}

#[test]
fn mir_loop() {
    let mir = build_fn_mir("fn foo() { let mut i: i32 = 0; loop { i = i + 1; if i > 10 { break; } } }");
    let has_back_edge = mir.basic_blocks.iter().enumerate().any(|(idx, bb)|
        match &bb.terminator {
            Terminator::Goto(target) => target.0 <= idx,
            Terminator::SwitchInt { targets, default, .. } => {
                targets.iter().any(|(_, t)| t.0 <= idx) || default.0 <= idx
            }
            _ => false,
        }
    );
    assert!(has_back_edge, "loop should have a back edge");
}

#[test]
fn mir_fn_call() {
    let mir = build_mir("fn add(a: i32, b: i32) -> i32 { a + b } fn main() { add(1, 2); }");
    let main_mir = &mir[1];
    let has_call = main_mir.basic_blocks.iter().any(|bb|
        matches!(&bb.terminator, Terminator::Call { .. })
    );
    assert!(has_call);
}

#[test]
fn mir_let_binding() {
    let mir = build_fn_mir("fn foo() { let x: i32 = 5; let y: i32 = x + 1; }");
    assert!(mir.locals.len() >= 2);
}

#[test]
fn mir_struct_construction() {
    let bodies = build_mir(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 1, y: 2 }; }
    "#);
    let main_mir = bodies.iter().find(|b|
        b.locals.len() > 0
    ).unwrap();
    let has_aggregate = main_mir.basic_blocks.iter().any(|bb|
        bb.statements.iter().any(|s| matches!(&s.kind, StatementKind::Assign(_, Rvalue::Aggregate(..))))
    );
    assert!(has_aggregate);
}

#[test]
fn mir_reference() {
    let mir = build_fn_mir("fn foo() { let x: i32 = 5; let y: &i32 = &x; }");
    let has_ref = mir.basic_blocks.iter().any(|bb|
        bb.statements.iter().any(|s| matches!(&s.kind, StatementKind::Assign(_, Rvalue::Ref(..))))
    );
    assert!(has_ref);
}
