// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
use libjs::JsEngine;

fn main() {
    let path = std::env::args().nth(1).expect("Usage: crash_test <file.js>");
    let source = std::fs::read_to_string(&path).expect("file read");

    let mut e = JsEngine::new();
    e.set_step_limit(50_000_000);
    e.eval(&source);

    // Always print console output
    let console = e.console_output();
    if !console.is_empty() {
        for line in console {
            println!("[console] {}", line);
        }
    }
    // Print engine log (debug messages)
    for line in e.vm().engine_log.iter() {
        if line.contains("super") || line.contains("SuperCall") {
            println!("[engine-log] {}", line);
        }
    }
    println!("[engine] Execution complete");

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
