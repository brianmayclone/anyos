fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| { eprintln!("Usage: parse_test <js-file>"); std::process::exit(1); });
    let source = std::fs::read_to_string(&path).expect("read failed");
    let mut engine = libjs::JsEngine::new();
    engine.set_step_limit(5_000_000);
    engine.eval(&source);
    if let Some(exc) = engine.last_exception() {
        let msg = exc.get_property("message"); let name = exc.get_property("name");
        eprintln!("EXCEPTION: {}: {}", name.to_js_string(), msg.to_js_string());
    }
    for line in engine.console_output() { eprintln!("console: {}", line); }
    for line in engine.vm().engine_log.iter() { eprintln!("engine: {}", line); }
}
