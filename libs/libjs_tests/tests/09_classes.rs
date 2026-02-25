//! Classes — declaration, constructor, methods, inheritance, super, static.
//!
//! Level 9: OOP patterns, prototype delegation, `instanceof` checks.

use libjs_tests::JsEngine;

fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }
fn bool_(src: &str) -> bool { js(src).to_boolean() }
fn str_(src: &str) -> String { js(src).to_js_string() }

// ── basic class ───────────────────────────────────────────────────────────────

#[test]
fn class_constructor_and_method() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class Point {
            constructor(x, y) {
                this.x = x;
                this.y = y;
            }
            toString() {
                return '(' + this.x + ',' + this.y + ')';
            }
        }
        var p = new Point(3, 4);
        var s = p.toString();
    "#);
    assert_eq!(e.get_global("s").to_js_string(), "(3,4)");
}

#[test]
fn class_instance_has_properties() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class Dog { constructor(name) { this.name = name; } }
        var d = new Dog('Rex');
    "#);
    assert_eq!(e.get_global("d").get_property("name").to_js_string(), "Rex");
}

#[test]
fn class_method_return_value() {
    assert_eq!(
        num(r#"
            class Calc {
                add(a, b) { return a + b; }
            }
            new Calc().add(17, 25)
        "#),
        42.0
    );
}

// ── this inside class methods ─────────────────────────────────────────────────

#[test]
fn class_this_access() {
    assert_eq!(
        num(r#"
            class Circle {
                constructor(r) { this.r = r; }
                area() { return this.r * this.r; }
            }
            new Circle(7).area()
        "#),
        49.0
    );
}

#[test]
fn class_method_mutates_this() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class Counter {
            constructor() { this.count = 0; }
            inc() { this.count++; }
        }
        var c = new Counter();
        c.inc(); c.inc(); c.inc();
    "#);
    assert_eq!(e.get_global("c").get_property("count").to_number(), 3.0);
}

// ── inheritance / extends / super ─────────────────────────────────────────────

#[test]
fn extends_inherits_method() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class Animal {
            speak() { return 'generic sound'; }
        }
        class Dog extends Animal {
            speak() { return 'woof'; }
        }
        var a = new Animal().speak();
        var d = new Dog().speak();
    "#);
    assert_eq!(e.get_global("a").to_js_string(), "generic sound");
    assert_eq!(e.get_global("d").to_js_string(), "woof");
}

#[test]
fn super_constructor_call() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class Vehicle {
            constructor(wheels) { this.wheels = wheels; }
        }
        class Car extends Vehicle {
            constructor() { super(4); this.type = 'car'; }
        }
        var c = new Car();
    "#);
    assert_eq!(e.get_global("c").get_property("wheels").to_number(), 4.0);
    assert_eq!(e.get_global("c").get_property("type").to_js_string(), "car");
}

#[test]
fn super_method_call() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class Animal {
            describe() { return 'I am an animal'; }
        }
        class Dog extends Animal {
            describe() { return super.describe() + ' (dog)'; }
        }
        var result = new Dog().describe();
    "#);
    assert_eq!(e.get_global("result").to_js_string(), "I am an animal (dog)");
}

#[test]
fn instanceof_with_inheritance() {
    assert!(bool_(r#"
        class A {}
        class B extends A {}
        var b = new B();
        (b instanceof B) && (b instanceof A)
    "#));
}

// ── static methods ────────────────────────────────────────────────────────────

#[test]
fn static_method() {
    assert_eq!(
        num(r#"
            class MathUtils {
                static square(n) { return n * n; }
            }
            MathUtils.square(9)
        "#),
        81.0
    );
}

#[test]
fn static_factory_method() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class Point {
            constructor(x, y) { this.x = x; this.y = y; }
            static origin() { return new Point(0, 0); }
        }
        var o = Point.origin();
    "#);
    assert_eq!(e.get_global("o").get_property("x").to_number(), 0.0);
    assert_eq!(e.get_global("o").get_property("y").to_number(), 0.0);
}

// ── class expression ──────────────────────────────────────────────────────────

#[test]
fn class_expression() {
    assert_eq!(
        num("var Sq = class { calc(n) { return n*n; } }; new Sq().calc(8)"),
        64.0
    );
}

// ── method chaining ───────────────────────────────────────────────────────────

#[test]
fn method_chaining_via_return_this() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class Builder {
            constructor() { this.parts = []; }
            add(part) { this.parts.push(part); return this; }
            build() { return this.parts.join('-'); }
        }
        var result = new Builder().add('a').add('b').add('c').build();
    "#);
    assert_eq!(e.get_global("result").to_js_string(), "a-b-c");
}

// ── multi-level inheritance ───────────────────────────────────────────────────

#[test]
fn three_level_inheritance() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class A { greet() { return 'A'; } }
        class B extends A { greet() { return super.greet() + 'B'; } }
        class C extends B { greet() { return super.greet() + 'C'; } }
        var result = new C().greet();
    "#);
    assert_eq!(e.get_global("result").to_js_string(), "ABC");
}

// ── toString via prototype ────────────────────────────────────────────────────

#[test]
fn class_toString_method() {
    let mut e = JsEngine::new();
    e.eval(r#"
        class Box {
            constructor(v) { this.v = v; }
            toString() { return 'Box(' + this.v + ')'; }
        }
        var b = new Box(42);
        var s = b.toString();
    "#);
    assert_eq!(e.get_global("s").to_js_string(), "Box(42)");
}
