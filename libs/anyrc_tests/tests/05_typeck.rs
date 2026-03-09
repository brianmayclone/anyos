use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::{TypeChecker, TypeckResult, TyKind, IntTy, UintTy, FloatTy};

fn typecheck(src: &str) -> (TypeckResult, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let result = checker.check_crate(&hir);
    (result, interner)
}

fn assert_type_ok(src: &str) {
    let (result, _) = typecheck(src);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

fn assert_type_error(src: &str, expected_msg: &str) {
    let (result, _) = typecheck(src);
    assert!(!result.errors.is_empty(), "expected error containing '{}'", expected_msg);
    assert!(result.errors.iter().any(|e| e.message.to_lowercase().contains(&expected_msg.to_lowercase())),
        "expected error containing '{}', got: {:?}", expected_msg,
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn infer_integer_literal() {
    assert_type_ok("fn main() { let x: i32 = 42; }");
}

#[test]
fn infer_from_annotation() {
    assert_type_ok("fn main() { let x: u64 = 42; }");
}

#[test]
fn infer_bool() {
    assert_type_ok("fn main() { let x: bool = true; }");
}

#[test]
fn infer_fn_return() {
    assert_type_ok("fn foo() -> i32 { 42 } fn main() { let x: i32 = foo(); }");
}

#[test]
fn infer_binary_op() {
    assert_type_ok("fn main() { let x: i32 = 1 + 2; }");
}

#[test]
fn error_type_mismatch() {
    assert_type_error("fn main() { let x: u32 = true; }", "mismatch");
}

#[test]
fn infer_reference() {
    assert_type_ok("fn main() { let x: i32 = 5; let y: &i32 = &x; }");
}

#[test]
fn infer_if_else() {
    assert_type_ok("fn main() { let x: i32 = if true { 1 } else { 2 }; }");
}

#[test]
fn error_if_else_mismatch() {
    assert_type_error("fn main() { let x: i32 = if true { 1 } else { true }; }", "mismatch");
}

#[test]
fn infer_block_type() {
    assert_type_ok("fn main() { let x: i32 = { let y = 5; y }; }");
}

#[test]
fn infer_struct_construction() {
    assert_type_ok(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 1, y: 2 }; }
    "#);
}

#[test]
fn infer_field_access() {
    assert_type_ok(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 1, y: 2 }; let v: i32 = p.x; }
    "#);
}

#[test]
fn check_fn_args() {
    assert_type_ok(r#"
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() { let x: i32 = add(1, 2); }
    "#);
}

#[test]
fn error_wrong_arg_type() {
    assert_type_error(r#"
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() { add(true, 2); }
    "#, "mismatch");
}

#[test]
fn error_wrong_arg_count() {
    assert_type_error(r#"
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() { add(1); }
    "#, "argument");
}

#[test]
fn check_return_type() {
    assert_type_ok("fn foo() -> i32 { return 42; }");
}

#[test]
fn error_wrong_return_type() {
    assert_type_error("fn foo() -> i32 { return true; }", "mismatch");
}

#[test]
fn check_assign_types() {
    assert_type_ok("fn main() { let mut x: i32 = 0; x = 5; }");
}

#[test]
fn check_comparison_returns_bool() {
    assert_type_ok("fn main() { let x: bool = 1 < 2; }");
}

#[test]
fn check_logical_ops() {
    assert_type_ok("fn main() { let x: bool = true && false || true; }");
}

#[test]
fn infer_enum_variant() {
    assert_type_ok(r#"
        enum Color { Red, Green, Blue }
        fn main() { let c = Color::Red; }
    "#);
}
