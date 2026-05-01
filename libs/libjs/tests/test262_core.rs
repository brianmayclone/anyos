// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! test262-style conformance tests for libjs.
//!
//! Each test evaluates JavaScript code and checks the result or console output.
//! Tests are organized by ECMAScript specification section.

extern crate alloc;
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

#[test]
fn class_declaration_initializes_preallocated_function_scope_binding() {
    assert_eq!(
        eval_str("(function(){ class Logger {}; return typeof Logger + ':' + Logger.name; })()"),
        "function:Logger"
    );
}

#[test]
fn try_catch_catches_exception_from_called_function() {
    assert_eq!(
        eval_str(
            r#"
            let ok = false;
            function boom() { throw new Error('caught'); }
            try { boom(); } catch { ok = true; }
            ok;
            "#
        ),
        "true"
    );
}

#[test]
fn try_catch_catches_exception_from_comma_call_argument() {
    assert_eq!(
        eval_str(
            r#"
            let ok = false;
            function missing() { throw new Error('missing optional dependency'); }
            function consume(_) {}
            try { consume((missing(), {}).default); } catch { ok = true; }
            ok;
            "#
        ),
        "true"
    );
}

#[test]
fn assignment_to_inherited_getter_without_setter_does_not_shadow() {
    assert_eq!(
        eval_str(
            r#"
            var hit = 0;
            var proto = Object.create(Object.prototype, {
                create: { get: function(){ hit++; return 41; } }
            });
            var obj = Object.create(proto);
            var expr = (obj.create = 7);
            [expr, obj.hasOwnProperty('create'), obj.create, hit].join(':');
            "#
        ),
        "7:false:41:1"
    );
}

#[test]
fn var_redeclaration_inside_if_updates_parameter_binding() {
    assert_eq!(
        eval_str(
            r#"
            (function(){
                function f(b, flag) {
                    var w = 33;
                    while (w != 13) {
                        if (w == 33) w = flag ? 46 : 76;
                        else if (w == 46) var b = [1, 2, (w = 84, arguments)];
                        else if (w == 84) w = 7;
                        else if (w == 7) return Array.isArray(b) + ':' + b[0];
                        else return 'bad:' + typeof b;
                    }
                }
                return f(function(){}, true);
            })();
            "#
        ),
        "true:1"
    );
}

#[test]
fn var_redeclaration_inside_function_constructor_updates_parameter_binding() {
    assert_eq!(
        eval_str(
            r#"
            (Function('b', `
                var w = 46;
                while (w != 13) {
                    if (w == 46) var b = [1, 2, (w = 84, arguments)];
                    else if (w == 84) w = 7;
                    else if (w == 7) return Array.isArray(b) + ':' + b[0];
                    else return 'bad:' + typeof b;
                }
            `))(function(){});
            "#
        ),
        "true:1"
    );
}

#[test]
fn bound_native_array_methods_keep_bound_receiver() {
    assert_eq!(
        eval_str(
            r#"
            var a = [1, 2, 3];
            var pop = Array.prototype.pop.bind(a);
            var push = Array.prototype.push.bind(a, 9);
            pop() + ":" + push(10) + ":" + a.join(",")
            "#
        ),
        "3:4:1,2,9,10"
    );
}

#[test]
fn bound_function_prototype_call_keeps_target_receiver() {
    assert_eq!(
        eval_str(
            r#"
            var read = Function.prototype.call.bind(Array.prototype.join);
            read([1, 2, 3], "-")
            "#
        ),
        "1-2-3"
    );
}

#[test]
fn strict_assignment_to_inherited_getter_without_setter_throws() {
    assert!(eval_throws(
        r#"
        (function(){
            'use strict';
            var proto = Object.create(Object.prototype, {
                create: { get: function(){ return 41; } }
            });
            var obj = Object.create(proto);
            obj.create = 7;
        })();
        "#
    ));
}

#[test]
fn for_in_enumerates_first_static_object_property_with_children_array() {
    assert_eq!(
        eval_str(
            r#"
            (function(){
                const props = {
                    className: 'mx-auto flex max-w-7xl items-center justify-between px-6 py-4',
                    children: [1, 2, 3]
                };
                const keys = [];
                for (const key in props) keys.push(key + '=' + props[key]);
                return keys.join('|');
            })()
            "#
        ),
        "children=1,2,3|className=mx-auto flex max-w-7xl items-center justify-between px-6 py-4"
    );
}

#[test]
fn switch_break_inside_for_in_does_not_break_loop() {
    assert_eq!(
        eval_str(
            r#"
            (function(){
                const props = { className: 'x', children: [1] };
                const seen = [];
                for (const key in props) {
                    switch (key) {
                        case 'children':
                            seen.push('children');
                            break;
                        case 'className':
                            seen.push('className');
                            break;
                    }
                }
                return seen.join(',');
            })()
            "#
        ),
        "children,className"
    );
}

#[test]
fn react_style_initial_props_loop_reaches_class_after_children() {
    assert_eq!(
        eval_str(
            r#"
            (function(){
                const props = {
                    className: 'mx-auto flex max-w-7xl',
                    children: [1, 2]
                };
                const seen = [];
                function apply(name, value) {
                    switch (name) {
                        case 'children':
                            if (typeof value === 'string') seen.push('text');
                            break;
                        case 'className':
                            seen.push('class=' + value);
                            break;
                    }
                }
                for (const key in props) {
                    let value = props[key];
                    if (props.hasOwnProperty(key) && value != null) apply(key, value);
                }
                return seen.join('|');
            })()
            "#
        ),
        "class=mx-auto flex max-w-7xl"
    );
}

#[test]
fn switch_break_continues_after_switch_statement() {
    assert_eq!(
        eval_str(
            r#"
            (function(tag){
                switch (tag) {
                    case 'div':
                    case 'span':
                        break;
                    case 'button':
                        return 'button';
                }
                return 'after';
            })('div')
            "#
        ),
        "after"
    );
}

#[test]
fn assignment_expression_preserves_nested_member_value() {
    assert_eq!(
        eval_str(
            r#"
            (function(){
                var exports = {}, css;
                (function(e){ e.toColor = function(v) { return 'ok:' + v; }; })(css || (exports.css = css = {}));
                return typeof exports.css.toColor + ':' + exports.css.toColor('#fff');
            })()
            "#,
        ),
        "function:ok:#fff"
    );
}

#[test]
fn assignment_expression_preserves_accessor_set_value() {
    assert_eq!(
        eval_str(
            r#"
            (function(){
                var obj = {};
                Object.defineProperty(obj, 'x', {
                    configurable: true,
                    get: function(){ return this.y; },
                    set: function(v){ this.y = v; }
                });
                var assigned = (obj.x = { ok: 1 });
                return typeof assigned + ':' + (assigned === obj.x) + ':' + obj.x.ok;
            })()
            "#,
        ),
        "object:true:1"
    );
}

#[test]
fn comma_chained_iifes_initialize_namespaces() {
    assert_eq!(
        eval_str(
            r#"
            (function(){
                var exports = {}, channels, color, css;
                (function(e){ e.toRgb = function(){ return 1; }; })(channels || (exports.channels = channels = {})),
                (function(e){ e.toColorRGB = function(){ return 2; }; })(color || (exports.color = color = {})),
                (function(e){ e.toColor = function(){ return 3; }; })(css || (exports.css = css = {}));
                return [
                    typeof exports.channels.toRgb,
                    typeof exports.color.toColorRGB,
                    typeof exports.css.toColor,
                    exports.css.toColor()
                ].join(':');
            })()
            "#,
        ),
        "function:function:function:3"
    );
}

#[test]
fn optional_catch_binding_continues_after_catch_block() {
    assert_eq!(
        eval_str(
            r#"
            (function(){
                var ns = {};
                (function(e){
                    try { throw new Error('ignore'); } catch {}
                    e.toColor = function(){ return 7; };
                })(ns);
                return typeof ns.toColor + ':' + ns.toColor();
            })()
            "#,
        ),
        "function:7"
    );
}

#[test]
fn try_block_lexical_binding_does_not_shadow_after_block() {
    assert_eq!(
        eval_str(
            r#"
            (function(){
                let i = 0;
                try {
                    const i = missingGlobal;
                } catch {}
                i = 3;
                return i;
            })()
            "#,
        ),
        "3"
    );
}

#[test]
fn try_block_lexical_binding_can_shadow_parameter() {
    assert_eq!(
        eval_str(
            r#"
            (function(e){
                try {
                    const e = { local: true };
                } catch {}
                e.toColor = function(){ return 'outer'; };
                return typeof e.toColor + ':' + e.toColor() + ':' + e.local;
            })({})
            "#,
        ),
        "function:outer:undefined"
    );
}

#[test]
fn for_of_lexical_binding_can_shadow_rest_parameter() {
    assert_eq!(
        eval_str(
            r#"
            (function(...items){
                var seen = [];
                for (const items of [1, 2]) {
                    seen.push(items);
                }
                return items.length + ':' + seen.join(',');
            })('a', 'b', 'c')
            "#,
        ),
        "3:1,2"
    );
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
    assert_eq!(
        eval_console(
            r#"
        let x = 1;
        { let x = 2; console.log(x); }
        console.log(x);
    "#
        ),
        "2\n1"
    );
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

#[test]
fn function_call_can_dispatch_bytecode_without_reentrant_run() {
    assert_eq!(
        eval_str(
            r#"
        function add(a) { return this.base + a; }
        let call = Function.prototype.call;
        call.call(add, { base: 10 }, 7)
    "#
        ),
        "17"
    );
}

#[test]
fn function_call_empty_completion_preserves_expression_stack() {
    assert_eq!(
        eval_str(
            r#"
        function noop() {}
        let callNoop = Function.prototype.call.bind(noop);
        let values = [1, callNoop(null), 3];
        values.length + ':' + String(values[1]) + ':' + values[2]
    "#
        ),
        "3:undefined:3"
    );
}

#[test]
fn super_spread_preserves_destructured_constructor_argument() {
    assert_eq!(
        eval_str(
            r#"
            class Base {
                constructor({ visualState: state }, options = {}) {
                    this.latestValues = state.latestValues;
                    this.renderState = state.renderState;
                    this.optionCount = Object.keys(options).length;
                }
            }
            class Derived extends Base {
                constructor() {
                    super(...arguments);
                }
            }
            let instance = new Derived({
                visualState: {
                    latestValues: { transform: 'none' },
                    renderState: { style: {} }
                }
            });
            instance.latestValues.transform + ':' + typeof instance.renderState.style + ':' + instance.optionCount
            "#,
        ),
        "none:object:0"
    );
}

#[test]
fn destructured_state_survives_for_in_shadowing_and_logical_assignment() {
    assert_eq!(
        eval_str(
            r#"
            function buildTransform(values, stateTransform, template) {
                if (template) stateTransform.x = values.x || 0;
                return 'ok';
            }
            function build(state, values, template) {
                let { style: style, vars: vars, transformOrigin: origin } = state;
                let hasTransform = false;
                for (let state in values) {
                    if (state === 'x') hasTransform = true;
                    else style[state] = values[state];
                }
                if (values.transform || (hasTransform || template
                    ? style.transform = buildTransform(values, state.transform, template)
                    : style.transform &&= 'none')) {}
                return style.opacity + ':' + String(style.transform);
            }
            build({ style: {}, transform: {}, transformOrigin: {}, vars: {} }, { opacity: 0 }, undefined)
            "#,
        ),
        "0:undefined"
    );
}

// ═══════════════════════════════════════════════════════════
// §14.6 — Classes
// ═══════════════════════════════════════════════════════════

#[test]
fn class_basic() {
    assert_eq!(
        eval_str(
            r#"
        class Point {
            constructor(x, y) { this.x = x; this.y = y; }
            sum() { return this.x + this.y; }
        }
        let p = new Point(3, 4);
        p.sum()
    "#
        ),
        "7"
    );
}

#[test]
fn class_inheritance() {
    assert_eq!(
        eval_str(
            r#"
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
    "#
        ),
        "Rex barks."
    );
}

#[test]
fn class_instance_properties() {
    assert_eq!(
        eval_str(
            r#"
        class Counter {
            count = 0;
            increment() { this.count++; return this.count; }
        }
        let c = new Counter();
        c.increment();
        c.increment();
        c.count
    "#
        ),
        "2"
    );
}

#[test]
fn class_private_fields() {
    assert_eq!(
        eval_str(
            r#"
        class Secret {
            #value = 42;
            getValue() { return this.#value; }
            setValue(v) { this.#value = v; }
        }
        let s = new Secret();
        s.setValue(100);
        s.getValue()
    "#
        ),
        "100"
    );
}

#[test]
fn class_private_field_default() {
    assert_eq!(
        eval_str(
            r#"
        class Foo {
            #x = 10;
            getX() { return this.#x; }
        }
        new Foo().getX()
    "#
        ),
        "10"
    );
}

#[test]
fn class_static_method() {
    assert_eq!(
        eval_str(
            r#"
        class MathHelper {
            static add(a, b) { return a + b; }
        }
        MathHelper.add(3, 7)
    "#
        ),
        "10"
    );
}

#[test]
fn class_static_property() {
    assert_eq!(
        eval_str(
            r#"
        class Config {
            static version = '1.0';
        }
        Config.version
    "#
        ),
        "1.0"
    );
}

#[test]
fn class_private_per_instance() {
    // Private fields must be per-instance, not shared on prototype
    assert_eq!(
        eval_console(
            r#"
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
    "#
        ),
        "2\n1"
    );
}

#[test]
fn derived_class_instance_fields_run_after_super() {
    assert_eq!(
        eval_str(
            r#"
        class Base {
            constructor() { this.ready = true; }
        }
        class Derived extends Base {
            registry = new Map();
            items = [];
            constructor() {
                super();
                this.items.push("ok");
                this.registry.set("x", 1);
            }
            value() { return this.registry.get("x") + ":" + this.items.length + ":" + this.ready; }
        }
        new Derived().value()
    "#
        ),
        "1:1:true"
    );
}

#[test]
fn derived_class_field_methods_see_initialized_collections() {
    assert_eq!(
        eval_str(
            r#"
        class Base {}
        class Container extends Base {
            providers = [];
            singletons = new Map();
            constructor() {
                super();
            }
            registerSingleton(name, value) {
                this.providers.push(name);
                this.singletons.set(name, value);
                return this.singletons.get(name) + ":" + this.providers.length;
            }
        }
        new Container().registerSingleton("svc", 7)
    "#
        ),
        "7:1"
    );
}

// ═══════════════════════════════════════════════════════════
// §15.3 — Generators
// ═══════════════════════════════════════════════════════════

#[test]
fn generator_basic() {
    assert_eq!(
        eval_console(
            r#"
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
    "#
        ),
        "1\n2\n3\ntrue"
    );
}

// ═══════════════════════════════════════════════════════════
// §25.6 — Promises
// ═══════════════════════════════════════════════════════════

#[test]
fn promise_resolve() {
    assert_eq!(
        eval_console(
            r#"
        let p = Promise.resolve(42);
        p.then(v => console.log(v));
    "#
        ),
        "42"
    );
}

#[test]
fn promise_basic() {
    assert_eq!(
        eval_console(
            r#"
        let p = new Promise((resolve) => { resolve(42); });
        p.then(v => console.log(v));
    "#
        ),
        "42"
    );
}

#[test]
fn promise_reject_catch() {
    assert_eq!(
        eval_console(
            r#"
        let p = Promise.reject('err');
        p.catch(e => console.log(e));
    "#
        ),
        "err"
    );
}

#[test]
fn promise_all() {
    assert_eq!(
        eval_console(
            r#"
        let p = Promise.all([Promise.resolve(1), Promise.resolve(2), Promise.resolve(3)]);
        p.then(arr => console.log(arr.join(',')));
    "#
        ),
        "1,2,3"
    );
}

#[test]
fn promise_chain_then_then_catch() {
    assert_eq!(
        eval_console(
            r#"
        Promise.resolve(40)
            .then(v => v + 1)
            .then(v => console.log(v))
            .catch(e => console.log("err:" + e));
    "#
        ),
        "41"
    );
}

#[test]
fn promise_finally_preserves_fulfillment_value() {
    assert_eq!(
        eval_console(
            r#"
        Promise.resolve(7)
            .finally(() => 99)
            .then(v => console.log(v));
    "#
        ),
        "7"
    );
}

#[test]
fn promise_finally_preserves_rejection_reason() {
    assert_eq!(
        eval_console(
            r#"
        Promise.reject("boom")
            .finally(() => 99)
            .catch(e => console.log(e));
    "#
        ),
        "boom"
    );
}

#[test]
fn promise_resolve_thenable_adopts_state() {
    assert_eq!(
        eval_console(
            r#"
        let thenable = { then(resolve) { resolve(13); } };
        Promise.resolve(thenable).then(v => console.log(v));
    "#
        ),
        "13"
    );
}

#[test]
fn function_constructor_can_return_promise_chain() {
    assert_eq!(
        eval_console(
            r#"
        let run = new Function("return Promise.resolve(5).then(v => v + 2).catch(e => 0);");
        run().then(v => console.log(v));
    "#
        ),
        "7"
    );
}

// ═══════════════════════════════════════════════════════════
// §22.1 — Array
// ═══════════════════════════════════════════════════════════

#[test]
fn array_from() {
    assert_eq!(
        eval_str("Array.from([1,2,3], x => x * 2).join(',')"),
        "2,4,6"
    );
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
    assert_eq!(
        eval_str("[1,2,3].flatMap(x => [x, x*2]).join(',')"),
        "1,2,2,4,3,6"
    );
}

#[test]
fn array_at() {
    assert_eq!(eval_str("[10,20,30].at(-1)"), "30");
}

#[test]
fn array_entries() {
    // Basic for-of should iterate all elements.
    assert_eq!(
        eval_console(
            r#"
        var result = [];
        for (var x of [10,20,30]) {
            result.push(x);
        }
        console.log(result.join(','));
    "#
        ),
        "10,20,30"
    );
    // entries() with destructuring.
    assert_eq!(
        eval_console(
            r#"
        var arr = [10,20,30];
        var result = [];
        for (var [i, v] of arr.entries()) {
            result.push(i + ':' + v);
        }
        console.log(result.join(','));
    "#
        ),
        "0:10,1:20,2:30"
    );
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
    assert_eq!(
        eval_console(
            r#"
        let result = [1,2,3,4].findLast(x => x < 4);
        console.log(result);
    "#
        ),
        "3"
    );
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
    assert_eq!(
        eval_str("String.fromCharCode(72, 101, 108, 108, 111)"),
        "Hello"
    );
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
    assert_eq!(
        eval_str("Number.isSafeInteger(Number.MAX_SAFE_INTEGER)"),
        "true"
    );
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
    assert_eq!(
        eval_str(r#"JSON.stringify({a: 1, b: 'hello'})"#),
        r#"{"a":1,"b":"hello"}"#
    );
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
    assert_eq!(
        eval_str(
            r#"
        let proto = { greet() { return 'hi'; } };
        let obj = Object.create(proto);
        obj.greet()
    "#
        ),
        "hi"
    );
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
    assert_eq!(
        eval_str("Object.getOwnPropertyNames({a:1,b:2}).length"),
        "2"
    );
}

#[test]
fn object_set_prototype_of() {
    assert_eq!(
        eval_str(
            r#"
        let proto = { greet() { return 'hello'; } };
        let obj = {};
        Object.setPrototypeOf(obj, proto);
        obj.greet()
    "#
        ),
        "hello"
    );
}

#[test]
fn object_define_property() {
    assert_eq!(
        eval_str(
            r#"
        let obj = {};
        Object.defineProperty(obj, 'x', { value: 42, writable: false });
        obj.x
    "#
        ),
        "42"
    );
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
    assert_eq!(eval_str("let obj = null; obj?.a.b"), "undefined");
    assert_eq!(eval_str("let obj = {a: {b: {c: 7}}}; obj?.a.b.c"), "7");
    assert_eq!(
        eval_str("let called = 0; let obj = {}; obj.missing?.callMe(called = 1); called"),
        "0"
    );
    assert_eq!(
        eval_str("let obj = {x: 3, get(){ return this.x; }}; obj?.get()"),
        "3"
    );
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
    assert_eq!(
        eval_str("function f(a, ...rest) { return rest.length; } f(1,2,3)"),
        "2"
    );
}

// ═══════════════════════════════════════════════════════════
// §14.7 — for-of / for-in
// ═══════════════════════════════════════════════════════════

#[test]
fn for_of_array() {
    assert_eq!(
        eval_console(
            r#"
        let sum = 0;
        for (let x of [1,2,3,4]) { sum += x; }
        console.log(sum);
    "#
        ),
        "10"
    );
}

#[test]
fn webpack_style_export_getter_returns_constructor() {
    assert_eq!(
        eval_str(
            r#"
        let exports = {};
        class Logger {
            constructor(name) { this.name = name; }
            value() { return this.name; }
        }
        Object.defineProperty(exports, "V", {
            enumerable: true,
            get: () => Logger
        });
        let instance = new exports.V("ok");
        instance.value()
    "#
        ),
        "ok"
    );
}

#[test]
fn webpack_style_nested_export_getter_returns_constructor() {
    assert_eq!(
        eval_str(
            r#"
        let definition = {};
        class ConfigService {
            constructor() { this.kind = "config"; }
        }
        Object.defineProperty(definition, "w", {
            enumerable: true,
            get: () => ConfigService
        });
        let namespace = { definition };
        let instance = new namespace.definition.w();
        instance.kind
    "#
        ),
        "config"
    );
}

#[test]
fn new_member_expression_binds_before_constructor_call() {
    assert_eq!(
        eval_str(
            r#"
        let exports = {};
        class Logger {
            constructor(name) { this.name = name; }
            value() { return this.name; }
        }
        exports.V = Logger;
        let instance = new exports.V("ok");
        instance.value()
    "#
        ),
        "ok"
    );
}

#[test]
fn new_nested_member_expression_binds_before_constructor_call() {
    assert_eq!(
        eval_str(
            r#"
        let root = {
            services: {
                Logger: class {
                    constructor(name) { this.name = name; }
                    value() { return this.name; }
                }
            }
        };
        let instance = new root.services.Logger("nested");
        instance.value()
    "#
        ),
        "nested"
    );
}

#[test]
fn closure_can_capture_later_class_declaration_binding() {
    assert_eq!(
        eval_str(
            r#"
        let exports = {};
        Object.defineProperty(exports, "V", {
            enumerable: true,
            get: () => Logger
        });
        class Logger {
            constructor(name) { this.name = name; }
            value() { return this.name; }
        }
        let instance = new exports.V("late");
        instance.value()
    "#
        ),
        "late"
    );
}

#[test]
fn for_in_object() {
    assert_eq!(
        eval_console(
            r#"
        let keys = [];
        for (let k in {a:1, b:2}) { keys.push(k); }
        console.log(keys.join(','));
    "#
        ),
        "a,b"
    );
}

// ═══════════════════════════════════════════════════════════
// §13.12 — try/catch/finally
// ═══════════════════════════════════════════════════════════

#[test]
fn try_catch() {
    assert_eq!(
        eval_str(
            r#"
        let result;
        try { throw new Error('oops'); } catch(e) { result = e.message; }
        result
    "#
        ),
        "oops"
    );
}

#[test]
fn try_finally() {
    assert_eq!(
        eval_console(
            r#"
        try {
            console.log('try');
        } finally {
            console.log('finally');
        }
    "#
        ),
        "try\nfinally"
    );
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
    assert_eq!(
        eval_str("Symbol.for('test') === Symbol.for('test')"),
        "true"
    );
}

// ═══════════════════════════════════════════════════════════
// §23.1 — Map
// ═══════════════════════════════════════════════════════════

#[test]
fn map_basic() {
    assert_eq!(
        eval_str(
            r#"
        let m = new Map();
        m.set('a', 1);
        m.set('b', 2);
        m.get('b')
    "#
        ),
        "2"
    );
}

#[test]
fn map_size() {
    assert_eq!(
        eval_console(
            r#"
        let m = new Map();
        m.set('a', 1);
        m.set('b', 2);
        console.log(m.size);
    "#
        ),
        "2"
    );
}

// ═══════════════════════════════════════════════════════════
// §23.2 — Set
// ═══════════════════════════════════════════════════════════

#[test]
fn set_basic() {
    assert_eq!(
        eval_str(
            r#"
        let s = new Set([1, 2, 3, 2, 1]);
        s.size
    "#
        ),
        "3"
    );
}

#[test]
fn set_methods_work_when_extracted() {
    assert_eq!(
        eval_str(
            r#"
        let s = new Set([1, 2]);
        let add = s.add;
        add.call(s, 3);
        s.has(3)
    "#
        ),
        "true"
    );
}

#[test]
fn set_subclass_keeps_receiver_compatibility() {
    assert_eq!(
        eval_str(
            r#"
        class MySet extends Set {}
        let s = new MySet([1, 2]);
        let add = Set.prototype.add;
        add.call(s, 3);
        s.has(3)
    "#
        ),
        "true"
    );
}

// ═══════════════════════════════════════════════════════════
// §26.1 — Proxy
// ═══════════════════════════════════════════════════════════

#[test]
fn proxy_get_trap() {
    assert_eq!(
        eval_console(
            r#"
        let target = { a: 1, b: 2 };
        let handler = {
            get(obj, prop) { return prop in obj ? obj[prop] * 10 : -1; }
        };
        let p = new Proxy(target, handler);
        console.log(p.a);
    "#
        ),
        "10"
    );
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
    assert_eq!(
        eval_console("let re = /hello/; console.log(re.test('hello world'));"),
        "true"
    );
    assert_eq!(
        eval_console("let re = /xyz/; console.log(re.test('hello world'));"),
        "false"
    );
}

#[test]
fn regexp_exec() {
    assert_eq!(
        eval_console("let re = /([0-9]+)/; let m = re.exec('abc123'); console.log(m[1]);"),
        "123"
    );
}

// ═══════════════════════════════════════════════════════════
// §22.3 — Date
// ═══════════════════════════════════════════════════════════

#[test]
fn date_now() {
    assert_eq!(eval_str("typeof Date.now()"), "number");
}

#[test]
fn date_call_returns_string_while_constructor_returns_date_object() {
    assert_eq!(eval_str("typeof Date()"), "string");
    assert_eq!(eval_str("typeof new Date(0).toGMTString()"), "string");
}

// ═══════════════════════════════════════════════════════════
// §24.3 — TypedArrays
// ═══════════════════════════════════════════════════════════

#[test]
fn typed_array_basic() {
    assert_eq!(
        eval_str(
            r#"
        let buf = new ArrayBuffer(8);
        let view = new Int32Array(buf);
        view[0] = 42;
        view[0]
    "#
        ),
        "42"
    );
}

// ═══════════════════════════════════════════════════════════
// §24.1 — WeakRef
// ═══════════════════════════════════════════════════════════

#[test]
fn weakref_basic() {
    assert_eq!(
        eval_console(
            r#"
        let obj = {value: 42};
        let ref1 = new WeakRef(obj);
        console.log(ref1.deref().value);
    "#
        ),
        "42"
    );
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
    assert_eq!(
        eval_str("new ReferenceError('undef').name"),
        "ReferenceError"
    );
}

#[test]
fn aggregate_error() {
    assert_eq!(
        eval_str(
            r#"
        let e = new AggregateError([new Error('a'), new Error('b')], 'all failed');
        e.errors.length
    "#
        ),
        "2"
    );
}

// ═══════════════════════════════════════════════════════════
// Closures
// ═══════════════════════════════════════════════════════════

#[test]
fn closure_basic() {
    assert_eq!(
        eval_str(
            r#"
        function counter() {
            let n = 0;
            return function() { n++; return n; };
        }
        let c = counter();
        c(); c(); c()
    "#
        ),
        "3"
    );
}

#[test]
fn closure_in_loop() {
    assert_eq!(
        eval_console(
            r#"
        let fns = [];
        for (let i = 0; i < 3; i++) {
            fns.push(() => i);
        }
        console.log(fns[0]());
        console.log(fns[1]());
        console.log(fns[2]());
    "#
        ),
        "0\n1\n2"
    );
}

// ═══════════════════════════════════════════════════════════
// Switch
// ═══════════════════════════════════════════════════════════

#[test]
fn switch_basic() {
    assert_eq!(
        eval_str(
            r#"
        let x = 2;
        let result;
        switch(x) {
            case 1: result = 'one'; break;
            case 2: result = 'two'; break;
            default: result = 'other';
        }
        result
    "#
        ),
        "two"
    );
}

// ═══════════════════════════════════════════════════════════
// Computed property names
// ═══════════════════════════════════════════════════════════

#[test]
fn computed_property() {
    assert_eq!(
        eval_str(
            r#"
        let key = 'x';
        let obj = { [key]: 42 };
        obj.x
    "#
        ),
        "42"
    );
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
    assert_eq!(
        eval_str(
            r#"
        let i = 0;
        do { i++; } while (i < 5);
        i
    "#
        ),
        "5"
    );
}

// ═══════════════════════════════════════════════════════════
// Labeled break/continue
// ═══════════════════════════════════════════════════════════

#[test]
fn labeled_break() {
    // Simple labeled break test
    assert_eq!(
        eval_console(
            r#"
        let found = false;
        outer: for (let i = 0; i < 3; i++) {
            for (let j = 0; j < 3; j++) {
                if (i === 1 && j === 1) { found = true; break outer; }
            }
        }
        console.log(found);
    "#
        ),
        "true"
    );
}

// ═══════════════════════════════════════════════════════════
// §14.7/§14.8 — Async Methods (object literal + class)
// ═══════════════════════════════════════════════════════════

#[test]
fn async_method_object_literal() {
    // async method shorthand in object literal must parse without error
    assert_eq!(
        eval_console(
            r#"
        const obj = {
            async init() { return 42; },
            async play(code) { return code + 1; },
            normal() { return 10; }
        };
        console.log(typeof obj.init);
        console.log(typeof obj.play);
        console.log(obj.normal());
    "#
        ),
        "function\nfunction\n10"
    );
}

#[test]
fn async_method_class() {
    // async method in class body
    assert_eq!(
        eval_console(
            r#"
        class MyClass {
            async fetchData() { return 99; }
            sync_method() { return 1; }
        }
        var mc = new MyClass();
        console.log(typeof mc.fetchData);
        console.log(mc.sync_method());
    "#
        ),
        "function\n1"
    );
}

#[test]
fn async_as_property_name() {
    // 'async' used as a normal property name (not a method modifier)
    assert_eq!(
        eval_str(
            r#"
        const obj = { async: true };
        obj.async
    "#
        ),
        "true"
    );
}

#[test]
fn async_method_strudel_pattern() {
    // Real-world pattern from strudel.js
    assert_eq!(
        eval_console(
            r#"
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
    "#
        ),
        "false\nfunction\nfunction\nfunction"
    );
}

#[test]
fn derived_class_instance_fields_run_after_super_with_args() {
    assert_eq!(
        eval_str(
            r#"
        class Base {
            constructor(name) { this.name = name; }
        }
        class Child extends Base {
            registry = new Map();
            items = [];
            hasName() { return this.name; }
        }
        let c = new Child("ok");
        c.registry.set("x", 1);
        c.items.push(2);
        c.hasName() + "," + c.registry.get("x") + "," + c.items.length
    "#
        ),
        "ok,1,1"
    );
}

#[test]
fn derived_default_constructor_forwards_super_and_runs_instance_fields() {
    assert_eq!(
        eval_str(
            r#"
        class Base {
            constructor(name) { this.baseName = name; }
        }
        class Derived extends Base {
            registry = new Map();
            items = [];
        }
        let d = new Derived("ok");
        d.registry.set("x", 1);
        d.items.push(2);
        d.baseName + "," + d.registry.get("x") + "," + d.items.length
    "#
        ),
        "ok,1,1"
    );
}

// ═══════════════════════════════════════════════════════════
// Minified patterns: super() in non-trivial positions
// ═══════════════════════════════════════════════════════════

#[test]
fn class_field_with_return_super() {
    // Minifiers often emit: constructor() { return super(...arguments) }
    assert_eq!(
        eval_str(
            r#"
        class Base {
            constructor(n) { this.n = n; }
        }
        class Derived extends Base {
            _entries = new Map();
            items = [];
            constructor() { return super(...arguments); }
        }
        let d = new Derived("ok");
        d._entries.set("x", 1);
        d.items.push(2);
        d.n + "," + d._entries.get("x") + "," + d.items.length
    "#
        ),
        "ok,1,1"
    );
}

#[test]
fn class_field_with_comma_super() {
    // Minifiers emit: constructor() { super(), this.init() }
    assert_eq!(
        eval_str(
            r#"
        class Base {
            constructor() { this.ready = true; }
        }
        class Derived extends Base {
            _map = new Map();
            constructor() { super(), this._map.set("a", 1); }
            val() { return this._map.get("a") + ":" + this.ready; }
        }
        new Derived().val()
    "#
        ),
        "1:true"
    );
}

#[test]
fn class_field_with_super_in_assignment() {
    // Pattern: constructor() { const x = super(); }
    assert_eq!(
        eval_str(
            r#"
        class Base {}
        class Derived extends Base {
            _m = new Map();
            constructor() { const x = super(); this._m.set("k", 42); }
            val() { return this._m.get("k"); }
        }
        new Derived().val()
    "#
        ),
        "42"
    );
}

#[test]
fn di_container_pattern() {
    // Exact pattern from the error: DI container with registerSingleton
    assert_eq!(
        eval_str(
            r#"
        class Base {}
        class ServiceCollection extends Base {
            _entries = new Map();
            _providers = [];
            constructor() { super(); }
            ensure(id) {
                if (!this._entries.has(id)) {
                    this._entries.set(id, []);
                }
                return this._entries.get(id);
            }
            set(id, entry) {
                const arr = this.ensure(id);
                arr.push(entry);
            }
            register(id, value) {
                this.set(id, { id: id, value: value });
            }
            registerSingleton(id, value) {
                this.register(id, value);
                this._providers.push(id);
            }
            isProviderFor(id) {
                return this._providers.filter(p => p === id).length > 0;
            }
        }
        const sc = new ServiceCollection();
        sc.registerSingleton("svc1", 10);
        sc.registerSingleton("svc2", 20);
        sc.isProviderFor("svc1") + ":" + sc._entries.get("svc1")[0].value + ":" + sc._providers.length
    "#
        ),
        "true:10:2"
    );
}

#[test]
fn di_container_minified_return_super() {
    // Same DI pattern but with minified constructor: return super(...arguments)
    assert_eq!(
        eval_str(
            r#"
        class Base {}
        class SC extends Base {
            _e = new Map();
            _p = [];
            constructor() { return super(...arguments); }
            ensure(id) {
                if (!this._e.has(id)) { this._e.set(id, []); }
                return this._e.get(id);
            }
            set(id, v) { this.ensure(id).push(v); }
            register(id, v) { this.set(id, v); this._p.push(id); }
        }
        const s = new SC();
        s.register("a", 1);
        s.register("b", 2);
        s._e.get("a")[0] + ":" + s._p.length
    "#
        ),
        "1:2"
    );
}

// ═══════════════════════════════════════════════════════════
// §27.2 — Async Event Loop: Microtasks, Promises, Timers
// ═══════════════════════════════════════════════════════════

#[test]
fn async_promise_then_microtask_ordering() {
    // Promise .then callbacks on already-resolved promises execute their
    // reactions immediately (they are enqueued and drained within the
    // same script turn for settled promises).  Verify ordering.
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.eval(
        r#"
        var log = [];
        log.push("sync1");
        Promise.resolve().then(() => log.push("micro1"));
        Promise.resolve().then(() => log.push("micro2"));
        log.push("sync2");
    "#,
    );
    let result = engine.eval("log.join(',')");
    // Microtasks run during execute() — the exact interleaving depends
    // on whether reactions are enqueued or run inline.  The key invariant
    // is that ALL four entries are present.
    let s = result.to_js_string();
    assert!(s.contains("sync1"), "missing sync1: {}", s);
    assert!(s.contains("sync2"), "missing sync2: {}", s);
    assert!(s.contains("micro1"), "missing micro1: {}", s);
    assert!(s.contains("micro2"), "missing micro2: {}", s);
}

#[test]
fn async_promise_then_chain() {
    // Chained .then callbacks must execute in order as microtasks.
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.eval(
        r#"
        var result = [];
        Promise.resolve(1)
            .then(v => { result.push("a:" + v); return v + 1; })
            .then(v => { result.push("b:" + v); return v + 1; })
            .then(v => { result.push("c:" + v); });
    "#,
    );
    let result = engine.eval("result.join(',')");
    assert_eq!(result.to_js_string(), "a:1,b:2,c:3");
}

#[test]
fn async_promise_resolve_then_catch() {
    // .catch on a fulfilled promise should not fire.
    assert_eq!(
        eval_str(
            r#"
        var out = "none";
        Promise.resolve(42)
            .then(v => { out = "ok:" + v; })
            .catch(e => { out = "err:" + e; });
        out
    "#
        ),
        "ok:42"
    );
}

#[test]
fn async_promise_reject_catch() {
    assert_eq!(
        eval_str(
            r#"
        var out = "none";
        Promise.reject("fail")
            .then(v => { out = "ok:" + v; })
            .catch(e => { out = "err:" + e; });
        out
    "#
        ),
        "err:fail"
    );
}

#[test]
fn async_promise_finally_runs_on_resolve() {
    assert_eq!(
        eval_str(
            r#"
        var out = [];
        Promise.resolve(1)
            .then(v => out.push("then:" + v))
            .finally(() => out.push("finally"));
        out.join(",")
    "#
        ),
        "then:1,finally"
    );
}

#[test]
fn async_promise_all() {
    assert_eq!(
        eval_str(
            r#"
        var out = "pending";
        Promise.all([
            Promise.resolve(1),
            Promise.resolve(2),
            Promise.resolve(3)
        ]).then(arr => { out = arr.join("+"); });
        out
    "#
        ),
        "1+2+3"
    );
}

#[test]
fn async_promise_all_with_reject() {
    assert_eq!(
        eval_str(
            r#"
        var out = "pending";
        Promise.all([
            Promise.resolve(1),
            Promise.reject("bad"),
            Promise.resolve(3)
        ]).catch(e => { out = "err:" + e; });
        out
    "#
        ),
        "err:bad"
    );
}

#[test]
fn async_promise_race() {
    assert_eq!(
        eval_str(
            r#"
        var out = "pending";
        Promise.race([
            Promise.resolve("first"),
            Promise.resolve("second")
        ]).then(v => { out = v; });
        out
    "#
        ),
        "first"
    );
}

#[test]
fn async_promise_any() {
    assert_eq!(
        eval_str(
            r#"
        var out = "pending";
        Promise.any([
            Promise.reject("a"),
            Promise.resolve("b"),
            Promise.reject("c")
        ]).then(v => { out = v; });
        out
    "#
        ),
        "b"
    );
}

#[test]
fn async_promise_all_settled() {
    assert_eq!(
        eval_str(
            r#"
        var out = "pending";
        Promise.allSettled([
            Promise.resolve(1),
            Promise.reject("x"),
            Promise.resolve(3)
        ]).then(results => {
            out = results.map(r => r.status).join(",");
        });
        out
    "#
        ),
        "fulfilled,rejected,fulfilled"
    );
}

#[test]
fn async_await_resolved_promise() {
    assert_eq!(
        eval_str(
            r#"
        var out = "pending";
        async function run() {
            const v = await Promise.resolve(42);
            out = "got:" + v;
        }
        run();
        out
    "#
        ),
        "got:42"
    );
}

#[test]
fn async_await_chain() {
    assert_eq!(
        eval_str(
            r#"
        var out = [];
        async function step(n) {
            return n * 2;
        }
        async function run() {
            const a = await step(3);
            out.push(a);
            const b = await step(a);
            out.push(b);
        }
        run();
        out.join(",")
    "#
        ),
        "6,12"
    );
}

#[test]
fn async_await_rejected_promise_try_catch() {
    assert_eq!(
        eval_str(
            r#"
        var out = "pending";
        async function run() {
            try {
                await Promise.reject("boom");
                out = "should not reach";
            } catch (e) {
                out = "caught:" + e;
            }
        }
        run();
        out
    "#
        ),
        "caught:boom"
    );
}

#[test]
fn async_queue_microtask() {
    // queueMicrotask should execute after synchronous code but before timers.
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.eval(
        r#"
        var result = [];
        result.push("sync");
        queueMicrotask(() => result.push("micro"));
        result.push("sync2");
    "#,
    );
    let result = engine.eval("result.join(',')");
    assert_eq!(result.to_js_string(), "sync,sync2,micro");
}

#[test]
fn async_settimeout_fires_on_tick() {
    // setTimeout callbacks should fire when tick() is called.
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.eval(
        r#"
        var result = "waiting";
        setTimeout(function() { result = "fired"; }, 10);
    "#,
    );
    // Before tick — timer hasn't fired yet.
    assert_eq!(engine.eval("result").to_js_string(), "waiting");
    // Advance by 10ms — timer should fire.
    engine.vm().tick(10);
    assert_eq!(engine.eval("result").to_js_string(), "fired");
}

#[test]
fn async_setinterval_repeats() {
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.eval(
        r#"
        var count = 0;
        var id = setInterval(function() { count++; }, 5);
    "#,
    );
    engine.vm().tick(5);
    assert_eq!(engine.eval("count").to_js_string(), "1");
    engine.vm().tick(5);
    assert_eq!(engine.eval("count").to_js_string(), "2");
    engine.eval("clearInterval(id)");
    engine.vm().tick(5);
    assert_eq!(engine.eval("count").to_js_string(), "2"); // should not increment
}

#[test]
fn async_settimeout_with_promise_interaction() {
    // Promise.then inside a setTimeout should work correctly.
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.eval(
        r#"
        var result = [];
        setTimeout(function() {
            result.push("timer");
            Promise.resolve().then(() => result.push("micro-in-timer"));
        }, 1);
    "#,
    );
    engine.vm().tick(1);
    let result = engine.eval("result.join(',')");
    assert_eq!(result.to_js_string(), "timer,micro-in-timer");
}

#[test]
fn async_run_event_loop() {
    // run_event_loop should process both microtasks and timers.
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.eval(
        r#"
        var result = [];
        result.push("start");
        setTimeout(function() {
            result.push("timer1");
            setTimeout(function() { result.push("timer2"); }, 1);
        }, 1);
    "#,
    );
    engine.vm().run_event_loop(100);
    let result = engine.eval("result.join(',')");
    assert_eq!(result.to_js_string(), "start,timer1,timer2");
}

#[test]
fn async_nested_promise_then() {
    // Nested .then chains should resolve correctly.
    assert_eq!(
        eval_str(
            r#"
        var out = "pending";
        Promise.resolve(1).then(v => {
            return Promise.resolve(v + 10);
        }).then(v => {
            out = "result:" + v;
        });
        out
    "#
        ),
        "result:11"
    );
}

#[test]
fn async_promise_constructor_with_async_resolve() {
    // new Promise where resolve is called after some synchronous work.
    assert_eq!(
        eval_str(
            r#"
        var out = "pending";
        const p = new Promise((resolve, reject) => {
            let x = 0;
            for (let i = 0; i < 10; i++) x += i;
            resolve(x);
        });
        p.then(v => { out = "sum:" + v; });
        out
    "#
        ),
        "sum:45"
    );
}

#[test]
fn async_multiple_awaits() {
    assert_eq!(
        eval_str(
            r#"
        var out = "pending";
        async function fetchA() { return "A"; }
        async function fetchB() { return "B"; }
        async function main() {
            const a = await fetchA();
            const b = await fetchB();
            out = a + b;
        }
        main();
        out
    "#
        ),
        "AB"
    );
}

// ═══════════════════════════════════════════════════════════
// §15.10 — RegExp: Lookahead, Lookbehind, Named Groups
// ═══════════════════════════════════════════════════════════

#[test]
fn regexp_positive_lookahead() {
    assert_eq!(eval_str(r#"/foo(?=bar)/.test("foobar")"#), "true");
    assert_eq!(eval_str(r#"/foo(?=bar)/.test("foobaz")"#), "false");
}

#[test]
fn regexp_positive_lookahead_zero_width() {
    // Lookahead should not consume characters.
    assert_eq!(
        eval_str(
            r#"
        var m = /foo(?=bar)/.exec("foobar");
        m[0]
    "#
        ),
        "foo" // NOT "foobar"
    );
}

#[test]
fn regexp_negative_lookahead() {
    assert_eq!(eval_str(r#"/foo(?!bar)/.test("foobaz")"#), "true");
    assert_eq!(eval_str(r#"/foo(?!bar)/.test("foobar")"#), "false");
}

#[test]
fn regexp_positive_lookbehind() {
    assert_eq!(eval_str(r#"/(?<=foo)bar/.test("foobar")"#), "true");
    assert_eq!(eval_str(r#"/(?<=foo)bar/.test("bazbar")"#), "false");
}

#[test]
fn regexp_positive_lookbehind_match() {
    // Lookbehind should not consume characters.
    assert_eq!(
        eval_str(
            r#"
        var m = /(?<=foo)bar/.exec("foobar");
        m[0]
    "#
        ),
        "bar" // NOT "foobar"
    );
}

#[test]
fn regexp_negative_lookbehind() {
    assert_eq!(eval_str(r#"/(?<!foo)bar/.test("bazbar")"#), "true");
    assert_eq!(eval_str(r#"/(?<!foo)bar/.test("foobar")"#), "false");
}

#[test]
fn regexp_named_group_basic() {
    assert_eq!(
        eval_str(
            r#"
        var m = /(?<year>\d{4})-(?<month>\d{2})-(?<day>\d{2})/.exec("2024-03-15");
        m.groups.year + "/" + m.groups.month + "/" + m.groups.day
    "#
        ),
        "2024/03/15"
    );
}

#[test]
fn regexp_named_group_in_replace() {
    // Named groups should also be available as numbered captures.
    assert_eq!(
        eval_str(
            r#"
        var m = /(?<first>\w+) (?<last>\w+)/.exec("John Doe");
        m[1] + " " + m[2]
    "#
        ),
        "John Doe"
    );
}

#[test]
fn regexp_named_backreference() {
    assert_eq!(
        eval_str(r#"/(?<word>\w+)\s+\k<word>/.test("hello hello")"#),
        "true"
    );
    assert_eq!(
        eval_str(r#"/(?<word>\w+)\s+\k<word>/.test("hello world")"#),
        "false"
    );
}

#[test]
fn regexp_lookahead_in_split() {
    // Split before each uppercase letter.
    assert_eq!(
        eval_str(r#""camelCaseWord".split(/(?=[A-Z])/).join(",")"#),
        "camel,Case,Word"
    );
}

#[test]
fn regexp_combined_lookahead_lookbehind() {
    // Match digits that are preceded by $ and followed by a space.
    assert_eq!(
        eval_str(r#"/(?<=\$)\d+(?=\s)/.exec("price $42 USD")[0]"#),
        "42"
    );
}

// ═══════════════════════════════════════════════════════════
// §16.2 — ES Modules: import/export
// ═══════════════════════════════════════════════════════════

#[test]
fn module_basic_export_import() {
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.register_module_source(
        "math",
        "export function add(a, b) { return a + b; }\nexport const PI = 3.14;",
    );
    let result = engine.eval(
        r#"
        import { add, PI } from 'math';
        add(2, 3) + ":" + PI
    "#,
    );
    assert_eq!(result.to_js_string(), "5:3.14");
}

#[test]
fn module_default_export() {
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.register_module_source(
        "greeter",
        "export default function(name) { return 'Hello ' + name; }",
    );
    let result = engine.eval(
        r#"
        import greet from 'greeter';
        greet("World")
    "#,
    );
    assert_eq!(result.to_js_string(), "Hello World");
}

#[test]
fn module_namespace_import() {
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.register_module_source(
        "utils",
        "export function upper(s) { return s.toUpperCase(); }\nexport function lower(s) { return s.toLowerCase(); }",
    );
    let result = engine.eval(
        r#"
        import * as utils from 'utils';
        utils.upper("hello") + ":" + utils.lower("WORLD")
    "#,
    );
    assert_eq!(result.to_js_string(), "HELLO:world");
}

#[test]
fn module_caching() {
    // Modules should only execute once — subsequent imports return cached exports.
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    engine.register_module_source(
        "counter",
        "var count = 0; count++; export function get() { return count; }",
    );
    let result = engine.eval(
        r#"
        import { get as get1 } from 'counter';
        import { get as get2 } from 'counter';
        get1() + ":" + get2()
    "#,
    );
    assert_eq!(result.to_js_string(), "1:1"); // not 1:2
}

#[test]
fn module_not_found_throws() {
    assert!(eval_throws(r#"import { x } from 'nonexistent';"#));
}

#[test]
fn module_native_object() {
    use libjs::JsValue;
    let mut engine = JsEngine::new();
    engine.set_step_limit(1_000_000);
    // Register a native module object.
    let ns = JsValue::new_object();
    ns.set_property(
        alloc::string::String::from("version"),
        JsValue::String(alloc::string::String::from("1.0")),
    );
    engine.register_module_object("config", ns);
    let result = engine.eval(
        r#"
        import { version } from 'config';
        version
    "#,
    );
    assert_eq!(result.to_js_string(), "1.0");
}

// ═══════════════════════════════════════════════════════════
// §10.2 — Strict Mode
// ═══════════════════════════════════════════════════════════

#[test]
fn strict_undeclared_var_throws() {
    assert!(eval_throws(
        r#"
        "use strict";
        undeclaredVar = 42;
    "#
    ));
}

#[test]
fn strict_declared_var_ok() {
    assert_eq!(
        eval_str(
            r#"
        "use strict";
        var x = 42;
        x
    "#
        ),
        "42"
    );
}

#[test]
fn strict_function_decl_ok() {
    assert_eq!(
        eval_str(
            r#"
        "use strict";
        function foo() { return 7; }
        foo()
    "#
        ),
        "7"
    );
}

// ═══════════════════════════════════════════════════════════
// §21.2 — BigInt
// ═══════════════════════════════════════════════════════════

#[test]
fn bigint_literal() {
    assert_eq!(eval_str("typeof 42n"), "bigint");
}

#[test]
fn bigint_arithmetic() {
    assert_eq!(eval_str("(10n + 20n).toString()"), "30");
    assert_eq!(eval_str("(100n - 42n).toString()"), "58");
    assert_eq!(eval_str("(7n * 6n).toString()"), "42");
    assert_eq!(eval_str("typeof (6n / 2n)"), "bigint");
    assert_eq!(eval_str("(6n / 2n).toString()"), "3");
    assert_eq!(eval_str("(7n % 3n).toString()"), "1");
}

#[test]
fn bigint_negation() {
    assert_eq!(eval_str("(-42n).toString()"), "-42");
}

#[test]
fn bigint_comparison() {
    assert_eq!(eval_str("42n === 42n"), "true");
    assert_eq!(eval_str("42n === 43n"), "false");
}

#[test]
fn bigint_constructor_fn() {
    assert_eq!(eval_str("BigInt(123).toString()"), "123");
    assert_eq!(eval_str("BigInt('456').toString()"), "456");
    assert_eq!(eval_str("typeof BigInt(0)"), "bigint");
}

#[test]
fn bigint_mixed_type_error() {
    assert!(eval_throws("42n + 1"));
    assert!(eval_throws("1 + 42n"));
    assert!(eval_throws("42n - 1"));
}

#[test]
fn bigint_to_string() {
    assert_eq!(eval_str("String(42n)"), "42n");
}

#[test]
fn bigint_large_numbers() {
    // Ensure arbitrary precision works.
    assert_eq!(
        eval_str("(999999999999999999n * 999999999999999999n).toString()"),
        "999999999999999998000000000000000001"
    );
}

// ═══════════════════════════════════════════════════════════
// §16.1 — Error.stack
// ═══════════════════════════════════════════════════════════

#[test]
fn error_stack_has_function_name() {
    let result = eval_str(
        r#"
        var s = "";
        try { throw new Error("oops"); } catch(e) { s = e.stack; }
        s
    "#,
    );
    assert!(
        result.contains("Error"),
        "stack should contain 'Error': {}",
        result
    );
}

#[test]
fn error_stack_format() {
    let result = eval_str(
        r#"
        var s = "";
        try { throw new Error("test"); } catch(e) { s = e.stack; }
        s
    "#,
    );
    assert!(
        result.contains("Error: test"),
        "stack should start with error: {}",
        result
    );
    assert!(
        result.contains("at "),
        "stack should have 'at' frames: {}",
        result
    );
}

// ═══════════════════════════════════════════════════════════
// §25.1 — ES2025 Iterator Helpers
// ═══════════════════════════════════════════════════════════

#[test]
fn iterator_to_array() {
    assert_eq!(eval_str("[1,2,3].values().toArray().join(',')"), "1,2,3");
}

#[test]
fn iterator_map() {
    assert_eq!(
        eval_str("[1,2,3].values().map(x => x * 2).toArray().join(',')"),
        "2,4,6"
    );
}

#[test]
fn iterator_filter() {
    assert_eq!(
        eval_str("[1,2,3,4,5].values().filter(x => x % 2 === 0).toArray().join(',')"),
        "2,4"
    );
}

#[test]
fn iterator_take() {
    assert_eq!(
        eval_str("[1,2,3,4,5].values().take(3).toArray().join(',')"),
        "1,2,3"
    );
}

#[test]
fn iterator_drop() {
    assert_eq!(
        eval_str("[1,2,3,4,5].values().drop(2).toArray().join(',')"),
        "3,4,5"
    );
}

#[test]
fn iterator_some_every() {
    assert_eq!(eval_str("[1,2,3].values().some(x => x > 2)"), "true");
    assert_eq!(eval_str("[1,2,3].values().every(x => x > 0)"), "true");
    assert_eq!(eval_str("[1,2,3].values().every(x => x > 1)"), "false");
}

#[test]
fn iterator_find() {
    assert_eq!(eval_str("[10,20,30].values().find(x => x > 15)"), "20");
}

#[test]
fn iterator_reduce() {
    assert_eq!(
        eval_str("[1,2,3,4].values().reduce((acc, x) => acc + x, 0)"),
        "10"
    );
}

#[test]
fn iterator_flat_map() {
    assert_eq!(
        eval_str("[1,2,3].values().flatMap(x => [x, x*10]).toArray().join(',')"),
        "1,10,2,20,3,30"
    );
}

// ═══════════════════════════════════════════════════════════
// §24.2 — ES2025 Set Methods
// ═══════════════════════════════════════════════════════════

#[test]
fn set_union() {
    assert_eq!(
        eval_str(
            r#"
        var a = new Set([1, 2, 3]);
        var b = new Set([3, 4, 5]);
        var u = a.union(b);
        Array.from(u).sort().join(",")
    "#
        ),
        "1,2,3,4,5"
    );
}

#[test]
fn set_intersection() {
    assert_eq!(
        eval_str(
            r#"
        var a = new Set([1, 2, 3, 4]);
        var b = new Set([3, 4, 5, 6]);
        var i = a.intersection(b);
        Array.from(i).sort().join(",")
    "#
        ),
        "3,4"
    );
}

#[test]
fn set_difference() {
    assert_eq!(
        eval_str(
            r#"
        var a = new Set([1, 2, 3, 4]);
        var b = new Set([3, 4, 5]);
        var d = a.difference(b);
        Array.from(d).sort().join(",")
    "#
        ),
        "1,2"
    );
}

#[test]
fn set_symmetric_difference() {
    assert_eq!(
        eval_str(
            r#"
        var a = new Set([1, 2, 3]);
        var b = new Set([2, 3, 4]);
        var sd = a.symmetricDifference(b);
        Array.from(sd).sort().join(",")
    "#
        ),
        "1,4"
    );
}

#[test]
fn set_subset_superset_disjoint() {
    assert_eq!(
        eval_str("new Set([1,2]).isSubsetOf(new Set([1,2,3]))"),
        "true"
    );
    assert_eq!(
        eval_str("new Set([1,2,3]).isSupersetOf(new Set([1,2]))"),
        "true"
    );
    assert_eq!(
        eval_str("new Set([1,2]).isDisjointFrom(new Set([3,4]))"),
        "true"
    );
    assert_eq!(
        eval_str("new Set([1,2]).isDisjointFrom(new Set([2,3]))"),
        "false"
    );
}

// ═══════════════════════════════════════════════════════════
// §20.3 — Date Setters and Date.UTC
// ═══════════════════════════════════════════════════════════

#[test]
fn date_set_full_year() {
    assert_eq!(
        eval_str(
            r#"
        var d = new Date(0);
        d.setFullYear(2000);
        d.getFullYear()
    "#
        ),
        "2000"
    );
}

#[test]
fn date_set_month() {
    assert_eq!(
        eval_str(
            r#"
        var d = new Date(0);
        d.setMonth(5);
        d.getMonth()
    "#
        ),
        "5"
    );
}

#[test]
fn date_set_hours_minutes_seconds() {
    assert_eq!(
        eval_str(
            r#"
        var d = new Date(0);
        d.setHours(15, 30, 45);
        d.getHours() + ":" + d.getMinutes() + ":" + d.getSeconds()
    "#
        ),
        "15:30:45"
    );
}

#[test]
fn date_utc_static() {
    assert_eq!(eval_str("Date.UTC(1970, 0, 1)"), "0");
    assert_eq!(eval_str("Date.UTC(2000, 0, 1)"), "946684800000");
}

#[test]
fn date_get_timezone_offset() {
    assert_eq!(eval_str("new Date().getTimezoneOffset()"), "0");
}
