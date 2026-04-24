use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::hir_lower::LoweringContext;
use anyrc::resolve::Resolver;
use anyrc::typeck::{TypeChecker, TypeckResult, TyKind, IntTy, UintTy, FloatTy};

fn typecheck(src: &str) -> (TypeckResult, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let result = checker.check_crate(&hir);
    (result, interner)
}

fn typecheck_with_cfg(src: &str, cfg_flags: &[&str]) -> (TypeckResult, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    let flags = cfg_flags.iter().map(|flag| flag.to_string()).collect::<Vec<_>>();
    let cfg_ctx = anyrc::cfg::CfgContext::from_flags(&flags);
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    expand_macros(&mut krate, &mut interner);
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    expand_macros(&mut krate, &mut interner);
    anyrc::cfg::strip_cfg(&mut krate, &cfg_ctx, &interner);
    let mut lower_ctx = LoweringContext::new(&mut interner);
    let hir = lower_ctx.lower_crate(&krate);
    let mut resolver = Resolver::new(&mut interner);
    let resolve_result = resolver.resolve_crate(&hir);
    let mut checker = TypeChecker::new(&interner, &resolve_result);
    let result = checker.check_crate(&hir);
    (result, interner)
}

fn assert_type_ok(src: &str) {
    let (result, _) = typecheck(src);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

fn assert_type_error(src: &str, expected_msg: &str) {
    let (result, _) = typecheck(src);
    assert!(!result.errors.is_empty(), "expected error containing '{}'", expected_msg);
    assert!(result.errors.iter().any(|e| e.message.to_lowercase().contains(&expected_msg.to_lowercase())),
        "expected error containing '{}', got: {:?}", expected_msg,
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn infer_integer_literal() {
    assert_type_ok("fn main() { let x: i32 = 42; }");
}

#[test]
fn infer_from_annotation() {
    assert_type_ok("fn main() { let x: u64 = 42; }");
}

#[test]
fn infer_bool() {
    assert_type_ok("fn main() { let x: bool = true; }");
}

#[test]
fn primitive_assoc_from_is_typed() {
    assert_type_ok("fn main() { let b: u8 = 7; let x: u32 = u32::from(b); }");
}

#[test]
fn primitive_assoc_try_from_is_result_typed() {
    assert_type_ok("fn main() { let n: usize = 7; let x = u32::try_from(n); }");
}

#[test]
fn impl_method_body_resolves_self_enum_variants_against_impl_type() {
    assert_type_ok(r#"
        enum First {
            A,
            B,
        }

        enum Second {
            A,
            B,
        }

        impl First {
            fn from_u32(v: u32) -> Self {
                match v {
                    1 => Self::A,
                    _ => Self::B,
                }
            }
        }

        impl Second {
            fn from_u32(v: u32) -> Self {
                match v {
                    1 => Self::A,
                    _ => Self::B,
                }
            }
        }
    "#);
}

#[test]
fn self_enum_variant_resolution_wins_over_global_type_names() {
    assert_type_ok(r#"
        struct String {}

        enum SortType {
            String,
            Numeric,
        }

        impl SortType {
            fn from_u8(v: u8) -> Self {
                match v {
                    1 => Self::Numeric,
                    _ => Self::String,
                }
            }
        }
    "#);
}

#[test]
fn function_arg_autoderefs_nested_slice_references() {
    assert_type_ok(r#"
        fn takes_slice(item: &[u8]) -> bool {
            true
        }

        fn main() {
            let items: [&[u8]; 1] = [b"x"];
            for item in &items {
                let ok = takes_slice(item);
            }
        }
    "#);
}

#[test]
fn vec_u8_compares_with_byte_string_arrays() {
    assert_type_ok(r#"
        fn main() {
            let t: Vec<u8> = Vec::new();
            let is_time = &t == b"time";
            let is_date = t == b"date";
        }
    "#);
}

#[test]
fn if_arms_allow_byte_string_literals_with_different_lengths_for_slice_use() {
    assert_type_ok(r#"
        fn takes_slice(item: &[u8]) {}

        fn main() {
            let flag = true;
            takes_slice(if flag { b"Folder" } else { b"File name" });
        }
    "#);
}

#[test]
fn shift_rhs_accepts_usize_without_forcing_lhs_type() {
    assert_type_ok(r#"
        fn main() {
            let index: usize = 3;
            let mask = 1u64 << index;
        }
    "#);
}

#[test]
fn qualified_module_types_do_not_collide_by_leaf_name() {
    assert_type_ok(r#"
        mod fallback {
            pub struct TokenStream {}
            pub fn make() -> TokenStream { TokenStream {} }
        }

        use crate::fallback as imp;

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        fn wrap(inner: fallback::TokenStream) -> TokenStream {
            TokenStream { inner }
        }
    "#);
}

#[test]
fn qualified_assoc_fns_do_not_collide_by_leaf_name() {
    assert_type_ok(r#"
        mod fallback {
            pub struct TokenStream {}
            impl TokenStream {
                pub fn new() -> TokenStream { TokenStream {} }
            }
        }

        use crate::fallback as imp;

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        fn wrap() -> TokenStream {
            TokenStream { inner: imp::TokenStream::new() }
        }
    "#);
}

#[test]
fn assoc_from_selects_matching_impl_on_module_alias() {
    assert_type_ok(r#"
        mod fallback {
            pub struct TokenTree {}
            pub struct TokenStream {}

            impl TokenStream {
                pub fn from(tree: TokenTree) -> TokenStream {
                    TokenStream {}
                }
            }
        }

        use crate::fallback as imp;

        fn wrap(tree: fallback::TokenTree) -> fallback::TokenStream {
            imp::TokenStream::from(tree)
        }
    "#);
}

#[test]
fn assoc_from_identity_uses_target_type() {
    assert_type_ok(r#"
        mod fallback {
            pub struct TokenStream {}
        }

        use crate::fallback as imp;

        fn wrap(inner: fallback::TokenStream) -> fallback::TokenStream {
            imp::TokenStream::from(inner)
        }
    "#);
}

#[test]
fn assoc_from_identity_wins_when_single_impl_does_not_match() {
    assert_type_ok(r#"
        mod fallback {
            pub struct Foreign {}
            pub struct TokenStream {}

            impl TokenStream {
                pub fn from(value: Foreign) -> TokenStream {
                    TokenStream {}
                }
            }
        }

        use crate::fallback as imp;

        fn wrap(inner: fallback::TokenStream) -> fallback::TokenStream {
            imp::TokenStream::from(inner)
        }
    "#);
}

#[test]
fn cfg_selects_module_alias_before_wrapper_module() {
    let (result, _) = typecheck_with_cfg(r#"
        mod fallback {
            pub struct TokenStream {}
        }

        #[cfg(not(wrap_proc_macro))]
        use crate::fallback as imp;

        #[cfg(wrap_proc_macro)]
        mod imp {
            pub struct TokenStream {}
        }

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        impl TokenStream {
            fn _new_fallback(inner: fallback::TokenStream) -> Self {
                TokenStream {
                    inner: imp::TokenStream::from(inner),
                }
            }
        }
    "#, &[]);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn cfg_wrapper_module_assoc_from_uses_wrapper_impl() {
    let (result, _) = typecheck_with_cfg(r#"
        trait From<T> {
            fn from(value: T) -> Self;
        }

        mod fallback {
            pub struct TokenStream {}
        }

        #[cfg(not(wrap_proc_macro))]
        use crate::fallback as imp;

        #[cfg(wrap_proc_macro)]
        mod imp {
            use crate::fallback;

            pub struct TokenStream {}

            impl crate::From<fallback::TokenStream> for TokenStream {
                fn from(inner: fallback::TokenStream) -> TokenStream {
                    TokenStream {}
                }
            }
        }

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        impl TokenStream {
            fn _new_fallback(inner: fallback::TokenStream) -> Self {
                TokenStream {
                    inner: imp::TokenStream::from(inner),
                }
            }
        }
    "#, &["wrap_proc_macro"]);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn proc_macro2_style_fallback_wrappers_typecheck() {
    let (result, _) = typecheck_with_cfg(r#"
        trait From<T> {
            fn from(value: T) -> Self;
        }

        pub enum TokenTree {
            Group,
        }

        pub mod fallback {
            use crate::TokenTree;

            pub struct TokenStream {}
            pub struct Span {}
            pub struct Ident {}

            impl TokenStream {
                pub fn new() -> TokenStream { TokenStream {} }
            }

            impl From<TokenTree> for TokenStream {
                fn from(token: TokenTree) -> TokenStream {
                    TokenStream {}
                }
            }

            impl Ident {
                pub fn new_checked(string: &str, span: Span) -> Ident {
                    Ident {}
                }
            }
        }

        #[cfg(not(wrap_proc_macro))]
        use crate::fallback as imp;

        #[cfg(wrap_proc_macro)]
        mod imp {
            pub struct TokenStream {}
            pub struct Span {}
            pub struct Ident {}
        }

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        impl TokenStream {
            fn _new(inner: imp::TokenStream) -> Self {
                TokenStream { inner }
            }

            pub fn new() -> Self {
                TokenStream::_new(imp::TokenStream::new())
            }
        }

        impl From<TokenTree> for TokenStream {
            fn from(token: TokenTree) -> Self {
                TokenStream::_new(imp::TokenStream::from(token))
            }
        }

        pub struct Span {
            inner: imp::Span,
        }

        pub struct Ident {
            inner: imp::Ident,
        }

        impl Ident {
            fn _new(inner: imp::Ident) -> Self {
                Ident { inner }
            }

            pub fn new(string: &str, span: Span) -> Self {
                Ident::_new(imp::Ident::new_checked(string, span.inner))
            }
        }
    "#, &[]);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn proc_macro2_style_wrapper_from_overloads_typecheck() {
    let (result, _) = typecheck_with_cfg(r#"
        trait From<T> {
            fn from(value: T) -> Self;
        }

        pub enum TokenTree {
            Group,
        }

        pub mod fallback {
            use crate::TokenTree;

            pub struct TokenStream {}

            impl From<TokenTree> for TokenStream {
                fn from(token: TokenTree) -> TokenStream {
                    TokenStream {}
                }
            }
        }

        #[cfg(not(wrap_proc_macro))]
        use crate::fallback as imp;

        #[cfg(wrap_proc_macro)]
        mod imp {
            use crate::{fallback, TokenTree};

            pub struct TokenStream {}
            pub struct CompilerTokenStream {}

            impl From<CompilerTokenStream> for TokenStream {
                fn from(inner: CompilerTokenStream) -> TokenStream {
                    TokenStream {}
                }
            }

            impl From<fallback::TokenStream> for TokenStream {
                fn from(inner: fallback::TokenStream) -> TokenStream {
                    TokenStream {}
                }
            }

            impl From<TokenTree> for TokenStream {
                fn from(token: TokenTree) -> TokenStream {
                    TokenStream::from(fallback::TokenStream::from(token))
                }
            }
        }

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        impl TokenStream {
            fn _new(inner: imp::TokenStream) -> Self {
                TokenStream { inner }
            }

            fn _new_fallback(inner: fallback::TokenStream) -> Self {
                TokenStream {
                    inner: imp::TokenStream::from(inner),
                }
            }
        }

        impl From<TokenTree> for TokenStream {
            fn from(token: TokenTree) -> Self {
                TokenStream::_new(imp::TokenStream::from(token))
            }
        }
    "#, &["wrap_proc_macro"]);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn raw_pointer_add_result_can_initialize_local() {
    assert_type_ok(r#"
        struct Header {
            buckets: *const u32,
        }

        fn f(hash_ptr: *const u32) -> Header {
            let buckets = unsafe { hash_ptr.add(2) };
            Header { buckets }
        }
    "#);
}

#[test]
fn referenced_raw_struct_array_field_can_be_indexed() {
    assert_type_ok(r#"
        struct Elf64Ehdr {
            e_ident: [u8; 16],
        }

        fn magic(base: u64) -> bool {
            let ehdr = unsafe { &*(base as *const Elf64Ehdr) };
            ehdr.e_ident[0] == 0x7F
                || ehdr.e_ident[1] == b'E'
                || ehdr.e_ident[2] == b'L'
                || ehdr.e_ident[3] == b'F'
        }
    "#);
}

#[test]
fn str_as_bytes_matches_slice_parameter_after_pointer_add() {
    assert_type_ok(r#"
        struct Elf64Sym {
            st_name: u32,
        }

        struct Handle {
            strtab: *const u8,
        }

        fn cstr_eq(cstr: *const u8, name: &[u8]) -> bool {
            let _ = cstr;
            let _ = name;
            true
        }

        fn probe(handle: &Handle, sym: &Elf64Sym, name: &str) -> bool {
            unsafe { cstr_eq(handle.strtab.add(sym.st_name as usize), name.as_bytes()) }
        }
    "#);
}

#[test]
fn builtin_dll_exports_generated_loader_typechecks() {
    assert_type_ok(r#"
        mod core {
            pub mod mem {
                pub fn transmute_copy<T, U>(src: &T) -> U { loop {} }
            }
        }

        mod anyos_std {
            pub mod dll {
                pub fn dll_load(path: &str) -> usize {
                    let _ = path;
                    0
                }
            }
        }

        enum Option<T> {
            Some(T),
            None,
        }

        impl<T> Option<T> {
            fn as_ref(&self) -> Option<&T> { loop {} }
        }

        impl u64 {
            const MAX: u64 = 0;
        }

        dynlink::dll_exports! {
            lib_path: "/Libraries/libm.so",
            lib_struct: MathLib,
            symbols: {
                math_sqrt(x: f64) -> f64,
            }
        }
    "#);
}

#[test]
fn slice_iter_enumerate_binds_index_and_referenced_item() {
    assert_type_ok(r#"
        fn sum(bytes: &[u8]) -> usize {
            let mut out: usize = 0;
            for (i, &b) in bytes.iter().enumerate() {
                out = out + i + b as usize;
            }
            out
        }
    "#);
}

#[test]
fn for_loop_over_mut_vec_reference_binds_mut_referenced_tuple_fields() {
    assert_type_ok(r#"
        struct String;

        impl String {
            fn from(s: &str) -> String {
                let _ = s;
                String
            }
        }

        fn set(attrs: &mut Vec<(String, String)>, name: &str, value: &str) {
            for (k, v) in attrs {
                if k == name {
                    *v = String::from(value);
                    return;
                }
            }
        }
    "#);
}

#[test]
fn core_index_impl_supplies_index_expression_output() {
    assert_type_ok(r#"
        mod core {
            pub mod ops {
                pub trait Index<Idx> {
                    type Output;
                    fn index(&self, index: Idx) -> &Self::Output;
                }
            }
        }

        struct Value;

        impl core::ops::Index<&str> for Value {
            type Output = Value;
            fn index(&self, key: &str) -> &Value {
                let _ = key;
                self
            }
        }

        impl Value {
            fn as_bool(&self) -> bool { true }
        }

        fn get(v: Value) -> bool {
            v["enabled"].as_bool()
        }
    "#);
}

#[test]
fn root_unqualified_types_do_not_collide_with_nested_leaf_names() {
    assert_type_ok(r#"
        mod fallback {
            pub struct Span {}
            impl Span {
                pub fn call_site() -> Span { Span {} }
            }
        }

        use crate::fallback as imp;

        pub struct Span {
            inner: imp::Span,
        }

        impl Span {
            fn _new(inner: imp::Span) -> Self {
                Span { inner }
            }

            pub fn call_site() -> Self {
                Span::_new(imp::Span::call_site())
            }
        }

        fn get() -> Span {
            Span::call_site()
        }
    "#);
}

#[test]
fn struct_like_enum_variant_construction_resolves_parent_enum() {
    assert_type_ok(r#"
        mod helper {
            pub trait Sized {}
        }

        enum SizeInfo {
            Sized { size: usize },
            Other,
        }

        fn make() -> SizeInfo {
            SizeInfo::Sized { size: 0 }
        }

        fn read(info: SizeInfo) -> usize {
            match info {
                SizeInfo::Sized { size } => size,
                SizeInfo::Other => 0,
            }
        }
    "#);
}

#[test]
fn panic_macro_diverges_in_match_arm() {
    assert_type_ok(r#"
        enum SizeInfo {
            Sized { size: usize },
            Slice,
        }

        fn unpack(info: SizeInfo) -> (usize, SizeInfo) {
            match info {
                SizeInfo::Slice => panic!("cannot unpack"),
                SizeInfo::Sized { size } => (size, SizeInfo::Sized { size }),
            }
        }
    "#);
}

#[test]
fn cfg_attributed_panic_macro_is_expanded_after_strip() {
    let (result, _) = typecheck_with_cfg(r#"
        enum SizeInfo {
            Sized { size: usize },
            Slice,
        }

        macro_rules! const_panic {
            ($($arg:tt)+) => {{
                #[cfg(not(no_panic))]
                panic!($($arg)+);
                #[cfg(no_panic)]
                ()
            }};
        }

        fn unpack(info: SizeInfo) -> (usize, SizeInfo) {
            match info {
                SizeInfo::Slice => const_panic!("cannot unpack"),
                SizeInfo::Sized { size } => (size, SizeInfo::Sized { size }),
            }
        }
    "#, &[]);

    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn infer_fn_return() {
    assert_type_ok("fn foo() -> i32 { 42 } fn main() { let x: i32 = foo(); }");
}

#[test]
fn infer_binary_op() {
    assert_type_ok("fn main() { let x: i32 = 1 + 2; }");
}

#[test]
fn error_type_mismatch() {
    assert_type_error("fn main() { let x: u32 = true; }", "mismatch");
}

#[test]
fn infer_reference() {
    assert_type_ok("fn main() { let x: i32 = 5; let y: &i32 = &x; }");
}

#[test]
fn infer_if_else() {
    assert_type_ok("fn main() { let x: i32 = if true { 1 } else { 2 }; }");
}

#[test]
fn error_if_else_mismatch() {
    assert_type_error("fn main() { let x: i32 = if true { 1 } else { true }; }", "mismatch");
}

#[test]
fn infer_block_type() {
    assert_type_ok("fn main() { let x: i32 = { let y = 5; y }; }");
}

#[test]
fn infer_struct_construction() {
    assert_type_ok(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 1, y: 2 }; }
    "#);
}

#[test]
fn infer_field_access() {
    assert_type_ok(r#"
        struct Point { x: i32, y: i32 }
        fn main() { let p = Point { x: 1, y: 2 }; let v: i32 = p.x; }
    "#);
}

#[test]
fn check_fn_args() {
    assert_type_ok(r#"
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() { let x: i32 = add(1, 2); }
    "#);
}

#[test]
fn error_wrong_arg_type() {
    assert_type_error(r#"
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() { add(true, 2); }
    "#, "mismatch");
}

#[test]
fn error_wrong_arg_count() {
    assert_type_error(r#"
        fn add(a: i32, b: i32) -> i32 { a + b }
        fn main() { add(1); }
    "#, "argument");
}

#[test]
fn check_return_type() {
    assert_type_ok("fn foo() -> i32 { return 42; }");
}

#[test]
fn error_wrong_return_type() {
    assert_type_error("fn foo() -> i32 { return true; }", "mismatch");
}

#[test]
fn check_assign_types() {
    assert_type_ok("fn main() { let mut x: i32 = 0; x = 5; }");
}

#[test]
fn check_comparison_returns_bool() {
    assert_type_ok("fn main() { let x: bool = 1 < 2; }");
}

#[test]
fn check_logical_ops() {
    assert_type_ok("fn main() { let x: bool = true && false || true; }");
}

#[test]
fn infer_enum_variant() {
    assert_type_ok(r#"
        enum Color { Red, Green, Blue }
        fn main() { let c = Color::Red; }
    "#);
}

#[test]
fn block_like_statement_does_not_absorb_following_deref_expr() {
    assert_type_ok(r#"
        unsafe fn cstr_eq_sym(strtab: *const u8, offset: usize, name: &[u8]) -> bool {
            let s = strtab.add(offset);
            for (i, &b) in name.iter().enumerate() {
                if *s.add(i) != b { return false; }
            }
            *s.add(name.len()) == 0
        }
    "#);
}

#[test]
fn vec_new_infers_element_type_through_slice_coercion() {
    assert_type_ok(r#"
        fn main() {
            let data = Vec::new();
            let _text = core::str::from_utf8(&data);
        }
    "#);
}
