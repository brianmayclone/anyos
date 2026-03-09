use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
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
    let dir = std::env::temp_dir().join(format!("anyrc_test_enum_data_{}_{}", std::process::id(), id));
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

#[test]
fn enum_option_none() {
    assert_compiles(r#"
        enum MyOption { None, Some(i64) }
        fn main() -> i32 {
            let x = MyOption::None;
            match x {
                MyOption::None => 0,
                MyOption::Some(v) => 1,
            }
        }
    "#);
}

#[test]
fn enum_option_some() {
    assert_compiles(r#"
        enum MyOption { None, Some(i64) }
        fn main() -> i32 {
            let x = MyOption::Some(42);
            match x {
                MyOption::None => 0,
                MyOption::Some(v) => 1,
            }
        }
    "#);
}

#[test]
fn enum_multiple_fields() {
    assert_compiles(r#"
        enum Pair { Empty, Two(i64, i64) }
        fn main() -> i32 {
            let p = Pair::Two(10, 20);
            match p {
                Pair::Empty => 0,
                Pair::Two(a, b) => 1,
            }
        }
    "#);
}

#[test]
fn enum_nested_match() {
    assert_compiles(r#"
        enum MyOption { None, Some(i64) }
        fn main() -> i32 {
            let a = MyOption::None;
            let b = MyOption::Some(42);
            let x: i32 = match a {
                MyOption::None => 0,
                MyOption::Some(v) => 1,
            };
            let y: i32 = match b {
                MyOption::None => 0,
                MyOption::Some(v) => 1,
            };
            x + y
        }
    "#);
}

#[test]
fn run_option_none() {
    assert_run_returns(r#"
        enum MyOption { None, Some(i64) }
        fn main() -> i32 {
            let x = MyOption::None;
            match x {
                MyOption::None => 0,
                MyOption::Some(v) => 1,
            }
        }
    "#, 0);
}

#[test]
fn run_option_some() {
    assert_run_returns(r#"
        enum MyOption { None, Some(i64) }
        fn main() -> i32 {
            let x = MyOption::Some(42);
            match x {
                MyOption::None => 0,
                MyOption::Some(v) => v as i32,
            }
        }
    "#, 42);
}

#[test]
fn run_enum_multi_variant() {
    assert_run_returns(r#"
        enum Shape { Circle(i64), Rect(i64, i64), Empty }
        fn area(s: Shape) -> i32 {
            match s {
                Shape::Circle(r) => r as i32,
                Shape::Rect(w, h) => (w * h) as i32,
                Shape::Empty => 0,
            }
        }
        fn main() -> i32 {
            let a = area(Shape::Circle(5));
            let b = area(Shape::Rect(3, 4));
            let c = area(Shape::Empty);
            a + b + c
        }
    "#, 17);
}
