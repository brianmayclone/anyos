//! Control flow — if/else, switch, while, do-while, for, for-in, for-of,
//!               break, continue, labeled breaks.
//!
//! Level 8: Interaction between loops, conditions, and variable state.

use libjs_tests::{JsEngine, JsValueExt};

fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }
fn bool_(src: &str) -> bool { js(src).to_boolean() }
fn str_(src: &str) -> String { js(src).to_js_string() }

// ── if / else if / else ───────────────────────────────────────────────────────

#[test]
fn if_true_branch() {
    assert_eq!(num("var x = 0; if (true) { x = 1; } x"), 1.0);
}

#[test]
fn if_false_skips_body() {
    assert_eq!(num("var x = 0; if (false) { x = 1; } x"), 0.0);
}

#[test]
fn if_else() {
    assert_eq!(str_("var r; if (5 > 10) { r = 'big'; } else { r = 'small'; } r"), "small");
}

#[test]
fn else_if_chain() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function classify(n) {
            if (n < 0)       return 'negative';
            else if (n === 0) return 'zero';
            else              return 'positive';
        }
        var a = classify(-1);
        var b = classify(0);
        var c = classify(5);
    "#);
    assert_eq!(e.get_global("a").to_js_string(), "negative");
    assert_eq!(e.get_global("b").to_js_string(), "zero");
    assert_eq!(e.get_global("c").to_js_string(), "positive");
}

// ── switch ────────────────────────────────────────────────────────────────────

#[test]
fn switch_matches_case() {
    assert_eq!(
        str_(r#"var x = 2; var r; switch(x){ case 1: r='one'; break; case 2: r='two'; break; default: r='other'; } r"#),
        "two"
    );
}

#[test]
fn switch_default() {
    assert_eq!(
        str_(r#"var x = 99; var r; switch(x){ case 1: r='one'; break; default: r='other'; } r"#),
        "other"
    );
}

#[test]
fn switch_fall_through() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var result = '';
        switch(1) {
            case 1: result += 'a';
            case 2: result += 'b';
            case 3: result += 'c'; break;
            case 4: result += 'd';
        }
    "#);
    assert_eq!(e.get_global("result").to_js_string(), "abc");
}

#[test]
fn switch_strict_equality() {
    // switch uses ===, so string "1" does not match number 1
    let mut e = JsEngine::new();
    e.eval(r#"var r = 'no'; switch("1"){ case 1: r='yes'; break; default: r='no'; }"#);
    assert_eq!(e.get_global("r").to_js_string(), "no");
}

// ── while ─────────────────────────────────────────────────────────────────────

#[test]
fn while_counts_up() {
    let mut e = JsEngine::new();
    e.eval("var i = 0; var s = 0; while(i < 5){ s += i; i++; }");
    assert_eq!(e.get_global("s").to_number(), 10.0); // 0+1+2+3+4
}

#[test]
fn while_never_executes_when_false() {
    let mut e = JsEngine::new();
    e.eval("var x = 99; while(false){ x = 0; }");
    assert_eq!(e.get_global("x").to_number(), 99.0);
}

// ── do-while ──────────────────────────────────────────────────────────────────

#[test]
fn do_while_executes_at_least_once() {
    let mut e = JsEngine::new();
    e.eval("var x = 0; do { x = 1; } while(false);");
    assert_eq!(e.get_global("x").to_number(), 1.0);
}

#[test]
fn do_while_loop() {
    let mut e = JsEngine::new();
    e.eval("var i = 0; var sum = 0; do { sum += i; i++; } while(i <= 5);");
    assert_eq!(e.get_global("sum").to_number(), 15.0); // 0+1+2+3+4+5
}

// ── for ───────────────────────────────────────────────────────────────────────

#[test]
fn for_loop_sum() {
    assert_eq!(num("var s = 0; for(var i = 1; i <= 10; i++) { s += i; } s"), 55.0);
}

#[test]
fn for_loop_countdown() {
    let mut e = JsEngine::new();
    e.eval("var a = []; for(var i = 5; i > 0; i--) { a.push(i); }");
    let a = e.get_global("a");
    assert_eq!(a.get_index(0).to_number(), 5.0);
    assert_eq!(a.get_index(4).to_number(), 1.0);
}

#[test]
fn for_loop_can_omit_init() {
    assert_eq!(num("var i = 0; var s = 0; for(; i < 5; i++) s += i; s"), 10.0);
}

// ── break / continue ─────────────────────────────────────────────────────────

#[test]
fn break_exits_loop() {
    let mut e = JsEngine::new();
    e.eval("var i = 0; while(true){ if(i===5) break; i++; }");
    assert_eq!(e.get_global("i").to_number(), 5.0);
}

#[test]
fn continue_skips_iteration() {
    let mut e = JsEngine::new();
    // Sum only even numbers 0-9
    e.eval("var s = 0; for(var i=0;i<10;i++){ if(i%2!==0) continue; s+=i; }");
    assert_eq!(e.get_global("s").to_number(), 20.0); // 0+2+4+6+8
}

#[test]
fn break_in_switch_inside_loop() {
    let mut e = JsEngine::new();
    // break in switch should NOT break the outer loop
    e.eval(r#"
        var count = 0;
        for (var i = 0; i < 5; i++) {
            switch(i) {
                case 3: count += 100; break;
                default: count += 1;
            }
        }
    "#);
    // i=0→+1, i=1→+1, i=2→+1, i=3→+100, i=4→+1 → 104
    assert_eq!(e.get_global("count").to_number(), 104.0);
}

// ── for-of ────────────────────────────────────────────────────────────────────

#[test]
fn for_of_array() {
    let mut e = JsEngine::new();
    e.eval("var sum = 0; for(var v of [1,2,3,4,5]){ sum += v; }");
    assert_eq!(e.get_global("sum").to_number(), 15.0);
}

#[test]
fn for_of_string_chars() {
    let mut e = JsEngine::new();
    e.eval(r#"var result = ''; for(var ch of "hello"){ result += ch.toUpperCase(); }"#);
    assert_eq!(e.get_global("result").to_js_string(), "HELLO");
}

#[test]
fn for_of_with_destructuring() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var pairs = [[1,'a'],[2,'b'],[3,'c']];
        var keys = []; var vals = [];
        for (var [k, v] of pairs) { keys.push(k); vals.push(v); }
    "#);
    assert_eq!(e.get_global("keys").get_index(1).to_number(), 2.0);
    assert_eq!(e.get_global("vals").get_index(2).to_js_string(), "c");
}

// ── for-in ────────────────────────────────────────────────────────────────────

#[test]
fn for_in_object_keys() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var o = { a: 1, b: 2, c: 3 };
        var count = 0;
        for (var k in o) { count++; }
    "#);
    assert_eq!(e.get_global("count").to_number(), 3.0);
}

#[test]
fn for_in_access_values() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var o = { x: 10, y: 20, z: 12 };
        var sum = 0;
        for (var k in o) { sum += o[k]; }
    "#);
    assert_eq!(e.get_global("sum").to_number(), 42.0);
}

// ── nested loops ─────────────────────────────────────────────────────────────

#[test]
fn nested_for_loops_multiplication_table() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var sum = 0;
        for (var i = 1; i <= 3; i++) {
            for (var j = 1; j <= 3; j++) {
                sum += i * j;
            }
        }
    "#);
    // 1*1+1*2+1*3 + 2*1+2*2+2*3 + 3*1+3*2+3*3 = 6 + 12 + 18 = 36
    assert_eq!(e.get_global("sum").to_number(), 36.0);
}

#[test]
fn break_inner_does_not_break_outer() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var outer_count = 0;
        for (var i = 0; i < 3; i++) {
            outer_count++;
            for (var j = 0; j < 10; j++) {
                if (j === 2) break;
            }
        }
    "#);
    assert_eq!(e.get_global("outer_count").to_number(), 3.0);
}

// ── labeled break ─────────────────────────────────────────────────────────────

#[test]
fn labeled_break_exits_outer_loop() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var found_i = -1;
        var found_j = -1;
        outer: for (var i = 0; i < 5; i++) {
            for (var j = 0; j < 5; j++) {
                if (i === 2 && j === 3) {
                    found_i = i;
                    found_j = j;
                    break outer;
                }
            }
        }
    "#);
    assert_eq!(e.get_global("found_i").to_number(), 2.0);
    assert_eq!(e.get_global("found_j").to_number(), 3.0);
    // The outer loop stopped, so i didn't advance past 2
}
