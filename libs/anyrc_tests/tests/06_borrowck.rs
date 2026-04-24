use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::TypeChecker;
use anyrc::mir_build::MirBuilder;
use anyrc::mir::MirBody;
use anyrc::borrowck::{self, BorrowckResult};

fn build_and_check(src: &str) -> Vec<BorrowckResult> {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let typeck_result = checker.check_crate(&hir);

    let bodies = MirBuilder::build_crate(&mut interner, &resolve_result, &typeck_result, &hir);
    bodies.iter().map(|body| borrowck::check_borrows(body, &interner, &typeck_result.struct_defs)).collect()
}

fn assert_borrowck_ok(src: &str) {
    let results = build_and_check(src);
    for result in &results {
        assert!(result.errors.is_empty(), "unexpected borrow errors: {:?}",
            result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
    }
}

fn assert_borrowck_error(src: &str, expected_msg: &str) {
    let results = build_and_check(src);
    let all_errors: Vec<_> = results.iter().flat_map(|r| &r.errors).collect();
    assert!(!all_errors.is_empty(), "expected borrow error containing '{}'", expected_msg);
    assert!(all_errors.iter().any(|e| e.message.to_lowercase().contains(&expected_msg.to_lowercase())),
        "expected error containing '{}', got: {:?}", expected_msg,
        all_errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn borrowck_simple_ref() {
    assert_borrowck_ok("fn foo() { let x: i32 = 5; let y: &i32 = &x; }");
}

#[test]
fn borrowck_simple_mut_ref() {
    assert_borrowck_ok("fn foo() { let mut x: i32 = 5; let y: &mut i32 = &mut x; }");
}

#[test]
fn borrowck_use_after_move() {
    assert_borrowck_error(r#"
        struct S { x: &mut i32 }
        fn take(s: S) {}
        fn foo() { let mut v: i32 = 1; let s = S { x: &mut v }; take(s); take(s); }
    "#, "moved");
}

#[test]
fn borrowck_two_mut_borrows() {
    assert_borrowck_error(r#"
        fn foo() { let mut x: i32 = 5; let a: &mut i32 = &mut x; let b: &mut i32 = &mut x; }
    "#, "borrow");
}

#[test]
fn borrowck_assign_while_borrowed() {
    assert_borrowck_error(r#"
        fn foo() { let mut x: i32 = 5; let r: &i32 = &x; x = 6; }
    "#, "borrow");
}

#[test]
fn borrowck_copy_types_ok() {
    assert_borrowck_ok("fn foo() { let x: i32 = 5; let y: i32 = x; let z: i32 = x; }");
}

#[test]
fn borrowck_no_error_after_scope() {
    assert_borrowck_ok("fn foo() { let mut x: i32 = 5; let y: i32 = x; x = 6; }");
}

#[test]
fn borrowck_slice_reference_can_be_reassigned_to_subslice() {
    assert_borrowck_ok("fn foo(mut buf: &[u8], n: usize) { buf = &buf[n..]; }");
}
