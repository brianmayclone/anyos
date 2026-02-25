//! Primitives — literals, typeof, type coercion.
//!
//! Level 1: Every test operates on a single expression or assignment.
//! No functions, no objects, no control flow.

use libjs_tests::JsEngine;

// ── helpers ───────────────────────────────────────────────────────────────────

/// Evaluate `src` in a fresh engine, return the JsValue.
fn js(src: &str) -> libjs_tests::JsValue {
    JsEngine::new().eval(src)
}

/// Evaluate and return the numeric result.
fn num(src: &str) -> f64 {
    js(src).to_number()
}

/// Evaluate and return the boolean result.
fn bool_(src: &str) -> bool {
    js(src).to_boolean()
}

/// Evaluate and return the string result.
fn str_(src: &str) -> String {
    js(src).to_js_string()
}

// ── number literals ───────────────────────────────────────────────────────────

#[test]
fn integer_literal() {
    assert_eq!(num("42"), 42.0);
}

#[test]
fn float_literal() {
    assert!((num("3.14") - 3.14).abs() < 1e-10);
}

#[test]
fn negative_literal() {
    assert_eq!(num("-7"), -7.0);
}

#[test]
fn hex_literal() {
    assert_eq!(num("0xFF"), 255.0);
    assert_eq!(num("0x10"), 16.0);
}

#[test]
fn octal_literal() {
    assert_eq!(num("0o17"), 15.0);
    assert_eq!(num("0o10"), 8.0);
}

#[test]
fn binary_literal() {
    assert_eq!(num("0b1010"), 10.0);
    assert_eq!(num("0b1111"), 15.0);
}

#[test]
fn number_infinity() {
    assert!(num("Infinity").is_infinite());
    assert!(num("-Infinity").is_infinite());
    assert!(num("Infinity") > 0.0);
    assert!(num("-Infinity") < 0.0);
}

#[test]
fn number_nan() {
    assert!(num("NaN").is_nan());
}

// ── string literals ───────────────────────────────────────────────────────────

#[test]
fn double_quoted_string() {
    assert_eq!(str_(r#""hello""#), "hello");
}

#[test]
fn single_quoted_string() {
    assert_eq!(str_("'world'"), "world");
}

#[test]
fn empty_string() {
    assert_eq!(str_(r#""""#), "");
}

#[test]
fn string_escape_newline() {
    assert_eq!(str_(r#""a\nb""#), "a\nb");
}

#[test]
fn string_escape_tab() {
    assert_eq!(str_(r#""a\tb""#), "a\tb");
}

#[test]
fn string_escape_backslash() {
    assert_eq!(str_(r#""a\\b""#), "a\\b");
}

// ── boolean literals ──────────────────────────────────────────────────────────

#[test]
fn bool_true() {
    assert!(bool_("true"));
}

#[test]
fn bool_false() {
    assert!(!bool_("false"));
}

// ── null / undefined ──────────────────────────────────────────────────────────

#[test]
fn null_is_falsy() {
    assert!(!bool_("null"));
}

#[test]
fn null_to_number_is_zero() {
    assert_eq!(num("null"), 0.0);
}

#[test]
fn undefined_is_falsy() {
    assert!(!bool_("undefined"));
}

#[test]
fn undefined_to_number_is_nan() {
    assert!(num("undefined").is_nan());
}

// ── typeof ────────────────────────────────────────────────────────────────────

#[test]
fn typeof_number() {
    assert_eq!(str_("typeof 42"), "number");
}

#[test]
fn typeof_string() {
    assert_eq!(str_(r#"typeof "hi""#), "string");
}

#[test]
fn typeof_boolean() {
    assert_eq!(str_("typeof true"), "boolean");
}

#[test]
fn typeof_undefined() {
    assert_eq!(str_("typeof undefined"), "undefined");
}

#[test]
fn typeof_null() {
    // ECMAScript historical quirk: typeof null === "object"
    assert_eq!(str_("typeof null"), "object");
}

#[test]
fn typeof_object() {
    assert_eq!(str_("typeof {}"), "object");
}

#[test]
fn typeof_array() {
    assert_eq!(str_("typeof []"), "object");
}

#[test]
fn typeof_function() {
    assert_eq!(str_("typeof function(){}"), "function");
}

// ── type coercion (implicit conversions) ─────────────────────────────────────

#[test]
fn number_plus_string_concatenates() {
    assert_eq!(str_(r#"1 + "2""#), "12");
    assert_eq!(str_(r#""3" + 4"#), "34");
}

#[test]
fn string_minus_number_coerces() {
    assert_eq!(num(r#""10" - 3"#), 7.0);
}

#[test]
fn string_times_number_coerces() {
    assert_eq!(num(r#""4" * "3""#), 12.0);
}

#[test]
fn bool_to_number() {
    assert_eq!(num("true + 1"), 2.0);
    assert_eq!(num("false + 1"), 1.0);
}

#[test]
fn null_plus_number() {
    assert_eq!(num("null + 5"), 5.0);
}

#[test]
fn undefined_plus_number_is_nan() {
    assert!(num("undefined + 1").is_nan());
}

#[test]
fn empty_string_is_falsy() {
    assert!(!bool_(r#""""#));
}

#[test]
fn zero_is_falsy() {
    assert!(!bool_("0"));
}

#[test]
fn nan_is_falsy() {
    assert!(!bool_("NaN"));
}

#[test]
fn nonempty_string_is_truthy() {
    assert!(bool_(r#""0""#)); // "0" is truthy (non-empty string)
}

#[test]
fn object_is_truthy() {
    assert!(bool_("({})"));
}

#[test]
fn array_is_truthy() {
    assert!(bool_("[]"));
}

// ── strict vs loose equality edge cases ──────────────────────────────────────

#[test]
fn loose_eq_null_undefined() {
    // null == undefined is true (ECMA spec)
    assert!(bool_("null == undefined"));
    assert!(bool_("undefined == null"));
}

#[test]
fn strict_eq_null_undefined() {
    assert!(!bool_("null === undefined"));
}

#[test]
fn loose_eq_zero_false() {
    assert!(bool_("0 == false"));
    assert!(bool_("1 == true"));
}

#[test]
fn strict_eq_zero_false() {
    assert!(!bool_("0 === false"));
}

#[test]
fn loose_eq_string_number() {
    assert!(bool_(r#""42" == 42"#));
}

#[test]
fn strict_eq_string_number() {
    assert!(!bool_(r#""42" === 42"#));
}

#[test]
fn nan_not_equal_to_itself() {
    assert!(!bool_("NaN === NaN"));
    assert!(!bool_("NaN == NaN"));
}
