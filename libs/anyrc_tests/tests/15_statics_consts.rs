mod common;
use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
use anyrc::hir_lower::LoweringContext;
use anyrc::intern::Interner;
use anyrc::parser::Parser;
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
        ..CompileOptions::default()
    };
    let exe_bytes = compile(src, "test.rs", &options).expect("compilation failed");
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("anyrc_test_statics_{}_{}", std::process::id(), id));
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
    let status = common::run_executable(&exe_path);
    let _ = std::fs::remove_dir_all(&dir);
    let code = status.code().unwrap_or(-1);
    assert_eq!(
        code, expected,
        "expected exit code {}, got {}",
        expected, code
    );
}

fn assert_compiles(src: &str) {
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
        ..CompileOptions::default()
    };
    compile(src, "test.rs", &options).expect("compilation failed");
}

fn assert_parses(src: &str) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let _krate = parser.parse_crate();
}

fn assert_typechecks(src: &str) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    anyrc::macros::expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    assert!(
        resolve_result.errors.is_empty(),
        "resolve errors: {:?}",
        resolve_result.errors
    );
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let typeck_result = checker.check_crate(&hir);
    assert!(
        typeck_result.errors.is_empty(),
        "typeck errors: {:?}",
        typeck_result.errors
    );
}

// ── Parse tests ──

#[test]
fn parse_const_item() {
    assert_parses("const X: i32 = 42;");
}

#[test]
fn parse_static_item() {
    assert_parses("static mut Y: i32 = 0;");
}

// ── Typecheck tests ──

#[test]
fn typecheck_const_usage() {
    assert_typechecks("const X: i32 = 42; fn main() -> i32 { X }");
}

// ── Compile tests ──

#[test]
fn compile_const_inline() {
    assert_compiles("const X: i32 = 42; fn main() -> i32 { X }");
}

// ── Runtime tests ──

#[test]
fn run_const_value() {
    assert_run_returns("const X: i32 = 42; fn main() -> i32 { X }", 42);
}

#[test]
fn run_static_read() {
    assert_run_returns("static X: i32 = 7; fn main() -> i32 { unsafe { X } }", 7);
}

#[test]
fn run_static_mut_write() {
    assert_run_returns(
        "static mut X: i32 = 0; fn main() -> i32 { unsafe { X = 10; X } }",
        10,
    );
}
