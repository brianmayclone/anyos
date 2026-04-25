use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use std::io::Write;

fn resolve_src(src: &str) -> (anyrc::resolve::ResolveResult, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut ctx = LoweringContext::new(&mut interner);
    let hir = ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
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

fn resolve_file_crate(src: &str, src_dir: &str, cfg_flags: &[&str]) -> (anyrc::resolve::ResolveResult, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    let cfg_ctx = anyrc::cfg::CfgContext::from_flags(
        &cfg_flags.iter().map(|flag| flag.to_string()).collect::<Vec<_>>(),
    );
    let loader = anyrc::loader::OsFileLoader;
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    expand_macros(&mut krate, &mut interner);
    anyrc::loader::resolve_includes(&mut krate, src_dir, &mut interner, &loader);
    anyrc::loader::resolve_modules(&mut krate, src_dir, &mut interner, &loader);
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    expand_macros(&mut krate, &mut interner);
    anyrc::loader::resolve_includes(&mut krate, src_dir, &mut interner, &loader);
    anyrc::loader::resolve_modules(&mut krate, src_dir, &mut interner, &loader);
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);

    let mut ctx = LoweringContext::new(&mut interner);
    let hir = ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
    let result = resolver.resolve_crate(&hir);
    (result, interner)
}

#[test]
fn resolve_local_variable() {
    assert_resolves("fn main() { let x = 5; let y = x; }");
}

#[test]
fn resolve_fn_call() {
    assert_resolves("fn foo() -> i32 { 0 } fn main() { foo(); }");
}

#[test]
fn error_undefined_variable() {
    assert_resolve_error("fn main() { let x = y; }", "not found");
}

#[test]
fn resolve_fn_params() {
    assert_resolves("fn add(a: i32, b: i32) -> i32 { a + b }");
}

#[test]
fn resolve_struct_and_fields() {
    assert_resolves("struct S { x: i32 } fn main() { let s = S { x: 1 }; }");
}

#[test]
fn resolve_enum_variants() {
    assert_resolves("enum E { A, B } fn main() { let x = E::A; }");
}

#[test]
fn resolve_impl_methods() {
    assert_resolves(r#"
        struct S {}
        impl S { fn new() -> S { S {} } }
        fn main() { S::new(); }
    "#);
}

#[test]
fn resolve_macro_generated_trait_assoc_fn_through_module_path() {
    assert_resolves(r#"
        macro_rules! declare_trait {
            () => {
                pub trait Error: Sized {
                    fn custom<T>(msg: T) -> Self
                    where
                        T: Display;
                    #[cold]
                    fn invalid_value() -> Self {
                        Error::custom()
                    }
                }
            }
        }

        mod de {
            pub trait Display {}
            declare_trait!();
        }

        fn visit() {
            de::Error::invalid_value();
        }
    "#);
}

#[test]
fn resolve_float_primitive_assoc_items() {
    assert_resolves(r#"
        fn main() {
            let a = f32::MAX;
            let b = f32::MIN;
            let c = f64::MAX;
            let d = f64::MIN;
            let e = f64::MIN_POSITIVE;
            let f = f64::from_bits(0);
            let g = char::is_whitespace(' ');
        }
    "#);
}

#[test]
fn resolve_extern_ordering_glob_variants() {
    assert_resolves(r#"
        fn main() {
            use core::cmp::Ordering::*;
            let a = Less;
            let b = Greater;
        }
    "#);
}

#[test]
fn resolve_root_module_alias_from_nested_module() {
    assert_resolves(r#"
        mod libgl_client {
            pub const GL_DEPTH_TEST: u32 = 1;
            pub fn init() -> bool { true }
        }

        mod workloads {
            mod gl3d_common {
                use libgl_client as gl;

                fn ensure_gl_init() {
                    if !gl::init() {}
                    let mode = gl::GL_DEPTH_TEST;
                }
            }
        }
    "#);
}

#[test]
fn resolve_self_associated_type_projection_in_trait() {
    assert_resolves(r#"
        struct NonNull<T> {}
        struct DstLayout {}

        pub unsafe trait KnownLayout {
            fn only_derive_is_allowed_to_implement_this_trait()
            where
                Self: Sized;

            type PointerMetadata: PointerMetadata;
            type MaybeUninit: ?Sized + KnownLayout<PointerMetadata = Self::PointerMetadata>;
            const LAYOUT: DstLayout;
            fn raw_from_ptr_len(bytes: NonNull<u8>, meta: Self::PointerMetadata) -> NonNull<Self>;
            fn pointer_to_metadata(ptr: *mut Self) -> Self::PointerMetadata;
        }

        trait PointerMetadata {}
    "#);
}

#[test]
fn resolve_nested_scopes() {
    assert_resolves(r#"
        fn main() {
            let x = 1;
            {
                let y = x;
                {
                    let z = y;
                }
            }
        }
    "#);
}

#[test]
fn resolve_shadowing() {
    assert_resolves(r#"
        fn main() {
            let x = 1;
            let x = x + 1;
            let y = x;
        }
    "#);
}

#[test]
fn error_undefined_type() {
    assert_resolve_error("fn foo(x: Nonexistent) {}", "not found");
}

#[test]
fn resolve_generic_params() {
    assert_resolves("fn id<T>(x: T) -> T { x }");
}

#[test]
fn resolve_forward_generic_param_in_bounds() {
    assert_resolves(r#"
        trait Cmp<Rhs> {}
        trait Unsigned {}
        struct PInt<U> { n: U }
        impl<Pl: Cmp<Pr> + Unsigned, Pr: Unsigned> Cmp<PInt<Pr>> for PInt<Pl> {}
    "#);
}

#[test]
fn resolve_self_in_struct_body() {
    assert_resolves(r#"
        struct Block<T> { value: T }
        struct BlockCtx<BS> {
            block: Block<Self>,
        }
    "#);
}

#[test]
fn resolve_core_convert_prelude_traits() {
    assert_resolves(r#"
        struct Wrapper<T> { inner: T }

        impl<T, Z> AsRef<T> for Wrapper<Z>
        where
            Z: AsRef<T>,
        {
            fn as_ref(&self) -> &T {
                self.inner.as_ref()
            }
        }

        impl<T, Z> AsMut<T> for Wrapper<Z>
        where
            Z: AsMut<T>,
        {
            fn as_mut(&mut self) -> &mut T {
                self.inner.as_mut()
            }
        }

        impl<T, Z> TryInto<T> for Wrapper<Z>
        where
            Z: TryInto<T>,
        {
            type Error = ();

            fn try_into(self) -> Result<T, Self::Error> {
                self.inner.try_into()
            }
        }
    "#);
}

#[test]
fn resolve_core_arch_intrinsics_as_compiler_known_names() {
    assert_resolves(r#"
        use core::arch::x86_64::*;

        fn add(a: __m256i, b: __m256i) {
            let c = _mm256_add_epi64(a, b);
        }
    "#);
}

#[test]
fn resolve_known_external_crate_group_imports_and_assoc_variants() {
    assert_resolves(r#"
        use proc_macro2::{Ident, Span};
        use quote::{ToTokens, TokenStreamExt as _};
        use syn::{Path, PathArguments, Type};
        use syn::meta::ParseNestedMeta;
        use syn::parse::ParseStream;

        fn main(path: Path, ty: Type, meta: ParseNestedMeta, input: ParseStream) {
            let span = Span::call_site();
            let ident = Ident::new("x", span);
            let args = PathArguments::AngleBracketed;
            let path_ty = Type::Path;
        }
    "#);
}

#[test]
fn resolve_enum_glob_imports_variants() {
    assert_resolves(r#"
        enum RenameRule {
            LowerCase,
            SnakeCase,
        }

        mod case {
            use super::RenameRule::*;

            fn main() {
                let a = LowerCase;
                let b = SnakeCase;
            }
        }
    "#);
}

#[test]
fn resolve_known_external_glob_imports() {
    assert_resolves(r#"
        use alloc::collections::*;
        use serde::de::*;
        use serde::ser::*;

        fn needs_alloc(
            heap: BinaryHeap<u32>,
            map: BTreeMap<u32, u32>,
            set: BTreeSet<u32>,
            queue: VecDeque<u32>,
        ) {
        }

        trait DecodeWithSerde<'de>: Deserialize<'de> + DeserializeSeed<'de> {}
        trait EncodeWithSerde: Serialize {}

        fn needs_de<'de, D, S, M, E, V>()
        where
            D: Deserializer<'de>,
            S: SeqAccess<'de>,
            M: MapAccess<'de>,
            E: EnumAccess<'de>,
            V: VariantAccess<'de>,
        {
        }

        fn needs_ser<S, A, T, TS, TV, M, ST, SV>()
        where
            S: Serializer,
            A: SerializeSeq,
            T: SerializeTuple,
            TS: SerializeTupleStruct,
            TV: SerializeTupleVariant,
            M: SerializeMap,
            ST: SerializeStruct,
            SV: SerializeStructVariant,
        {
        }
    "#);
}

#[test]
fn resolve_extern_glob_reexports_through_local_facade_module() {
    assert_resolves(r#"
        mod lib {
            mod core {
                pub use std::*;
            }

            pub use self::core::fmt::{self, Display};
            pub use self::core::marker::PhantomData;
        }

        mod value {
            use crate::lib::*;

            struct UnitDeserializer<E> {
                marker: PhantomData<E>,
            }

            fn fmt_value(f: &mut fmt::Formatter) {
            }

            trait UsesDisplay: Display {}
        }
    "#);
}

#[test]
fn resolve_extern_glob_reexports_from_macro_generated_facade_module() {
    assert_resolves(r#"
        macro_rules! crate_root {
            () => {
                mod lib {
                    mod core {
                        pub use std::*;
                    }

                    pub use self::core::{iter, num, str};
                    pub use self::core::{cmp, mem};
                    pub use self::core::cell::{Cell, RefCell};
                    pub use self::core::cmp::Reverse;
                    pub use self::core::fmt::{self, Debug, Display, Write as FmtWrite};
                    pub use self::core::marker::PhantomData;
                    pub use self::core::num::{Saturating, Wrapping};
                    pub use self::core::ops::{Bound, Range, RangeFrom, RangeInclusive, RangeTo};
                    pub use self::core::time::Duration;
                }
            };
        }

        crate_root!();

        mod value {
            use crate::lib::*;

            struct UnitDeserializer<E> {
                marker: PhantomData<E>,
            }

            fn fmt_value(f: &mut fmt::Formatter) -> fmt::Result {
                fmt::Result
            }

            fn use_modules<T>(it: iter::Fuse<T>) {
                let _ = num::Wrapping(1);
                let _ = str::from_utf8;
                let _ = cmp::min;
                let _ = mem::size_of::<usize>;
            }

            trait UsesDisplay: Display {}
            trait UsesDebug: Debug {}
        }
    "#);
}

#[test]
fn resolve_extern_glob_reexports_from_macro_facade_into_file_modules() {
    let dir = std::env::temp_dir().join(format!(
        "anyrc_resolve_facade_files_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("de")).unwrap();

    std::fs::write(
        dir.join("de").join("mod.rs"),
        r#"
            use crate::lib::*;

            pub mod value;

            trait UsesDisplay: Display {}
            fn fmt_de(f: &mut fmt::Formatter) -> fmt::Result {
                fmt::Result
            }
        "#,
    ).unwrap();

    let mut value_file = std::fs::File::create(dir.join("de").join("value.rs")).unwrap();
    value_file.write_all(
        br#"
            use crate::lib::*;

            struct UnitDeserializer<E> {
                marker: PhantomData<E>,
            }

            fn fmt_value(f: &mut fmt::Formatter) -> fmt::Result {
                fmt::Result
            }

            fn use_modules<T>(it: iter::Fuse<T>) {
                let _ = num::Wrapping(1);
                let _ = str::from_utf8;
                let _ = cmp::min;
                let _ = mem::size_of::<usize>;
            }
        "#,
    ).unwrap();

    let root = r#"
        macro_rules! crate_root {
            () => {
                mod lib {
                    mod core {
                        #[cfg(not(feature = "std"))]
                        pub use core::*;
                        #[cfg(feature = "std")]
                        pub use std::*;
                    }

                    pub use self::core::{iter, num, str};
                    #[cfg(any(feature = "std", feature = "alloc"))]
                    pub use self::core::{cmp, mem};
                    pub use self::core::fmt::{self, Debug, Display, Write as FmtWrite};
                    pub use self::core::marker::PhantomData;
                }

                pub mod de;
            };
        }

        crate_root!();
    "#;

    let src_dir = dir.to_string_lossy().to_string();
    let (result, _) = resolve_file_crate(
        root,
        &src_dir,
        &["feature=\"std\"", "target_os=\"anyos\"", "target_arch=\"x86_64\""],
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
}

#[test]
fn resolve_macro_use_file_module_facade_into_file_modules() {
    let dir = std::env::temp_dir().join(format!(
        "anyrc_resolve_macro_use_facade_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("de")).unwrap();

    std::fs::write(
        dir.join("crate_root.rs"),
        r#"
            macro_rules! crate_root {
                () => {
                    mod lib {
                        mod core {
                            #[cfg(feature = "std")]
                            pub use std::*;
                        }

                        pub use self::core::{iter, num, str};
                        pub use self::core::{cmp, mem};
                        pub use self::core::fmt::{self, Display};
                        pub use self::core::marker::PhantomData;
                    }

                    pub mod de;
                };
            }
        "#,
    ).unwrap();

    std::fs::write(
        dir.join("de").join("mod.rs"),
        r#"
            use crate::lib::*;
            pub mod value;
            trait UsesDisplay: Display {}
        "#,
    ).unwrap();

    std::fs::write(
        dir.join("de").join("value.rs"),
        r#"
            use crate::lib::*;

            struct UnitDeserializer<E> {
                marker: PhantomData<E>,
            }

            fn fmt_value(f: &mut fmt::Formatter) -> fmt::Result {
                fmt::Result
            }
        "#,
    ).unwrap();

    let root = r#"
        #[macro_use]
        mod crate_root;

        crate_root!();
    "#;

    let src_dir = dir.to_string_lossy().to_string();
    let (result, _) = resolve_file_crate(
        root,
        &src_dir,
        &["feature=\"std\"", "target_os=\"anyos\"", "target_arch=\"x86_64\""],
    );
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>()
    );
}

#[test]
fn resolve_bare_core_extern_glob_inside_local_core_facade_module() {
    assert_resolves(r#"
        mod lib {
            mod core {
                pub use core::*;
            }

            pub use self::core::fmt::{self, Display};
            pub use self::core::marker::PhantomData;
        }

        mod value {
            use crate::lib::*;

            struct UnitDeserializer<E> {
                marker: PhantomData<E>,
            }

            fn fmt_value(f: &mut fmt::Formatter) -> fmt::Result {
                fmt::Result
            }

            trait UsesDisplay: Display {}
        }
    "#);
}

#[test]
fn resolve_item_macro_generated_by_block_macro_expansion() {
    assert_resolves(r#"
        macro_rules! define_kind {
            () => {
                enum Kind {
                    A,
                    B,
                }
            };
        }

        macro_rules! use_kind {
            () => {
                define_kind!();
                let k = Kind::A;
            };
        }

        fn main() {
            use_kind!();
        }
    "#);
}

#[test]
fn resolve_trait_associated_functions_through_trait_path() {
    assert_resolves(r#"
        trait Display {}

        trait Error: Sized {
            fn custom<T>(msg: T) -> Self
            where
                T: Display;

            fn invalid_value<T>(msg: T) -> Self
            where
                T: Display,
            {
                Error::custom(msg)
            }
        }

        struct Unexpected;

        fn make_error<E: Error>(value: Unexpected) -> E {
            Error::invalid_value(value)
        }
    "#);
}

#[test]
fn cfg_false_strips_file_module_item_macro_invocation_before_expansion() {
    let tmp = std::env::temp_dir().join(format!(
        "anyrc-cfg-file-module-macro-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let module_path = tmp.join("impls.rs");
    let mut module = std::fs::File::create(&module_path).unwrap();
    write!(module, r#"
        macro_rules! make_kind {{
            () => {{
                enum OsStringKind {{
                    Unix,
                    Windows,
                }}

                impl OsStringKind {{
                    fn from_u64(value: u64) -> OsStringKind {{
                        match value {{
                            0 => OsStringKind::Unix,
                            _ => OsStringKind::Windows,
                        }}
                    }}
                }}
            }};
        }}

        #[cfg(all(feature = "std", any(unix, windows)))]
        make_kind!();

        #[cfg(all(feature = "std", any(unix, windows)))]
        fn decode(value: u64) -> OsStringKind {{
            OsStringKind::from_u64(value)
        }}

        fn always_available() -> u64 {{
            0
        }}
    "#).unwrap();

    let src = r#"
        mod impls;
        fn main() {}
    "#;
    let (result, _) = resolve_file_crate(src, tmp.to_str().unwrap(), &[
        "target_os=\"anyos\"",
        "feature=\"std\"",
    ]);
    let _ = std::fs::remove_dir_all(&tmp);

    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn resolve_macro_generated_file_module_trait_import_in_submodule() {
    let tmp = std::env::temp_dir().join(format!(
        "anyrc-file-module-trait-import-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("de")).unwrap();

    let mut de_mod = std::fs::File::create(tmp.join("de.rs")).unwrap();
    write!(de_mod, r#"
        mod impls;

        macro_rules! declare_error_trait {{
            (Error: Sized $(+ $($supertrait:ident)::+)*) => {{
                pub trait Error: Sized $(+ $($supertrait)::+)* {{
                    fn custom() -> Self;

                    fn invalid_value() -> Self {{
                        Error::custom()
                    }}
                }}
            }}
        }}

        pub trait StdError {{}}

        #[cfg(feature = "std")]
        declare_error_trait!(Error: Sized + StdError);

        #[cfg(not(feature = "std"))]
        declare_error_trait!(Error: Sized + Debug + Display);

        pub trait Deserialize {{}}
        pub trait Deserializer {{}}
        pub struct Unexpected;
        pub trait Visitor {{}}
    "#).unwrap();

    let mut impls = std::fs::File::create(tmp.join("de").join("impls.rs")).unwrap();
    write!(impls, r#"
        use crate::de::{{Deserialize, Deserializer, Error, Unexpected, Visitor}};

        fn visit<E>() -> E
        where
            E: Error,
        {{
            Error::invalid_value()
        }}
    "#).unwrap();

    let src = r#"
        macro_rules! crate_root {
            () => {
                pub mod de;
            }
        }

        crate_root!();
    "#;

    let (result, _) = resolve_file_crate(src, tmp.to_str().unwrap(), &[
        "target_os=\"anyos\"",
        "feature=\"std\"",
    ]);
    let _ = std::fs::remove_dir_all(&tmp);

    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn resolve_crate_prefixed_extern_crate_imports() {
    assert_resolves(r#"
        extern crate alloc;
        use crate::alloc::alloc::{handle_alloc_error, Layout};

        fn needs_layout(layout: Layout) {
            handle_alloc_error(layout);
        }
    "#);
}

#[test]
fn resolve_dynlink_imports_as_external_crate_items() {
    assert_resolves(r#"
        use dynlink::{dl_open, dl_sym, DlHandle};

        unsafe fn resolve(handle: &DlHandle, name: &str) {
            dl_sym(handle, name);
        }

        fn init() {
            let handle = match dl_open("/Libraries/libgl.so") {
                Some(h) => h,
                None => return,
            };
            unsafe {
                resolve(&handle, "gl_init");
            }
        }
    "#);
}

#[test]
fn resolve_trait_def() {
    assert_resolves("trait Foo { fn foo(&self) -> i32; }");
}

#[test]
fn resolve_trait_impl() {
    assert_resolves(r#"
        trait Foo { fn foo(&self) -> i32; }
        struct S {}
        impl Foo for S { fn foo(&self) -> i32 { 42 } }
    "#);
}

#[test]
fn resolve_const_and_static() {
    assert_resolves(r#"
        const MAX: i32 = 100;
        static COUNT: i32 = 0;
        fn main() { let x = MAX; let y = COUNT; }
    "#);
}

#[test]
fn resolve_type_alias() {
    assert_resolves(r#"
        type Num = i32;
        fn foo(x: Num) -> Num { x }
    "#);
}

#[test]
fn resolve_if_let() {
    assert_resolves(r#"
        enum Option { Some(i32), None }
        fn main() {
            let x = Option::Some(5);
        }
    "#);
}

#[test]
fn resolve_match_patterns() {
    assert_resolves(r#"
        fn main() {
            let x = 5;
            match x {
                0 => 0,
                n => n,
            }
        }
    "#);
}
