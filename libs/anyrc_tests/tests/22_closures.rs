mod common;
use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
use anyrc::intern::Interner;
use anyrc::parser::Parser;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

fn compile_and_run(source: &str) -> i32 {
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
        ..CompileOptions::default()
    };
    let exe_bytes = compile(source, "test.rs", &options).expect("compilation failed");
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("anyrc_closure_test_{}_{}", std::process::id(), id));
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
    status.code().unwrap_or(-1)
}

// ── Parse tests ──

#[test]
fn parse_closure_no_args() {
    let mut interner = Interner::new();
    let mut parser = Parser::new("fn main() { || 42 }", &mut interner);
    let krate = parser.parse_crate();
    // Should parse without panicking; closure is in the body
    assert!(!krate.items.is_empty());
}

#[test]
fn parse_closure_with_params() {
    let mut interner = Interner::new();
    let mut parser = Parser::new("fn main() { |x: i32, y: i32| x + y }", &mut interner);
    let krate = parser.parse_crate();
    assert!(!krate.items.is_empty());
}

// ── Compile test ──

#[test]
fn compile_closure_as_value() {
    let source = "fn main() { let f = |x: i32| -> i32 { x + 1 }; }";
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
        ..CompileOptions::default()
    };
    let result = compile(source, "test.rs", &options);
    assert!(
        result.is_ok(),
        "closure compilation failed: {:?}",
        result.err()
    );
}

#[test]
fn compile_closure_calling_captured_fn_param() {
    let source = r#"
        fn inc(x: i32) -> i32 { x + 1 }
        fn apply(f: fn(i32) -> i32, x: i32) -> i32 {
            let g = |y: i32| -> i32 { f(y) };
            g(x)
        }
        fn main() -> i32 { apply(inc, 41) }
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
        ..CompileOptions::default()
    };
    let result = compile(source, "test.rs", &options);
    assert!(
        result.is_ok(),
        "closure capture compilation failed: {:?}",
        result.err()
    );
}

#[test]
fn compile_match_tuple_variant_binds_callback_name() {
    let source = r#"
        enum MaybeFn {
            Some(fn() -> i32),
            None,
        }
        fn value() -> i32 { 7 }
        fn apply(opt: MaybeFn) -> i32 {
            match opt {
                MaybeFn::Some(f) => f(),
                MaybeFn::None => 0,
            }
        }
        fn main() -> i32 { apply(MaybeFn::Some(value)) }
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
        ..CompileOptions::default()
    };
    let result = compile(source, "test.rs", &options);
    assert!(
        result.is_ok(),
        "tuple variant callback binding failed: {:?}",
        result.err()
    );
}

// ── Runtime tests ──

#[test]
fn run_closure_call() {
    let code = r#"
        fn main() -> i32 {
            let f = |x: i32| -> i32 { x + 1 };
            f(41)
        }
    "#;
    assert_eq!(compile_and_run(code), 42);
}

#[test]
fn run_closure_no_capture() {
    let code = r#"
        fn main() -> i32 {
            let add = |a: i32, b: i32| -> i32 { a + b };
            add(20, 22)
        }
    "#;
    assert_eq!(compile_and_run(code), 42);
}

#[test]
fn run_closure_passed_to_fn() {
    let code = r#"
        fn apply(f: fn(i32) -> i32, x: i32) -> i32 {
            f(x)
        }
        fn main() -> i32 {
            let inc = |x: i32| -> i32 { x + 1 };
            apply(inc, 41)
        }
    "#;
    assert_eq!(compile_and_run(code), 42);
}
