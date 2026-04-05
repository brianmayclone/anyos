use libjs::JsEngine;

fn main() {
    let path = std::env::args().nth(1).expect("Usage: crash_test <file.js>");
    let source = std::fs::read_to_string(&path).expect("file read");

    let mut e = JsEngine::new();
    e.set_step_limit(5_000_000);
    e.eval(&source);

    // Print console output
    for line in e.console_output() {
        println!("{}", line);
    }

    if let Some(exc) = e.last_exception() {
        let msg = match exc {
            libjs::JsValue::Object(o) => {
                let o = o.borrow();
                let name = o.get("name").to_js_string();
                let m = o.get("message").to_js_string();
                format!("{}: {}", name, m)
            },
            other => other.to_js_string(),
        };
        eprintln!("EXCEPTION: {}", msg);
        std::process::exit(1);
    }
}
