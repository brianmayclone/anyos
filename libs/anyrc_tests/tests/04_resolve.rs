use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;

fn resolve_src(src: &str) -> (anyrc::resolve::ResolveResult, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut ctx = LoweringContext::new(&mut interner);
    let hir = ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&interner);
    let result = resolver.resolve_crate(&hir);
    (result, interner)
}

fn assert_resolves(src: &str) {
    let (result, _) = resolve_src(src);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

fn assert_resolve_error(src: &str, expected_msg: &str) {
    let (result, _) = resolve_src(src);
    assert!(!result.errors.is_empty(), "expected error containing '{}' but got none", expected_msg);
    assert!(result.errors.iter().any(|e| e.message.contains(expected_msg)),
        "expected error containing '{}', got: {:?}", expected_msg,
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn resolve_local_variable() {
    assert_resolves("fn main() { let x = 5; let y = x; }");
}

#[test]
fn resolve_fn_call() {
    assert_resolves("fn foo() -> i32 { 0 } fn main() { foo(); }");
}

#[test]
fn error_undefined_variable() {
    assert_resolve_error("fn main() { let x = y; }", "not found");
}

#[test]
fn resolve_fn_params() {
    assert_resolves("fn add(a: i32, b: i32) -> i32 { a + b }");
}

#[test]
fn resolve_struct_and_fields() {
    assert_resolves("struct S { x: i32 } fn main() { let s = S { x: 1 }; }");
}

#[test]
fn resolve_enum_variants() {
    assert_resolves("enum E { A, B } fn main() { let x = E::A; }");
}

#[test]
fn resolve_impl_methods() {
    assert_resolves(r#"
        struct S {}
        impl S { fn new() -> S { S {} } }
        fn main() { S::new(); }
    "#);
}

#[test]
fn resolve_nested_scopes() {
    assert_resolves(r#"
        fn main() {
            let x = 1;
            {
                let y = x;
                {
                    let z = y;
                }
            }
        }
    "#);
}

#[test]
fn resolve_shadowing() {
    assert_resolves(r#"
        fn main() {
            let x = 1;
            let x = x + 1;
            let y = x;
        }
    "#);
}

#[test]
fn error_undefined_type() {
    assert_resolve_error("fn foo(x: Nonexistent) {}", "not found");
}

#[test]
fn resolve_generic_params() {
    assert_resolves("fn id<T>(x: T) -> T { x }");
}

#[test]
fn resolve_trait_def() {
    assert_resolves("trait Foo { fn foo(&self) -> i32; }");
}

#[test]
fn resolve_trait_impl() {
    assert_resolves(r#"
        trait Foo { fn foo(&self) -> i32; }
        struct S {}
        impl Foo for S { fn foo(&self) -> i32 { 42 } }
    "#);
}

#[test]
fn resolve_const_and_static() {
    assert_resolves(r#"
        const MAX: i32 = 100;
        static COUNT: i32 = 0;
        fn main() { let x = MAX; let y = COUNT; }
    "#);
}

#[test]
fn resolve_type_alias() {
    assert_resolves(r#"
        type Num = i32;
        fn foo(x: Num) -> Num { x }
    "#);
}

#[test]
fn resolve_if_let() {
    assert_resolves(r#"
        enum Option { Some(i32), None }
        fn main() {
            let x = Option::Some(5);
        }
    "#);
}

#[test]
fn resolve_match_patterns() {
    assert_resolves(r#"
        fn main() {
            let x = 5;
            match x {
                0 => 0,
                n => n,
            }
        }
    "#);
}
