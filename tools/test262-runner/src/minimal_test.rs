use libjs::JsEngine;

fn test(label: &str, code: &str) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut engine = JsEngine::new();
        engine.set_step_limit(5_000_000);
        engine.eval(code);
        let exc = engine.last_exception().cloned();
        exc
    }));
    match result {
        Ok(None) => println!("OK    {}", label),
        Ok(Some(exc)) => {
            let msg = match &exc {
                libjs::JsValue::Object(o) => {
                    let o = o.borrow();
                    let m = o.get("message");
                    format!("{}", m.to_js_string())
                }
                libjs::JsValue::String(s) => s.clone(),
                other => other.to_js_string(),
            };
            println!("EXC   {} => {}", label, msg);
        }
        Err(panic) => {
            let msg = if let Some(s) = panic.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = panic.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "unknown".to_string()
            };
            println!("PANIC {} => {}", label, msg);
        }
    }
}

fn main() {
    // Test harness parts individually
    test("constructor-fn", r#"
        function Test262Error(message) {
            this.message = message || "";
        }
    "#);

    test("prototype-toString", r#"
        function Test262Error(message) {
            this.message = message || "";
        }
        Test262Error.prototype.toString = function () {
            return "Test262Error: " + this.message;
        };
    "#);

    test("assert-fn", r#"
        function assert(mustBeTrue, message) {
            if (mustBeTrue === true) { return; }
        }
    "#);

    test("assert-_isSameValue", r#"
        function assert() {}
        assert._isSameValue = function (a, b) {
            if (a === b) {
                return a !== 0 || 1 / a === 1 / b;
            }
            return a !== a && b !== b;
        };
    "#);

    test("switch-stmt", r#"
        function test(value) {
            switch (value === null ? 'null' : typeof value) {
                case 'string':
                    return "str";
                case 'number':
                    return "num";
                case 'boolean':
                    return "bool";
                default:
                    return "other";
            }
        }
        test("hello");
        test(42);
    "#);

    test("template-literal-simple", r#"
        var x = `hello`;
    "#);

    test("template-literal-expr", r#"
        var a = 1;
        var x = `value: ${a}`;
    "#);

    test("template-literal-complex", r#"
        function f(v) {
            return `Actual argument [${v}] test`;
        }
        f("hello");
    "#);

    test("eval-simple", "eval(\"1+1\")");

    test("eval-whitespace", "eval(\"1\\t+\\t1\")");

    // Test eval return value
    test("eval-returns-2", r#"
        var result = eval("1+1");
        if (result !== 2) {
            throw new Error("eval returned " + result + " (type: " + typeof result + ")");
        }
    "#);

    // Test \u0009 in string literal
    test("unicode-escape-tab", r#"
        var s = "1\u0009+\u00091";
        if (s !== "1\t+\t1") {
            throw new Error("unicode escape mismatch: got " + s.length + " chars");
        }
    "#);

    // Test eval with unicode escape tab
    test("eval-unicode-tab", r#"
        if (eval("1\u0009+\u00091") !== 2) {
            throw new Error("eval with unicode tab failed");
        }
    "#);

    test("obj-valueOf", r#"
        var o = {valueOf: function() {return 1}};
        o + 1;
    "#);

    test("obj-valueOf-in-if", r#"
        if ({valueOf: function() {return 1}} + 1 !== 2) {
            throw "fail";
        }
    "#);

    test("new-Date", r#"
        var d = new Date(0);
    "#);

    test("date-add", r#"
        var d = new Date(0);
        d + d;
    "#);

    test("fn-toString", r#"
        function f1() { return 0; }
        f1.toString();
    "#);

    test("fn-add", r#"
        function f1() { return 0; }
        f1 + 1;
    "#);

    // Now test full harness
    let harness = r#"
        function Test262Error(message) {
            this.message = message || "";
        }
        Test262Error.prototype.toString = function () {
            return "Test262Error: " + this.message;
        };
        Test262Error.thrower = function (message) {
            throw new Test262Error(message);
        };
        function $DONOTEVALUATE() {
            throw "Test262: This statement should not be evaluated.";
        }
    "#;

    test("harness-only", harness);

    test("harness+eval", &format!("{}\neval(\"1+1\");", harness));

    test("harness+obj-add", &format!("{}\n{}", harness, r#"
        if ({valueOf: function() {return 1}} + 1 !== 2) {
            throw new Test262Error('fail');
        }
    "#));

    // Full assert.js harness
    let full_harness = r#"
function Test262Error(message) {
  this.message = message || "";
}
Test262Error.prototype.toString = function () {
  return "Test262Error: " + this.message;
};
Test262Error.thrower = function (message) {
  throw new Test262Error(message);
};
function $DONOTEVALUATE() {
  throw "Test262: This statement should not be evaluated.";
}

function assert(mustBeTrue, message) {
  if (mustBeTrue === true) {
    return;
  }
  if (message === undefined) {
    message = 'Expected true but got ' + assert._toString(mustBeTrue);
  }
  throw new Test262Error(message);
}

assert._isSameValue = function (a, b) {
  if (a === b) {
    return a !== 0 || 1 / a === 1 / b;
  }
  return a !== a && b !== b;
};

assert.sameValue = function (actual, expected, message) {
  try {
    if (assert._isSameValue(actual, expected)) {
      return;
    }
  } catch (error) {
    throw new Test262Error(message + ' (_isSameValue operation threw) ' + error);
    return;
  }
  if (message === undefined) {
    message = '';
  } else {
    message += ' ';
  }
  message += 'Expected SameValue(«' + assert._toString(actual) + '», «' + assert._toString(expected) + '») to be true';
  throw new Test262Error(message);
};
"#;

    test("full-harness-part1", full_harness);

    let full_harness2 = r#"
function Test262Error(message) { this.message = message || ""; }
function assert() {}

assert._formatIdentityFreeValue = function formatIdentityFreeValue(value) {
  switch (value === null ? 'null' : typeof value) {
    case 'string':
      return '"' + value + '"';
    case 'number':
      if (value === 0 && 1 / value === -Infinity) return '-0';
    case 'boolean':
    case 'undefined':
    case 'null':
      return String(value);
  }
};
"#;

    test("full-harness-formatIdentityFreeValue", full_harness2);

    let full_harness3 = r#"
function Test262Error(message) { this.message = message || ""; }
function assert() {}

assert._toString = function (value) {
  try {
    return String(value);
  } catch (err) {
    if (err.name === 'TypeError') {
      return Object.prototype.toString.call(value);
    }
    throw err;
  }
};
"#;

    test("full-harness-_toString", full_harness3);

    let full_harness4 = r#"
function isPrimitive(value) {
  return !value || (typeof value !== 'object' && typeof value !== 'function');
}
"#;

    test("full-harness-isPrimitive", full_harness4);

    let full_harness5 = r#"
function assert() {}
assert.compareArray = function (actual, expected, message) {
  message = message === undefined ? '' : message;
  if (typeof message === 'symbol') {
    message = message.toString();
  }
};
"#;

    test("full-harness-compareArray-part", full_harness5);

    let full_harness6 = r#"
function compareArray(a, b) {
  if (b.length !== a.length) {
    return false;
  }
  for (var i = 0; i < a.length; i++) {
    if (a[i] !== b[i]) {
      return false;
    }
  }
  return true;
}

compareArray.format = function (arrayLike) {
  return "formatted";
};
"#;

    test("full-harness-compareArray-fn", full_harness6);

    // Now test with the real assert.js file if available
    let assert_path = "/daten1/development/brian/anyos/libs/libjs_tests/test262/harness/assert.js";
    if let Ok(assert_js) = std::fs::read_to_string(assert_path) {
        let sta_path = "/daten1/development/brian/anyos/libs/libjs_tests/test262/harness/sta.js";
        let sta_js = std::fs::read_to_string(sta_path).unwrap_or_default();
        let real_harness = format!("{}\n{}", sta_js, assert_js);
        test("real-harness-only", &real_harness);

        test("real-harness+simple", &format!("{}\nvar x = 1 + 1;", real_harness));

        test("real-harness+eval", &format!("{}\neval(\"1+1\");", real_harness));

        // Actual failing test: S11.6.1_A1.js
        test("real-harness+A1", &format!("{}\n{}", real_harness, r#"
if (eval("1\u{0009}+\u{0009}1") !== 2) {
  throw new Test262Error('#1: 1\\u0009+\\u00091 === 2');
}
"#));

        // S11.6.1_A2.2_T1.js
        test("real-harness+A2.2_T1", &format!("{}\n{}", real_harness, r#"
if ({valueOf: function() {return 1}} + 1 !== 2) {
  throw new Test262Error('#1: fail');
}
"#));

        // S11.6.1_A2.2_T2.js - Date
        test("real-harness+A2.2_T2", &format!("{}\n{}", real_harness, r#"
var date = new Date(0);
if (date + date !== date.toString() + date.toString()) {
  throw new Test262Error('#1: fail');
}
"#));

        // S11.6.1_A2.2_T3.js - Function
        test("real-harness+A2.2_T3", &format!("{}\n{}", real_harness, r#"
function f1(){ return 0; }
if (f1 + 1 !== f1.toString() + 1) {
  throw new Test262Error('#1: fail');
}
"#));

        // S11.6.1_A2.3_T1.js - eval order
        test("real-harness+A2.3_T1", &format!("{}\n{}", real_harness, r#"
var x = { valueOf: function () { throw "x"; } };
var y = { valueOf: function () { throw "y"; } };
try {
   x + y;
   throw new Test262Error('#1.1: should throw');
} catch (e) {
   if (e !== "x") {
     throw new Test262Error('#1.2: expected "x", got ' + e);
   }
}
"#));

        // Test constructor property on prototype
        test("constructor-on-prototype", r#"
            function Foo() { this.x = 1; }
            var f = new Foo();
            if (f.constructor !== Foo) {
                throw new Error("constructor is " + typeof f.constructor + ": " + f.constructor);
            }
        "#);

        test("proto-has-constructor", r#"
            function Foo() {}
            var p = Foo.prototype;
            if (typeof p !== 'object') {
                throw new Error("prototype is " + typeof p);
            }
            if (typeof p.constructor === 'undefined') {
                throw new Error("prototype.constructor is undefined. Keys: " + Object.keys(p));
            }
        "#);

        test("test262error-constructor", &format!("{}\n{}", real_harness, r#"
            var e = new Test262Error("test");
            if (e.constructor !== Test262Error) {
                throw new Error("constructor is " + typeof e.constructor + ": " + e.constructor);
            }
        "#));

        test("assert-throws-typeerror", &format!("{}\n{}", real_harness, r#"
            assert.throws(TypeError, function() {
                var s = Symbol('test');
                s + '';
            });
        "#));

        // Now try loading the ACTUAL test files
        let test_base = "/daten1/development/brian/anyos/libs/libjs_tests/test262/language/expressions/addition";
        for name in &["S11.6.1_A1.js", "S11.6.1_A2.2_T1.js", "S11.6.1_A2.2_T2.js", "S11.6.1_A2.2_T3.js", "S11.6.1_A2.3_T1.js"] {
            let path = format!("{}/{}", test_base, name);
            if let Ok(src) = std::fs::read_to_string(&path) {
                let full = format!("{}\n{}", real_harness, src);
                test(&format!("real-file-{}", name), &full);
            }
        }
    }
}
