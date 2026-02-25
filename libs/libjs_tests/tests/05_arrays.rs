//! Arrays — creation, indexing, Array.prototype methods, destructuring, spread.
//!
//! Level 5: Multi-method chains, higher-order functions (map/filter/reduce).

use libjs_tests::{JsEngine, JsValueExt};

fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }
fn bool_(src: &str) -> bool { js(src).to_boolean() }
fn str_(src: &str) -> String { js(src).to_js_string() }

// ── creation and access ───────────────────────────────────────────────────────

#[test] fn array_literal() { assert_eq!(num("[1,2,3][1]"), 2.0); }
#[test] fn array_length()  { assert_eq!(num("[10,20,30].length"), 3.0); }
#[test] fn empty_array_length() { assert_eq!(num("[].length"), 0.0); }

#[test]
fn nested_array() {
    assert_eq!(num("[[1,2],[3,4]][1][0]"), 3.0);
}

// ── push / pop / shift / unshift ──────────────────────────────────────────────

#[test]
fn push_returns_new_length() {
    assert_eq!(num("var a = [1,2]; a.push(3)"), 3.0);
}

#[test]
fn push_appends_element() {
    let mut e = JsEngine::new();
    e.eval("var a = [1,2]; a.push(99);");
    assert_eq!(e.get_global("a").get_index(2).to_number(), 99.0);
}

#[test]
fn pop_returns_last() {
    assert_eq!(num("var a = [1,2,3]; a.pop()"), 3.0);
}

#[test]
fn pop_shortens_array() {
    let mut e = JsEngine::new();
    e.eval("var a = [1,2,3]; a.pop();");
    assert_eq!(e.get_global("a").get_property("length").to_number(), 2.0);
}

#[test]
fn shift_removes_first() {
    assert_eq!(num("var a = [10,20,30]; a.shift()"), 10.0);
}

#[test]
fn unshift_prepends_and_returns_length() {
    assert_eq!(num("var a = [3,4]; a.unshift(1,2)"), 4.0);
}

// ── indexOf / includes / find ─────────────────────────────────────────────────

#[test]
fn index_of_element() {
    assert_eq!(num("[5,10,15].indexOf(10)"), 1.0);
    assert_eq!(num("[5,10,15].indexOf(99)"), -1.0);
}

#[test]
fn includes() {
    assert!(bool_("[1,2,3].includes(2)"));
    assert!(!bool_("[1,2,3].includes(9)"));
}

#[test]
fn find_returns_first_match() {
    assert_eq!(num("[1,5,8,3].find(function(x){ return x > 4; })"), 5.0);
}

#[test]
fn find_index() {
    assert_eq!(num("[10,20,30].findIndex(function(x){ return x >= 20; })"), 1.0);
}

// ── slice / splice ────────────────────────────────────────────────────────────

#[test]
fn slice() {
    let mut e = JsEngine::new();
    e.eval("var s = [1,2,3,4,5].slice(1,4);");
    let s = e.get_global("s");
    assert_eq!(s.get_index(0).to_number(), 2.0);
    assert_eq!(s.get_index(2).to_number(), 4.0);
}

#[test]
fn splice_removes_elements() {
    let mut e = JsEngine::new();
    e.eval("var a = [1,2,3,4,5]; a.splice(1,2);");
    let a = e.get_global("a");
    assert_eq!(a.get_property("length").to_number(), 3.0);
    assert_eq!(a.get_index(0).to_number(), 1.0);
    assert_eq!(a.get_index(1).to_number(), 4.0);
}

// ── concat / join / reverse ───────────────────────────────────────────────────

#[test]
fn concat() {
    let mut e = JsEngine::new();
    e.eval("var c = [1,2].concat([3,4],[5]);");
    assert_eq!(e.get_global("c").get_property("length").to_number(), 5.0);
}

#[test]
fn join() {
    assert_eq!(str_("[1,2,3].join('-')"), "1-2-3");
    assert_eq!(str_("[1,2,3].join()"), "1,2,3");
}

#[test]
fn reverse() {
    let mut e = JsEngine::new();
    e.eval("var a = [1,2,3]; a.reverse();");
    let a = e.get_global("a");
    assert_eq!(a.get_index(0).to_number(), 3.0);
    assert_eq!(a.get_index(2).to_number(), 1.0);
}

// ── sort ──────────────────────────────────────────────────────────────────────

#[test]
fn sort_numbers_with_comparator() {
    let mut e = JsEngine::new();
    e.eval("var a = [3,1,4,1,5,9]; a.sort(function(x,y){ return x - y; });");
    let a = e.get_global("a");
    assert_eq!(a.get_index(0).to_number(), 1.0);
    assert_eq!(a.get_index(5).to_number(), 9.0);
}

#[test]
fn sort_strings_default() {
    let mut e = JsEngine::new();
    e.eval(r#"var a = ["banana","apple","cherry"]; a.sort();"#);
    assert_eq!(e.get_global("a").get_index(0).to_js_string(), "apple");
}

// ── map ───────────────────────────────────────────────────────────────────────

#[test]
fn map_doubles() {
    let mut e = JsEngine::new();
    e.eval("var doubled = [1,2,3].map(function(x){ return x * 2; });");
    let d = e.get_global("doubled");
    assert_eq!(d.get_index(0).to_number(), 2.0);
    assert_eq!(d.get_index(1).to_number(), 4.0);
    assert_eq!(d.get_index(2).to_number(), 6.0);
}

#[test]
fn map_with_arrow() {
    let mut e = JsEngine::new();
    e.eval("var sq = [1,2,3,4].map(x => x * x);");
    let sq = e.get_global("sq");
    assert_eq!(sq.get_index(3).to_number(), 16.0);
}

// ── filter ────────────────────────────────────────────────────────────────────

#[test]
fn filter_evens() {
    let mut e = JsEngine::new();
    e.eval("var evens = [1,2,3,4,5,6].filter(x => x % 2 === 0);");
    let evens = e.get_global("evens");
    assert_eq!(evens.get_property("length").to_number(), 3.0);
    assert_eq!(evens.get_index(0).to_number(), 2.0);
    assert_eq!(evens.get_index(2).to_number(), 6.0);
}

// ── reduce ────────────────────────────────────────────────────────────────────

#[test]
fn reduce_sum() {
    assert_eq!(num("[1,2,3,4,5].reduce(function(acc,x){ return acc + x; }, 0)"), 15.0);
}

#[test]
fn reduce_product() {
    assert_eq!(num("[1,2,3,4].reduce((acc,x) => acc * x, 1)"), 24.0);
}

#[test]
fn reduce_no_initial_value() {
    // Without initial value, first element is accumulator
    assert_eq!(num("[1,2,3,4].reduce((a,b) => a + b)"), 10.0);
}

// ── some / every ─────────────────────────────────────────────────────────────

#[test]
fn some_true_when_any_matches() {
    assert!(bool_("[1,2,3].some(x => x > 2)"));
    assert!(!bool_("[1,2,3].some(x => x > 10)"));
}

#[test]
fn every_true_when_all_match() {
    assert!(bool_("[2,4,6].every(x => x % 2 === 0)"));
    assert!(!bool_("[2,3,6].every(x => x % 2 === 0)"));
}

// ── flat / flatMap ────────────────────────────────────────────────────────────

#[test]
fn flat_one_level() {
    let mut e = JsEngine::new();
    e.eval("var f = [[1,2],[3,4]].flat();");
    let f = e.get_global("f");
    assert_eq!(f.get_property("length").to_number(), 4.0);
    assert_eq!(f.get_index(2).to_number(), 3.0);
}

#[test]
fn flat_map() {
    let mut e = JsEngine::new();
    e.eval("var fm = [1,2,3].flatMap(x => [x, x * 2]);");
    let fm = e.get_global("fm");
    // [1,2, 2,4, 3,6]
    assert_eq!(fm.get_property("length").to_number(), 6.0);
    assert_eq!(fm.get_index(3).to_number(), 4.0);
}

// ── fill ──────────────────────────────────────────────────────────────────────

#[test]
fn fill_all() {
    let mut e = JsEngine::new();
    e.eval("var a = [1,2,3,4]; a.fill(0);");
    assert_eq!(e.get_global("a").get_index(2).to_number(), 0.0);
}

#[test]
fn fill_range() {
    let mut e = JsEngine::new();
    e.eval("var a = [1,2,3,4,5]; a.fill(9, 1, 3);");
    let a = e.get_global("a");
    assert_eq!(a.get_index(0).to_number(), 1.0);
    assert_eq!(a.get_index(1).to_number(), 9.0);
    assert_eq!(a.get_index(3).to_number(), 4.0);
}

// ── spread in array literals ──────────────────────────────────────────────────

#[test]
fn spread_concat_arrays() {
    let mut e = JsEngine::new();
    e.eval("var a = [1,2]; var b = [3,4]; var c = [...a, ...b];");
    let c = e.get_global("c");
    assert_eq!(c.get_property("length").to_number(), 4.0);
    assert_eq!(c.get_index(2).to_number(), 3.0);
}

#[test]
fn spread_clone_array() {
    let mut e = JsEngine::new();
    e.eval("var orig = [1,2,3]; var copy = [...orig]; copy.push(4);");
    // Original must not be affected
    assert_eq!(e.get_global("orig").get_property("length").to_number(), 3.0);
    assert_eq!(e.get_global("copy").get_property("length").to_number(), 4.0);
}

// ── at() ─────────────────────────────────────────────────────────────────────

#[test]
fn at_positive() { assert_eq!(num("[10,20,30].at(1)"), 20.0); }
#[test]
fn at_negative() { assert_eq!(num("[10,20,30].at(-1)"), 30.0); }

// ── forEach ───────────────────────────────────────────────────────────────────

#[test]
fn for_each_sums() {
    let mut e = JsEngine::new();
    e.eval("var sum = 0; [1,2,3,4].forEach(function(x){ sum += x; });");
    assert_eq!(e.get_global("sum").to_number(), 10.0);
}
