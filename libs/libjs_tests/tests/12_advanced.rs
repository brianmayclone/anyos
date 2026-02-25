//! Advanced patterns — closures, prototype chain, generators/iterators,
//!                     complex destructuring, memoization, currying,
//!                     functional patterns, scope edge cases.
//!
//! Level 12: Multi-concept programs; closest to real-world code.

use libjs_tests::{JsEngine, JsValueExt};

fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }
fn bool_(src: &str) -> bool { js(src).to_boolean() }
fn str_(src: &str) -> String { js(src).to_js_string() }

// ── memoization ───────────────────────────────────────────────────────────────

#[test]
fn memoize_fibonacci() {
    let mut e = JsEngine::new();
    e.set_step_limit(50_000_000);
    e.eval(r#"
        function memoize(fn) {
            var cache = {};
            return function(n) {
                if (n in cache) return cache[n];
                return (cache[n] = fn(n));
            };
        }
        var fib = memoize(function f(n) {
            if (n < 2) return n;
            return fib(n - 1) + fib(n - 2);
        });
        var result = fib(20);
    "#);
    assert_eq!(e.get_global("result").to_number(), 6765.0);
}

// ── currying ─────────────────────────────────────────────────────────────────

#[test]
fn manual_curry() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function curry(fn) {
            return function curried() {
                var args = Array.prototype.slice.call(arguments);
                if (args.length >= fn.length) {
                    return fn.apply(null, args);
                }
                return function() {
                    var newArgs = args.concat(Array.prototype.slice.call(arguments));
                    return curried.apply(null, newArgs);
                };
            };
        }
        var add = curry(function(a, b, c) { return a + b + c; });
        var result = add(1)(2)(3);
    "#);
    assert_eq!(e.get_global("result").to_number(), 6.0);
}

// ── prototype chain ───────────────────────────────────────────────────────────

#[test]
fn prototype_chain_lookup() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function Animal(name) { this.name = name; }
        Animal.prototype.speak = function() { return this.name + ' makes a noise.'; };
        function Dog(name) { Animal.call(this, name); }
        Dog.prototype = Object.create(Animal.prototype);
        Dog.prototype.constructor = Dog;
        Dog.prototype.bark = function() { return this.name + ' barks.'; };
        var d = new Dog('Rex');
        var speak = d.speak();
        var bark  = d.bark();
    "#);
    assert_eq!(e.get_global("speak").to_js_string(), "Rex makes a noise.");
    assert_eq!(e.get_global("bark").to_js_string(), "Rex barks.");
}

// ── closure loops (classic pitfall & fix) ────────────────────────────────────

#[test]
fn closure_loop_with_let() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var fns = [];
        for (let i = 0; i < 5; i++) {
            fns.push(function() { return i; });
        }
        // With let, each iteration has its own binding
        var r0 = fns[0]();
        var r4 = fns[4]();
    "#);
    assert_eq!(e.get_global("r0").to_number(), 0.0);
    assert_eq!(e.get_global("r4").to_number(), 4.0);
}

#[test]
fn closure_loop_iife_fix() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var fns = [];
        for (var i = 0; i < 5; i++) {
            fns.push((function(j) { return function() { return j; }; })(i));
        }
        var r0 = fns[0]();
        var r3 = fns[3]();
    "#);
    assert_eq!(e.get_global("r0").to_number(), 0.0);
    assert_eq!(e.get_global("r3").to_number(), 3.0);
}

// ── complex destructuring ────────────────────────────────────────────────────

#[test]
fn nested_array_destructuring() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var [[a, b], [c, d]] = [[1, 2], [3, 4]];
    "#);
    assert_eq!(e.get_global("a").to_number(), 1.0);
    assert_eq!(e.get_global("d").to_number(), 4.0);
}

#[test]
fn mixed_array_object_destructuring() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var [{ x }, { y }] = [{ x: 10 }, { y: 20 }];
    "#);
    assert_eq!(e.get_global("x").to_number(), 10.0);
    assert_eq!(e.get_global("y").to_number(), 20.0);
}

#[test]
fn rest_in_destructuring() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var [first, ...rest] = [1, 2, 3, 4, 5];
    "#);
    assert_eq!(e.get_global("first").to_number(), 1.0);
    assert_eq!(e.get_global("rest").get_property("length").to_number(), 4.0);
    assert_eq!(e.get_global("rest").get_index(0).to_number(), 2.0);
}

#[test]
fn object_rest_spread() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var { a, ...rest } = { a: 1, b: 2, c: 3 };
    "#);
    assert_eq!(e.get_global("a").to_number(), 1.0);
    assert_eq!(e.get_global("rest").get_property("b").to_number(), 2.0);
    assert_eq!(e.get_global("rest").get_property("c").to_number(), 3.0);
}

// ── functional pipeline ───────────────────────────────────────────────────────

#[test]
fn functional_pipeline() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var pipe = (...fns) => x => fns.reduce((v, f) => f(v), x);
        var process = pipe(
            x => x * 2,
            x => x + 3,
            x => x * x
        );
        var result = process(4); // (4*2+3)^2 = 11^2 = 121
    "#);
    assert_eq!(e.get_global("result").to_number(), 121.0);
}

// ── generator-style with closures ────────────────────────────────────────────

#[test]
fn range_generator_closure() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function range(start, end) {
            var current = start;
            return {
                next: function() {
                    if (current < end) {
                        return { value: current++, done: false };
                    }
                    return { value: undefined, done: true };
                }
            };
        }
        var gen = range(0, 5);
        var vals = [];
        var step;
        while(!(step = gen.next()).done) {
            vals.push(step.value);
        }
    "#);
    let vals = e.get_global("vals");
    assert_eq!(vals.get_property("length").to_number(), 5.0);
    assert_eq!(vals.get_index(0).to_number(), 0.0);
    assert_eq!(vals.get_index(4).to_number(), 4.0);
}

// ── observer pattern ─────────────────────────────────────────────────────────

#[test]
fn simple_event_emitter() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function EventEmitter() {
            this.handlers = {};
        }
        EventEmitter.prototype.on = function(event, fn) {
            if (!this.handlers[event]) this.handlers[event] = [];
            this.handlers[event].push(fn);
        };
        EventEmitter.prototype.emit = function(event) {
            var args = Array.prototype.slice.call(arguments, 1);
            var list = this.handlers[event] || [];
            for (var i = 0; i < list.length; i++) {
                list[i].apply(null, args);
            }
        };
        var emitter = new EventEmitter();
        var results = [];
        emitter.on('data', function(v) { results.push(v * 2); });
        emitter.on('data', function(v) { results.push(v + 10); });
        emitter.emit('data', 5);
    "#);
    let results = e.get_global("results");
    assert_eq!(results.get_index(0).to_number(), 10.0); // 5*2
    assert_eq!(results.get_index(1).to_number(), 15.0); // 5+10
}

// ── tagged template literals ──────────────────────────────────────────────────

// (basic — tagged templates call a function with strings and values)
#[test]
fn template_literal_as_string() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var a = 3;
        var b = 4;
        var result = `${a} + ${b} = ${a + b}`;
    "#);
    assert_eq!(e.get_global("result").to_js_string(), "3 + 4 = 7");
}

// ── nullish coalescing assignment ─────────────────────────────────────────────

#[test]
fn nullish_coalescing_in_chain() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var config = { timeout: null, retries: undefined, maxSize: 0 };
        var timeout  = config.timeout  ?? 5000;
        var retries  = config.retries  ?? 3;
        var maxSize  = config.maxSize  ?? 100; // 0 is NOT null/undefined
    "#);
    assert_eq!(e.get_global("timeout").to_number(), 5000.0);
    assert_eq!(e.get_global("retries").to_number(), 3.0);
    assert_eq!(e.get_global("maxSize").to_number(), 0.0); // !! stays 0
}

// ── sorting complex objects ───────────────────────────────────────────────────

#[test]
fn sort_objects_by_field() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var people = [
            { name: 'Charlie', age: 30 },
            { name: 'Alice',   age: 25 },
            { name: 'Bob',     age: 35 }
        ];
        people.sort(function(a, b) { return a.age - b.age; });
        var first = people[0].name;
        var last  = people[2].name;
    "#);
    assert_eq!(e.get_global("first").to_js_string(), "Alice");
    assert_eq!(e.get_global("last").to_js_string(), "Bob");
}

// ── matrix operations ─────────────────────────────────────────────────────────

#[test]
fn matrix_multiplication() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function matMul(A, B) {
            var n = A.length;
            var m = B[0].length;
            var p = B.length;
            var C = [];
            for (var i = 0; i < n; i++) {
                C.push([]);
                for (var j = 0; j < m; j++) {
                    var sum = 0;
                    for (var k = 0; k < p; k++) {
                        sum += A[i][k] * B[k][j];
                    }
                    C[i].push(sum);
                }
            }
            return C;
        }
        var A = [[1,2],[3,4]];
        var B = [[5,6],[7,8]];
        var C = matMul(A, B);
        // C[0][0] = 1*5+2*7 = 19
        // C[0][1] = 1*6+2*8 = 22
        // C[1][0] = 3*5+4*7 = 43
        // C[1][1] = 3*6+4*8 = 50
        var c00 = C[0][0];
        var c11 = C[1][1];
    "#);
    assert_eq!(e.get_global("c00").to_number(), 19.0);
    assert_eq!(e.get_global("c11").to_number(), 50.0);
}

// ── sieve of eratosthenes ─────────────────────────────────────────────────────

#[test]
fn sieve_of_eratosthenes() {
    let mut e = JsEngine::new();
    e.eval(r#"
        function sieve(limit) {
            var flags = new Array(limit + 1).fill(true);
            flags[0] = flags[1] = false;
            for (var i = 2; i * i <= limit; i++) {
                if (flags[i]) {
                    for (var j = i * i; j <= limit; j += i) {
                        flags[j] = false;
                    }
                }
            }
            var primes = [];
            for (var k = 2; k <= limit; k++) {
                if (flags[k]) primes.push(k);
            }
            return primes;
        }
        var primes = sieve(50);
        var count = primes.length;
        var last  = primes[primes.length - 1];
    "#);
    assert_eq!(e.get_global("count").to_number(), 15.0); // primes up to 50
    assert_eq!(e.get_global("last").to_number(), 47.0);
}

// ── linked list ───────────────────────────────────────────────────────────────

#[test]
fn linked_list_operations() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class Node {
            constructor(val) { this.val = val; this.next = null; }
        }
        class LinkedList {
            constructor() { this.head = null; this.length = 0; }
            push(val) {
                var node = new Node(val);
                if (!this.head) { this.head = node; }
                else {
                    var cur = this.head;
                    while (cur.next) cur = cur.next;
                    cur.next = node;
                }
                this.length++;
            }
            toArray() {
                var arr = [];
                var cur = this.head;
                while (cur) { arr.push(cur.val); cur = cur.next; }
                return arr;
            }
        }
        var list = new LinkedList();
        list.push(1); list.push(2); list.push(3);
        var arr = list.toArray();
        var len = list.length;
    "#);
    assert_eq!(e.get_global("len").to_number(), 3.0);
    let arr = e.get_global("arr");
    assert_eq!(arr.get_index(0).to_number(), 1.0);
    assert_eq!(arr.get_index(2).to_number(), 3.0);
}

// ── deep clone ────────────────────────────────────────────────────────────────

#[test]
fn deep_clone_via_json() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var original = { a: { b: { c: 42 } }, list: [1, 2, 3] };
        var clone = JSON.parse(JSON.stringify(original));
        clone.a.b.c = 99;
        clone.list.push(4);
        var orig_c    = original.a.b.c;
        var orig_len  = original.list.length;
        var clone_c   = clone.a.b.c;
    "#);
    assert_eq!(e.get_global("orig_c").to_number(), 42.0);   // untouched
    assert_eq!(e.get_global("orig_len").to_number(), 3.0);  // untouched
    assert_eq!(e.get_global("clone_c").to_number(), 99.0);  // changed
}

// ── reduce to group-by ────────────────────────────────────────────────────────

#[test]
fn group_by_with_reduce() {
    let mut e = JsEngine::new();
    e.eval(r#"
        var items = [
            { cat: 'a', val: 1 },
            { cat: 'b', val: 2 },
            { cat: 'a', val: 3 },
            { cat: 'b', val: 4 },
            { cat: 'c', val: 5 }
        ];
        var groups = items.reduce(function(acc, item) {
            if (!acc[item.cat]) acc[item.cat] = [];
            acc[item.cat].push(item.val);
            return acc;
        }, {});
        var a_sum = groups['a'].reduce((s,v) => s+v, 0);
        var b_count = groups['b'].length;
    "#);
    assert_eq!(e.get_global("a_sum").to_number(), 4.0);   // 1+3
    assert_eq!(e.get_global("b_count").to_number(), 2.0); // [2,4]
}
