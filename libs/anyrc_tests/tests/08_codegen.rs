use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::TypeChecker;
use anyrc::mir_build::MirBuilder;
use anyrc::codegen::regalloc;
use anyrc::codegen::emit::CodeEmitter;

fn compile_fn(src: &str) -> Vec<u8> {
    let (code, _) = compile_fn_with_relocs(src);
    code
}

fn compile_fn_with_relocs(src: &str) -> (Vec<u8>, Vec<anyrc::codegen::x86asm::Relocation>) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new();
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let typeck_result = checker.check_crate(&hir);
    let bodies = MirBuilder::build_crate(&interner, &resolve_result, &typeck_result, &hir);
    let body = &bodies[0];
    let alloc = regalloc::allocate(body);
    CodeEmitter::emit_fn(body, &alloc, &interner)
}

#[test]
fn codegen_simple_return() {
    let code = compile_fn("fn foo() -> i32 { 42 }");
    assert!(!code.is_empty());
}

#[test]
fn codegen_binary_add() {
    let code = compile_fn("fn foo(a: i32, b: i32) -> i32 { a + b }");
    assert!(!code.is_empty());
}

#[test]
fn codegen_if_else() {
    let code = compile_fn("fn foo(x: bool) -> i32 { if x { 1 } else { 2 } }");
    assert!(!code.is_empty());
}

#[test]
fn codegen_loop_and_break() {
    let code = compile_fn("fn foo() -> i32 { let mut i: i32 = 0; loop { i = i + 1; if i > 10 { break; } } i }");
    assert!(!code.is_empty());
}

#[test]
fn codegen_fn_call() {
    let (code, _relocs) = compile_fn_with_relocs("fn bar() -> i32 { 0 } fn foo() -> i32 { bar() }");
    assert!(!code.is_empty());
}

#[test]
fn codegen_struct_and_field() {
    let code = compile_fn(r#"
        struct Point { x: i32, y: i32 }
        fn foo() -> i32 { let p = Point { x: 10, y: 20 }; p.x }
    "#);
    assert!(!code.is_empty());
}
