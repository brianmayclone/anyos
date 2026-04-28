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
    let dir = std::env::temp_dir().join(format!("anyrc_dyn_test_{}_{}", std::process::id(), id));
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

fn compile_to_mir(src: &str) -> String {
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test.mir".to_string(),
        emit: EmitKind::Mir,
        opt_level: 0,
        crate_type: CrateType::Bin,
        crate_name: None,
        ..CompileOptions::default()
    };
    String::from_utf8(compile(src, "test.rs", &options).expect("compilation failed"))
        .expect("mir utf8")
}

#[test]
fn parse_dyn_trait_type() {
    let src = r#"
        trait MyTrait {
            fn do_thing(&self) -> i32;
        }
        fn foo(x: &dyn MyTrait) -> i32 { 0 }
        fn main() -> i32 { 0 }
    "#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    assert!(krate.items.len() >= 3);
}

#[test]
fn parse_dyn_trait_multi_bound_type() {
    let src = r#"
        trait MyTrait {
            fn do_thing(&self) -> i32;
        }
        fn foo(x: alloc::boxed::Box<dyn MyTrait + Send + Sync>) -> i32 { 0 }
        fn main() -> i32 { 0 }
    "#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    assert!(krate.items.len() >= 3);
}

#[test]
fn parse_dyn_trait_ref() {
    let src = r#"
        trait Foo {
            fn bar(&self) -> i32;
        }
        struct Baz { val: i32 }
        impl Foo for Baz {
            fn bar(&self) -> i32 { 42 }
        }
        fn main() -> i32 {
            let b = Baz { val: 0 };
            0
        }
    "#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    assert!(krate.items.len() >= 3);
}

#[test]
fn typecheck_dyn_trait() {
    let src = r#"
        trait Animal {
            fn sound(&self) -> i32;
        }
        struct Cat { id: i32 }
        impl Animal for Cat {
            fn sound(&self) -> i32 { 1 }
        }
        fn get_sound(a: &dyn Animal) -> i32 { a.sound() }
        fn main() -> i32 {
            let c = Cat { id: 0 };
            get_sound(&c)
        }
    "#;
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let krate = parser.parse_crate();
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    drop(lower_ctx);
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

#[test]
fn compile_dyn_trait() {
    let src = r#"
        trait Animal {
            fn sound(&self) -> i32;
        }
        struct Cat { id: i32 }
        impl Animal for Cat {
            fn sound(&self) -> i32 { 1 }
        }
        fn get_sound(a: &dyn Animal) -> i32 { a.sound() }
        fn main() -> i32 {
            let c = Cat { id: 0 };
            get_sound(&c)
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
    let result = compile(src, "test.rs", &options);
    assert!(result.is_ok(), "compilation failed: {:?}", result.err());
}

#[test]
fn run_dyn_trait_call() {
    assert_run_returns(
        r#"
        trait Animal {
            fn sound(&self) -> i32;
        }
        struct Cat { id: i32 }
        impl Animal for Cat {
            fn sound(&self) -> i32 { 1 }
        }
        fn get_sound(a: &dyn Animal) -> i32 {
            a.sound()
        }
        fn main() -> i32 {
            let c = Cat { id: 0 };
            get_sound(&c)
        }
    "#,
        1,
    );
}

#[test]
fn mir_if_let_option_dyn_trait_call_uses_indirect_dispatch() {
    let mir = compile_to_mir(
        r#"
        trait Filesystem {
            fn lookup(&self, id: i32) -> i32;
            fn stat(&self, id: i32) -> i32 { self.lookup(id) }
        }
        trait Send {}
        trait Sync {}
        struct RamFs { base: i32 }
        impl Send for RamFs {}
        impl Sync for RamFs {}
        impl Filesystem for RamFs {
            fn lookup(&self, id: i32) -> i32 { self.base + id }
        }
        fn root_fs(fs: &RamFs) -> Option<&(dyn Filesystem + Send + Sync)> {
            Some(fs)
        }
        fn main() -> i32 {
            let fs = RamFs { base: 40 };
            if let Some(driver) = root_fs(&fs) {
                driver.lookup(2)
            } else {
                0
            }
        }
    "#,
    );
    assert!(
        !mir.contains("fn lookup("),
        "dyn dispatch should not lower to a plain lookup symbol:\n{mir}"
    );
}

#[test]
fn mir_trait_ufcs_on_concrete_receiver_resolves_impl_symbol() {
    let mir = compile_to_mir(
        r#"
        trait Filesystem {
            fn lookup(&self, id: i32) -> i32;
        }
        struct RamFs { base: i32 }
        impl Filesystem for RamFs {
            fn lookup(&self, id: i32) -> i32 { self.base + id }
        }
        fn call_lookup(driver: &RamFs) -> i32 {
            Filesystem::lookup(driver, 2)
        }
        fn main() -> i32 {
            let fs = RamFs { base: 40 };
            call_lookup(&fs)
        }
    "#,
    );
    assert!(
        mir.contains("fn RamFs::lookup("),
        "UFCS trait call should resolve to the concrete impl symbol:\n{mir}"
    );
    assert!(
        !mir.contains("fn lookup("),
        "UFCS trait call should not lower to a bare trait method symbol:\n{mir}"
    );
}

#[test]
fn mir_trait_ufcs_resolves_imported_trait_impl_declared_before_trait_module() {
    let mir = compile_to_mir(
        r#"
        mod driver {
            use crate::api::Filesystem;

            pub struct RamFs { base: i32 }
            impl Filesystem for RamFs {
                fn lookup(&self, id: i32) -> i32 { self.base + id }
            }

            pub fn call_lookup(driver: &RamFs) -> i32 {
                Filesystem::lookup(driver, 2)
            }
        }

        mod api {
            pub trait Filesystem {
                fn lookup(&self, id: i32) -> i32;
            }
        }

        fn main() -> i32 {
            let fs = driver::RamFs { base: 40 };
            driver::call_lookup(&fs)
        }
    "#,
    );
    assert!(
        mir.contains("RamFs::lookup"),
        "imported trait UFCS call should resolve to the concrete impl symbol:\n{mir}"
    );
    assert!(
        !mir.contains("Filesystem::lookup"),
        "imported trait UFCS call should not lower to a bare trait method symbol:\n{mir}"
    );
}
