mod common;
use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType, ExternCrateSpec};
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
fn extern_interface_preserves_private_scope_imports() {
    let lib_source = r#"
        mod hidden {
            pub trait Marker {}
        }

        pub mod api {
            use super::hidden::Marker;
            pub trait Api: Marker {}
        }
    "#;
    let lib_options = CompileOptions {
        input: "provider.rs".to_string(),
        output: "libprovider.rlib".to_string(),
        emit: EmitKind::Rlib,
        crate_type: CrateType::Lib,
        crate_name: Some("provider".to_string()),
        ..CompileOptions::default()
    };
    let rlib = compile(lib_source, "provider.rs", &lib_options)
        .expect("provider compilation failed");

    static COUNTER: AtomicU64 = AtomicU64::new(1000);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_iface_test_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let rlib_path = dir.join("libprovider.rlib");
    {
        let mut f = std::fs::File::create(&rlib_path).unwrap();
        f.write_all(&rlib).unwrap();
        f.sync_all().unwrap();
    }

    let use_options = CompileOptions {
        input: "consumer.rs".to_string(),
        output: "consumer.o".to_string(),
        emit: EmitKind::Obj,
        crate_type: CrateType::Lib,
        crate_name: Some("consumer".to_string()),
        extern_crates: vec![ExternCrateSpec {
            name: "provider".to_string(),
            rlib_path: rlib_path.to_string_lossy().into_owned(),
        }],
        ..CompileOptions::default()
    };
    let result = compile("fn touch() {}", "consumer.rs", &use_options);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_ok(),
        "extern interface lost private use imports: {:?}",
        result.err().map(|errs| errs.into_iter().map(|e| e.message).collect::<Vec<_>>())
    );
}

#[test]
fn resolver_handles_macro_generated_float_type_alias_in_module() {
    let source = r#"
        pub struct PhantomData<T>;
        pub trait ByteOrder {}
        pub enum BigEndian {}

        macro_rules! define_type {
            (
                $name:ident,
                $native:ident,
                $bytes:expr,
                $from:path,
                $to:path,
                [$($larger:ty),*]
            ) => {
                pub struct $name<O>([u8; $bytes], PhantomData<O>);

                impl<O: ByteOrder> $name<O> {
                    pub fn new(n: $native) -> $name<O> {
                        $name($to(n), PhantomData)
                    }

                    pub fn get(self) -> $native {
                        $from(self.0)
                    }
                }

                $(
                    impl<O: ByteOrder> From<$name<O>> for $larger {
                        fn from(x: $name<O>) -> $larger {
                            x.get().into()
                        }
                    }
                )*
            };
        }

        macro_rules! module {
            ($name:ident, $trait:ident) => {
                pub mod $name {
                    use super::$trait;
                    pub type F32 = crate::F32<$trait>;
                }
            };
        }

        mod f32_ext {
            pub fn from_be_bytes(_: [u8; 4]) -> f32 { 0.0 }
            pub fn to_be_bytes(_: f32) -> [u8; 4] { [0, 0, 0, 0] }
        }

        define_type!(F32, f32, 4, f32_ext::from_be_bytes, f32_ext::to_be_bytes, [f64]);
        module!(big_endian, BigEndian);

        pub fn touch(_: big_endian::F32) {}
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test.o".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: Some("test".to_string()),
        ..CompileOptions::default()
    };
    let result = compile(source, "test.rs", &options);
    assert!(
        result.is_ok(),
        "resolver failed on macro-generated F32 alias: {:?}",
        result.err().map(|errs| errs.into_iter().map(|e| e.message).collect::<Vec<_>>())
    );
}

#[test]
fn extern_interface_preserves_private_consts_used_in_public_array_lengths() {
    let lib_source = r#"
        pub mod args {
            const BASE: usize = 16;
            const MAX_POSITIONAL: usize = BASE + 16;

            pub struct ParsedArgs<'a> {
                pub positional: [&'a str; MAX_POSITIONAL],
            }
        }
    "#;
    let lib_options = CompileOptions {
        input: "provider.rs".to_string(),
        output: "libprovider.rlib".to_string(),
        emit: EmitKind::Rlib,
        crate_type: CrateType::Lib,
        crate_name: Some("provider".to_string()),
        ..CompileOptions::default()
    };
    let rlib = compile(lib_source, "provider.rs", &lib_options)
        .expect("provider compilation failed");

    static COUNTER: AtomicU64 = AtomicU64::new(2000);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_iface_const_test_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let rlib_path = dir.join("libprovider.rlib");
    {
        let mut f = std::fs::File::create(&rlib_path).unwrap();
        f.write_all(&rlib).unwrap();
        f.sync_all().unwrap();
    }

    let use_options = CompileOptions {
        input: "consumer.rs".to_string(),
        output: "consumer.o".to_string(),
        emit: EmitKind::Obj,
        crate_type: CrateType::Lib,
        crate_name: Some("consumer".to_string()),
        extern_crates: vec![ExternCrateSpec {
            name: "provider".to_string(),
            rlib_path: rlib_path.to_string_lossy().into_owned(),
        }],
        ..CompileOptions::default()
    };
    let result = compile("fn touch() {}", "consumer.rs", &use_options);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_ok(),
        "extern interface lost private array-length consts: {:?}",
        result.err().map(|errs| errs.into_iter().map(|e| e.message).collect::<Vec<_>>())
    );
}

#[test]
fn extern_interface_preserves_transitive_private_type_dependencies() {
    let lib_source = r#"
        pub mod layout {
            pub struct DstLayout {
                size_info: SizeInfo,
            }

            pub(crate) enum SizeInfo<E = usize> {
                Sized { size: usize },
                SliceDst(TrailingSliceLayout<E>),
            }

            pub(crate) struct TrailingSliceLayout<E = usize> {
                offset: usize,
                elem_size: E,
            }
        }
    "#;
    let lib_options = CompileOptions {
        input: "provider.rs".to_string(),
        output: "libprovider.rlib".to_string(),
        emit: EmitKind::Rlib,
        crate_type: CrateType::Lib,
        crate_name: Some("provider".to_string()),
        ..CompileOptions::default()
    };
    let rlib = compile(lib_source, "provider.rs", &lib_options)
        .expect("provider compilation failed");

    static COUNTER: AtomicU64 = AtomicU64::new(2250);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_iface_type_test_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let rlib_path = dir.join("libprovider.rlib");
    {
        let mut f = std::fs::File::create(&rlib_path).unwrap();
        f.write_all(&rlib).unwrap();
        f.sync_all().unwrap();
    }

    let use_options = CompileOptions {
        input: "consumer.rs".to_string(),
        output: "consumer.o".to_string(),
        emit: EmitKind::Obj,
        crate_type: CrateType::Lib,
        crate_name: Some("consumer".to_string()),
        extern_crates: vec![ExternCrateSpec {
            name: "provider".to_string(),
            rlib_path: rlib_path.to_string_lossy().into_owned(),
        }],
        ..CompileOptions::default()
    };
    let result = compile("fn touch() {}", "consumer.rs", &use_options);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_ok(),
        "extern interface lost transitive private types: {:?}",
        result.err().map(|errs| errs.into_iter().map(|e| e.message).collect::<Vec<_>>())
    );
}

#[test]
fn item_include_splices_source_before_resolve() {
    static COUNTER: AtomicU64 = AtomicU64::new(2500);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_include_test_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("included.rs"),
        "pub struct Included { pub value: i32 }\n",
    ).unwrap();

    let source = r#"
        mod child {
            pub fn make() -> crate::Included {
                crate::Included { value: 7 }
            }
        }

        include!("included.rs");
    "#;
    let options = CompileOptions {
        input: dir.join("lib.rs").to_string_lossy().into_owned(),
        output: dir.join("out.o").to_string_lossy().into_owned(),
        emit: EmitKind::Obj,
        crate_type: CrateType::Lib,
        crate_name: Some("include_test".to_string()),
        src_dir: Some(dir.to_string_lossy().into_owned()),
        ..CompileOptions::default()
    };
    let result = compile(source, "lib.rs", &options);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_ok(),
        "item include was not visible during module resolution: {:?}",
        result.err().map(|errs| errs.into_iter().map(|e| e.message).collect::<Vec<_>>())
    );
}

#[test]
fn nested_module_can_use_extern_crate_item() {
    let lib_source = r#"
        pub mod race {
            pub use self::once_box::OnceBox;
            mod once_box {
                pub struct OnceBox<T> {
                    __private: (),
                }
                impl<T> OnceBox<T> {
                    pub const fn new() -> Self {
                        loop {}
                    }
                }
            }
        }
    "#;
    let lib_options = CompileOptions {
        input: "once_cell.rs".to_string(),
        output: "libonce_cell.rlib".to_string(),
        emit: EmitKind::Rlib,
        crate_type: CrateType::Lib,
        crate_name: Some("once_cell".to_string()),
        ..CompileOptions::default()
    };
    let rlib = compile(lib_source, "once_cell.rs", &lib_options)
        .expect("once_cell provider compilation failed");

    static COUNTER: AtomicU64 = AtomicU64::new(2000);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_extern_use_test_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let rlib_path = dir.join("libonce_cell.rlib");
    {
        let mut f = std::fs::File::create(&rlib_path).unwrap();
        f.write_all(&rlib).unwrap();
        f.sync_all().unwrap();
    }

    let consumer = r#"
        mod random_state {
            use once_cell::race::OnceBox;
            static SEEDS: OnceBox<u8> = OnceBox::new();
        }
    "#;
    let use_options = CompileOptions {
        input: "consumer.rs".to_string(),
        output: "consumer.o".to_string(),
        emit: EmitKind::Obj,
        crate_type: CrateType::Lib,
        crate_name: Some("consumer".to_string()),
        extern_crates: vec![ExternCrateSpec {
            name: "once_cell".to_string(),
            rlib_path: rlib_path.to_string_lossy().into_owned(),
        }],
        ..CompileOptions::default()
    };
    let result = compile(consumer, "consumer.rs", &use_options);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_ok(),
        "nested module extern use failed: {:?}",
        result.err().map(|errs| errs.into_iter().map(|e| e.message).collect::<Vec<_>>())
    );
}

#[test]
fn cfg_if_keeps_extern_use_in_nested_module() {
    let lib_source = r#"
        pub mod race {
            pub use self::once_box::OnceBox;
            mod once_box {
                pub struct OnceBox<T> {
                    __private: (),
                }
                impl<T> OnceBox<T> {
                    pub const fn new() -> Self {
                        loop {}
                    }
                }
            }
        }
    "#;
    let lib_options = CompileOptions {
        input: "once_cell.rs".to_string(),
        output: "libonce_cell.rlib".to_string(),
        emit: EmitKind::Rlib,
        crate_type: CrateType::Lib,
        crate_name: Some("once_cell".to_string()),
        ..CompileOptions::default()
    };
    let rlib = compile(lib_source, "once_cell.rs", &lib_options)
        .expect("once_cell provider compilation failed");

    static COUNTER: AtomicU64 = AtomicU64::new(3000);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("anyrc_cfg_if_extern_use_test_{}_{}", std::process::id(), id));
    std::fs::create_dir_all(&dir).unwrap();
    let rlib_path = dir.join("libonce_cell.rlib");
    {
        let mut f = std::fs::File::create(&rlib_path).unwrap();
        f.write_all(&rlib).unwrap();
        f.sync_all().unwrap();
    }

    let consumer = r#"
        mod random_state {
            cfg_if::cfg_if! {
                if #[cfg(not(all(target_arch = "arm", target_os = "none")))] {
                    use once_cell::race::OnceBox;
                }
            }
            static SEEDS: OnceBox<u8> = OnceBox::new();
        }
    "#;
    let use_options = CompileOptions {
        input: "consumer.rs".to_string(),
        output: "consumer.o".to_string(),
        emit: EmitKind::Obj,
        crate_type: CrateType::Lib,
        crate_name: Some("consumer".to_string()),
        extern_crates: vec![ExternCrateSpec {
            name: "once_cell".to_string(),
            rlib_path: rlib_path.to_string_lossy().into_owned(),
        }],
        cfg_flags: vec![
            "target_arch=\"x86_64\"".to_string(),
            "target_os=\"anyos\"".to_string(),
        ],
        ..CompileOptions::default()
    };
    let result = compile(consumer, "consumer.rs", &use_options);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.is_ok(),
        "cfg_if extern use failed: {:?}",
        result.err().map(|errs| errs.into_iter().map(|e| e.message).collect::<Vec<_>>())
    );
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
fn cfg_false_strips_impl_block() {
    let source = r#"
        struct S;

        #[cfg(feature = "std")]
        impl MissingTrait for S {
            fn missing(&self) -> MissingType { missing_symbol }
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
    assert!(result.is_ok(), "cfg-disabled impl was typechecked: {:?}", result.err());
}

#[test]
fn cfg_false_strips_const_and_macro_items() {
    let source = r#"
        #[cfg(feature = "nightly-only")]
        const BAD: MissingType = missing_symbol;

        #[cfg(feature = "nightly-only")]
        macro_rules! bad_macro {
            () => { missing_symbol }
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
    assert!(result.is_ok(), "cfg-disabled const/macro item was typechecked: {:?}", result.err());
}

#[test]
fn cfg_false_strips_statements_inside_const_initializer() {
    let source = r#"
        trait Marker {}

        macro_rules! unsafe_impl {
            ($ty:ty) => {
                impl Marker for $ty {}
            };
        }

        const _: () = unsafe {
            #[cfg(feature = "float-nightly")]
            unsafe_impl!(f16);
        };

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
    assert!(result.is_ok(), "cfg-disabled const initializer statement was typechecked: {:?}", result.err());
}

#[test]
fn cfg_false_strips_item_macro_call() {
    let source = r#"
        macro_rules! impl_marker {
            ($ty:ty) => {
                impl Marker for $ty {}
            };
        }

        trait Marker {}

        #[cfg(feature = "float-nightly")]
        impl_marker!(f16);

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
    assert!(result.is_ok(), "cfg-disabled item macro call was expanded/typechecked: {:?}", result.err());
}

#[test]
fn cfg_false_strips_struct_literal_fields() {
    let source = r#"
        struct Cursor {
            rest: i32,
            #[cfg(span_locations)]
            off: u32,
        }

        fn make(rest: i32) -> Cursor {
            Cursor {
                rest,
                #[cfg(span_locations)]
                off: missing_symbol,
            }
        }
    "#;
    let options = CompileOptions {
        input: "test.rs".to_string(),
        output: "test.o".to_string(),
        emit: EmitKind::Obj,
        opt_level: 0,
        crate_type: CrateType::Lib,
        crate_name: Some("test".to_string()),
        ..CompileOptions::default()
    };
    let result = compile(source, "test.rs", &options);
    assert!(result.is_ok(), "cfg-disabled struct literal field was typechecked: {:?}", result.err());
}

#[test]
fn block_items_are_visible_before_declaration() {
    let source = r#"
        fn main() -> i32 {
            struct UsesLater {
                field: Later,
            }
            enum Later { Value }
            0
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
    assert!(result.is_ok(), "block item forward reference failed: {:?}", result.err());
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
