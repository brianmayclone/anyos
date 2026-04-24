mod common;
use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

/// Compile source, write to a temp file, execute, return exit code.
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
    let exe_bytes = compile(source, "test.rs", &options)
        .expect("compilation failed");
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_test_{}_{}", std::process::id(), id));
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

#[test]
fn compile_returns_ok() {
    let source = "fn main() -> i32 { 42 }";
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
    assert!(result.is_ok());
    let exe = result.unwrap();
    assert_eq!(&exe[0..4], &[0x7f, b'E', b'L', b'F']);
}

#[test]
fn compile_with_error_returns_err() {
    let source = "fn main() { let x: i32 = true; }";
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
    assert!(result.is_err());
}

#[test]
fn cfg_all_false_strips_item() {
    let source = r#"
        #[cfg(all(test, feature = "host"))]
        fn host_only() -> i32 {
            missing.field
        }

        fn main() -> i32 { 0 }
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
    assert!(result.is_ok(), "cfg-disabled item was typechecked: {:?}", result.err());
}

#[test]
fn compile_emit_obj() {
    let source = "fn foo() -> i32 { 42 }";
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test.o".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: None,
        ..CompileOptions::default()
    };
    let result = compile(source, "test.rs", &options);
    assert!(result.is_ok());
    let obj = result.unwrap();
    assert_eq!(&obj[0..4], &[0x7f, b'E', b'L', b'F']);
    assert_eq!(obj[16], 1);  // ET_REL
}

#[test]
fn compile_complex_program() {
    let source = r#"
        fn add(a: i32, b: i32) -> i32 { a + b }

        fn main() -> i32 {
            let sum: i32 = add(10, 20);
            sum
        }
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
    assert!(result.is_ok(), "compilation failed: {:?}", result.err().unwrap().iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn compile_with_optimization() {
    let source = "fn main() -> i32 { let x: i32 = 5; x }";
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 1,
        crate_type: CrateType::Bin,
        crate_name: None,
        ..CompileOptions::default()
    };
    let result = compile(source, "test.rs", &options);
    assert!(result.is_ok());
}

#[test]
fn compile_enum_and_match() {
    let source = r#"
        enum Color { Red, Green, Blue }
        fn value(c: Color) -> i32 {
            match c {
                Color::Red => 1,
                Color::Green => 2,
                Color::Blue => 3,
            }
        }
        fn main() -> i32 { value(Color::Green) }
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
    assert!(result.is_ok(), "failed: {:?}", result.err().unwrap().iter().map(|e| &e.message).collect::<Vec<_>>());
}

// ── Runtime tests ──

#[test]
fn run_return_literal() {
    assert_eq!(compile_and_run("fn main() -> i32 { 42 }"), 42);
}

#[test]
fn run_arithmetic() {
    assert_eq!(compile_and_run("fn main() -> i32 { 10 + 20 + 12 }"), 42);
}

#[test]
fn run_function_call() {
    let src = r#"
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() -> i32 { add(10, 32) }
    "#;
    assert_eq!(compile_and_run(src), 42);
}

#[test]
fn run_struct_field_access() {
    let src = r#"
        struct Point { x: i32, y: i32 }
        fn main() -> i32 {
            let p: Point = Point { x: 10, y: 20 };
            p.y
        }
    "#;
    assert_eq!(compile_and_run(src), 20);
}

#[test]
fn run_struct_field_sum() {
    let src = r#"
        struct Pair { a: i32, b: i32 }
        fn main() -> i32 {
            let p: Pair = Pair { a: 3, b: 7 };
            p.a + p.b
        }
    "#;
    assert_eq!(compile_and_run(src), 10);
}

#[test]
fn run_if_else() {
    let src = r#"
        fn main() -> i32 {
            let x: i32 = 5;
            if x > 3 { 1 } else { 0 }
        }
    "#;
    assert_eq!(compile_and_run(src), 1);
}

#[test]
fn run_nested_calls() {
    let src = r#"
        fn double(x: i32) -> i32 { x + x }
        fn main() -> i32 { double(double(5)) }
    "#;
    assert_eq!(compile_and_run(src), 20);
}

#[test]
fn run_method_self_by_ref() {
    let src = r#"
        struct Point { x: i32, y: i32 }
        impl Point {
            fn get_x(&self) -> i32 { self.x }
        }
        fn main() -> i32 {
            let p = Point { x: 42, y: 0 };
            p.get_x()
        }
    "#;
    assert_eq!(compile_and_run(src), 42);
}

#[test]
fn run_method_self_by_value() {
    let src = r#"
        struct Val { x: i32 }
        impl Val {
            fn get(self) -> i32 { self.x }
        }
        fn main() -> i32 {
            let v = Val { x: 42 };
            v.get()
        }
    "#;
    assert_eq!(compile_and_run(src), 42);
}

#[test]
fn run_method_self_by_value_with_constructor() {
    let src = r#"
        struct Val { x: i32 }
        impl Val {
            fn new(x: i32) -> Val { Val { x: x } }
            fn get(self) -> i32 { self.x }
        }
        fn main() -> i32 {
            let v = Val::new(7);
            v.get()
        }
    "#;
    assert_eq!(compile_and_run(src), 7);
}
