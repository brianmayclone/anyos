//! Strings — String.prototype methods, template literals, Unicode.
//!
//! Level 4: Method calls on string values, dynamic construction.

use libjs_tests::{JsEngine, JsValueExt};

fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }
fn bool_(src: &str) -> bool { js(src).to_boolean() }
fn str_(src: &str) -> String { js(src).to_js_string() }

// ── length ────────────────────────────────────────────────────────────────────

#[test]
fn string_length_basic() {
    assert_eq!(num(r#""hello".length"#), 5.0);
}

#[test]
fn empty_string_length() {
    assert_eq!(num(r#""".length"#), 0.0);
}

// ── charAt / index access ─────────────────────────────────────────────────────

#[test]
fn char_at() {
    assert_eq!(str_(r#""hello".charAt(1)"#), "e");
}

#[test]
fn char_at_out_of_bounds() {
    assert_eq!(str_(r#""hi".charAt(99)"#), "");
}

#[test]
fn bracket_index_access() {
    assert_eq!(str_(r#""world"[0]"#), "w");
    assert_eq!(str_(r#""world"[4]"#), "d");
}

#[test]
fn char_code_at() {
    // 'A' = 65
    assert_eq!(num(r#""ABC".charCodeAt(0)"#), 65.0);
}

// ── indexOf / lastIndexOf / includes ─────────────────────────────────────────

#[test]
fn index_of_found() {
    assert_eq!(num(r#""hello world".indexOf("world")"#), 6.0);
}

#[test]
fn index_of_not_found() {
    assert_eq!(num(r#""hello".indexOf("xyz")"#), -1.0);
}

#[test]
fn last_index_of() {
    assert_eq!(num(r#""abcabc".lastIndexOf("b")"#), 4.0);
}

#[test]
fn includes_true() {
    assert!(bool_(r#""foobar".includes("oba")"#));
}

#[test]
fn includes_false() {
    assert!(!bool_(r#""foobar".includes("xyz")"#));
}

#[test]
fn starts_with() {
    assert!(bool_(r#""hello".startsWith("hel")"#));
    assert!(!bool_(r#""hello".startsWith("ell")"#));
}

#[test]
fn ends_with() {
    assert!(bool_(r#""hello".endsWith("llo")"#));
    assert!(!bool_(r#""hello".endsWith("hel")"#));
}

// ── case conversion ───────────────────────────────────────────────────────────

#[test]
fn to_upper_case() {
    assert_eq!(str_(r#""hello World".toUpperCase()"#), "HELLO WORLD");
}

#[test]
fn to_lower_case() {
    assert_eq!(str_(r#""Hello WORLD".toLowerCase()"#), "hello world");
}

// ── trim ──────────────────────────────────────────────────────────────────────

#[test]
fn trim() {
    assert_eq!(str_(r#""  hello  ".trim()"#), "hello");
}

#[test]
fn trim_start() {
    assert_eq!(str_(r#""  hello  ".trimStart()"#), "hello  ");
}

#[test]
fn trim_end() {
    assert_eq!(str_(r#""  hello  ".trimEnd()"#), "  hello");
}

// ── slice / substring ─────────────────────────────────────────────────────────

#[test]
fn slice_two_args() {
    assert_eq!(str_(r#""hello world".slice(6, 11)"#), "world");
}

#[test]
fn slice_one_arg() {
    assert_eq!(str_(r#""hello world".slice(6)"#), "world");
}

#[test]
fn slice_negative_index() {
    assert_eq!(str_(r#""hello".slice(-3)"#), "llo");
}

#[test]
fn substring_two_args() {
    assert_eq!(str_(r#""hello".substring(1, 3)"#), "el");
}

// ── split / join ──────────────────────────────────────────────────────────────

#[test]
fn split_by_comma() {
    let mut e = JsEngine::new();
    e.eval(r#"var parts = "a,b,c".split(",");"#);
    let arr = e.get_global("parts");
    assert_eq!(arr.get_index(0).to_js_string(), "a");
    assert_eq!(arr.get_index(1).to_js_string(), "b");
    assert_eq!(arr.get_index(2).to_js_string(), "c");
}

#[test]
fn split_by_empty_string_gives_chars() {
    let mut e = JsEngine::new();
    e.eval(r#"var chars = "abc".split("");"#);
    let arr = e.get_global("chars");
    assert_eq!(arr.get_index(0).to_js_string(), "a");
    assert_eq!(arr.get_index(2).to_js_string(), "c");
}

// ── replace / replaceAll ──────────────────────────────────────────────────────

#[test]
fn replace_first_occurrence() {
    assert_eq!(
        str_(r#""aabbaa".replace("aa", "X")"#),
        "Xbbaa"
    );
}

#[test]
fn replace_all_occurrences() {
    assert_eq!(
        str_(r#""aabbaa".replaceAll("a", "X")"#),
        "XXbbXX"
    );
}

// ── repeat / padStart / padEnd ────────────────────────────────────────────────

#[test]
fn repeat() {
    assert_eq!(str_(r#""ab".repeat(3)"#), "ababab");
    assert_eq!(str_(r#""x".repeat(0)"#), "");
}

#[test]
fn pad_start() {
    assert_eq!(str_(r#""5".padStart(3, "0")"#), "005");
    assert_eq!(str_(r#""42".padStart(5)"#), "   42");
}

#[test]
fn pad_end() {
    assert_eq!(str_(r#""hi".padEnd(5, ".")"#), "hi...");
}

// ── concat / at ───────────────────────────────────────────────────────────────

#[test]
fn concat_method() {
    assert_eq!(str_(r#""hello".concat(" ", "world")"#), "hello world");
}

#[test]
fn at_positive() {
    assert_eq!(str_(r#""abcde".at(2)"#), "c");
}

#[test]
fn at_negative() {
    assert_eq!(str_(r#""abcde".at(-1)"#), "e");
    assert_eq!(str_(r#""abcde".at(-2)"#), "d");
}

// ── template literals ─────────────────────────────────────────────────────────

#[test]
fn template_literal_simple() {
    assert_eq!(str_(r#"var x = 42; `value is ${x}`"#), "value is 42");
}

#[test]
fn template_literal_expression() {
    assert_eq!(str_(" `${2 + 3}` "), "5");
}

#[test]
fn template_literal_multi_interpolation() {
    assert_eq!(
        str_(r#"var a = 'foo'; var b = 'bar'; `${a}-${b}`"#),
        "foo-bar"
    );
}

#[test]
fn template_literal_nested_expression() {
    assert_eq!(str_(r#"`${true ? 'yes' : 'no'}`"#), "yes");
}

#[test]
fn template_literal_no_interpolation() {
    assert_eq!(str_("`just text`"), "just text");
}

// ── string numeric conversions ────────────────────────────────────────────────

#[test]
fn number_to_string() {
    assert_eq!(str_("(42).toString()"), "42");
    assert_eq!(str_("(3.14).toString()"), "3.14");
    assert_eq!(str_("(0).toString()"), "0");
}

#[test]
fn to_fixed() {
    assert_eq!(str_("(3.14159).toFixed(2)"), "3.14");
    assert_eq!(str_("(1.0).toFixed(3)"), "1.000");
}

#[test]
fn string_string_method_chaining() {
    assert_eq!(
        str_(r#""  Hello World  ".trim().toLowerCase().replace("hello", "hi")"#),
        "hi world"
    );
}
