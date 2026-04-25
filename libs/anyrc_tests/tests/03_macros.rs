use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::macros::expand_macros;
use anyrc::ast::*;

fn parse_and_expand(src: &str) -> Crate {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    krate
}

fn parse_and_expand_with_interner(src: &str) -> (Crate, Interner) {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    let mut krate = parser.parse_crate();
    expand_macros(&mut krate, &mut interner);
    (krate, interner)
}

#[test]
fn expand_simple_macro() {
    let krate = parse_and_expand(r#"
        macro_rules! five {
            () => { 5 }
        }
        fn main() -> i32 { five!() }
    "#);
    // After expansion, five!() should be replaced with the literal 5
    match &krate.items[1] {
        Item::Fn(f) => {
            let body = f.body.as_ref().unwrap();
            match &body.stmts[0] {
                Stmt::Expr(Expr::Lit(Literal::Int(5), _)) => {} // ok
                other => panic!("expected literal 5, got {:?}", std::mem::discriminant(match other { Stmt::Expr(e) => e, _ => panic!() })),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn expand_macro_with_expr_arg() {
    let krate = parse_and_expand(r#"
        macro_rules! double {
            ($x:expr) => { $x + $x }
        }
        fn main() -> i32 { double!(21) }
    "#);
    match &krate.items[1] {
        Item::Fn(f) => {
            let body = f.body.as_ref().unwrap();
            // Should be 21 + 21
            match &body.stmts[0] {
                Stmt::Expr(Expr::Binary(BinOp::Add, _, _, _)) => {}
                _ => panic!("expected binary add"),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn expand_global_app_state_builtin_item_macro() {
    let (krate, interner) = parse_and_expand_with_interner(r#"
        struct AppState {}
        anyos_std::global_app_state!(AppState);
    "#);

    assert_eq!(krate.items.len(), 3);
    match &krate.items[1] {
        Item::Static(s) => assert_eq!(interner.resolve(s.name), "APP"),
        _ => panic!("expected generated APP static"),
    }
    match &krate.items[2] {
        Item::Fn(f) => assert_eq!(interner.resolve(f.name), "app"),
        _ => panic!("expected generated app accessor"),
    }
}

#[test]
fn expand_format_macro_preserves_escaped_quotes_in_string_args() {
    let krate = parse_and_expand(r#"
        fn main(attr_name: i32, meta_item_name: i32) {
            format!(
                "expected serde {} attribute to be a string: `{} = \"...\"`",
                attr_name,
                meta_item_name,
            );
        }
    "#);

    match &krate.items[0] {
        Item::Fn(f) => {
            let body = f.body.as_ref().unwrap();
            let Stmt::Semi(Expr::Call(_, args, _), _) = &body.stmts[0] else {
                panic!("expected expanded format call");
            };
            match &args[0] {
                Expr::Lit(Literal::String(s), _) => {
                    assert_eq!(s, "expected serde {} attribute to be a string: `{} = \"...\"`");
                }
                _ => panic!("expected first format arg to remain a string literal"),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn expand_format_macro_strips_named_argument_labels() {
    let (krate, interner) = parse_and_expand_with_interner(r#"
        fn main(name: &str) {
            format!("hello {name}", name = name);
        }
    "#);

    match &krate.items[0] {
        Item::Fn(f) => {
            let body = f.body.as_ref().unwrap();
            let Stmt::Semi(Expr::Call(_, args, _), _) = &body.stmts[0] else {
                panic!("expected expanded format call");
            };
            assert_eq!(args.len(), 2);
            match &args[1] {
                Expr::Path(path) => {
                    let ident = path.segments.last().unwrap().ident;
                    assert_eq!(interner.resolve(ident), "name");
                }
                Expr::Assign(_, _, _) => panic!("named format argument label leaked as assignment"),
                _ => panic!("expected named argument value to be preserved as a path"),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn expand_quote_macros_to_proc_macro2_token_stream_constructor() {
    let krate = parse_and_expand(r#"
        fn main(span: i32) {
            quote! { struct Demo; };
            quote_spanned!(span=> impl Demo {});
        }
    "#);

    match &krate.items[0] {
        Item::Fn(f) => {
            let body = f.body.as_ref().unwrap();
            for stmt in &body.stmts {
                let Stmt::Semi(Expr::Call(callee, args, _), _) = stmt else {
                    panic!("expected quote macro to expand to constructor call");
                };
                assert!(args.is_empty());
                let Expr::Path(path) = callee.as_ref() else {
                    panic!("expected path callee");
                };
                let segments = path
                    .segments
                    .iter()
                    .map(|segment| segment.ident)
                    .collect::<Vec<_>>();
                assert_eq!(segments.len(), 3);
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn expand_macro_with_repetition() {
    let krate = parse_and_expand(r#"
        macro_rules! count {
            ($($x:expr),*) => { 0 $(+ 1)* }
        }
        fn main() -> i32 { count!(a, b, c) }
    "#);
    // count!(a, b, c) should expand to 0 + 1 + 1 + 1
    match &krate.items[1] {
        Item::Fn(f) => {
            assert!(f.body.is_some());
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn expand_unseparated_ty_repetition() {
    let krate = parse_and_expand(r#"
        trait SeekNum {}

        macro_rules! impl_seek_num {
            {$($t:ty )*} => {
                $(
                    impl SeekNum for $t {}
                )*
            };
        }

        impl_seek_num! { i32 u32 u64 u128 usize }
    "#);
    assert_eq!(krate.items.len(), 7);
}

#[test]
fn expand_macro_with_ident_arg() {
    let krate = parse_and_expand(r#"
        macro_rules! make_fn {
            ($name:ident) => { fn $name() -> i32 { 42 } }
        }
        make_fn!(hello);
    "#);
    // Should have two items: the macro def and the generated fn
    assert!(krate.items.len() >= 2);
}

#[test]
fn expand_syn_style_ast_struct_keyword_idents_and_meta_attrs() {
    let krate = parse_and_expand(r#"
        macro_rules! ast_struct {
            (
                $(#[$attr:meta])*
                $pub:ident $struct:ident $name:ident $body:tt
            ) => {
                $(#[$attr])* $pub $struct $name $body
            };
        }

        ast_struct! {
            #[doc = "demo"]
            pub struct LitStr {
                repr: usize,
            }
        }
    "#);
    assert!(krate.items.iter().any(|item| matches!(item, Item::Struct(s) if s.fields.len() == 1)));
}

#[test]
fn expand_syn_style_punctuation_struct_repetition() {
    let (krate, interner) = parse_and_expand_with_interner(r#"
        struct Span;

        macro_rules! define_punctuation_structs {
            ($($token:literal pub struct $name:ident/$len:tt #[doc = $usage:literal])*) => {
                $(
                    #[doc = concat!("Token `", $token, "`.")]
                    pub struct $name {
                        pub spans: [Span; $len],
                    }
                )*
            };
        }

        define_punctuation_structs! {
            "_" pub struct Underscore/1 /// wildcard patterns
            "(" pub struct Paren/1 /// parentheses
        }
    "#);

    assert!(krate.items.iter().any(|item| {
        matches!(item, Item::Struct(s) if interner.resolve(s.name) == "Underscore")
    }));
    assert!(krate.items.iter().any(|item| {
        matches!(item, Item::Struct(s) if interner.resolve(s.name) == "Paren")
    }));
}

#[test]
fn expand_prefers_local_macro_definition_over_same_named_macro_elsewhere() {
    let (krate, interner) = parse_and_expand_with_interner(r#"
        mod a {
            macro_rules! make {
                () => {
                    struct A {}
                }
            }
        }

        mod b {
            macro_rules! make {
                () => {
                    struct B {}
                }
            }

            make!();
        }
    "#);

    let Item::Mod(module) = &krate.items[1] else {
        panic!("expected module");
    };
    let items = module.items.as_ref().expect("module should have items");
    assert!(items.iter().any(|item| {
        matches!(item, Item::Struct(s) if interner.resolve(s.name) == "B")
    }));
    assert!(!items.iter().any(|item| {
        matches!(item, Item::Struct(s) if interner.resolve(s.name) == "A")
    }));
}

#[test]
fn macro_not_found_is_preserved() {
    // Unknown macros should remain as MacroCall (or could be an error)
    let krate = parse_and_expand(r#"
        fn main() { unknown_macro!(1, 2, 3); }
    "#);
    // Should not crash
    assert_eq!(krate.items.len(), 1);
}

#[test]
fn empty_expr_macro_expansion_is_not_parsed_as_expr() {
    let krate = parse_and_expand(r#"
        macro_rules! maybe_empty {
            () => {}
        }

        fn main() {
            maybe_empty!();
        }
    "#);
    assert_eq!(krate.items.len(), 2);
}

#[test]
fn expand_cfg_if_items() {
    let krate = parse_and_expand(r#"
        cfg_if::cfg_if! {
            if #[cfg(target_arch = "x86_64")] {
                pub use crate::fallback::AHasher;
            } else if #[cfg(target_arch = "aarch64")] {
                mod aes_hash;
            } else {
                mod fallback_hash;
            }
        }
    "#);
    assert_eq!(krate.items.len(), 3);
    for item in &krate.items {
        match item {
            Item::Use(u) => assert_eq!(u.attrs.len(), 1),
            Item::Mod(m) => assert_eq!(m.attrs.len(), 1),
            _ => panic!("expected cfg_if expansion item"),
        }
    }
}

#[test]
fn expand_cfg_if_protects_macro_calls_inside_item_branches() {
    let krate = parse_and_expand(r#"
        macro_rules! platform_items {
            () => {
                mod selected_a;
                mod selected_b;
            }
        }

        cfg_if::cfg_if! {
            if #[cfg(target_os = "switch")] {
                platform_items!();
            } else {
                mod fallback;
            }
        }
    "#);

    let mut cfg_protected_mods = 0;
    for item in &krate.items {
        if let Item::Mod(m) = item {
            assert_eq!(m.attrs.len(), 1);
            cfg_protected_mods += 1;
        }
    }
    assert_eq!(cfg_protected_mods, 3);
}

#[test]
fn expand_cfg_if_statements() {
    let krate = parse_and_expand(r#"
        fn main() {
            cfg_if! {
                if #[cfg(target_arch = "x86_64")] {
                    let tokens = ();
                } else {
                    let tokens = ();
                }
            }

            tokens;
        }
    "#);
    let Item::Fn(f) = &krate.items[0] else {
        panic!("expected fn");
    };
    let body = f.body.as_ref().unwrap();
    assert!(matches!(body.stmts[0], Stmt::Attributed(_, _, _)));
    assert!(matches!(body.stmts[1], Stmt::Attributed(_, _, _)));
    assert!(matches!(body.stmts[2], Stmt::Semi(Expr::Path(_), _)));
}

#[test]
fn expand_attributed_statement_macro_to_item() {
    let krate = parse_and_expand(r#"
        macro_rules! make_mod {
            () => {
                #[cfg_attr(doc_cfg, doc(cfg(rust = "1.89.0")))]
                mod generated {}
            }
        }

        fn main() {
            #[cfg(not(disabled))]
            make_mod!();
        }
    "#);
    let Item::Fn(f) = &krate.items[1] else {
        panic!("expected fn");
    };
    let body = f.body.as_ref().unwrap();
    let Stmt::Item(Item::Mod(m)) = &body.stmts[0] else {
        panic!("expected generated module item");
    };
    assert_eq!(m.attrs.len(), 2);
}

#[test]
fn expand_nested_macro_calls_inside_generated_module() {
    let krate = parse_and_expand(r#"
        macro_rules! module {
            ($name:ident) => {
                pub mod $name {
                    module!(@ty U16);
                }
            };
            (@ty $ty:ident) => {
                pub type $ty = u16;
            };
        }

        module!(big_endian);
    "#);
    let Item::Mod(m) = &krate.items[1] else {
        panic!("expected generated module");
    };
    let items = m.items.as_ref().unwrap();
    assert!(matches!(items[0], Item::TypeAlias(_)));
}

#[test]
fn expand_macro_path_fragments() {
    let krate = parse_and_expand(r#"
        macro_rules! make_fn {
            ($name:ident, $ctor:path) => {
                pub fn $name() -> u16 {
                    $ctor(1)
                }
            };
        }

        make_fn!(from_be, u16::from_be);
    "#);
    assert!(matches!(krate.items[1], Item::Fn(_)));
}

#[test]
fn expand_macro_path_fragments_with_float_primitive_idents() {
    let krate = parse_and_expand(r#"
        macro_rules! define_type {
            (
                $name:ident,
                $native:ident,
                $from:path,
                $to:path
            ) => {
                pub struct $name([u8; 4]);

                impl $name {
                    pub fn get(self) -> $native {
                        $from(self.0)
                    }
                    pub fn new(value: $native) -> Self {
                        Self($to(value))
                    }
                }
            };
        }

        mod f32_ext {
            pub fn from_be_bytes(_: [u8; 4]) -> f32 { 0.0 }
            pub fn to_be_bytes(_: f32) -> [u8; 4] { [0, 0, 0, 0] }
        }

        define_type!(F32, f32, f32_ext::from_be_bytes, f32_ext::to_be_bytes);

        pub mod big_endian {
            pub type F32 = crate::F32;
        }
    "#);
    assert!(krate.items.iter().any(|item| matches!(item, Item::Struct(_))));
    assert!(krate.items.iter().any(|item| {
        matches!(
            item,
            Item::Mod(m)
                if m.items.as_ref().is_some_and(|items| {
                    items.iter().any(|item| matches!(item, Item::TypeAlias(_)))
                })
        )
    }));
}

#[test]
fn expand_doc_comment_wrapped_float_type_macro_and_module_alias() {
    let krate = parse_and_expand(r#"
        struct PhantomData<T>;
        trait ByteOrder {}
        enum BigEndian {}

        macro_rules! doc_comment {
            ($doc:expr, $($tt:tt)*) => {
                #[doc = $doc]
                $($tt)*
            };
        }

        macro_rules! define_type {
            (
                $article:ident,
                $description:expr,
                $name:ident,
                $native:ident,
                $bits:expr,
                $bytes:expr,
                $from_be_fn:path,
                $to_be_fn:path,
                $number_kind:tt,
                [$($larger_native:ty),*],
                [$($larger_byteorder:ident),*]
            ) => {
                doc_comment! {
                    concat!($description, " stored in a given byte order."),
                    #[derive(Copy, Clone, Eq, PartialEq, Hash)]
                    #[cfg_attr(any(feature = "derive", test), derive(KnownLayout, Immutable))]
                    #[repr(transparent)]
                    pub struct $name<O>([u8; $bytes], PhantomData<O>);
                }

                impl<O: ByteOrder> $name<O> {
                    pub fn new(n: $native) -> $name<O> {
                        $name($to_be_fn(n), PhantomData)
                    }

                    pub fn get(self) -> $native {
                        $from_be_fn(self.0)
                    }
                }

                $(
                    impl<O: ByteOrder> From<$name<O>> for $larger_native {
                        fn from(x: $name<O>) -> $larger_native {
                            x.get().into()
                        }
                    }
                )*

                $(
                    impl<O: ByteOrder, P: ByteOrder> From<$name<O>> for $larger_byteorder<P> {
                        fn from(x: $name<O>) -> $larger_byteorder<P> {
                            $larger_byteorder::new(x.get().into())
                        }
                    }
                )*
            };
        }

        macro_rules! module {
            ($name:ident, $trait:ident) => {
                pub mod $name {
                    use super::$trait;
                    module!(@ty F32, $trait);
                }
            };
            (@ty $ty:ident, $trait:ident) => {
                pub type $ty = crate::$ty<$trait>;
            };
        }

        mod f32_ext {
            pub fn from_be_bytes(_: [u8; 4]) -> f32 { 0.0 }
            pub fn to_be_bytes(_: f32) -> [u8; 4] { [0, 0, 0, 0] }
        }

        define_type!(
            An,
            "A 32-bit floating point number",
            F32,
            f32,
            32,
            4,
            f32_ext::from_be_bytes,
            f32_ext::to_be_bytes,
            "floating point number",
            [f64],
            []
        );

        module!(big_endian, BigEndian);
    "#);
    assert!(krate.items.iter().any(|item| matches!(item, Item::Struct(_))));
    assert!(krate.items.iter().any(|item| {
        matches!(
            item,
            Item::Mod(m)
                if m.items.as_ref().is_some_and(|items| {
                    items.iter().any(|item| matches!(item, Item::TypeAlias(_)))
                })
        )
    }));
}

#[test]
fn expand_define_type_with_zerocopy_style_float_arguments() {
    let (krate, interner) = parse_and_expand_with_interner(r#"
        struct PhantomData<T>;
        trait ByteOrder {}
        enum BigEndian {}

        macro_rules! define_type {
            (
                $article:ident,
                $description:expr,
                $name:ident,
                $native:ident,
                $bits:expr,
                $bytes:expr,
                $from_be_fn:path,
                $to_be_fn:path,
                $from_le_fn:path,
                $to_le_fn:path,
                $number_kind:tt,
                [$($larger_native:ty),*],
                [$($larger_native_try:ty),*],
                [$($larger_byteorder:ident),*],
                [$($larger_byteorder_try:ident),*]
            ) => {
                pub struct $name<O>([u8; $bytes], PhantomData<O>);

                impl<O: ByteOrder> $name<O> {
                    pub fn new(n: $native) -> $name<O> {
                        $name($to_be_fn(n), PhantomData)
                    }

                    pub fn get(self) -> $native {
                        $from_be_fn(self.0)
                    }
                }

                $(
                    impl<O: ByteOrder> From<$name<O>> for $larger_native {
                        fn from(x: $name<O>) -> $larger_native {
                            x.get().into()
                        }
                    }
                )*

                $(
                    impl<O: ByteOrder> From<$larger_native_try> for $name<O> {
                        fn from(x: $larger_native_try) -> $name<O> {
                            $name::new(x as $native)
                        }
                    }
                )*

                $(
                    impl<O: ByteOrder, P: ByteOrder> From<$name<O>> for $larger_byteorder<P> {
                        fn from(x: $name<O>) -> $larger_byteorder<P> {
                            $larger_byteorder::new(x.get().into())
                        }
                    }
                )*

                $(
                    impl<O: ByteOrder, P: ByteOrder> From<$larger_byteorder_try<P>> for $name<O> {
                        fn from(x: $larger_byteorder_try<P>) -> $name<O> {
                            $name::new(x.get() as $native)
                        }
                    }
                )*
            };
        }

        macro_rules! module {
            ($name:ident, $trait:ident) => {
                pub mod $name {
                    use super::$trait;
                    module!(@ty F32, $trait);
                }
            };
            (@ty $ty:ident, $trait:ident) => {
                pub type $ty = crate::$ty<$trait>;
            };
        }

        mod f32_ext {
            pub fn from_be_bytes(_: [u8; 4]) -> f32 { 0.0 }
            pub fn to_be_bytes(_: f32) -> [u8; 4] { [0, 0, 0, 0] }
            pub fn from_le_bytes(_: [u8; 4]) -> f32 { 0.0 }
            pub fn to_le_bytes(_: f32) -> [u8; 4] { [0, 0, 0, 0] }
        }

        define_type!(
            An,
            "A 32-bit floating point number",
            F32,
            f32,
            32,
            4,
            f32_ext::from_be_bytes,
            f32_ext::to_be_bytes,
            f32_ext::from_le_bytes,
            f32_ext::to_le_bytes,
            "floating point number",
            [f64],
            [],
            [F64],
            []
        );

        define_type!(
            An,
            "A 64-bit floating point number",
            F64,
            f64,
            64,
            8,
            f32_ext::from_be_bytes,
            f32_ext::to_be_bytes,
            f32_ext::from_le_bytes,
            f32_ext::to_le_bytes,
            "floating point number",
            [],
            [],
            [],
            []
        );

        module!(big_endian, BigEndian);
    "#);
    assert!(krate.items.iter().any(|item| {
        matches!(item, Item::Struct(s) if interner.resolve(s.name) == "F32")
    }));
    assert!(krate.items.iter().any(|item| {
        matches!(item, Item::Struct(s) if interner.resolve(s.name) == "F64")
    }));
    assert!(krate.items.iter().any(|item| {
        matches!(
            item,
            Item::Mod(m)
                if m.items.as_ref().is_some_and(|items| {
                    items.iter().any(|item| matches!(item, Item::TypeAlias(_)))
                })
        )
    }));
}

#[test]
fn expand_cpufeatures_new_item_macro() {
    let krate = parse_and_expand(r#"
        cpufeatures::new!(avx2_cpuid, "avx2");
    "#);
    assert_eq!(krate.items.len(), 1);
    match &krate.items[0] {
        Item::Mod(m) => assert!(m.items.as_ref().is_some_and(|items| items.len() >= 4)),
        _ => panic!("expected generated module"),
    }
}
