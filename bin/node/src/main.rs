#![cfg_attr(not(feature = "host"), no_std)]
#![cfg_attr(not(feature = "host"), no_main)]

use alloc::string::String;
use alloc::vec::Vec;

#[cfg(feature = "host")]
extern crate alloc;

anyos_std::entry!(node_main);

fn node_main() {
    let mut args_buf = [0u8; 1024];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"e");
    if args.pos_count == 0 && args.opt(b'e').is_none() && !raw.contains("--version") {
        print_usage();
        return;
    }

    let mut options = libnode::NodeOptions::default();
    options.argv = argv_from_raw(raw);
    options.cwd = String::from(".");

    let mut runtime = libnode::NodeRuntime::new(options);
    let result = if raw.contains("--version") || args.has(b'v') {
        anyos_std::println!("v{}", libnode::VERSION);
        return;
    } else if let Some(code) = args.opt(b'e') {
        runtime.eval(code)
    } else {
        match runtime.run_file(args.positional[0]) {
            Ok(value) => value,
            Err(err) => {
                anyos_std::println!("node: {}", err);
                return;
            }
        }
    };

    runtime.run_event_loop();
    for msg in runtime.engine().console_output() {
        anyos_std::println!("{}", msg);
    }
    runtime.engine().clear_console();

    if let Some(exception) = runtime.engine().last_exception() {
        anyos_std::println!("{}", exception.to_js_string());
    } else if !matches!(result, libjs::JsValue::Undefined) {
        anyos_std::println!("{}", result.to_js_string());
    }
}

fn argv_from_raw(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split_whitespace() {
        out.push(String::from(part));
    }
    out
}

fn print_usage() {
    anyos_std::println!("node {}", libnode::VERSION);
    anyos_std::println!("Usage:");
    anyos_std::println!("  node <file.js>");
    anyos_std::println!("  node -e <source>");
    anyos_std::println!("  node --version");
}
