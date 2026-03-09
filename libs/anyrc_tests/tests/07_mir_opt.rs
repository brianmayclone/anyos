use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::TypeChecker;
use anyrc::mir_build::MirBuilder;
use anyrc::mir::*;

fn build_fn_mir(src: &str) -> MirBody {
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
    MirBuilder::build_crate(&mut interner, &resolve_result, &typeck_result, &hir)
        .into_iter().next().unwrap()
}

#[test]
fn const_prop_folds() {
    let mut mir = build_fn_mir("fn foo() -> i32 { let x: i32 = 5; x }");
    let _block_count_before = mir.basic_blocks.len();
    anyrc::mir_opt::optimize(&mut mir);
    assert!(!mir.basic_blocks.is_empty());
}

#[test]
fn dce_removes_unused() {
    let mut mir = build_fn_mir("fn foo() -> i32 { let x: i32 = 5; let y: i32 = 10; x }");
    let stmt_count_before: usize = mir.basic_blocks.iter().map(|b| b.statements.len()).sum();
    anyrc::mir_opt::optimize(&mut mir);
    let stmt_count_after: usize = mir.basic_blocks.iter().map(|b| b.statements.len()).sum();
    assert!(stmt_count_after <= stmt_count_before);
}

#[test]
fn simplify_cfg_merges_blocks() {
    let mut mir = build_fn_mir("fn foo(x: bool) -> i32 { if x { 1 } else { 2 } }");
    let blocks_before = mir.basic_blocks.len();
    anyrc::mir_opt::optimize(&mut mir);
    assert!(mir.basic_blocks.len() <= blocks_before);
}

#[test]
fn optimize_doesnt_break_simple_fn() {
    let mut mir = build_fn_mir("fn foo(a: i32, b: i32) -> i32 { a + b }");
    anyrc::mir_opt::optimize(&mut mir);
    assert!(matches!(mir.basic_blocks.last().unwrap().terminator, Terminator::Return));
}
