use libjs::JsEngine;
use std::fs;

fn run_test_file(path: &str, harness: &str) {
    let source = fs::read_to_string(path).expect("file read");
    let full = format!("{}\n{}", harness, source);

    let result = std::panic::catch_unwind(|| {
        let mut e = JsEngine::new();
        e.set_step_limit(5_000_000);
        e.eval(&full);
        let exc = e.last_exception().cloned();
        exc
    });

    match result {
        Ok(None) => println!("PASS  {}", path),
        Ok(Some(exc)) => {
            let msg = match &exc {
                libjs::JsValue::Object(o) => {
                    let o = o.borrow();
                    let m = o.get("message");
                    format!("{}", m.to_js_string())
                },
                other => other.to_js_string(),
            };
            println!("FAIL  {} => {}", path, msg);
        }
        Err(err) => {
            let msg = if let Some(s) = err.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = err.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "unknown panic".to_string()
            };
            println!("PANIC {} => {}", path, msg);
        }
    }
}

fn main() {
    let sta = fs::read_to_string("/tmp/test262-main/harness/sta.js").unwrap_or_default();
    let assert = fs::read_to_string("/tmp/test262-main/harness/assert.js").unwrap_or_default();
    let harness = format!("{}\n{}", sta, assert);

    let base = "/tmp/test262-main/test/language/expressions/addition";
    for name in &[
        "S11.6.1_A1.js",
        "coerce-symbol-to-prim-invocation.js",
        "coerce-symbol-to-prim-err.js",
        "coerce-symbol-to-prim-return-obj.js",
        "coerce-symbol-to-prim-return-prim.js",
        "get-symbol-to-prim-err.js",
        "order-of-evaluation.js",
        "symbol-to-string.js",
        "bigint-arithmetic.js",
        "bigint-errors.js",
        "bigint-and-number.js",
        "S11.6.1_A2.2_T1.js",
        "S11.6.1_A3.1_T1.1.js",
        "S11.6.1_A3.1_T1.2.js",
        "S11.6.1_A3.1_T2.1.js",
        "S11.6.1_A3.1_T2.2.js",
        "S11.6.1_A3.2_T1.1.js",
    ] {
        run_test_file(&format!("{}/{}", base, name), &harness);
    }

    let base2 = "/tmp/test262-main/test/language/expressions/array";
    for f in std::fs::read_dir(base2).unwrap().flatten() {
        let p = f.path();
        if p.extension().map(|e| e == "js").unwrap_or(false) {
            run_test_file(p.to_str().unwrap(), &harness);
        }
    }
}
