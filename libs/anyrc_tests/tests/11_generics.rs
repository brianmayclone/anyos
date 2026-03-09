use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::{TypeChecker, TypeckResult};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

fn typecheck(src: &str) -> TypeckResult {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new();
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&interner);
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
    let dir = std::env::temp_dir().join(format!("anyrc_gen_test_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let exe_path = dir.join("test_exe");
    {
        let mut f = std::fs::File::create(&exe_path).unwrap();
        f.write_all(&exe_bytes).unwrap();
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

// ── Type checking tests ──

#[test]
fn typecheck_generic_fn() {
    assert_type_ok("fn id<T>(x: T) -> T { x } fn main() { let x: i32 = id(42); }");
}

#[test]
fn typecheck_generic_two_calls_same_type() {
    assert_type_ok(r#"
        fn id<T>(x: T) -> T { x }
        fn main() {
            let a: i32 = id(42);
            let b: i32 = id(99);
        }
    "#);
}

#[test]
fn typecheck_generic_multi_param() {
    assert_type_ok(r#"
        fn choose<T>(a: T, b: T, pick_first: bool) -> T {
            if pick_first { a } else { b }
        }
        fn main() { let x: i32 = choose(1, 2, true); }
    "#);
}

// ── Runtime tests ──

#[test]
fn run_generic_identity() {
    assert_eq!(compile_and_run("fn id<T>(x: T) -> T { x } fn main() -> i32 { id(42) }"), 42);
}

#[test]
fn run_generic_identity_bool() {
    // bool true = 1 as exit code
    assert_eq!(compile_and_run("fn id<T>(x: T) -> T { x } fn main() -> i32 { if id(true) { 1 } else { 0 } }"), 1);
}

#[test]
fn run_generic_choose_first() {
    assert_eq!(compile_and_run(r#"
        fn choose<T>(a: T, b: T, pick_first: bool) -> T {
            if pick_first { a } else { b }
        }
        fn main() -> i32 { choose(10, 20, true) }
    "#), 10);
}

#[test]
fn run_generic_choose_second() {
    assert_eq!(compile_and_run(r#"
        fn choose<T>(a: T, b: T, pick_first: bool) -> T {
            if pick_first { a } else { b }
        }
        fn main() -> i32 { choose(10, 20, false) }
    "#), 20);
}

#[test]
fn run_generic_two_calls_same_instantiation() {
    assert_eq!(compile_and_run(r#"
        fn id<T>(x: T) -> T { x }
        fn main() -> i32 {
            let a: i32 = id(10);
            let b: i32 = id(32);
            a + b
        }
    "#), 42);
}

#[test]
fn run_generic_nested_call() {
    assert_eq!(compile_and_run(r#"
        fn id<T>(x: T) -> T { x }
        fn main() -> i32 { id(id(42)) }
    "#), 42);
}

#[test]
fn run_generic_with_arithmetic() {
    assert_eq!(compile_and_run(r#"
        fn double<T>(x: T, y: T) -> T { x }
        fn main() -> i32 {
            let a: i32 = double(21, 0);
            a + a
        }
    "#), 42);
}
