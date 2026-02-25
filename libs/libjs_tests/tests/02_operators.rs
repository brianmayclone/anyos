//! Operators — arithmetic, comparison, logical, bitwise, assignment, ternary.
//!
//! Level 2: Expressions with multiple operators, precedence, and edge cases.

use libjs_tests::JsEngine;

fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }
fn bool_(src: &str) -> bool { js(src).to_boolean() }
fn str_(src: &str) -> String { js(src).to_js_string() }

// ── basic arithmetic ──────────────────────────────────────────────────────────

#[test] fn add() { assert_eq!(num("3 + 4"), 7.0); }
#[test] fn sub() { assert_eq!(num("10 - 3"), 7.0); }
#[test] fn mul() { assert_eq!(num("6 * 7"), 42.0); }
#[test] fn div() { assert_eq!(num("10 / 4"), 2.5); }
#[test] fn rem() { assert_eq!(num("10 % 3"), 1.0); }
#[test] fn exp() { assert_eq!(num("2 ** 10"), 1024.0); }
#[test] fn unary_minus() { assert_eq!(num("-(-5)"), 5.0); }
#[test] fn unary_plus_string() { assert_eq!(num(r#"+"42""#), 42.0); }

// ── operator precedence ───────────────────────────────────────────────────────

#[test]
fn mul_before_add() {
    assert_eq!(num("2 + 3 * 4"), 14.0);
}

#[test]
fn parentheses_override_precedence() {
    assert_eq!(num("(2 + 3) * 4"), 20.0);
}

#[test]
fn exp_is_right_associative() {
    // 2 ** 3 ** 2  →  2 ** (3 ** 2)  →  2 ** 9  →  512
    assert_eq!(num("2 ** 3 ** 2"), 512.0);
}

#[test]
fn div_and_mul_left_associative() {
    // 12 / 4 * 3  →  (12 / 4) * 3  →  9
    assert_eq!(num("12 / 4 * 3"), 9.0);
}

// ── division edge cases ───────────────────────────────────────────────────────

#[test]
fn div_by_zero_positive() {
    assert!(num("1 / 0").is_infinite());
    assert!(num("1 / 0") > 0.0);
}

#[test]
fn div_by_zero_negative() {
    assert!(num("-1 / 0").is_infinite());
    assert!(num("-1 / 0") < 0.0);
}

#[test]
fn zero_div_zero_is_nan() {
    assert!(num("0 / 0").is_nan());
}

// ── increment / decrement ─────────────────────────────────────────────────────

#[test]
fn pre_increment() {
    assert_eq!(num("var x = 5; ++x"), 6.0);
}

#[test]
fn post_increment_returns_old() {
    assert_eq!(num("var x = 5; x++"), 5.0);
}

#[test]
fn pre_decrement() {
    assert_eq!(num("var x = 5; --x"), 4.0);
}

#[test]
fn post_decrement_returns_old() {
    assert_eq!(num("var x = 5; x--"), 5.0);
}

#[test]
fn increment_effect_on_variable() {
    let mut e = JsEngine::new();
    e.eval("var x = 0; x++; x++; x++;");
    assert_eq!(e.get_global("x").to_number(), 3.0);
}

// ── compound assignment ───────────────────────────────────────────────────────

#[test]
fn plus_assign() {
    let mut e = JsEngine::new();
    e.eval("var x = 10; x += 5;");
    assert_eq!(e.get_global("x").to_number(), 15.0);
}

#[test]
fn minus_assign() {
    let mut e = JsEngine::new();
    e.eval("var x = 10; x -= 3;");
    assert_eq!(e.get_global("x").to_number(), 7.0);
}

#[test]
fn mul_assign() {
    let mut e = JsEngine::new();
    e.eval("var x = 4; x *= 3;");
    assert_eq!(e.get_global("x").to_number(), 12.0);
}

#[test]
fn div_assign() {
    let mut e = JsEngine::new();
    e.eval("var x = 15; x /= 5;");
    assert_eq!(e.get_global("x").to_number(), 3.0);
}

#[test]
fn rem_assign() {
    let mut e = JsEngine::new();
    e.eval("var x = 17; x %= 5;");
    assert_eq!(e.get_global("x").to_number(), 2.0);
}

#[test]
fn exp_assign() {
    let mut e = JsEngine::new();
    e.eval("var x = 3; x **= 4;");
    assert_eq!(e.get_global("x").to_number(), 81.0);
}

// ── comparison ────────────────────────────────────────────────────────────────

#[test] fn less_than()         { assert!(bool_("3 < 5")); }
#[test] fn less_than_eq()      { assert!(bool_("5 <= 5")); }
#[test] fn greater_than()      { assert!(bool_("7 > 3")); }
#[test] fn greater_than_eq()   { assert!(bool_("3 >= 3")); }
#[test] fn not_equal_strict()  { assert!(bool_("1 !== '1'")); }
#[test] fn equal_strict()      { assert!(bool_("42 === 42")); }

#[test]
fn string_comparison_lexicographic() {
    assert!(bool_(r#""apple" < "banana""#));
    assert!(bool_(r#""z" > "a""#));
}

// ── logical operators ─────────────────────────────────────────────────────────

#[test]
fn logical_and_truthy() {
    // Returns last evaluated value (right side when both truthy)
    assert_eq!(num("1 && 2"), 2.0);
}

#[test]
fn logical_and_short_circuit() {
    // Left is falsy → returns left side (0)
    assert_eq!(num("0 && 99"), 0.0);
}

#[test]
fn logical_or_truthy() {
    // Left truthy → returns left side (1)
    assert_eq!(num("1 || 99"), 1.0);
}

#[test]
fn logical_or_short_circuit() {
    // Left falsy → returns right side (42)
    assert_eq!(num("0 || 42"), 42.0);
}

#[test]
fn logical_not() {
    assert!(!bool_("!true"));
    assert!(bool_("!false"));
    assert!(!bool_("!1"));
    assert!(bool_("!0"));
    assert!(bool_("!null"));
    assert!(bool_("!undefined"));
}

#[test]
fn double_not_converts_to_bool() {
    assert_eq!(str_("!!1"), "true");
    assert_eq!(str_("!!0"), "false");
    assert_eq!(str_("!!null"), "false");
    assert_eq!(str_(r#"!!'hello'"#), "true");
}

// ── nullish coalescing (??) ───────────────────────────────────────────────────

#[test]
fn nullish_coalescing_null_gives_right() {
    assert_eq!(num("null ?? 42"), 42.0);
}

#[test]
fn nullish_coalescing_undefined_gives_right() {
    assert_eq!(num("undefined ?? 7"), 7.0);
}

#[test]
fn nullish_coalescing_zero_gives_left() {
    // Unlike ||, ?? only triggers on null/undefined, NOT on 0 or ""
    assert_eq!(num("0 ?? 99"), 0.0);
}

#[test]
fn nullish_coalescing_empty_string_gives_left() {
    let mut e = JsEngine::new();
    let v = e.eval(r#"var r = "" ?? "default"; r"#);
    assert_eq!(v.to_js_string(), "");
}

// ── bitwise operators ─────────────────────────────────────────────────────────

#[test] fn bitwise_and()  { assert_eq!(num("0b1100 & 0b1010"), 8.0);  } // 1000
#[test] fn bitwise_or()   { assert_eq!(num("0b1100 | 0b1010"), 14.0); } // 1110
#[test] fn bitwise_xor()  { assert_eq!(num("0b1100 ^ 0b1010"), 6.0);  } // 0110
#[test] fn bitwise_not()  { assert_eq!(num("~0"), -1.0); }
#[test] fn left_shift()   { assert_eq!(num("1 << 4"), 16.0); }
#[test] fn right_shift()  { assert_eq!(num("16 >> 2"), 4.0); }
#[test] fn right_shift_signed() {
    // Signed right shift preserves sign
    assert_eq!(num("-8 >> 1"), -4.0);
}
#[test] fn unsigned_right_shift() {
    // >>> treats operand as unsigned 32-bit
    assert_eq!(num("16 >>> 2"), 4.0);
}

// ── bitwise assignment ────────────────────────────────────────────────────────

#[test]
fn bitwise_and_assign() {
    let mut e = JsEngine::new();
    e.eval("var x = 0b1111; x &= 0b1010;");
    assert_eq!(e.get_global("x").to_number(), 10.0);
}

#[test]
fn left_shift_assign() {
    let mut e = JsEngine::new();
    e.eval("var x = 1; x <<= 3;");
    assert_eq!(e.get_global("x").to_number(), 8.0);
}

// ── ternary operator ──────────────────────────────────────────────────────────

#[test]
fn ternary_truthy() {
    assert_eq!(num("true ? 1 : 2"), 1.0);
}

#[test]
fn ternary_falsy() {
    assert_eq!(num("false ? 1 : 2"), 2.0);
}

#[test]
fn ternary_nested() {
    // true ? (false ? 10 : 20) : 30  →  20
    assert_eq!(num("true ? false ? 10 : 20 : 30"), 20.0);
}

#[test]
fn ternary_with_side_effect() {
    let mut e = JsEngine::new();
    e.eval("var x = 0; var r = (x > 0) ? 'pos' : 'nonpos';");
    assert_eq!(e.get_global("r").to_js_string(), "nonpos");
}

// ── comma operator ────────────────────────────────────────────────────────────

#[test]
fn comma_operator_returns_last() {
    assert_eq!(num("(1, 2, 3)"), 3.0);
}
