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
