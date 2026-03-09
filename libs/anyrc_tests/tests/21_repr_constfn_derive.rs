use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::TypeChecker;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

fn assert_parses(src: &str) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let _krate = parser.parse_crate();
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
    let dir = std::env::temp_dir().join(format!("anyrc_test_repr_{}_{}", std::process::id(), id));
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

// ── Parse tests ──

#[test]
fn parse_repr_c() {
    assert_parses(r#"
        #[repr(C)]
        struct Foo {
            x: i32,
            y: i32,
        }
        fn main() {}
    "#);
}

#[test]
fn parse_repr_packed() {
    assert_parses(r#"
        #[repr(C, packed)]
        struct Bar {
            a: u8,
            b: u32,
        }
        fn main() {}
    "#);
}

#[test]
fn parse_repr_transparent() {
    assert_parses(r#"
        #[repr(transparent)]
        struct Wrapper(i32);
        fn main() {}
    "#);
}

#[test]
fn parse_const_fn() {
    assert_parses(r#"
        struct Foo { x: i32, y: i32 }
        const fn new(x: i32) -> Foo { Foo { x: x, y: 0 } }
        fn main() {}
    "#);
}

#[test]
fn parse_derive() {
    assert_parses(r#"
        #[derive(Clone, Copy)]
        struct Point {
            x: i32,
            y: i32,
        }
        fn main() {}
    "#);
}

// ── Compile tests ──

#[test]
fn compile_repr_c_struct() {
    assert_compiles(r#"
        #[repr(C)]
        struct Foo {
            x: i32,
            y: i32,
        }
        fn main() -> i32 {
            let f: Foo = Foo { x: 10, y: 20 };
            f.x + f.y
        }
    "#);
}

#[test]
fn compile_const_fn() {
    assert_compiles(r#"
        struct Foo { x: i32, y: i32 }
        const fn make_foo(x: i32) -> Foo {
            Foo { x: x, y: 0 }
        }
        fn main() -> i32 {
            let f: Foo = make_foo(42);
            f.x
        }
    "#);
}

// ── Runtime tests ──

#[test]
fn run_const_fn() {
    assert_run_returns(r#"
        struct Foo { x: i32, y: i32 }
        const fn make_foo(x: i32) -> Foo {
            Foo { x: x, y: 0 }
        }
        fn main() -> i32 {
            let f: Foo = make_foo(42);
            f.x
        }
    "#, 42);
}

#[test]
fn run_repr_c_struct() {
    assert_run_returns(r#"
        #[repr(C)]
        struct Pair {
            a: i32,
            b: i32,
        }
        fn main() -> i32 {
            let p: Pair = Pair { a: 10, b: 20 };
            p.a + p.b
        }
    "#, 30);
}
