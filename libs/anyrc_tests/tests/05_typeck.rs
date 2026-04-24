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
fn binary_ops_autoderef_shared_primitive_refs() {
    assert_type_ok(r#"
        fn f(a: &u8, b: &u8) -> u8 {
            a ^ b
        }
    "#);
}

#[test]
fn binary_ops_autoderef_mixed_primitive_ref_and_value() {
    assert_type_ok(r#"
        fn f(a: &u8, b: u8) -> u8 {
            a ^ b
        }
    "#);
}

#[test]
fn impl_generic_params_are_visible_in_method_bodies() {
    assert_type_ok(r#"
        trait Add<Rhs> {
            type Output;
        }

        struct B0 {}
        struct UTerm {}
        struct UInt<U, B> { msb: U, lsb: B }

        impl<U, B> UInt<U, B> {
            fn new() -> Self {
                UInt { msb: UTerm, lsb: B0 }
            }
        }

        impl<U, B> Add<B0> for UInt<U, B> {
            type Output = UInt<U, B>;

            fn add(self, _: B0) -> Self::Output {
                UInt::new()
            }
        }
    "#);
}

#[test]
fn self_type_is_available_in_struct_fields() {
    assert_type_ok(r#"
        struct Block<T> { value: T }
        struct BlockCtx<BS> {
            block: Block<Self>,
            marker: BS,
        }
    "#);
}

#[test]
fn self_output_uses_current_impl_associated_type() {
    assert_type_ok(r#"
        trait Trait {
            type Output;
        }

        struct A {}
        struct B {}
        struct X {}
        struct Y {}

        impl Trait for A {
            type Output = X;

            fn get(self) -> Self::Output {
                X {}
            }
        }

        impl Trait for B {
            type Output = Y;

            fn get(self) -> Self::Output {
                Y {}
            }
        }
    "#);
}

#[test]
fn method_resolution_uses_argument_types_for_same_receiver_impls() {
    assert_type_ok(r#"
        trait BitOr<Rhs> {
            type Output;
            fn bitor(self, rhs: Rhs) -> Self::Output;
        }

        struct B0 {}
        struct B1 {}

        impl BitOr<B0> for B0 {
            type Output = B0;
            fn bitor(self, _: B0) -> Self::Output { B0 {} }
        }

        impl BitOr<B1> for B0 {
            type Output = B1;
            fn bitor(self, _: B1) -> Self::Output { B1 {} }
        }

        fn f(lhs: B0, rhs: B1) -> B1 {
            lhs.bitor(rhs)
        }
    "#);
}

#[test]
fn binary_operator_uses_trait_output_type() {
    assert_type_ok(r#"
        mod core {
            pub mod ops {
                pub trait Mul<Rhs> {
                    type Output;
                    fn mul(self, rhs: Rhs) -> Self::Output;
                }
            }
        }

        struct Aligned130 {}
        struct PrecomputedMultiplier {}
        struct Unreduced130 {}

        impl core::ops::Mul<PrecomputedMultiplier> for Aligned130 {
            type Output = Unreduced130;
            fn mul(self, rhs: PrecomputedMultiplier) -> Self::Output {
                Unreduced130 {}
            }
        }

        fn multiply(lhs: Aligned130, rhs: PrecomputedMultiplier) -> Unreduced130 {
            lhs * rhs
        }
    "#);
}

#[test]
fn binary_operator_uses_reference_self_trait_output_type() {
    assert_type_ok(r#"
        mod core {
            pub mod ops {
                pub trait Mul<Rhs> {
                    type Output;
                    fn mul(self, rhs: Rhs) -> Self::Output;
                }
            }
        }

        struct Aligned4x130 {}
        struct PrecomputedMultiplier {}
        struct Unreduced4x130 {}

        impl core::ops::Mul<PrecomputedMultiplier> for &Aligned4x130 {
            type Output = Unreduced4x130;
            fn mul(self, rhs: PrecomputedMultiplier) -> Self::Output {
                Unreduced4x130 {}
            }
        }

        fn multiply(lhs: &Aligned4x130, rhs: PrecomputedMultiplier) -> Unreduced4x130 {
            lhs * rhs
        }
    "#);
}

#[test]
fn generic_param_associated_function_uses_trait_bound_signature() {
    assert_type_ok(r#"
        mod marker_traits {
            pub trait Bit {
                const U8: u8;
                fn to_u8() -> u8;
            }

            pub trait Unsigned {
                const U8: u8;
                const U16: u16;
                fn to_u8() -> u8;
                fn to_u16() -> u16;
            }
        }

        use crate::marker_traits::{Bit, Unsigned};

        struct B0 {}
        struct UTerm {}
        struct UInt<U, B> { msb: U, lsb: B }

        impl Bit for B0 {
            const U8: u8 = 0;
            fn to_u8() -> u8 { 0 }
        }

        impl Unsigned for UTerm {
            const U8: u8 = 0;
            const U16: u16 = 0;
            fn to_u8() -> u8 { 0 }
            fn to_u16() -> u16 { 0 }
        }

        impl<U: Unsigned, B: Bit> Unsigned for UInt<U, B> {
            const U8: u8 = B::U8 | U::U8 << 1;
            const U16: u16 = B::U8 as u16 | U::U16 << 1;

            fn to_u8() -> u8 {
                let bit: u8 = B::to_u8();
                let rest: u8 = U::to_u8();
                B::to_u8() | U::to_u8() << 1
            }

            fn to_u16() -> u16 {
                let bit: u8 = B::to_u8();
                let rest: u16 = U::to_u16();
                u16::from(B::to_u8()) | U::to_u16() << 1
            }
        }
    "#);
}

#[test]
fn generic_param_associated_const_mismatch_is_reported_from_bound() {
    assert_type_error(r#"
        mod bit {
            pub trait Bit {
                const U8: u8;
                fn to_u8() -> u8;
            }

            pub struct B0 {}

            impl Bit for B0 {
                const U8: u8 = 0;
                fn to_u8() -> u8 { 0 }
            }
        }

        mod marker_traits {
            pub trait Unsigned {
                const U16: u16;
                fn to_u16() -> u16;
            }
        }

        mod uint {
            use crate::{bit::{Bit, B0}, marker_traits::Unsigned};

            pub struct UTerm {}
            pub struct UInt<U, B> { msb: U, lsb: B }

            impl Unsigned for UTerm {
                const U16: u16 = 0;
                fn to_u16() -> u16 { 0 }
            }

            impl<U: Unsigned, B: Bit> UInt<U, B> {
                fn broken_const() -> u16 {
                    B::U8
                }
            }
        }
    "#, "mismatch");
}

#[test]
fn generic_param_associated_fn_mismatch_is_reported_from_bound() {
    assert_type_error(r#"
        mod bit {
            pub trait Bit {
                const U8: u8;
                fn to_u8() -> u8;
            }

            pub struct B0 {}

            impl Bit for B0 {
                const U8: u8 = 0;
                fn to_u8() -> u8 { 0 }
            }
        }

        mod marker_traits {
            pub trait Unsigned {
                const U16: u16;
                fn to_u16() -> u16;
            }
        }

        mod uint {
            use crate::{bit::{Bit, B0}, marker_traits::Unsigned};

            pub struct UTerm {}
            pub struct UInt<U, B> { msb: U, lsb: B }

            impl Unsigned for UTerm {
                const U16: u16 = 0;
                fn to_u16() -> u16 { 0 }
            }

            impl<U: Unsigned, B: Bit> UInt<U, B> {
                fn broken_fn() -> u16 {
                    B::to_u8()
                }
            }
        }
    "#, "mismatch");
}

#[test]
fn typenum_style_primitive_powi_impls_typecheck() {
    assert_type_ok(r#"
        trait Bit {}
        trait Unsigned {
            fn to_u32() -> u32;
        }

        trait Pow<Exp> {
            type Output;
            fn powi(self, exp: Exp) -> Self::Output;
        }

        struct UTerm {}
        struct UInt<U, B> { msb: U, lsb: B }
        struct B0 {}

        impl Bit for B0 {}

        impl Unsigned for UTerm {
            fn to_u32() -> u32 { 0 }
        }

        impl<U: Unsigned, B: Bit> Unsigned for UInt<U, B> {
            fn to_u32() -> u32 {
                U::to_u32() << 1
            }
        }

        impl Pow<UTerm> for f32 {
            type Output = f32;
            fn powi(self, _: UTerm) -> Self::Output {
                1.0
            }
        }

        impl<U: Unsigned, B: Bit> Pow<UInt<U, B>> for f32 {
            type Output = f32;
            fn powi(self, _: UInt<U, B>) -> Self::Output {
                let mut exp = <UInt<U, B> as Unsigned>::to_u32();
                let mut base = self;

                if exp == 0 {
                    return 1.0;
                }

                while exp & 1 == 0 {
                    base *= base;
                    exp >>= 1;
                }

                base
            }
        }
    "#);
}

#[test]
fn typenum_style_recursive_primitive_powi_macro_typechecks() {
    assert_type_ok(r#"
        trait Bit {}
        trait Unsigned {
            fn to_u32() -> u32;
        }

        trait Pow<Exp> {
            type Output;
            fn powi(self, exp: Exp) -> Self::Output;
        }

        struct UTerm {}
        struct UInt<U, B> { msb: U, lsb: B }
        struct B0 {}

        impl Bit for B0 {}

        impl Unsigned for UTerm {
            fn to_u32() -> u32 { 0 }
        }

        impl<U: Unsigned, B: Bit> Unsigned for UInt<U, B> {
            fn to_u32() -> u32 {
                U::to_u32() << 1
            }
        }

        macro_rules! impl_pow_i {
            () => ();
            ($t: ty $(, $tail:tt)*) => (
                impl Pow<UTerm> for $t {
                    type Output = $t;
                    fn powi(self, _: UTerm) -> Self::Output {
                        1
                    }
                }

                impl<U: Unsigned, B: Bit> Pow<UInt<U, B>> for $t {
                    type Output = $t;
                    fn powi(self, _: UInt<U, B>) -> Self::Output {
                        self.pow(<UInt<U, B> as Unsigned>::to_u32())
                    }
                }

                impl_pow_i!($($tail),*);
            );
        }

        impl_pow_i!(u8, u16, u32);
    "#);
}

#[test]
fn typenum_style_float_powi_macro_typechecks() {
    assert_type_ok(r#"
        trait Bit {}
        trait Unsigned {
            fn to_u32() -> u32;
        }

        trait Pow<Exp> {
            type Output;
            fn powi(self, exp: Exp) -> Self::Output;
        }

        struct UTerm {}
        struct UInt<U, B> { msb: U, lsb: B }
        struct B0 {}
        struct PInt<U> { inner: U }
        struct NInt<U> { inner: U }

        trait NonZero {}

        impl Bit for B0 {}

        impl PInt<UTerm> {
            fn new() -> Self { PInt { inner: UTerm {} } }
        }

        impl Unsigned for UTerm {
            fn to_u32() -> u32 { 0 }
        }

        impl<U: Unsigned, B: Bit> Unsigned for UInt<U, B> {
            fn to_u32() -> u32 {
                U::to_u32() << 1
            }
        }

        macro_rules! impl_pow_f {
            ($t:ty) => {
                impl Pow<UTerm> for $t {
                    type Output = $t;
                    fn powi(self, _: UTerm) -> Self::Output {
                        1.0
                    }
                }

                impl<U: Unsigned, B: Bit> Pow<UInt<U, B>> for $t {
                    type Output = $t;
                    fn powi(self, _: UInt<U, B>) -> Self::Output {
                        let mut exp = <UInt<U, B> as Unsigned>::to_u32();
                        let mut base = self;

                        if exp == 0 {
                            return 1.0;
                        }

                        while exp & 1 == 0 {
                            base *= base;
                            exp >>= 1;
                        }
                        if exp == 1 {
                            return base;
                        }

                        let mut acc = base.clone();
                        while exp > 1 {
                            exp >>= 1;
                            base *= base;
                            if exp & 1 == 1 {
                                acc *= base.clone();
                            }
                        }
                        acc
                    }
                }

                impl<U: Unsigned + NonZero> Pow<NInt<U>> for $t {
                    type Output = $t;
                    fn powi(self, _: NInt<U>) -> Self::Output {
                        <$t as Pow<PInt<U>>>::powi(self, PInt::new()).recip()
                    }
                }
            };
        }

        impl_pow_f!(f32);
        impl_pow_f!(f64);
    "#);
}

#[test]
fn typenum_style_generic_rem_uses_projection_not_concrete_operator_impl() {
    assert_type_ok(r#"
        trait Rem<Rhs = Self> {
            type Output;
            fn rem(self, rhs: Rhs) -> Self::Output;
        }

        trait Unsigned {}
        trait NonZero {}
        trait Bit {}

        mod bit {
            pub struct B0 {}
            impl crate::Bit for B0 {}
        }

        mod uint {
            use crate::bit::B0;
            use crate::{Bit, Rem, Unsigned};

            pub struct UTerm {}
            pub struct UInt<U: Unsigned, B: Bit> {
                pub msb: U,
                pub lsb: B,
            }

            impl Unsigned for UTerm {}

            impl<Ur: Unsigned, Br: Bit> Rem<UInt<Ur, Br>> for UTerm {
                type Output = UTerm;

                fn rem(self, rhs: UInt<Ur, Br>) -> Self::Output {
                    UTerm {}
                }
            }
        }

        mod consts {
            pub type U0 = crate::uint::UTerm;
        }

        mod int {
            use crate::consts::U0;
            use crate::uint::Unsigned;
            use crate::{NonZero, Rem};

            pub struct Z0;
            pub struct PInt<U: Unsigned + NonZero> {
                pub n: U,
            }

            pub trait PrivateRem<URem, Divisor> {
                type Output;
                fn private_rem(self, rem: URem, divisor: Divisor) -> Self::Output;
            }

            impl<I: NonZero> Rem<I> for Z0 {
                type Output = Z0;

                fn rem(self, rhs: I) -> Self::Output {
                    Z0
                }
            }

            impl<Ul: Unsigned + NonZero, Ur: Unsigned + NonZero> PrivateRem<U0, PInt<Ur>> for PInt<Ul> {
                type Output = Z0;

                fn private_rem(self, rem: U0, rhs: PInt<Ur>) -> Self::Output {
                    Z0
                }
            }

            impl<Ul: Unsigned + NonZero, Ur: Unsigned + NonZero> Rem<PInt<Ur>> for PInt<Ul>
            where
                Ul: Rem<Ur>,
                PInt<Ul>: PrivateRem<<Ul as Rem<Ur>>::Output, PInt<Ur>>,
            {
                type Output = <PInt<Ul> as PrivateRem<<Ul as Rem<Ur>>::Output, PInt<Ur>>>::Output;

                fn rem(self, rhs: PInt<Ur>) -> Self::Output {
                    self.private_rem(self.n % rhs.n, rhs)
                }
            }
        }
    "#);
}

#[test]
fn qualified_associated_type_projection_is_not_global_impl_output() {
    assert_type_ok(r#"
        trait Trait {
            type Output;
        }

        struct B0 {}
        struct UTerm {}
        struct Wrap<T> { value: T }

        impl<T> Trait for Wrap<T> {
            type Output = T;
        }

        impl Trait for B0 {
            type Output = B0;
        }

        type Out<T> = <Wrap<T> as Trait>::Output;

        fn f(x: UTerm) -> Out<UTerm> {
            x
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
fn proc_macro2_style_scoped_fallback_imports_do_not_pick_root_wrappers() {
    let (result, _) = typecheck_with_cfg(r#"
        pub mod fallback {
            pub struct TokenStream {}
            pub struct TokenStreamBuilder {}
            pub struct Group {}
            pub struct Span {}
            pub struct Literal {}

            impl TokenStreamBuilder {
                pub fn new() -> TokenStreamBuilder { TokenStreamBuilder {} }
                pub fn build(self) -> TokenStream { TokenStream {} }
                pub fn push_token_from_parser(&mut self, token: crate::TokenTree) {}
            }

            impl Group {
                pub fn new() -> Group { Group {} }
                pub fn span(&self) -> Span { Span {} }
            }

            impl Span {
                pub fn call_site() -> Span { Span {} }
            }

            impl Literal {
                pub fn new() -> Literal { Literal {} }
            }
        }

        #[cfg(not(wrap_proc_macro))]
        use crate::fallback as imp;

        pub struct TokenStream {
            inner: imp::TokenStream,
        }

        pub struct Group {
            inner: imp::Group,
        }

        pub struct Span {
            inner: imp::Span,
        }

        pub struct Literal {
            inner: imp::Literal,
        }

        pub enum TokenTree {
            Group(Group),
            Literal(Literal),
        }

        impl TokenStream {
            fn _new_fallback(inner: fallback::TokenStream) -> Self {
                TokenStream { inner: imp::TokenStream::from(inner) }
            }
        }

        impl Group {
            fn _new_fallback(inner: fallback::Group) -> Self {
                Group { inner }
            }
        }

        impl Span {
            fn _new_fallback(inner: fallback::Span) -> Self {
                Span { inner }
            }
        }

        impl Literal {
            fn _new_fallback(inner: fallback::Literal) -> Self {
                Literal { inner }
            }
        }

        mod parse {
            use crate::fallback::{
                self, Group, Literal, Span, TokenStream, TokenStreamBuilder,
            };
            use crate::TokenTree;

            fn token_stream() -> TokenStream {
                let mut tokens = TokenStreamBuilder::new();
                let g = Group::new();
                tokens.push_token_from_parser(TokenTree::Group(crate::Group::_new_fallback(g)));
                tokens.build()
            }

            fn leaf_token() -> TokenTree {
                let literal = Literal::new();
                TokenTree::Literal(crate::Literal::_new_fallback(literal))
            }

            fn doc_comment(tokens: &mut TokenStreamBuilder) {
                let span = crate::Span::_new_fallback(Span::call_site());
            }
        }
    "#, &[]);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}",
        result.errors.iter().map(|e| &e.message).collect::<Vec<_>>());
}

#[test]
fn proc_macro2_style_qualified_enum_variant_pattern_binds_payload_type() {
    assert_type_ok(r#"
        pub mod fallback {
            pub struct Span {}
            pub struct Group {}

            impl Group {
                pub fn span(&self) -> Span { Span {} }
            }
        }

        pub mod imp {
            use crate::fallback;

            pub enum Group {
                Fallback(fallback::Group),
            }

            pub enum Span {
                Fallback(fallback::Span),
            }
        }

        mod extra {
            use crate::fallback;
            use crate::imp;

            enum DelimSpanEnum {
                Fallback(fallback::Span),
            }

            fn new(group: &imp::Group) -> DelimSpanEnum {
                match group {
                    imp::Group::Fallback(group) => DelimSpanEnum::Fallback(group.span()),
                }
            }
        }
    "#);
}

#[test]
fn proc_macro_intrinsic_enum_variant_constructors_have_parent_type() {
    assert_type_ok(r#"
        fn into_compiler_token(g: proc_macro::Group, p: proc_macro::Punct) -> proc_macro::TokenTree {
            if true {
                proc_macro::TokenTree::Group(g)
            } else {
                proc_macro::TokenTree::Punct(p)
            }
        }
    "#);
}

#[test]
fn proc_macro2_style_extend_maps_generic_into_iterator_item() {
    assert_type_ok(r#"
        trait IntoIterator {
            type Item;
            type IntoIter;
            fn into_iter(self) -> Self::IntoIter;
        }

        trait Extend<A> {
            fn extend<T: IntoIterator<Item = A>>(&mut self, iter: T);
        }

        struct Map<I, F> {
            iter: I,
            f: F,
        }

        struct Ident {}

        mod imp {
            pub enum TokenTree {
                Ident(crate::Ident),
            }

            pub struct TokenStream {}

            impl Extend<TokenTree> for TokenStream {
                fn extend<I: IntoIterator<Item = TokenTree>>(&mut self, tokens: I) {}
            }
        }

        struct TokenStream {
            inner: imp::TokenStream,
        }

        impl Extend<Ident> for TokenStream {
            fn extend<I: IntoIterator<Item = Ident>>(&mut self, tokens: I) {
                self.inner.extend(tokens.into_iter().map(imp::TokenTree::Ident));
            }
        }
    "#);
}

#[test]
fn proc_macro2_style_wrapper_into_iter_preserves_fallback_variant_field_type() {
    assert_type_ok(r#"
        trait IntoIterator {
            type Item;
            type IntoIter;
            fn into_iter(self) -> Self::IntoIter;
        }

        struct RcVecIntoIter<T> {
            item: T,
        }

        enum TokenTree {
            Ident,
        }

        mod fallback {
            use crate::{RcVecIntoIter, TokenTree};

            pub struct TokenStream {}
            pub type TokenTreeIter = RcVecIntoIter<TokenTree>;

            impl IntoIterator for TokenStream {
                type Item = TokenTree;
                type IntoIter = TokenTreeIter;

                fn into_iter(self) -> TokenTreeIter {
                    RcVecIntoIter { item: TokenTree::Ident }
                }
            }
        }

        mod imp {
            use crate::fallback;
            use crate::TokenTree;

            pub enum TokenStream {
                Fallback(fallback::TokenStream),
            }

            pub enum TokenTreeIter {
                Fallback(fallback::TokenTreeIter),
            }

            impl IntoIterator for TokenStream {
                type Item = TokenTree;
                type IntoIter = TokenTreeIter;

                fn into_iter(self) -> TokenTreeIter {
                    match self {
                        TokenStream::Fallback(tts) => TokenTreeIter::Fallback(tts.into_iter()),
                    }
                }
            }
        }
    "#);
}

#[test]
fn generic_array_style_index_uses_deref_slice_target() {
    assert_type_ok(r#"
        trait Deref {
            type Target;
        }

        trait Unsigned {}
        unsafe trait ArrayLength<T>: Unsigned {
            type ArrayType;
        }

        struct UTerm {}
        impl Unsigned for UTerm {}
        unsafe impl<T> ArrayLength<T> for UTerm {
            type ArrayType = [T; 0];
        }

        struct GenericArray<T, N: ArrayLength<T>> {
            marker: T,
            len: N,
        }

        impl<T, N: ArrayLength<T>> Deref for GenericArray<T, N> {
            type Target = [T];
        }

        fn fmt<N: ArrayLength<u8>>(array: GenericArray<u8, N>) {
            let byte: u8 = array[0];
            let bytes = array[..1];
        }
    "#);
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
