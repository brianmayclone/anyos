//! test262-style conformance tests for libjs.
//!
//! Each test evaluates JavaScript code and checks the result or console output.
//! Tests are organized by ECMAScript specification section.

extern crate libjs;

use libjs::JsEngine;

/// Helper: evaluate JS and return the result as a string.
fn eval_str(code: &str) -> String {
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    let result = engine.eval(code);
    result.to_js_string()
}

/// Helper: evaluate JS and return console output joined by newlines.
fn eval_console(code: &str) -> String {
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.eval(code);
    engine.console_output().join("\n")
}

/// Helper: evaluate JS and check if it threw (last_exception is set).
fn eval_throws(code: &str) -> bool {
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.eval(code);
    engine.last_exception().is_some()
}

// ═══════════════════════════════════════════════════════════
// §13.3 — Variable Declarations (let, const, var)
// ═══════════════════════════════════════════════════════════

#[test]
fn var_declaration() {
    assert_eq!(eval_str("var x = 42; x"), "42");
}

#[test]
fn let_declaration() {
    assert_eq!(eval_str("let x = 10; x"), "10");
}

#[test]
fn const_declaration() {
    assert_eq!(eval_str("const x = 99; x"), "99");
}

#[test]
fn let_block_scoping() {
    assert_eq!(eval_console(r#"
        let x = 1;
        { let x = 2; console.log(x); }
        console.log(x);
    "#), "2\n1");
}

// ═══════════════════════════════════════════════════════════
// §13.6 — Destructuring
// ═══════════════════════════════════════════════════════════

#[test]
fn array_destructuring() {
    assert_eq!(eval_str("let [a, b, c] = [1, 2, 3]; a + b + c"), "6");
}

#[test]
fn object_destructuring() {
    assert_eq!(eval_str("let {x, y} = {x: 10, y: 20}; x + y"), "30");
}

#[test]
fn destructuring_default() {
    assert_eq!(eval_str("let [a = 5, b = 10] = [1]; a + b"), "11");
}

#[test]
fn rest_destructuring() {
    assert_eq!(eval_str("let [a, ...rest] = [1, 2, 3]; rest.length"), "2");
}

// ═══════════════════════════════════════════════════════════
// §14.3 — Arrow Functions
// ═══════════════════════════════════════════════════════════

#[test]
fn arrow_function_expression() {
    assert_eq!(eval_str("const f = (a, b) => a + b; f(3, 4)"), "7");
}

#[test]
fn arrow_function_body() {
    assert_eq!(eval_str("const f = x => { return x * 2; }; f(5)"), "10");
}

// ═══════════════════════════════════════════════════════════
// §14.6 — Classes
// ═══════════════════════════════════════════════════════════

#[test]
fn class_basic() {
    assert_eq!(eval_str(r#"
        class Point {
            constructor(x, y) { this.x = x; this.y = y; }
            sum() { return this.x + this.y; }
        }
        let p = new Point(3, 4);
        p.sum()
    "#), "7");
}

#[test]
fn class_inheritance() {
    assert_eq!(eval_str(r#"
        class Animal {
            constructor(name) { this.name = name; }
            speak() { return this.name + ' makes a noise.'; }
        }
        class Dog extends Animal {
            constructor(name) { super(name); }
            speak() { return this.name + ' barks.'; }
        }
        let d = new Dog('Rex');
        d.speak()
    "#), "Rex barks.");
}

#[test]
fn class_instance_properties() {
    assert_eq!(eval_str(r#"
        class Counter {
            count = 0;
            increment() { this.count++; return this.count; }
        }
        let c = new Counter();
        c.increment();
        c.increment();
        c.count
    "#), "2");
}

#[test]
fn class_private_fields() {
    assert_eq!(eval_str(r#"
        class Secret {
            #value = 42;
            getValue() { return this.#value; }
            setValue(v) { this.#value = v; }
        }
        let s = new Secret();
        s.setValue(100);
        s.getValue()
    "#), "100");
}

#[test]
fn class_private_field_default() {
    assert_eq!(eval_str(r#"
        class Foo {
            #x = 10;
            getX() { return this.#x; }
        }
        new Foo().getX()
    "#), "10");
}

#[test]
fn class_static_method() {
    assert_eq!(eval_str(r#"
        class MathHelper {
            static add(a, b) { return a + b; }
        }
        MathHelper.add(3, 7)
    "#), "10");
}

#[test]
fn class_static_property() {
    assert_eq!(eval_str(r#"
        class Config {
            static version = '1.0';
        }
        Config.version
    "#), "1.0");
}

#[test]
fn class_private_per_instance() {
    // Private fields must be per-instance, not shared on prototype
    assert_eq!(eval_console(r#"
        class Box {
            #items = [];
            add(item) { this.#items.push(item); }
            count() { return this.#items.length; }
        }
        let a = new Box();
        let b = new Box();
        a.add('x');
        a.add('y');
        b.add('z');
        console.log(a.count());
        console.log(b.count());
    "#), "2\n1");
}

// ═══════════════════════════════════════════════════════════
// §15.3 — Generators
// ═══════════════════════════════════════════════════════════

#[test]
fn generator_basic() {
    assert_eq!(eval_console(r#"
        function* gen() {
            yield 1;
            yield 2;
            yield 3;
        }
        let g = gen();
        console.log(g.next().value);
        console.log(g.next().value);
        console.log(g.next().value);
        console.log(g.next().done);
    "#), "1\n2\n3\ntrue");
}

// ═══════════════════════════════════════════════════════════
// §25.6 — Promises
// ═══════════════════════════════════════════════════════════

#[test]
fn promise_resolve() {
    assert_eq!(eval_console(r#"
        let p = Promise.resolve(42);
        p.then(v => console.log(v));
    "#), "42");
}

#[test]
fn promise_basic() {
    assert_eq!(eval_console(r#"
        let p = new Promise((resolve) => { resolve(42); });
        p.then(v => console.log(v));
    "#), "42");
}

#[test]
fn promise_reject_catch() {
    assert_eq!(eval_console(r#"
        let p = Promise.reject('err');
        p.catch(e => console.log(e));
    "#), "err");
}

#[test]
fn promise_all() {
    assert_eq!(eval_console(r#"
        let p = Promise.all([Promise.resolve(1), Promise.resolve(2), Promise.resolve(3)]);
        p.then(arr => console.log(arr.join(',')));
    "#), "1,2,3");
}

// ═══════════════════════════════════════════════════════════
// §22.1 — Array
// ═══════════════════════════════════════════════════════════

#[test]
fn array_from() {
    assert_eq!(eval_str("Array.from([1,2,3], x => x * 2).join(',')"), "2,4,6");
}

#[test]
fn array_of() {
    assert_eq!(eval_str("Array.of(1,2,3).join(',')"), "1,2,3");
}

#[test]
fn array_is_array() {
    assert_eq!(eval_str("Array.isArray([1,2])"), "true");
    assert_eq!(eval_str("Array.isArray({})"), "false");
}

#[test]
fn array_find() {
    assert_eq!(eval_str("[1,2,3,4].find(x => x > 2)"), "3");
}

#[test]
fn array_find_index() {
    assert_eq!(eval_str("[1,2,3,4].findIndex(x => x > 2)"), "2");
}

#[test]
fn array_includes() {
    assert_eq!(eval_str("[1,2,3].includes(2)"), "true");
    assert_eq!(eval_str("[1,2,3].includes(5)"), "false");
}

#[test]
fn array_flat() {
    assert_eq!(eval_str("[1,[2,[3]]].flat().join(',')"), "1,2,3");
}

#[test]
fn array_flat_map() {
    assert_eq!(eval_str("[1,2,3].flatMap(x => [x, x*2]).join(',')"), "1,2,2,4,3,6");
}

#[test]
fn array_at() {
    assert_eq!(eval_str("[10,20,30].at(-1)"), "30");
}

#[test]
fn array_entries() {
    assert_eq!(eval_console(r#"
        let arr = [10,20,30];
        let result = [];
        for (let [i, v] of arr.entries()) {
            result.push(i + ':' + v);
        }
        console.log(result.join(','));
    "#), "0:10,1:20,2:30");
}

// ES2023
#[test]
fn array_to_reversed() {
    assert_eq!(eval_str("[1,2,3].toReversed().join(',')"), "3,2,1");
}

#[test]
fn array_to_sorted() {
    assert_eq!(eval_str("[3,1,2].toSorted().join(',')"), "1,2,3");
}

#[test]
fn array_find_last() {
    assert_eq!(eval_console(r#"
        let result = [1,2,3,4].findLast(x => x < 4);
        console.log(result);
    "#), "3");
}

// ═══════════════════════════════════════════════════════════
// §22.1 — String
// ═══════════════════════════════════════════════════════════

#[test]
fn string_includes() {
    assert_eq!(eval_str("'hello world'.includes('world')"), "true");
}

#[test]
fn string_starts_with() {
    assert_eq!(eval_str("'hello'.startsWith('hel')"), "true");
}

#[test]
fn string_ends_with() {
    assert_eq!(eval_str("'hello'.endsWith('llo')"), "true");
}

#[test]
fn string_pad_start() {
    assert_eq!(eval_str("'5'.padStart(3, '0')"), "005");
}

#[test]
fn string_pad_end() {
    assert_eq!(eval_str("'x'.padEnd(4, '.')"), "x...");
}

#[test]
fn string_repeat() {
    assert_eq!(eval_str("'ab'.repeat(3)"), "ababab");
}

#[test]
fn string_trim() {
    assert_eq!(eval_str("'  hello  '.trim()"), "hello");
}

#[test]
fn string_trim_start() {
    assert_eq!(eval_str("'  hello  '.trimStart()"), "hello  ");
}

#[test]
fn string_replace_all() {
    assert_eq!(eval_str("'aabaa'.replaceAll('a', 'x')"), "xxbxx");
}

#[test]
fn string_at() {
    assert_eq!(eval_str("'hello'.at(-1)"), "o");
}

#[test]
fn string_from_char_code() {
    assert_eq!(eval_str("String.fromCharCode(72, 101, 108, 108, 111)"), "Hello");
}

#[test]
fn string_from_code_point() {
    assert_eq!(eval_str("String.fromCodePoint(65, 66, 67)"), "ABC");
}

// ═══════════════════════════════════════════════════════════
// §20.1 — Number
// ═══════════════════════════════════════════════════════════

#[test]
fn number_is_nan() {
    assert_eq!(eval_str("Number.isNaN(NaN)"), "true");
    assert_eq!(eval_str("Number.isNaN(42)"), "false");
    assert_eq!(eval_str("Number.isNaN('NaN')"), "false"); // strict
}

#[test]
fn number_is_finite() {
    assert_eq!(eval_str("Number.isFinite(42)"), "true");
    assert_eq!(eval_str("Number.isFinite(Infinity)"), "false");
}

#[test]
fn number_is_integer() {
    assert_eq!(eval_str("Number.isInteger(42)"), "true");
    assert_eq!(eval_str("Number.isInteger(42.5)"), "false");
}

#[test]
fn number_is_safe_integer() {
    assert_eq!(eval_str("Number.isSafeInteger(42)"), "true");
    assert_eq!(eval_str("Number.isSafeInteger(Number.MAX_SAFE_INTEGER)"), "true");
}

#[test]
fn number_to_fixed() {
    assert_eq!(eval_str("(3.14159).toFixed(2)"), "3.14");
}

#[test]
fn number_epsilon() {
    assert_eq!(eval_str("Number.EPSILON < 1"), "true");
    assert_eq!(eval_str("Number.EPSILON > 0"), "true");
}

// ═══════════════════════════════════════════════════════════
// §20.3 — Math
// ═══════════════════════════════════════════════════════════

#[test]
fn math_constants() {
    assert_eq!(eval_str("Math.PI > 3.14"), "true");
    assert_eq!(eval_str("Math.E > 2.71"), "true");
}

#[test]
fn math_abs() {
    assert_eq!(eval_str("Math.abs(-42)"), "42");
}

#[test]
fn math_floor_ceil() {
    assert_eq!(eval_str("Math.floor(4.7)"), "4");
    assert_eq!(eval_str("Math.ceil(4.1)"), "5");
}

#[test]
fn math_min_max() {
    assert_eq!(eval_str("Math.min(1,2,3)"), "1");
    assert_eq!(eval_str("Math.max(1,2,3)"), "3");
}

#[test]
fn math_pow() {
    assert_eq!(eval_str("Math.pow(2, 10)"), "1024");
}

// ═══════════════════════════════════════════════════════════
// §24.5 — JSON
// ═══════════════════════════════════════════════════════════

#[test]
fn json_parse_basic() {
    assert_eq!(eval_str(r#"JSON.parse('{"a":1,"b":"hello"}').a"#), "1");
}

#[test]
fn json_stringify_basic() {
    assert_eq!(eval_str(r#"JSON.stringify({a: 1, b: 'hello'})"#), r#"{"a":1,"b":"hello"}"#);
}

#[test]
fn json_parse_array() {
    assert_eq!(eval_str("JSON.parse('[1,2,3]').length"), "3");
}

// ═══════════════════════════════════════════════════════════
// §20.2 — Object
// ═══════════════════════════════════════════════════════════

#[test]
fn object_keys() {
    assert_eq!(eval_str("Object.keys({a:1, b:2}).join(',')"), "a,b");
}

#[test]
fn object_values() {
    assert_eq!(eval_str("Object.values({a:1, b:2}).join(',')"), "1,2");
}

#[test]
fn object_entries() {
    assert_eq!(eval_str("Object.entries({a:1}).length"), "1");
}

#[test]
fn object_assign() {
    assert_eq!(eval_str("Object.assign({}, {a:1}, {b:2}).b"), "2");
}

#[test]
fn object_freeze() {
    assert_eq!(eval_str("Object.isFrozen(Object.freeze({a:1}))"), "true");
}

#[test]
fn object_create() {
    assert_eq!(eval_str(r#"
        let proto = { greet() { return 'hi'; } };
        let obj = Object.create(proto);
        obj.greet()
    "#), "hi");
}

#[test]
fn object_from_entries() {
    assert_eq!(eval_str("Object.fromEntries([['a', 1], ['b', 2]]).b"), "2");
}

#[test]
fn object_is() {
    assert_eq!(eval_str("Object.is(NaN, NaN)"), "true");
    assert_eq!(eval_str("Object.is(0, -0)"), "false");
    assert_eq!(eval_str("Object.is(42, 42)"), "true");
}

#[test]
fn object_has_own() {
    assert_eq!(eval_str("Object.hasOwn({a:1}, 'a')"), "true");
    assert_eq!(eval_str("Object.hasOwn({a:1}, 'b')"), "false");
}

#[test]
fn object_get_own_property_names() {
    assert_eq!(eval_str("Object.getOwnPropertyNames({a:1,b:2}).length"), "2");
}

#[test]
fn object_set_prototype_of() {
    assert_eq!(eval_str(r#"
        let proto = { greet() { return 'hello'; } };
        let obj = {};
        Object.setPrototypeOf(obj, proto);
        obj.greet()
    "#), "hello");
}

#[test]
fn object_define_property() {
    assert_eq!(eval_str(r#"
        let obj = {};
        Object.defineProperty(obj, 'x', { value: 42, writable: false });
        obj.x
    "#), "42");
}

// ═══════════════════════════════════════════════════════════
// §13.2 — Template Literals
// ═══════════════════════════════════════════════════════════

#[test]
fn template_literal() {
    assert_eq!(eval_str(r#"let x = 5; `value: ${x}`"#), "value: 5");
}

#[test]
fn template_literal_expr() {
    assert_eq!(eval_str(r#"`${2 + 3}`"#), "5");
}

// ═══════════════════════════════════════════════════════════
// §13.13 — Optional Chaining
// ═══════════════════════════════════════════════════════════

#[test]
fn optional_chaining() {
    assert_eq!(eval_str("let obj = {a: {b: 42}}; obj?.a?.b"), "42");
    assert_eq!(eval_str("let obj = null; obj?.a?.b"), "undefined");
}

// ═══════════════════════════════════════════════════════════
// §13.14 — Nullish Coalescing
// ═══════════════════════════════════════════════════════════

#[test]
fn nullish_coalescing() {
    assert_eq!(eval_str("null ?? 'default'"), "default");
    assert_eq!(eval_str("undefined ?? 'default'"), "default");
    assert_eq!(eval_str("0 ?? 'default'"), "0");
    assert_eq!(eval_str("'' ?? 'default'"), "");
}

// ═══════════════════════════════════════════════════════════
// §13.4 — Spread / Rest
// ═══════════════════════════════════════════════════════════

#[test]
fn spread_array() {
    assert_eq!(eval_str("[...[1,2], ...[3,4]].join(',')"), "1,2,3,4");
}

#[test]
fn spread_object() {
    assert_eq!(eval_str("let o = {...{a:1}, ...{b:2}}; o.a + o.b"), "3");
}

#[test]
fn rest_params() {
    assert_eq!(eval_str("function f(a, ...rest) { return rest.length; } f(1,2,3)"), "2");
}

// ═══════════════════════════════════════════════════════════
// §14.7 — for-of / for-in
// ═══════════════════════════════════════════════════════════

#[test]
fn for_of_array() {
    assert_eq!(eval_console(r#"
        let sum = 0;
        for (let x of [1,2,3,4]) { sum += x; }
        console.log(sum);
    "#), "10");
}

#[test]
fn for_in_object() {
    assert_eq!(eval_console(r#"
        let keys = [];
        for (let k in {a:1, b:2}) { keys.push(k); }
        console.log(keys.join(','));
    "#), "a,b");
}

// ═══════════════════════════════════════════════════════════
// §13.12 — try/catch/finally
// ═══════════════════════════════════════════════════════════

#[test]
fn try_catch() {
    assert_eq!(eval_str(r#"
        let result;
        try { throw new Error('oops'); } catch(e) { result = e.message; }
        result
    "#), "oops");
}

#[test]
fn try_finally() {
    assert_eq!(eval_console(r#"
        try {
            console.log('try');
        } finally {
            console.log('finally');
        }
    "#), "try\nfinally");
}

// ═══════════════════════════════════════════════════════════
// §19.4 — Symbol
// ═══════════════════════════════════════════════════════════

#[test]
fn symbol_basic() {
    assert_eq!(eval_str("typeof Symbol('test')"), "symbol");
}

#[test]
fn symbol_for() {
    assert_eq!(eval_str("Symbol.for('test') === Symbol.for('test')"), "true");
}

// ═══════════════════════════════════════════════════════════
// §23.1 — Map
// ═══════════════════════════════════════════════════════════

#[test]
fn map_basic() {
    assert_eq!(eval_str(r#"
        let m = new Map();
        m.set('a', 1);
        m.set('b', 2);
        m.get('b')
    "#), "2");
}

#[test]
fn map_size() {
    assert_eq!(eval_console(r#"
        let m = new Map();
        m.set('a', 1);
        m.set('b', 2);
        console.log(m.size);
    "#), "2");
}

// ═══════════════════════════════════════════════════════════
// §23.2 — Set
// ═══════════════════════════════════════════════════════════

#[test]
fn set_basic() {
    assert_eq!(eval_str(r#"
        let s = new Set([1, 2, 3, 2, 1]);
        s.size
    "#), "3");
}

// ═══════════════════════════════════════════════════════════
// §26.1 — Proxy
// ═══════════════════════════════════════════════════════════

#[test]
fn proxy_get_trap() {
    assert_eq!(eval_console(r#"
        let target = { a: 1, b: 2 };
        let handler = {
            get(obj, prop) { return prop in obj ? obj[prop] * 10 : -1; }
        };
        let p = new Proxy(target, handler);
        console.log(p.a);
    "#), "10");
}

// ═══════════════════════════════════════════════════════════
// §26.1 — Reflect
// ═══════════════════════════════════════════════════════════

#[test]
fn reflect_get() {
    assert_eq!(eval_str("Reflect.get({a: 42}, 'a')"), "42");
}

// ═══════════════════════════════════════════════════════════
// §22.2 — RegExp
// ═══════════════════════════════════════════════════════════

#[test]
fn regexp_test() {
    assert_eq!(eval_console("let re = /hello/; console.log(re.test('hello world'));"), "true");
    assert_eq!(eval_console("let re = /xyz/; console.log(re.test('hello world'));"), "false");
}

#[test]
fn regexp_exec() {
    assert_eq!(eval_console("let re = /([0-9]+)/; let m = re.exec('abc123'); console.log(m[1]);"), "123");
}

// ═══════════════════════════════════════════════════════════
// §22.3 — Date
// ═══════════════════════════════════════════════════════════

#[test]
fn date_now() {
    assert_eq!(eval_str("typeof Date.now()"), "number");
}

// ═══════════════════════════════════════════════════════════
// §24.3 — TypedArrays
// ═══════════════════════════════════════════════════════════

#[test]
fn typed_array_basic() {
    assert_eq!(eval_str(r#"
        let buf = new ArrayBuffer(8);
        let view = new Int32Array(buf);
        view[0] = 42;
        view[0]
    "#), "42");
}

// ═══════════════════════════════════════════════════════════
// §24.1 — WeakRef
// ═══════════════════════════════════════════════════════════

#[test]
fn weakref_basic() {
    assert_eq!(eval_console(r#"
        let obj = {value: 42};
        let ref1 = new WeakRef(obj);
        console.log(ref1.deref().value);
    "#), "42");
}

// ═══════════════════════════════════════════════════════════
// Misc — typeof, instanceof, in
// ═══════════════════════════════════════════════════════════

#[test]
fn typeof_operator() {
    assert_eq!(eval_str("typeof 42"), "number");
    assert_eq!(eval_str("typeof 'hello'"), "string");
    assert_eq!(eval_str("typeof true"), "boolean");
    assert_eq!(eval_str("typeof undefined"), "undefined");
    assert_eq!(eval_str("typeof null"), "object");
    assert_eq!(eval_str("typeof {}"), "object");
    assert_eq!(eval_str("typeof []"), "object");
    assert_eq!(eval_str("typeof function(){}"), "function");
}

#[test]
fn instanceof_operator() {
    assert_eq!(eval_str("[] instanceof Array"), "true");
}

#[test]
fn in_operator() {
    assert_eq!(eval_str("'a' in {a: 1}"), "true");
    assert_eq!(eval_str("'b' in {a: 1}"), "false");
}

// ═══════════════════════════════════════════════════════════
// Error types
// ═══════════════════════════════════════════════════════════

#[test]
fn error_types() {
    assert_eq!(eval_str("new TypeError('bad').name"), "TypeError");
    assert_eq!(eval_str("new RangeError('out').name"), "RangeError");
    assert_eq!(eval_str("new SyntaxError('parse').name"), "SyntaxError");
    assert_eq!(eval_str("new ReferenceError('undef').name"), "ReferenceError");
}

#[test]
fn aggregate_error() {
    assert_eq!(eval_str(r#"
        let e = new AggregateError([new Error('a'), new Error('b')], 'all failed');
        e.errors.length
    "#), "2");
}

// ═══════════════════════════════════════════════════════════
// Closures
// ═══════════════════════════════════════════════════════════

#[test]
fn closure_basic() {
    assert_eq!(eval_str(r#"
        function counter() {
            let n = 0;
            return function() { n++; return n; };
        }
        let c = counter();
        c(); c(); c()
    "#), "3");
}

#[test]
fn closure_in_loop() {
    assert_eq!(eval_console(r#"
        let fns = [];
        for (let i = 0; i < 3; i++) {
            fns.push(() => i);
        }
        console.log(fns[0]());
        console.log(fns[1]());
        console.log(fns[2]());
    "#), "0\n1\n2");
}

// ═══════════════════════════════════════════════════════════
// Switch
// ═══════════════════════════════════════════════════════════

#[test]
fn switch_basic() {
    assert_eq!(eval_str(r#"
        let x = 2;
        let result;
        switch(x) {
            case 1: result = 'one'; break;
            case 2: result = 'two'; break;
            default: result = 'other';
        }
        result
    "#), "two");
}

// ═══════════════════════════════════════════════════════════
// Computed property names
// ═══════════════════════════════════════════════════════════

#[test]
fn computed_property() {
    assert_eq!(eval_str(r#"
        let key = 'x';
        let obj = { [key]: 42 };
        obj.x
    "#), "42");
}

// ═══════════════════════════════════════════════════════════
// Exponentiation operator
// ═══════════════════════════════════════════════════════════

#[test]
fn exponentiation() {
    assert_eq!(eval_str("2 ** 10"), "1024");
}

// ═══════════════════════════════════════════════════════════
// Logical assignment operators
// ═══════════════════════════════════════════════════════════

#[test]
fn logical_and_assign() {
    assert_eq!(eval_str("let a = 1; a &&= 2; a"), "2");
}

#[test]
fn logical_or_assign() {
    assert_eq!(eval_str("let a = 0; a ||= 42; a"), "42");
}

#[test]
fn nullish_assign() {
    assert_eq!(eval_str("let a = null; a ??= 99; a"), "99");
}

// ═══════════════════════════════════════════════════════════
// do-while
// ═══════════════════════════════════════════════════════════

#[test]
fn do_while_loop() {
    assert_eq!(eval_str(r#"
        let i = 0;
        do { i++; } while (i < 5);
        i
    "#), "5");
}

// ═══════════════════════════════════════════════════════════
// Labeled break/continue
// ═══════════════════════════════════════════════════════════

#[test]
fn labeled_break() {
    // Simple labeled break test
    assert_eq!(eval_console(r#"
        let found = false;
        outer: for (let i = 0; i < 3; i++) {
            for (let j = 0; j < 3; j++) {
                if (i === 1 && j === 1) { found = true; break outer; }
            }
        }
        console.log(found);
    "#), "true");
}

// ═══════════════════════════════════════════════════════════
// §14.7/§14.8 — Async Methods (object literal + class)
// ═══════════════════════════════════════════════════════════

#[test]
fn async_method_object_literal() {
    // async method shorthand in object literal must parse without error
    assert_eq!(eval_console(r#"
        const obj = {
            async init() { return 42; },
            async play(code) { return code + 1; },
            normal() { return 10; }
        };
        console.log(typeof obj.init);
        console.log(typeof obj.play);
        console.log(obj.normal());
    "#), "function\nfunction\n10");
}

#[test]
fn async_method_class() {
    // async method in class body
    assert_eq!(eval_console(r#"
        class MyClass {
            async fetchData() { return 99; }
            sync_method() { return 1; }
        }
        var mc = new MyClass();
        console.log(typeof mc.fetchData);
        console.log(mc.sync_method());
    "#), "function\n1");
}

#[test]
fn async_as_property_name() {
    // 'async' used as a normal property name (not a method modifier)
    assert_eq!(eval_str(r#"
        const obj = { async: true };
        obj.async
    "#), "true");
}

#[test]
fn async_method_strudel_pattern() {
    // Real-world pattern from strudel.js
    assert_eq!(eval_console(r#"
        const StrudelPlayer = {
            isPlaying: false,
            async init() {
                this.isPlaying = false;
                return true;
            },
            async play(code, onLog) {
                return "playing";
            },
            stop() {
                this.isPlaying = false;
            }
        };
        console.log(StrudelPlayer.isPlaying);
        console.log(typeof StrudelPlayer.init);
        console.log(typeof StrudelPlayer.play);
        console.log(typeof StrudelPlayer.stop);
    "#), "false\nfunction\nfunction\nfunction");
}
