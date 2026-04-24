use anyrc::parser::Parser;
use anyrc::intern::Interner;
use anyrc::ast::*;

fn parse(src: &str) -> Crate {
    let mut interner = Interner::new();
    let mut parser = Parser::new(src, &mut interner);
    parser.parse_crate()
}

#[test]
fn parse_simple_fn() {
    let krate = parse("fn add(a: i32, b: i32) -> i32 { a + b }");
    assert_eq!(krate.items.len(), 1);
    match &krate.items[0] {
        Item::Fn(f) => {
            assert_eq!(f.params.len(), 2);
            assert!(f.ret_ty.is_some());
            assert!(f.body.is_some());
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_generic_fn() {
    let krate = parse("fn max<T>(a: T, b: T) -> T { a }");
    match &krate.items[0] {
        Item::Fn(f) => {
            assert_eq!(f.generics.params.len(), 1);
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_higher_ranked_where_predicate() {
    let krate = parse("fn f<T>() where T: Copy, for<'a> &'a T: Clone {}");
    match &krate.items[0] {
        Item::Fn(f) => {
            assert_eq!(f.where_clause.predicates.len(), 2);
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_type_position_macro_in_cast_ty() {
    let krate = parse("fn f(x: u8) -> u8 { (x as to_signed_int!(u8)) as u8 }");
    match &krate.items[0] {
        Item::Fn(f) => {
            assert!(f.body.is_some());
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_struct() {
    let krate = parse("pub struct Point { pub x: i32, pub y: i32 }");
    match &krate.items[0] {
        Item::Struct(s) => {
            assert_eq!(s.fields.len(), 2);
            assert_eq!(s.vis, Visibility::Public);
        }
        _ => panic!("expected struct"),
    }
}

#[test]
fn parse_generic_struct() {
    let krate = parse("struct Pair<A, B> { first: A, second: B }");
    match &krate.items[0] {
        Item::Struct(s) => {
            assert_eq!(s.generics.params.len(), 2);
            assert_eq!(s.fields.len(), 2);
        }
        _ => panic!("expected struct"),
    }
}

#[test]
fn parse_struct_field_attrs() {
    let krate = parse(r#"struct Wire { #[serde(rename = "secs")] secs: u64, #[cfg(anyos)] pub nanos: u32 }"#);
    match &krate.items[0] {
        Item::Struct(s) => {
            assert_eq!(s.fields.len(), 2);
            assert_eq!(s.fields[0].attrs.len(), 1);
            assert_eq!(s.fields[1].attrs.len(), 1);
            assert_eq!(s.fields[1].vis, Visibility::Public);
        }
        _ => panic!("expected struct"),
    }
}

#[test]
fn parse_struct_literal_field_attrs() {
    let krate = parse(r#"fn f(rest: i32) { let _ = Cursor { rest, #[cfg(span_locations)] off: 1 }; }"#);
    match &krate.items[0] {
        Item::Fn(f) => {
            let body = f.body.as_ref().expect("expected body");
            match &body.stmts[0] {
                Stmt::Let(_, _, Some(init), _) => {
                    if let Expr::Struct(_, fields, _, _) = init.as_ref() {
                        assert_eq!(fields.len(), 2);
                        assert_eq!(fields[0].attrs.len(), 0);
                        assert_eq!(fields[1].attrs.len(), 1);
                    } else {
                        panic!("expected struct literal");
                    }
                }
                _ => panic!("expected struct literal"),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_struct_pattern_field_attrs() {
    let krate = parse(r#"fn f(lit: Literal) { match lit { Literal { #[cfg(wrap_proc_macro)] inner, repr } => repr } }"#);
    match &krate.items[0] {
        Item::Fn(f) => {
            let body = f.body.as_ref().expect("expected body");
            match &body.stmts[0] {
                Stmt::Expr(Expr::Match(_, arms, _)) => {
                    if let Pattern::Struct(_, fields, _, _) = &arms[0].pat {
                        assert_eq!(fields.len(), 2);
                        assert_eq!(fields[0].attrs.len(), 1);
                        assert_eq!(fields[1].attrs.len(), 0);
                    } else {
                        panic!("expected struct pattern");
                    }
                }
                _ => panic!("expected match expression"),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_enum_with_data() {
    let krate = parse("enum Option<T> { Some(T), None }");
    match &krate.items[0] {
        Item::Enum(e) => {
            assert_eq!(e.variants.len(), 2);
            assert_eq!(e.generics.params.len(), 1);
        }
        _ => panic!("expected enum"),
    }
}

#[test]
fn parse_enum_variant_and_field_attrs() {
    let krate = parse(r#"enum Event { #[cfg(anyos)] Ready { #[serde(rename = "fd")] fd: u32 }, Data(#[cfg(anyos)] u8) }"#);
    match &krate.items[0] {
        Item::Enum(e) => {
            assert_eq!(e.variants.len(), 2);
            assert_eq!(e.variants[0].attrs.len(), 1);
            match &e.variants[0].fields {
                VariantFields::Struct(fields) => assert_eq!(fields[0].attrs.len(), 1),
                _ => panic!("expected struct variant"),
            }
        }
        _ => panic!("expected enum"),
    }
}

#[test]
fn parse_impl_block() {
    let krate = parse("impl Point { fn new() -> Point { Point { x: 0, y: 0 } } }");
    match &krate.items[0] {
        Item::Impl(i) => {
            assert_eq!(i.items.len(), 1);
            assert!(i.trait_ref.is_none());
        }
        _ => panic!("expected impl"),
    }
}

#[test]
fn parse_trait_impl() {
    let krate = parse("impl Clone for Point { fn clone(&self) -> Point { Point { x: 0, y: 0 } } }");
    match &krate.items[0] {
        Item::Impl(i) => {
            assert!(i.trait_ref.is_some());
        }
        _ => panic!("expected impl"),
    }
}

#[test]
fn parse_extern_crate_self_alias() {
    let krate = parse("extern crate self as zerocopy;");
    match &krate.items[0] {
        Item::ExternCrate(ext) => {
            assert!(ext.alias.is_some());
        }
        _ => panic!("expected extern crate"),
    }
}

#[test]
fn parse_root_nested_use_tree() {
    let krate = parse("use {FromZeros as FromZeroes, IntoBytes as AsBytes, Ref as LayoutVerified};");
    match &krate.items[0] {
        Item::Use(u) => match &u.kind {
            UseTreeKind::Nested(items) => assert_eq!(items.len(), 3),
            _ => panic!("expected nested use"),
        },
        _ => panic!("expected use"),
    }
}

#[test]
fn parse_nested_use_globs() {
    let krate = parse("use crate::{layout::*, testutil::*, *};");
    match &krate.items[0] {
        Item::Use(u) => match &u.kind {
            UseTreeKind::Nested(items) => {
                assert_eq!(items.len(), 3);
                assert!(matches!(items[2].kind, UseTreeKind::Glob));
            }
            _ => panic!("expected nested use"),
        },
        _ => panic!("expected use"),
    }
}

#[test]
fn parse_macro_crate_use_path() {
    let krate = parse("fn f() { use $crate::util::macro_util::assert_dst_is_not_zst; }");
    assert!(matches!(&krate.items[0], Item::Fn(_)));
}

#[test]
fn parse_trait_def() {
    let krate = parse("trait Drawable { fn draw(&self); fn color(&self) -> u32; }");
    match &krate.items[0] {
        Item::Trait(t) => {
            assert_eq!(t.items.len(), 2);
        }
        _ => panic!("expected trait"),
    }
}

#[test]
fn parse_use_simple() {
    let krate = parse("use std::collections::HashMap;");
    assert!(matches!(&krate.items[0], Item::Use(_)));
}

#[test]
fn parse_where_clause() {
    let krate = parse("fn foo<T>(x: T) where T: Clone { }");
    match &krate.items[0] {
        Item::Fn(f) => {
            assert!(!f.where_clause.predicates.is_empty());
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_unsafe_fn() {
    let krate = parse("unsafe fn dangerous() { }");
    match &krate.items[0] {
        Item::Fn(f) => assert!(f.is_unsafe),
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_const_unsafe_fn() {
    let krate = parse("pub const unsafe fn dangerous() { }");
    match &krate.items[0] {
        Item::Fn(f) => {
            assert!(f.is_const);
            assert!(f.is_unsafe);
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_extern_block() {
    let krate = parse(r#"extern "C" { fn malloc(size: usize) -> *mut u8; }"#);
    assert!(matches!(&krate.items[0], Item::ExternBlock(_)));
}

#[test]
fn parse_const_def() {
    let krate = parse("const MAX: u32 = 100;");
    assert!(matches!(&krate.items[0], Item::Const(_)));
}

#[test]
fn parse_static_def() {
    let krate = parse("static mut COUNTER: i32 = 0;");
    match &krate.items[0] {
        Item::Static(s) => assert!(s.is_mut),
        _ => panic!("expected static"),
    }
}

#[test]
fn parse_type_alias() {
    let krate = parse("type Result<T> = core::result::Result<T, Error>;");
    assert!(matches!(&krate.items[0], Item::TypeAlias(_)));
}

#[test]
fn parse_dyn_callable_trait_alias() {
    let krate = parse("type PanicHook = dyn Fn(&PanicInfo) + Sync + Send + 'static;");
    match &krate.items[0] {
        Item::TypeAlias(alias) => match alias.ty.as_deref() {
            Some(Ty::DynTrait(bounds, _)) => {
                assert_eq!(bounds.len(), 3);
                let args = bounds[0].path.segments[0].args.as_ref().expect("Fn args");
                assert_eq!(args.args.len(), 1);
            }
            _ => panic!("expected dyn trait alias"),
        },
        _ => panic!("expected type alias"),
    }
}

#[test]
fn parse_trait_associated_type_bounds() {
    let krate = parse(
        "trait KnownLayout { type MaybeUninit: ?Sized + KnownLayout<PointerMetadata = Self::PointerMetadata>; }",
    );
    match &krate.items[0] {
        Item::Trait(t) => assert_eq!(t.items.len(), 1),
        _ => panic!("expected trait"),
    }
}

#[test]
fn parse_tuple_struct_trailing_where_clause() {
    let krate = parse("struct Projection<T>(T) where T: ?Sized + KnownLayout;");
    assert!(matches!(&krate.items[0], Item::Struct(_)));
}

#[test]
fn parse_union_item() {
    let krate = parse("union Transmute<Src, Dst> { src: Src, dst: Dst }");
    match &krate.items[0] {
        Item::Struct(s) => {
            assert!(s.is_union);
            assert_eq!(s.fields.len(), 2);
        }
        _ => panic!("expected union-backed adt"),
    }
}

#[test]
fn parse_unsafe_fn_pointer_type() {
    let krate = parse(
        r#"fn f(allocate: unsafe extern "C" fn(Layout) -> *mut u8, deallocate: unsafe fn(*mut u8)) {}"#,
    );
    assert!(matches!(&krate.items[0], Item::Fn(_)));
}

#[test]
fn parse_attributed_closure_expression() {
    let krate = parse("fn f() { ptr.try_with(#[inline(always)] |ptr| ptr.cast()); }");
    assert!(matches!(&krate.items[0], Item::Fn(_)));
}

#[test]
fn parse_restricted_visibility_use_items() {
    let krate = parse("mod m { pub(super) use rand::Rng; pub(in crate::m) struct S { value: u8 } }");
    assert!(matches!(&krate.items[0], Item::Mod(_)));
}

#[test]
fn parse_attributed_match_arm_pattern() {
    let krate = parse("fn f(v: u8) { match v { #[cfg(any())] 0 => 1, _ => 2 } }");
    match &krate.items[0] {
        Item::Fn(f) => {
            let body = f.body.as_ref().expect("body");
            match &body.stmts[0] {
                Stmt::Expr(Expr::Match(_, arms, _)) => assert_eq!(arms[0].attrs.len(), 1),
                _ => panic!("expected match expression"),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn cfg_strips_disabled_match_arm() {
    let mut interner = Interner::new();
    let mut parser = Parser::new(
        "fn f(v: u8) { match v { #[cfg(proc_macro_span)] 0 => missing(), _ => 2 } }",
        &mut interner,
    );
    let mut krate = parser.parse_crate();
    let cfg = anyrc::cfg::CfgContext::from_flags(&[]);
    anyrc::cfg::strip_cfg(&mut krate, &cfg, &interner);
    match &krate.items[0] {
        Item::Fn(f) => {
            let body = f.body.as_ref().expect("body");
            match &body.stmts[0] {
                Stmt::Expr(Expr::Match(_, arms, _)) => assert_eq!(arms.len(), 1),
                _ => panic!("expected match expression"),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_tuple_struct_rest_pattern() {
    let krate = parse("fn f(value: SizeInfo) { match value { SizeInfo::SliceDst(..) => 1, _ => 0 } }");
    assert!(matches!(&krate.items[0], Item::Fn(_)));
}

#[test]
fn parse_ref_slice_pattern() {
    let krate = parse("fn f(xs: &[u64; 4]) { let &[a, b, c, d] = xs; }");
    match &krate.items[0] {
        Item::Fn(f) => {
            let body = f.body.as_ref().expect("body");
            match &body.stmts[0] {
                Stmt::Let(pat, _, _, _) => match pat {
                    Pattern::Ref(inner, _, _) => {
                        assert!(matches!(inner.as_ref(), Pattern::Slice(pats, _) if pats.len() == 4));
                    }
                    _ => panic!("expected ref slice pattern"),
                },
                _ => panic!("expected let statement"),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_default_impl_fn() {
    let krate = parse("impl T { #[inline] default fn get_hash() -> u64 { 0 } }");
    match &krate.items[0] {
        Item::Impl(impl_block) => {
            assert!(matches!(impl_block.items.first(), Some(Item::Fn(_))));
        }
        _ => panic!("expected impl"),
    }
}

#[test]
fn parse_absolute_grouped_use() {
    let krate = parse("pub use ::{inner::PtrInner, outer::PtrOuter};");
    assert!(matches!(&krate.items[0], Item::Use(_)));
}

#[test]
fn parse_keyword_segment_use_tree() {
    let krate = parse("pub use core::{error::*, ref::*, split_at::*};");
    assert!(matches!(&krate.items[0], Item::Use(_)));
}

#[test]
fn parse_ref_named_module() {
    let krate = parse("mod ref { pub struct Item; }");
    assert!(matches!(&krate.items[0], Item::Mod(_)));
}

#[test]
fn parse_qualified_path_expr() {
    let krate = parse("fn f(len: usize) { <[Self]>::new_box_zeroed_with_elems(len); }");
    match &krate.items[0] {
        Item::Fn(f) => {
            let body = f.body.as_ref().expect("body");
            match &body.stmts[0] {
                Stmt::Semi(Expr::Call(callee, _, _), _) => {
                    assert!(matches!(callee.as_ref(), Expr::QualifiedPath(_)));
                }
                _ => panic!("expected qualified path call"),
            }
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_qualified_path_type() {
    let krate = parse("type Item = <Self as Iterator>::Item;");
    match &krate.items[0] {
        Item::TypeAlias(alias) => {
            assert!(matches!(alias.ty.as_deref(), Some(Ty::QualifiedPath(_))));
        }
        _ => panic!("expected type alias"),
    }
}

#[test]
fn parse_macro_crate_path_type_bound() {
    let krate = parse("fn cast<Src, Dst>() where Src: $crate::IntoBytes, Dst: $crate::FromBytes {}");
    assert!(matches!(&krate.items[0], Item::Fn(_)));
}

#[test]
fn parse_macro_crate_path_expression() {
    let krate = parse("fn f() { $crate::util::macro_util::assert_dst_is_not_zst::<T>(); }");
    assert!(matches!(&krate.items[0], Item::Fn(_)));
}

#[test]
fn parse_const_generic_call_args() {
    let krate = parse("fn f() { Parser::<8>::new(); Parser::<{ 4 + 4 }>::new(); }");
    assert!(matches!(&krate.items[0], Item::Fn(_)));
}

#[test]
fn parse_macro_rules_inside_block() {
    let krate = parse("fn f() { macro_rules! local { ($x:expr) => {{ $x }} } local!(1); }");
    match &krate.items[0] {
        Item::Fn(f) => {
            let body = f.body.as_ref().expect("body");
            assert!(matches!(&body.stmts[0], Stmt::Item(Item::MacroDef(_))));
        }
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_multiple_items() {
    let krate = parse("fn a() {} fn b() {} fn c() {}");
    assert_eq!(krate.items.len(), 3);
}

#[test]
fn parse_fn_with_self_param() {
    let krate = parse("impl S { fn method(&self) {} fn mut_method(&mut self) {} }");
    match &krate.items[0] {
        Item::Impl(i) => assert_eq!(i.items.len(), 2),
        _ => panic!("expected impl"),
    }
}

#[test]
fn parse_reference_type() {
    let krate = parse("fn foo(x: &i32, y: &mut i32) {}");
    match &krate.items[0] {
        Item::Fn(f) => assert_eq!(f.params.len(), 2),
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_raw_pointer_type() {
    let krate = parse("fn foo(x: *const u8, y: *mut u8) {}");
    match &krate.items[0] {
        Item::Fn(f) => assert_eq!(f.params.len(), 2),
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_tuple_type() {
    let krate = parse("fn foo() -> (i32, bool) { (1, true) }");
    match &krate.items[0] {
        Item::Fn(f) => assert!(f.ret_ty.is_some()),
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_array_type() {
    let krate = parse("fn foo(x: [u8; 4]) {}");
    match &krate.items[0] {
        Item::Fn(f) => assert_eq!(f.params.len(), 1),
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_slice_type() {
    let krate = parse("fn foo(x: &[u8]) {}");
    match &krate.items[0] {
        Item::Fn(f) => assert_eq!(f.params.len(), 1),
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_fn_ptr_type() {
    let krate = parse("fn foo(f: fn(i32, i32) -> bool) {}");
    match &krate.items[0] {
        Item::Fn(f) => assert_eq!(f.params.len(), 1),
        _ => panic!("expected fn"),
    }
}

#[test]
fn parse_never_type() {
    let krate = parse("fn foo() -> ! { loop {} }");
    match &krate.items[0] {
        Item::Fn(f) => assert!(f.ret_ty.is_some()),
        _ => panic!("expected fn"),
    }
}
