use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

fn resolve_src(src: &str) -> (anyrc::resolve::ResolveResult, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut ctx = LoweringContext::new(&mut interner);
    let hir = ctx.lower_crate(&krate);
    drop(ctx);
    let mut resolver = Resolver::new(&interner);
    let result = resolver.resolve_crate(&hir);
    (result, interner)
}

fn assert_resolves(src: &str) {
    let (result, _) = resolve_src(src);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

fn assert_resolve_error(src: &str, expected_msg: &str) {
    let (result, _) = resolve_src(src);
    assert!(!result.errors.is_empty(), "expected error containing '{}' but got none", expected_msg);
    assert!(result.errors.iter().any(|e| e.message.contains(expected_msg)),
        "expected error containing '{}', got: {:?}", expected_msg,
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
    let dir = std::env::temp_dir().join(format!("anyrc_test_mod_{}_{}", std::process::id(), id));
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
    status.code().unwrap_or(-1)
}

// ── Parse tests ──

#[test]
fn parse_inline_module() {
    let mut interner = Interner::new();
    let mut parser = Parser::new("mod foo { fn bar() -> i32 { 42 } }", &mut interner);
    let krate = parser.parse_crate();
    assert_eq!(krate.items.len(), 1);
    match &krate.items[0] {
        anyrc::ast::Item::Mod(m) => {
            let name = interner.resolve(m.name);
            assert_eq!(name, "foo");
            assert!(m.items.is_some());
            assert_eq!(m.items.as_ref().unwrap().len(), 1);
        }
        _ => panic!("expected Mod item"),
    }
}

// ── Resolve tests ──

#[test]
fn resolve_module_path() {
    assert_resolves(r#"
        mod foo {
            pub fn bar() -> i32 { 42 }
        }
        fn main() -> i32 { foo::bar() }
    "#);
}

#[test]
fn resolve_nested_modules() {
    assert_resolves(r#"
        mod a {
            pub mod b {
                pub fn c() -> i32 { 7 }
            }
        }
        fn main() -> i32 { a::b::c() }
    "#);
}

#[test]
fn resolve_use_import() {
    assert_resolves(r#"
        mod foo {
            pub fn bar() -> i32 { 42 }
        }
        use foo::bar;
        fn main() -> i32 { bar() }
    "#);
}

#[test]
fn resolve_use_alias() {
    assert_resolves(r#"
        mod foo {
            pub fn bar() -> i32 { 10 }
        }
        use foo::bar as baz;
        fn main() -> i32 { baz() }
    "#);
}

#[test]
fn resolve_crate_prefix() {
    assert_resolves(r#"
        mod foo {
            pub fn val() -> i32 { 5 }
        }
        fn main() -> i32 { crate::foo::val() }
    "#);
}

// ── Compile / pipeline tests ──

#[test]
fn compile_inline_module() {
    let src = r#"
        mod foo {
            pub fn bar() -> i32 { 42 }
        }
        fn main() -> i32 { foo::bar() }
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test".to_string(),
        emit: EmitKind::Exe,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
    };
    let result = compile(src, "test.rs", &options);
    assert!(result.is_ok(), "compilation failed: {:?}", result.err());
}

// ── Runtime tests ──

#[test]
fn run_module_fn_call() {
    let code = compile_and_run(r#"
        mod foo {
            pub fn bar() -> i32 { 42 }
        }
        fn main() -> i32 { foo::bar() }
    "#);
    assert_eq!(code, 42);
}

#[test]
fn run_use_import() {
    let code = compile_and_run(r#"
        mod foo {
            pub fn val() -> i32 { 17 }
        }
        use foo::val;
        fn main() -> i32 { val() }
    "#);
    assert_eq!(code, 17);
}

#[test]
fn run_nested_module() {
    let code = compile_and_run(r#"
        mod a {
            pub mod b {
                pub fn c() -> i32 { 99 }
            }
        }
        fn main() -> i32 { a::b::c() }
    "#);
    assert_eq!(code, 99);
}
