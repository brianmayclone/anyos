mod common;
use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use anyrc::parser::Parser;
use anyrc::intern::Interner;
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
    let exe_bytes = compile(src, "test.rs", &options)
        .expect("compilation failed");
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_test_no_std_{}_{}", std::process::id(), id));
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
        ..CompileOptions::default()
    };
    compile(src, "test.rs", &options).expect("compilation failed");
}

// 1. Parse inner attribute #![no_std]
#[test]
fn parse_inner_attribute() {
    let src = "#![no_std]\nfn main() -> i32 { 0 }";
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    assert_eq!(krate.attrs.len(), 1);
    let attr = &krate.attrs[0];
    assert_eq!(attr.path.segments.len(), 1);
    let name = interner.resolve(attr.path.segments[0].ident);
    assert_eq!(name, "no_std");
    assert_eq!(krate.items.len(), 1);
}

// 2. Parse #[no_mangle] on a function
#[test]
fn parse_no_mangle() {
    let src = "#[no_mangle]\nfn foo() -> i32 { 42 }";
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    assert_eq!(krate.items.len(), 1);
    match &krate.items[0] {
        anyrc::ast::Item::Fn(f) => {
            assert_eq!(f.attrs.len(), 1);
            let name = interner.resolve(f.attrs[0].path.segments[0].ident);
            assert_eq!(name, "no_mangle");
        }
        _ => panic!("expected FnDef"),
    }
}

// 3. Compile with #![no_std] attribute
#[test]
fn compile_no_std_crate() {
    assert_compiles("#![no_std]\nfn main() -> i32 { 0 }");
}

// 4. Compile a #[no_mangle] function
#[test]
fn compile_no_mangle_fn() {
    assert_compiles("#[no_mangle]\nfn foo() -> i32 { 42 }\nfn main() -> i32 { 0 }");
}

// 5. Runtime test: #[no_mangle] function can be called
#[test]
fn run_no_mangle_fn() {
    let src = r#"
        #[no_mangle]
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() -> i32 { add(20, 22) }
    "#;
    assert_run_returns(src, 42);
}

// 6. Compile with #![no_main] and custom _start
#[test]
fn compile_no_main() {
    let src = r#"
        #![no_main]
        #![no_std]
        #[no_mangle]
        fn _start() -> i32 { 0 }
    "#;
    // This should compile as an object without generating a _start stub
    assert_compiles(src);
}
