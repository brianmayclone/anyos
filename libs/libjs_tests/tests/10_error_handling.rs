//! Error handling — try/catch/finally, throw, Error types, re-throw, nested.
//!
//! Level 10: Exception propagation across function boundaries.

use libjs_tests::JsEngine;

fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }
fn bool_(src: &str) -> bool { js(src).to_boolean() }
fn str_(src: &str) -> String { js(src).to_js_string() }

// ── basic try / catch ─────────────────────────────────────────────────────────

#[test]
fn catch_thrown_string() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var caught = null;
        try {
            throw 'oops';
        } catch (e) {
            caught = e;
        }
    "#);
    assert_eq!(e.get_global("caught").to_js_string(), "oops");
}

#[test]
fn catch_thrown_number() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var val = 0;
        try { throw 42; }
        catch (e) { val = e; }
    "#);
    assert_eq!(e.get_global("val").to_number(), 42.0);
}

#[test]
fn catch_error_object() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var msg = '';
        try {
            throw new Error('something went wrong');
        } catch (e) {
            msg = e.message;
        }
    "#);
    assert_eq!(e.get_global("msg").to_js_string(), "something went wrong");
}

#[test]
fn no_throw_skips_catch() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var x = 0;
        try { x = 1; }
        catch (e) { x = 99; }
    "#);
    assert_eq!(e.get_global("x").to_number(), 1.0);
}

// ── finally ───────────────────────────────────────────────────────────────────

#[test]
fn finally_always_runs_without_throw() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var log = '';
        try { log += 'try'; }
        finally { log += '-finally'; }
    "#);
    assert_eq!(e.get_global("log").to_js_string(), "try-finally");
}

#[test]
fn finally_always_runs_with_throw() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var log = '';
        try {
            log += 'try';
            throw new Error('x');
        } catch(e) {
            log += '-catch';
        } finally {
            log += '-finally';
        }
    "#);
    assert_eq!(e.get_global("log").to_js_string(), "try-catch-finally");
}

#[test]
fn finally_runs_even_when_returning() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var ran = false;
        function f() {
            try { return 1; }
            finally { ran = true; }
        }
        var result = f();
    "#);
    assert!(e.get_global("ran").to_boolean());
    assert_eq!(e.get_global("result").to_number(), 1.0);
}

// ── Error types ───────────────────────────────────────────────────────────────

#[test]
fn error_has_message_property() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var err = new Error('test message');
        var m = err.message;
    "#);
    assert_eq!(e.get_global("m").to_js_string(), "test message");
}

#[test]
fn custom_error_via_class() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class AppError extends Error {
            constructor(msg, code) {
                super(msg);
                this.code = code;
            }
        }
        var caught_code = 0;
        var caught_msg = '';
        try {
            throw new AppError('not found', 404);
        } catch(e) {
            caught_code = e.code;
            caught_msg  = e.message;
        }
    "#);
    assert_eq!(e.get_global("caught_code").to_number(), 404.0);
    assert_eq!(e.get_global("caught_msg").to_js_string(), "not found");
}

// ── throw propagation ─────────────────────────────────────────────────────────

#[test]
fn throw_propagates_up_call_stack() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function inner() { throw new Error('deep'); }
        function middle() { inner(); }
        var msg = '';
        try {
            middle();
        } catch(e) {
            msg = e.message;
        }
    "#);
    assert_eq!(e.get_global("msg").to_js_string(), "deep");
}

#[test]
fn re_throw_same_error() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var final_msg = '';
        try {
            try {
                throw new Error('original');
            } catch(e) {
                throw e; // re-throw
            }
        } catch(e) {
            final_msg = e.message;
        }
    "#);
    assert_eq!(e.get_global("final_msg").to_js_string(), "original");
}

#[test]
fn selective_rethrow() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var result = '';
        try {
            try {
                throw new Error('inner error');
            } catch(e) {
                if (e.message === 'inner error') {
                    result = 'handled';
                } else {
                    throw e;
                }
            }
        } catch(e) {
            result = 'unhandled';
        }
    "#);
    assert_eq!(e.get_global("result").to_js_string(), "handled");
}

// ── multiple catch strategies ─────────────────────────────────────────────────

#[test]
fn catch_different_error_types() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class NetworkError extends Error { constructor(m) { super(m); this.type='network'; } }
        class ParseError   extends Error { constructor(m) { super(m); this.type='parse'; } }
        function handle(err) {
            try { throw err; }
            catch(e) { return e.type || 'unknown'; }
        }
        var r1 = handle(new NetworkError('timeout'));
        var r2 = handle(new ParseError('bad json'));
    "#);
    assert_eq!(e.get_global("r1").to_js_string(), "network");
    assert_eq!(e.get_global("r2").to_js_string(), "parse");
}

// ── try inside loop ───────────────────────────────────────────────────────────

#[test]
fn try_catch_inside_loop_continues() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var successes = 0;
        var failures  = 0;
        var inputs = [1, null, 2, null, 3];
        for (var v of inputs) {
            try {
                if (v === null) throw new Error('null!');
                successes++;
            } catch(e) {
                failures++;
            }
        }
    "#);
    assert_eq!(e.get_global("successes").to_number(), 3.0);
    assert_eq!(e.get_global("failures").to_number(), 2.0);
}

// ── throw non-Error values ────────────────────────────────────────────────────

#[test]
fn throw_object() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var code = 0;
        try {
            throw { code: 500, message: 'server error' };
        } catch(e) {
            code = e.code;
        }
    "#);
    assert_eq!(e.get_global("code").to_number(), 500.0);
}

#[test]
fn throw_in_conditional() {
    assert_eq!(
        str_(r#"
            function safe_div(a, b) {
                if (b === 0) throw new Error('div by zero');
                return a / b;
            }
            var msg = '';
            try { safe_div(1, 0); }
            catch(e) { msg = e.message; }
            msg
        "#),
        "div by zero"
    );
}
