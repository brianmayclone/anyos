//! Functions — declarations, expressions, closures, arrow, rest/spread, defaults,
//!            IIFE, recursion, higher-order functions.
//!
//! Level 7: Lexical scoping, upvalue capture, call/apply/bind.

use libjs_tests::JsEngine;

fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }
fn bool_(src: &str) -> bool { js(src).to_boolean() }
fn str_(src: &str) -> String { js(src).to_js_string() }

// ── basic declarations ────────────────────────────────────────────────────────

#[test]
fn function_declaration_and_call() {
    assert_eq!(num("function add(a,b){ return a+b; } add(3,4)"), 7.0);
}

#[test]
fn function_expression() {
    assert_eq!(num("var mul = function(a,b){ return a*b; }; mul(6,7)"), 42.0);
}

#[test]
fn function_expression_body_is_lazy() {
    let v = js("var hit = 0; var f = function(){ hit = 1; missingCall(); }; hit");
    assert_eq!(v.to_number(), 0.0);
}

#[test]
fn unicode_escape_identifier_property_name() {
    assert_eq!(num(r#"var o = {}; o.\u0275prov = 41; o.\u0275prov + 1"#), 42.0);
}

#[test]
fn parenthesized_iife_receives_multiple_arguments() {
    assert_eq!(
        num("(function(){ var G = {}; (function(y, v){ y.answer = v; })(G, 42); return G.answer; })()"),
        42.0
    );
}

#[test]
fn uri_globals_preserve_reserved_decode_uri() {
    assert_eq!(str_(r#"decodeURI("https%3A%2F%2Fx.test%2Fa%20b?x%3D1")"#), "https%3A%2F%2Fx.test%2Fa b?x%3D1");
    assert_eq!(str_(r#"decodeURIComponent("https%3A%2F%2Fx.test%2Fa%20b")"#), "https://x.test/a b");
    assert_eq!(str_(r#"encodeURI("https://x.test/a b?x=1")"#), "https://x.test/a%20b?x=1");
}

#[test]
fn apply_return_value_can_be_called() {
    assert_eq!(
        num("function id(x){ return x; } id.apply(null, [function(){ return 42; }])()"),
        42.0
    );
}

#[test]
fn apply_with_arguments_object_return_value_can_be_called() {
    assert_eq!(
        num("function id(x){ return x; } function guard(){ return id.apply(void 0, arguments); } guard(function(){ return 42; }, 'name', {root: true})()"),
        42.0
    );
}

#[test]
fn promise_resolve_reject_are_bound_to_their_own_promise() {
    let mut e = JsEngine::new();
    e.eval(
        r#"
        var out = '';
        var p = new Promise(function(resolveOuter) {
            new Promise(function(resolveInner) { resolveInner('inner'); });
            resolveOuter('outer');
        });
        p.then(function(v) { out = v; });
        "#,
    );
    assert_eq!(e.eval("out").to_js_string(), "outer");
}

#[test]
fn promise_all_waits_for_pending_entries() {
    let mut e = JsEngine::new();
    e.eval(
        r#"
        var resolveLater;
        var out = 'pending';
        var p = new Promise(function(resolve) { resolveLater = resolve; });
        Promise.all([p]).then(function(values) { out = 'all:' + values[0]; });
        "#,
    );
    assert_eq!(e.eval("out").to_js_string(), "pending");
    e.eval("resolveLater(42);");
    assert_eq!(e.eval("out").to_js_string(), "all:42");
}

#[test]
fn named_function_expression() {
    assert_eq!(
        num("var fn = function factorial(n){ return n<=1?1:n*factorial(n-1); }; fn(5)"),
        120.0
    );
}

#[test]
fn implicit_return_undefined() {
    let v = js("function f(){} f()");
    assert!(!v.to_boolean());
}

#[test]
fn early_return() {
    // to_number() on a string like "big" returns NaN
    assert!(num("function f(x){ if(x>5) return 'big'; return 'small'; } f(10)").is_nan());
    assert_eq!(str_("function f(x){ if(x>5) return 'big'; return 'small'; } f(10)"), "big");
    assert_eq!(str_("function f(x){ if(x>5) return 'big'; return 'small'; } f(3)"), "small");
}

// ── default parameters ────────────────────────────────────────────────────────

#[test]
fn default_param_used() {
    assert_eq!(num("function greet(name = 'world') { return name.length; } greet()"), 5.0);
}

#[test]
fn default_param_overridden() {
    assert_eq!(str_("function greet(name = 'world') { return name; } greet('Alice')"), "Alice");
}

#[test]
fn default_param_expression() {
    assert_eq!(num("function f(x, y = x * 2) { return y; } f(5)"), 10.0);
}

// ── rest parameters ───────────────────────────────────────────────────────────

#[test]
fn rest_params_collect() {
    assert_eq!(
        num("function sum(...args) { return args.reduce((a,b)=>a+b, 0); } sum(1,2,3,4,5)"),
        15.0
    );
}

#[test]
fn rest_params_after_named() {
    assert_eq!(
        num("function f(first, ...rest) { return rest.length; } f(1,2,3,4)"),
        3.0
    );
}

// ── spread in function calls ──────────────────────────────────────────────────

#[test]
fn spread_in_call() {
    assert_eq!(num("function add(a,b,c){ return a+b+c; } add(...[1,2,3])"), 6.0);
}

#[test]
fn spread_mixed_with_args() {
    assert_eq!(num("function f(a,b,c,d){ return a+b+c+d; } f(1, ...[2,3], 4)"), 10.0);
}

// ── arrow functions ───────────────────────────────────────────────────────────

#[test]
fn arrow_expression_body() {
    assert_eq!(num("var double = x => x * 2; double(21)"), 42.0);
}

#[test]
fn arrow_block_body() {
    assert_eq!(num("var add = (a, b) => { return a + b; }; add(10, 32)"), 42.0);
}

#[test]
fn arrow_no_params() {
    assert_eq!(num("var get42 = () => 42; get42()"), 42.0);
}

#[test]
fn arrow_multiple_params() {
    assert_eq!(num("var sum3 = (a,b,c) => a+b+c; sum3(1,2,3)"), 6.0);
}

// ── IIFE ─────────────────────────────────────────────────────────────────────

#[test]
fn iife_executes_immediately() {
    assert_eq!(num("(function() { return 42; })()"), 42.0);
}

#[test]
fn iife_with_argument() {
    assert_eq!(num("(function(x) { return x * 2; })(21)"), 42.0);
}

#[test]
fn arrow_iife() {
    assert_eq!(num("((x) => x + 1)(41)"), 42.0);
}

// ── closures ─────────────────────────────────────────────────────────────────

#[test]
fn closure_captures_variable() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function makeAdder(n) {
            return function(x) { return x + n; };
        }
        var add5 = makeAdder(5);
        var result = add5(37);
    "#);
    assert_eq!(e.get_global("result").to_number(), 42.0);
}

#[test]
fn closure_counter() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function makeCounter() {
            var count = 0;
            return {
                inc: function() { count++; },
                get: function() { return count; }
            };
        }
        var c = makeCounter();
        c.inc(); c.inc(); c.inc();
        var result = c.get();
    "#);
    assert_eq!(e.get_global("result").to_number(), 3.0);
}

#[test]
fn closure_shares_mutable_state() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var n = 0;
        var inc = function() { n++; };
        inc(); inc(); inc(); inc(); inc();
    "#);
    assert_eq!(e.get_global("n").to_number(), 5.0);
}

// ── recursion ─────────────────────────────────────────────────────────────────

#[test]
fn factorial_recursive() {
    assert_eq!(num("function f(n){ return n<=1?1:n*f(n-1); } f(6)"), 720.0);
}

#[test]
fn fibonacci_recursive() {
    assert_eq!(num("function fib(n){ return n<2?n:fib(n-1)+fib(n-2); } fib(10)"), 55.0);
}

#[test]
fn mutual_recursion_even_odd() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function isEven(n) { return n === 0 ? true : isOdd(n - 1); }
        function isOdd(n)  { return n === 0 ? false : isEven(n - 1); }
        var a = isEven(10);
        var b = isOdd(7);
    "#);
    assert!(e.get_global("a").to_boolean());
    assert!(e.get_global("b").to_boolean());
}

// ── higher-order functions ────────────────────────────────────────────────────

#[test]
fn function_returns_function() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function multiplier(factor) { return x => x * factor; }
        var triple = multiplier(3);
        var result = triple(14);
    "#);
    assert_eq!(e.get_global("result").to_number(), 42.0);
}

#[test]
fn function_takes_function_as_arg() {
    assert_eq!(
        num("function apply(f, x) { return f(x); } apply(x => x * x, 6)"),
        36.0
    );
}

#[test]
fn compose_two_functions() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function compose(f, g) { return x => f(g(x)); }
        var addOne = x => x + 1;
        var double = x => x * 2;
        var addOneThenDouble = compose(double, addOne);
        var result = addOneThenDouble(4);
    "#);
    // double(addOne(4)) = double(5) = 10
    assert_eq!(e.get_global("result").to_number(), 10.0);
}

// ── call / apply / bind ───────────────────────────────────────────────────────

#[test]
fn function_call_method() {
    assert_eq!(
        num(r#"
            var o = { val: 10 };
            function getVal() { return this.val; }
            getVal.call(o)
        "#),
        10.0
    );
}

#[test]
fn function_expression_call_can_initialize_global_namespace() {
    assert_eq!(
        num(r#"
            this.ns = this.ns || {};
            (function(_) { _.answer = 42; }).call(this, this.ns);
            this.ns.answer
        "#),
        42.0
    );
}

#[test]
fn strict_script_top_level_this_still_initializes_namespace() {
    assert_eq!(
        num(r#"
            "use strict";
            this.ns = this.ns || {};
            (function(_) { _.answer = 42; }).call(this, this.ns);
            this.ns.answer
        "#),
        42.0
    );
}

#[test]
fn function_expression_with_try_body_call_executes() {
    assert_eq!(
        num(r#"
            this.ns = this.ns || {};
            (function(_) { try { _.answer = 42; } catch(e) { _.answer = 1; } }).call(this, this.ns);
            this.ns.answer
        "#),
        42.0
    );
}

#[test]
fn function_apply_method() {
    assert_eq!(
        num("function add(a,b){ return a+b; } add.apply(null, [17, 25])"),
        42.0
    );
}

#[test]
fn function_bind() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function greet(greeting) { return greeting + ' ' + this.name; }
        var obj = { name: 'World' };
        var boundGreet = greet.bind(obj);
        var result = boundGreet('Hello');
    "#);
    assert_eq!(e.get_global("result").to_js_string(), "Hello World");
}

// ── arguments object (var-function) ───────────────────────────────────────────

#[test]
fn arguments_length() {
    assert_eq!(
        num("function f(){ return arguments.length; } f(1,2,3)"),
        3.0
    );
}

#[test]
fn arguments_index() {
    assert_eq!(
        num("function f(){ return arguments[2]; } f(10,20,30)"),
        30.0
    );
}
