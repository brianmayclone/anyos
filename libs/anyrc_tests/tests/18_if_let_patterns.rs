mod common;
use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::ast::*;
use anyrc::hir_lower::LoweringContext;
use anyrc::hir::*;
use anyrc::macros::expand_macros;
use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

fn parse_expr_src(src: &str) -> (Expr, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let e = parser.parse_expr();
    (e, interner)
}

fn lower(src: &str) -> HirCrate {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut ctx = LoweringContext::new(&mut interner);
    ctx.lower_crate(&krate)
}

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
    let dir = std::env::temp_dir().join(format!("anyrc_test_if_let_{}_{}", std::process::id(), id));
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

// ── Parse tests ──

#[test]
fn parse_if_let() {
    let (expr, _) = parse_expr_src("if let Some(x) = opt { x } else { 0 }");
    match expr {
        Expr::IfLet(pat, _scrutinee, _then, else_, _span) => {
            assert!(matches!(pat, Pattern::TupleStruct(_, _, _)));
            assert!(else_.is_some());
        }
        _ => panic!("expected IfLet, got {:?}", std::mem::discriminant(&expr)),
    }
}

#[test]
fn parse_while_let() {
    let (expr, _) = parse_expr_src("while let Some(x) = opt { }");
    match expr {
        Expr::WhileLet(pat, _scrutinee, _body, _label, _span) => {
            assert!(matches!(pat, Pattern::TupleStruct(_, _, _)));
        }
        _ => panic!("expected WhileLet"),
    }
}

#[test]
fn parse_range_pattern() {
    let (expr, _) = parse_expr_src("match x { 0..=9 => 1, _ => 2 }");
    match expr {
        Expr::Match(_, arms, _) => {
            assert_eq!(arms.len(), 2);
            assert!(matches!(&arms[0].pat, Pattern::Range(Some(_), Some(_), true, _)));
            assert!(matches!(&arms[1].pat, Pattern::Wildcard(_)));
        }
        _ => panic!("expected Match"),
    }
}

// ── Compile tests ──

#[test]
fn compile_if_let_some() {
    assert_compiles(r#"
        enum MyOption { None, Some(i64) }
        fn main() -> i32 {
            let x = MyOption::Some(42);
            if let MyOption::Some(v) = x {
                v as i32
            } else {
                0
            }
        }
    "#);
}

#[test]
fn compile_if_let_none() {
    assert_compiles(r#"
        enum MyOption { None, Some(i64) }
        fn main() -> i32 {
            let x = MyOption::None;
            if let MyOption::Some(v) = x {
                v as i32
            } else {
                0
            }
        }
    "#);
}

// ── Runtime tests ──

#[test]
fn run_if_let_some() {
    assert_run_returns(r#"
        enum MyOption { None, Some(i64) }
        fn main() -> i32 {
            let x = MyOption::Some(42);
            if let MyOption::Some(v) = x {
                v as i32
            } else {
                0
            }
        }
    "#, 42);
}

#[test]
fn run_if_let_none() {
    assert_run_returns(r#"
        enum MyOption { None, Some(i64) }
        fn main() -> i32 {
            let x = MyOption::None;
            if let MyOption::Some(v) = x {
                1
            } else {
                0
            }
        }
    "#, 0);
}

#[test]
fn run_match_range() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let x: i64 = 5;
            match x {
                0..=9 => 1,
                _ => 2,
            }
        }
    "#, 1);
}

#[test]
fn run_literal_match() {
    assert_run_returns(r#"
        fn main() -> i32 {
            let x: i64 = 2;
            match x {
                0 => 10,
                1 => 20,
                2 => 30,
                _ => 40,
            }
        }
    "#, 30);
}
