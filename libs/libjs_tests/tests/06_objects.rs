//! Objects — creation, property access, methods, prototype, spread, computed keys.
//!
//! Level 6: Property descriptors, prototype chain traversal, Object built-ins.

use libjs_tests::JsEngine;

fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }
fn bool_(src: &str) -> bool { js(src).to_boolean() }
fn str_(src: &str) -> String { js(src).to_js_string() }

// ── object literal ────────────────────────────────────────────────────────────

#[test]
fn object_literal_dot_access() {
    assert_eq!(num("var o = { x: 10, y: 20 }; o.x"), 10.0);
}

#[test]
fn object_literal_bracket_access() {
    assert_eq!(num(r#"var o = { a: 7 }; o["a"]"#), 7.0);
}

#[test]
fn object_property_assignment() {
    let mut e = JsEngine::new();
    e.eval("var o = {}; o.name = 'Alice'; o.age = 30;");
    assert_eq!(e.get_global("o").get_property("name").to_js_string(), "Alice");
    assert_eq!(e.get_global("o").get_property("age").to_number(), 30.0);
}

#[test]
fn object_computed_key() {
    assert_eq!(num(r#"var key = "x"; var o = { [key]: 42 }; o.x"#), 42.0);
}

#[test]
fn shorthand_property() {
    let mut e = JsEngine::new();
    e.eval("var name = 'Bob'; var o = { name };");
    assert_eq!(e.get_global("o").get_property("name").to_js_string(), "Bob");
}

#[test]
fn method_shorthand() {
    assert_eq!(num("var o = { greet() { return 42; } }; o.greet()"), 42.0);
}

// ── delete operator ───────────────────────────────────────────────────────────

#[test]
fn delete_property() {
    let mut e = JsEngine::new();
    e.eval("var o = { a: 1, b: 2 }; delete o.a;");
    assert!(e.get_global("o").get_property("a").to_number().is_nan());
}

// ── in operator ───────────────────────────────────────────────────────────────

#[test]
fn in_operator_present() {
    assert!(bool_(r#"var o = { x: 1 }; "x" in o"#));
}

#[test]
fn in_operator_absent() {
    assert!(!bool_(r#"var o = { x: 1 }; "y" in o"#));
}

// ── Object built-ins ──────────────────────────────────────────────────────────

#[test]
fn object_keys() {
    let mut e = JsEngine::new();
    e.eval("var keys = Object.keys({ a: 1, b: 2, c: 3 });");
    let keys = e.get_global("keys");
    assert_eq!(keys.get_property("length").to_number(), 3.0);
}

#[test]
fn object_assign_merges() {
    let mut e = JsEngine::new();
    e.eval("var r = Object.assign({}, { a: 1 }, { b: 2 });");
    let r = e.get_global("r");
    assert_eq!(r.get_property("a").to_number(), 1.0);
    assert_eq!(r.get_property("b").to_number(), 2.0);
}

#[test]
fn object_assign_overwrites() {
    let mut e = JsEngine::new();
    e.eval("var r = Object.assign({ a: 1 }, { a: 99, b: 2 });");
    assert_eq!(e.get_global("r").get_property("a").to_number(), 99.0);
}

#[test]
fn has_own_property() {
    assert!(bool_("({ a: 1 }).hasOwnProperty('a')"));
    assert!(!bool_("({ a: 1 }).hasOwnProperty('toString')"));
}

// ── spread in object literals ─────────────────────────────────────────────────

#[test]
fn object_spread_merge() {
    let mut e = JsEngine::new();
    e.eval("var a = { x: 1 }; var b = { y: 2 }; var c = { ...a, ...b };");
    assert_eq!(e.get_global("c").get_property("x").to_number(), 1.0);
    assert_eq!(e.get_global("c").get_property("y").to_number(), 2.0);
}

#[test]
fn object_spread_override() {
    let mut e = JsEngine::new();
    e.eval("var base = { a: 1, b: 2 }; var over = { ...base, b: 99 };");
    assert_eq!(e.get_global("over").get_property("a").to_number(), 1.0);
    assert_eq!(e.get_global("over").get_property("b").to_number(), 99.0);
}

// ── object destructuring ──────────────────────────────────────────────────────

#[test]
fn destructure_basic() {
    let mut e = JsEngine::new();
    e.eval("var { name, age } = { name: 'Alice', age: 30 };");
    assert_eq!(e.get_global("name").to_js_string(), "Alice");
    assert_eq!(e.get_global("age").to_number(), 30.0);
}

#[test]
fn destructure_with_rename() {
    let mut e = JsEngine::new();
    e.eval("var { x: myX, y: myY } = { x: 10, y: 20 };");
    assert_eq!(e.get_global("myX").to_number(), 10.0);
    assert_eq!(e.get_global("myY").to_number(), 20.0);
}

#[test]
fn destructure_with_default() {
    let mut e = JsEngine::new();
    e.eval("var { a = 5, b = 10 } = { a: 1 };");
    assert_eq!(e.get_global("a").to_number(), 1.0);
    assert_eq!(e.get_global("b").to_number(), 10.0);
}

#[test]
fn destructure_nested_objects() {
    let mut e = JsEngine::new();
    e.eval("var { outer: { inner } } = { outer: { inner: 42 } };");
    assert_eq!(e.get_global("inner").to_number(), 42.0);
}

// ── optional chaining ─────────────────────────────────────────────────────────

#[test]
fn optional_chain_present() {
    assert_eq!(num("var o = { a: { b: 42 } }; o?.a?.b"), 42.0);
}

#[test]
fn optional_chain_absent_returns_undefined() {
    let v = js("var o = {}; o?.missing?.deep");
    assert!(!v.to_boolean()); // undefined is falsy
}

#[test]
fn optional_chain_on_null() {
    let v = js("var o = null; o?.x");
    assert!(!v.to_boolean());
}

// ── this binding in methods ───────────────────────────────────────────────────

#[test]
fn this_in_method() {
    assert_eq!(
        num("var o = { val: 7, get() { return this.val; } }; o.get()"),
        7.0
    );
}

#[test]
fn this_arithmetic_method() {
    assert_eq!(
        num("var o = { a: 3, b: 4, sum() { return this.a + this.b; } }; o.sum()"),
        7.0
    );
}

// ── instanceof ────────────────────────────────────────────────────────────────

#[test]
fn instanceof_object() {
    assert!(bool_("({}) instanceof Object"));
}

#[test]
fn instanceof_array() {
    assert!(bool_("[] instanceof Array"));
    assert!(bool_("[] instanceof Object"));
}

// ── prototype chain via Object.create ────────────────────────────────────────

#[test]
fn object_create_inherits_property() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var proto = { greet: function() { return "hello"; } };
        var child = Object.create(proto);
        var result = child.greet();
    "#);
    assert_eq!(e.get_global("result").to_js_string(), "hello");
}
