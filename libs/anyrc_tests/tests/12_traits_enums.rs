mod common;
use anyrc::driver::{compile, CompileOptions, CrateType, EmitKind};
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
    let code = status.code().unwrap_or(-1);
    assert_eq!(
        code, expected,
        "expected exit code {}, got {}",
        expected, code
    );
}

// === Trait tests ===

#[test]
fn trait_method_call() {
    assert_run_returns(
        r#"
        struct Counter { n: i32 }
        impl Counter {
            fn get(&self) -> i32 { self.n }
        }
        fn main() -> i32 {
            let c = Counter { n: 42 };
            c.get()
        }
    "#,
        42,
    );
}

#[test]
fn trait_method_with_args() {
    assert_run_returns(
        r#"
        struct Adder { base: i32 }
        impl Adder {
            fn add(&self, x: i32) -> i32 { self.base + x }
        }
        fn main() -> i32 {
            let a = Adder { base: 10 };
            a.add(32)
        }
    "#,
        42,
    );
}

#[test]
fn trait_impl_for_type() {
    assert_run_returns(
        r#"
        trait GetValue {
            fn value(&self) -> i32;
        }
        struct Foo { x: i32 }
        impl GetValue for Foo {
            fn value(&self) -> i32 { self.x }
        }
        fn main() -> i32 {
            let f = Foo { x: 55 };
            f.value()
        }
    "#,
        55,
    );
}

#[test]
fn method_via_reference() {
    assert_run_returns(
        r#"
        struct S { val: i32 }
        impl S {
            fn get(&self) -> i32 { self.val }
        }
        fn use_ref(s: &S) -> i32 { s.get() }
        fn main() -> i32 {
            let s = S { val: 77 };
            use_ref(&s)
        }
    "#,
        77,
    );
}

// === Enum tests ===

#[test]
fn enum_c_like() {
    assert_run_returns(
        r#"
        enum Dir { N, S, E, W }
        fn main() -> i32 {
            let d = Dir::E;
            match d {
                Dir::N => 0,
                Dir::S => 1,
                Dir::E => 2,
                Dir::W => 3,
            }
        }
    "#,
        2,
    );
}

#[test]
fn enum_match_all_variants() {
    assert_run_returns(
        r#"
        enum Color { R, G, B }
        fn to_num(c: Color) -> i32 {
            match c {
                Color::R => 10,
                Color::G => 20,
                Color::B => 30,
            }
        }
        fn main() -> i32 { to_num(Color::B) }
    "#,
        30,
    );
}

#[test]
fn enum_as_fn_arg() {
    assert_run_returns(
        r#"
        enum Bool { T, F }
        fn is_true(b: Bool) -> i32 {
            match b {
                Bool::T => 1,
                Bool::F => 0,
            }
        }
        fn main() -> i32 { is_true(Bool::T) }
    "#,
        1,
    );
}

#[test]
fn enum_in_if() {
    assert_run_returns(
        r#"
        enum Choice { A, B }
        fn pick(c: Choice) -> i32 {
            let v: i32 = match c {
                Choice::A => 10,
                Choice::B => 20,
            };
            v + 5
        }
        fn main() -> i32 { pick(Choice::B) }
    "#,
        25,
    );
}
