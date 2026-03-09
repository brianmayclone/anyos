use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::TypeChecker;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

fn assert_run_returns(src: &str, expected: i32) {
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let exe_bytes = compile(src, "test.rs", &options)
        .expect("compilation failed");
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_test_unsafe_ptrs_{}_{}", std::process::id(), id));
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
    let code = status.code().unwrap_or(-1);
    assert_eq!(code, expected, "expected exit code {}, got {}", expected, code);
}

fn assert_compiles(src: &str) {
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    compile(src, "test.rs", &options).expect("compilation failed");
}

fn assert_parses(src: &str) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let _krate = parser.parse_crate();
    // If we get here without panic, parsing succeeded
}

fn assert_typechecks(src: &str) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    anyrc::macros::expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&interner);
    let resolve_result = resolver.resolve_crate(&hir);
    assert!(resolve_result.errors.is_empty(), "resolve errors: {:?}", resolve_result.errors);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let typeck_result = checker.check_crate(&hir);
    assert!(typeck_result.errors.is_empty(), "typeck errors: {:?}", typeck_result.errors);
}

// ── Parse tests ──

#[test]
fn parse_unsafe_block() {
    assert_parses("fn main() { unsafe { let x: i32 = 5; } }");
}

#[test]
fn parse_as_cast() {
    assert_parses("fn main() { let x: i32 = 5; let y = x as i64; }");
}

// ── Typecheck tests ──

#[test]
fn typecheck_integer_cast() {
    assert_typechecks("fn main() { let x: i32 = 5; let y = x as i64; }");
}

#[test]
fn typecheck_ptr_cast() {
    assert_typechecks("fn main() { let x: i32 = 5; let p: *const i32 = &x as *const i32; }");
}

// ── Compile tests ──

#[test]
fn compile_cast_numeric() {
    assert_compiles("fn main() -> i32 { let x: i64 = 42; x as i32 }");
}

// ── Runtime tests ──

#[test]
fn run_cast_identity() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let x: i64 = 42;
            x as i32
        }
    "#, 42);
}

#[test]
fn run_ptr_deref() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let x: i32 = 7;
            let p = &x as *const i32;
            unsafe { *p }
        }
    "#, 7);
}

#[test]
fn run_extern_c_fn() {
    assert_run_returns(r#"
        extern "C" fn add(a: i32, b: i32) -> i32 {
            a + b
        }
        fn main() -> i32 {
            add(30, 12)
        }
    "#, 42);
}
