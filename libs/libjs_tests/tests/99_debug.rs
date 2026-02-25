use libjs_tests::{JsEngine, JsValueExt};
fn js(src: &str) -> libjs_tests::JsValue { JsEngine::new().eval(src) }
fn num(src: &str) -> f64 { js(src).to_number() }

#[test]
fn test_map_single_eval() {
    assert_eq!(num("[1,2,3].map(function(x){ return x * 2; })[0]"), 2.0);
}

#[test]  
fn test_global_var() {
    let mut e = JsEngine::new();
    e.eval("var x = 42;");
    assert_eq!(e.get_global("x").to_number(), 42.0);
}

#[test]
fn test_global_var_array() {
    let mut e = JsEngine::new();
    e.eval("var arr = [1,2,3];");
    assert_eq!(e.get_global("arr").get_index(0).to_number(), 1.0);
}

#[test]
fn test_global_var_after_map() {
    let mut e = JsEngine::new();
    e.eval("var doubled = [1,2,3].map(function(x){ return x * 2; });");
    let d = e.get_global("doubled");
    eprintln!("doubled type: {:?}", d);
    assert_eq!(d.get_index(0).to_number(), 2.0);
}
