//! Built-in objects — Math, JSON, Map, Set, Number, parseInt / parseFloat.
//!
//! Level 11: Standard library coverage, edge cases in built-in methods.

use libjs_tests::{JsEngine, JsValueExt};

fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }
fn bool_(src: &str) -> bool { js(src).to_boolean() }
fn str_(src: &str) -> String { js(src).to_js_string() }

// ── parseInt / parseFloat ─────────────────────────────────────────────────────

#[test] fn parse_int_decimal()  { assert_eq!(num("parseInt('42')"), 42.0); }
#[test] fn parse_int_hex()      { assert_eq!(num("parseInt('0xff')"), 255.0); }
#[test] fn parse_int_radix()    { assert_eq!(num("parseInt('1010', 2)"), 10.0); }
#[test] fn parse_int_leading_spaces() { assert_eq!(num("parseInt('  7  ')"), 7.0); }
#[test] fn parse_int_stops_at_non_digit() { assert_eq!(num("parseInt('42px')"), 42.0); }
#[test] fn parse_int_nan()      { assert!(num("parseInt('abc')").is_nan()); }
#[test] fn parse_float_basic()  { assert!((num("parseFloat('3.14')") - 3.14).abs() < 1e-9); }
#[test] fn parse_float_exp()    { assert_eq!(num("parseFloat('1e3')"), 1000.0); }

// ── isNaN / isFinite ──────────────────────────────────────────────────────────

#[test] fn is_nan_true()        { assert!(bool_("isNaN(NaN)")); }
#[test] fn is_nan_string()      { assert!(bool_("isNaN('abc')")); }
#[test] fn is_nan_false()       { assert!(!bool_("isNaN(42)")); }
#[test] fn is_finite_true()     { assert!(bool_("isFinite(42)")); }
#[test] fn is_finite_infinity() { assert!(!bool_("isFinite(Infinity)")); }
#[test] fn is_finite_nan()      { assert!(!bool_("isFinite(NaN)")); }

// ── Number built-ins ─────────────────────────────────────────────────────────

#[test] fn number_is_nan()      { assert!(bool_("Number.isNaN(NaN)")); }
#[test] fn number_is_nan_str()  { assert!(!bool_("Number.isNaN('NaN')")); } // strict!
#[test] fn number_is_finite()   { assert!(bool_("Number.isFinite(42)")); }
#[test] fn number_is_integer()  { assert!(bool_("Number.isInteger(7)")); }
#[test] fn number_is_integer_float() { assert!(!bool_("Number.isInteger(7.1)")); }
#[test] fn number_max_safe_int() { assert!(bool_("Number.MAX_SAFE_INTEGER === 9007199254740991")); }
#[test] fn number_min_safe_int() { assert!(bool_("Number.MIN_SAFE_INTEGER === -9007199254740991")); }

// ── Math ──────────────────────────────────────────────────────────────────────

#[test] fn math_abs()      { assert_eq!(num("Math.abs(-7)"), 7.0); }
#[test] fn math_floor()    { assert_eq!(num("Math.floor(3.9)"), 3.0); }
#[test] fn math_ceil()     { assert_eq!(num("Math.ceil(3.1)"), 4.0); }
#[test] fn math_round_up() { assert_eq!(num("Math.round(3.5)"), 4.0); }
#[test] fn math_round_down() { assert_eq!(num("Math.round(3.4)"), 3.0); }
#[test] fn math_trunc()    { assert_eq!(num("Math.trunc(-3.7)"), -3.0); }
#[test] fn math_min()      { assert_eq!(num("Math.min(3, 1, 4, 1, 5)"), 1.0); }
#[test] fn math_max()      { assert_eq!(num("Math.max(3, 1, 4, 1, 5)"), 5.0); }
#[test] fn math_sqrt()     { assert_eq!(num("Math.sqrt(9)"), 3.0); }
#[test] fn math_pow()      { assert_eq!(num("Math.pow(2, 8)"), 256.0); }
#[test] fn math_pi()       { assert!((num("Math.PI") - std::f64::consts::PI).abs() < 1e-10); }
#[test] fn math_e()        { assert!((num("Math.E") - std::f64::consts::E).abs() < 1e-10); }

#[test]
fn math_min_no_args() {
    assert!(num("Math.min()").is_infinite());
    assert!(num("Math.min()") > 0.0); // +Infinity
}

#[test]
fn math_max_no_args() {
    assert!(num("Math.max()").is_infinite());
    assert!(num("Math.max()") < 0.0); // -Infinity
}

#[test]
fn math_random_in_range() {
    let v = num("Math.random()");
    assert!(v >= 0.0 && v < 1.0);
}

// ── JSON ──────────────────────────────────────────────────────────────────────

#[test]
fn json_stringify_object() {
    let s = str_(r#"JSON.stringify({ a: 1, b: 2 })"#);
    assert!(s.contains('"') && s.contains("a") && s.contains("b"));
}

#[test]
fn json_stringify_array() {
    assert_eq!(str_("JSON.stringify([1,2,3])"), "[1,2,3]");
}

#[test]
fn json_stringify_null() {
    assert_eq!(str_("JSON.stringify(null)"), "null");
}

#[test]
fn json_stringify_number() {
    assert_eq!(str_("JSON.stringify(42)"), "42");
}

#[test]
fn json_stringify_string() {
    assert_eq!(str_(r#"JSON.stringify("hello")"#), r#""hello""#);
}

#[test]
fn json_parse_object() {
    let mut e = JsEngine::new();
    e.eval(r#"var o = JSON.parse('{"x":10,"y":20}');"#);
    assert_eq!(e.get_global("o").get_property("x").to_number(), 10.0);
    assert_eq!(e.get_global("o").get_property("y").to_number(), 20.0);
}

#[test]
fn json_parse_array() {
    let mut e = JsEngine::new();
    e.eval(r#"var a = JSON.parse('[1,2,3]');"#);
    assert_eq!(e.get_global("a").get_index(2).to_number(), 3.0);
}

#[test]
fn json_parse_nested() {
    let mut e = JsEngine::new();
    e.eval(r#"var v = JSON.parse('{"a":{"b":42}}'); var r = v.a.b;"#);
    assert_eq!(e.get_global("r").to_number(), 42.0);
}

#[test]
fn json_roundtrip() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var original = { name: 'Alice', age: 30, scores: [10, 20, 30] };
        var parsed = JSON.parse(JSON.stringify(original));
        var name = parsed.name;
        var score = parsed.scores[1];
    "#);
    assert_eq!(e.get_global("name").to_js_string(), "Alice");
    assert_eq!(e.get_global("score").to_number(), 20.0);
}

// ── Map ───────────────────────────────────────────────────────────────────────

#[test]
fn map_set_and_get() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var m = new Map();
        m.set('key', 42);
        var v = m.get('key');
    "#);
    assert_eq!(e.get_global("v").to_number(), 42.0);
}

#[test]
fn map_has() {
    assert!(bool_(r#"var m = new Map(); m.set('x', 1); m.has('x')"#));
    assert!(!bool_(r#"var m = new Map(); m.has('missing')"#));
}

#[test]
fn map_size() {
    let mut e = JsEngine::new();
    e.eval(r#"var m = new Map(); m.set('a',1); m.set('b',2); var s = m.size;"#);
    assert_eq!(e.get_global("s").to_number(), 2.0);
}

#[test]
fn map_delete() {
    let mut e = JsEngine::new();
    e.eval(r#"var m = new Map(); m.set('a',1); m.delete('a'); var s = m.size;"#);
    assert_eq!(e.get_global("s").to_number(), 0.0);
}

#[test]
fn map_object_keys() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var m = new Map();
        var k = { id: 1 };
        m.set(k, 'value');
        var v = m.get(k);
    "#);
    assert_eq!(e.get_global("v").to_js_string(), "value");
}

// ── Set ───────────────────────────────────────────────────────────────────────

#[test]
fn set_add_and_has() {
    assert!(bool_(r#"var s = new Set(); s.add(42); s.has(42)"#));
}

#[test]
fn set_deduplicates() {
    let mut e = JsEngine::new();
    e.eval(r#"var s = new Set(); s.add(1); s.add(2); s.add(1); var sz = s.size;"#);
    assert_eq!(e.get_global("sz").to_number(), 2.0);
}

#[test]
fn set_delete() {
    let mut e = JsEngine::new();
    e.eval(r#"var s = new Set([1,2,3]); s.delete(2); var sz = s.size;"#);
    assert_eq!(e.get_global("sz").to_number(), 2.0);
}

#[test]
fn set_iteration() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var s = new Set([10, 20, 30]);
        var sum = 0;
        s.forEach(function(v) { sum += v; });
    "#);
    assert_eq!(e.get_global("sum").to_number(), 60.0);
}
