use libjs::JsEngine;

fn main() {
    let path = std::env::args().nth(1).expect("Usage: minimal_test <file.js>");
    let source = std::fs::read_to_string(&path).expect("file read");

    // Run in a thread with 16MB stack (same as test262-runner)
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let mut e = JsEngine::new();
            e.set_step_limit(1_000_000);
            e.eval(&source);
            if let Some(exc) = e.last_exception() {
                let msg = match exc {
                    libjs::JsValue::Object(o) => {
                        let o = o.borrow();
                        format!("{}: {}", o.get("name").to_js_string(), o.get("message").to_js_string())
                    }
                    other => other.to_js_string(),
                };
                println!("EXCEPTION: {}", msg);
            } else {
                println!("OK");
            }
        })
        .unwrap();
    handle.join().unwrap();
}
