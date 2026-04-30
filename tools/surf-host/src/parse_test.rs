// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: parse_test <js-file>");
        std::process::exit(1);
    });
    let source = std::fs::read_to_string(&path).expect("read failed");
    let mut engine = libjs::JsEngine::new();
    let step_limit = std::env::var("LIBJS_STEP_LIMIT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5_000_000);
    engine.set_step_limit(step_limit);
    engine.eval(&source);
    if let Some(exc) = engine.last_exception() {
        let msg = exc.get_property("message");
        let name = exc.get_property("name");
        eprintln!("EXCEPTION: {}: {}", name.to_js_string(), msg.to_js_string());
    }
    for line in engine.console_output() {
        eprintln!("console: {}", line);
    }
    for line in engine.vm().engine_log.iter() {
        eprintln!("engine: {}", line);
    }
}
