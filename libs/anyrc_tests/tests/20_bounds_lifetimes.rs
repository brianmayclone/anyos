use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::{TypeChecker, TypeckResult};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

fn parse_ok(src: &str) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let _krate = parser.parse_crate();
    // If parsing didn't panic, it's OK
}

fn typecheck(src: &str) -> TypeckResult {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    checker.check_crate(&hir)
}

fn assert_type_ok(src: &str) {
    let result = typecheck(src);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

fn compile_and_run(source: &str) -> i32 {
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let exe_bytes = compile(source, "test.rs", &options)
        .expect("compilation failed");
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_bounds_test_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let exe_path = dir.join("test_exe");
    {
        let mut f = std::fs::File::create(&exe_path).unwrap();
        f.write_all(&exe_bytes).unwrap();
        f.sync_all().unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&exe_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let status = std::process::Command::new(&exe_path)
        .status()
        .expect("failed to execute compiled binary");
    let _ = std::fs::remove_dir_all(&dir);
    status.code().unwrap_or(-1)
}

// ── Parse tests ──

#[test]
fn parse_trait_bound() {
    parse_ok("fn foo<T: Copy>(x: T) -> T { x }");
}

#[test]
fn parse_multiple_bounds() {
    parse_ok("fn foo<T: Copy + Clone>(x: T) { }");
}

#[test]
fn parse_where_clause() {
    parse_ok("fn foo<T>(x: T) where T: Copy { }");
}

#[test]
fn parse_lifetime_param() {
    parse_ok("fn foo<'a>(x: &'a i32) -> &'a i32 { x }");
}

#[test]
fn parse_lifetime_struct() {
    parse_ok("struct Ref<'a, T> { data: &'a T }");
}

// ── Typeck tests ──

#[test]
fn typecheck_bounded_generic() {
    assert_type_ok("fn id<T: Copy>(x: T) -> T { x } fn main() { let x: i32 = id(42); }");
}

// ── Compile tests ──

#[test]
fn compile_bounded_generic() {
    let src = r#"
        fn id<T: Copy>(x: T) -> T { x }
        fn main() -> i32 { id(42) }
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let _exe_bytes = compile(src, "test.rs", &options).expect("compilation failed");
}

// ── Runtime tests ──

#[test]
fn run_bounded_generic() {
    let code = compile_and_run(r#"
        fn id<T: Copy>(x: T) -> T { x }
        fn main() -> i32 { id(42) }
    "#);
    assert_eq!(code, 42);
}

#[test]
fn run_lifetime_fn() {
    let code = compile_and_run(r#"
        fn pass<'a>(x: &'a i32) -> &'a i32 { x }
        fn main() -> i32 {
            let val: i32 = 99;
            let r: &i32 = pass(&val);
            *r
        }
    "#);
    assert_eq!(code, 99);
}
