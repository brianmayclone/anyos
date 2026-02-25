//! Variables — var / let / const, scoping, shadowing, re-assignment.
//!
//! Level 3: Multi-statement scripts; variable lifetimes matter.

use libjs_tests::JsEngine;

fn num(src: &str) -> f64 { JsEngine::new().eval(src).to_number() }
fn bool_(src: &str) -> bool { JsEngine::new().eval(src).to_boolean() }
fn str_(src: &str) -> String { JsEngine::new().eval(src).to_js_string() }

// ── var ───────────────────────────────────────────────────────────────────────

#[test]
fn var_declaration_and_use() {
    assert_eq!(num("var x = 10; x"), 10.0);
}

#[test]
fn var_reassignment() {
    assert_eq!(num("var x = 5; x = 20; x"), 20.0);
}

#[test]
fn var_uninitialized_is_undefined() {
    assert!(JsEngine::new().eval("var x; x").to_number().is_nan());
}

#[test]
fn var_multiple_in_one_statement() {
    let mut e = JsEngine::new();
    e.eval("var a = 1, b = 2, c = 3;");
    assert_eq!(e.get_global("a").to_number(), 1.0);
    assert_eq!(e.get_global("b").to_number(), 2.0);
    assert_eq!(e.get_global("c").to_number(), 3.0);
}

// ── let ───────────────────────────────────────────────────────────────────────

#[test]
fn let_declaration_and_use() {
    assert_eq!(num("let y = 7; y"), 7.0);
}

#[test]
fn let_reassignment() {
    assert_eq!(num("let y = 1; y = 99; y"), 99.0);
}

#[test]
fn let_block_scope() {
    // `y` declared inside a block must NOT leak outside
    let mut e = JsEngine::new();
    e.eval("var result = 'outer'; { let inner = 'inside'; result = inner; }");
    assert_eq!(e.get_global("result").to_js_string(), "inside");
}

// ── const ─────────────────────────────────────────────────────────────────────

#[test]
fn const_declaration_and_use() {
    assert_eq!(num("const C = 42; C"), 42.0);
}

#[test]
fn const_object_mutation() {
    // const prevents rebinding, but object properties can change
    assert_eq!(num("const obj = { x: 1 }; obj.x = 99; obj.x"), 99.0);
}

// ── shadowing ─────────────────────────────────────────────────────────────────

#[test]
fn inner_let_shadows_outer_var() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var x = 'outer';
        var inner_val = 'not set';
        {
            let x = 'inner';
            inner_val = x;
        }
        var outer_val = x;
    "#);
    assert_eq!(e.get_global("inner_val").to_js_string(), "inner");
    assert_eq!(e.get_global("outer_val").to_js_string(), "outer");
}

// ── sequential dependencies ────────────────────────────────────────────────────

#[test]
fn chain_of_assignments() {
    let mut e = JsEngine::new();
    e.eval("var a = 1; var b = a + 2; var c = b * 3;");
    assert_eq!(e.get_global("c").to_number(), 9.0);
}

#[test]
fn swap_via_temp() {
    let mut e = JsEngine::new();
    e.eval("var a = 10; var b = 20; var t = a; a = b; b = t;");
    assert_eq!(e.get_global("a").to_number(), 20.0);
    assert_eq!(e.get_global("b").to_number(), 10.0);
}

// ── variable in expressions ────────────────────────────────────────────────────

#[test]
fn variable_used_in_arithmetic() {
    assert_eq!(num("var n = 6; n * n + n"), 42.0);
}

#[test]
fn variable_used_in_comparison() {
    assert!(bool_("var x = 5; x > 3"));
    assert!(!bool_("var x = 5; x > 10"));
}

#[test]
fn string_variable_concatenation() {
    assert_eq!(str_("var s = 'hello'; s + ' world'"), "hello world");
}

// ── compound assignment with variables ────────────────────────────────────────

#[test]
fn compound_add_assign_chain() {
    let mut e = JsEngine::new();
    e.eval("var sum = 0; sum += 10; sum += 20; sum += 12;");
    assert_eq!(e.get_global("sum").to_number(), 42.0);
}

// ── destructuring assignment (basic, covered more in 05_arrays / 06_objects) ──

#[test]
fn array_destructure_two_vars() {
    let mut e = JsEngine::new();
    e.eval("var [a, b] = [3, 7];");
    assert_eq!(e.get_global("a").to_number(), 3.0);
    assert_eq!(e.get_global("b").to_number(), 7.0);
}

#[test]
fn object_destructure_basic() {
    let mut e = JsEngine::new();
    e.eval("var { x, y } = { x: 10, y: 20 };");
    assert_eq!(e.get_global("x").to_number(), 10.0);
    assert_eq!(e.get_global("y").to_number(), 20.0);
}

#[test]
fn destructure_with_default() {
    let mut e = JsEngine::new();
    e.eval("var { a = 5, b = 10 } = { a: 1 };");
    assert_eq!(e.get_global("a").to_number(), 1.0);  // provided
    assert_eq!(e.get_global("b").to_number(), 10.0); // default
}

#[test]
fn array_destructure_skip_element() {
    let mut e = JsEngine::new();
    e.eval("var [, second, , fourth] = [1, 2, 3, 4];");
    assert_eq!(e.get_global("second").to_number(), 2.0);
    assert_eq!(e.get_global("fourth").to_number(), 4.0);
}
