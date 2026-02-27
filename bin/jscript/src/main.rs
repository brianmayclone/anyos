//! jscript — Command-line JavaScript interpreter for anyOS.
//!
//! Executes JavaScript files using the libjs engine.
//!
//! # Usage
//! ```text
//! jscript <file.js>           Execute a JavaScript file
//! jscript -e "code"           Evaluate a JavaScript expression
//! ```

#![no_std]
#![no_main]

anyos_std::entry!(main);

use alloc::string::String;
use libjs::JsEngine;

/// Print usage information and exit.
fn usage() {
    anyos_std::println!("jscript - anyOS JavaScript interpreter");
    anyos_std::println!("");
    anyos_std::println!("Usage:");
    anyos_std::println!("  jscript <file.js>        Execute a JavaScript file");
    anyos_std::println!("  jscript -e \"code\"         Evaluate a JavaScript expression");
    anyos_std::println!("");
    anyos_std::println!("Options:");
    anyos_std::println!("  -e <code>   Evaluate code from argument");
    anyos_std::println!("  -p <code>   Evaluate and print the result");
}

/// Read a file and execute its contents as JavaScript.
fn run_file(engine: &mut JsEngine, path: &str) {
    let source = match anyos_std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(_) => {
            anyos_std::println!("jscript: cannot open '{}': No such file or directory", path);
            anyos_std::process::exit(1);
        }
    };

    engine.eval(&source);

    // Flush captured console output to stdout.
    for msg in engine.console_output() {
        anyos_std::println!("{}", msg);
    }
    engine.clear_console();
}

/// Evaluate a code string and optionally print the result.
fn run_string(engine: &mut JsEngine, code: &str, print_result: bool) {
    let result = engine.eval(code);

    // Flush captured console output to stdout.
    for msg in engine.console_output() {
        anyos_std::println!("{}", msg);
    }
    engine.clear_console();

    if print_result {
        let s = result.to_js_string();
        if s != "undefined" {
            anyos_std::println!("{}", s);
        }
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"ep");

    let eval_code = args.opt(b'e');
    let print_code = args.opt(b'p');

    if eval_code.is_none() && print_code.is_none() && args.pos_count == 0 {
        usage();
        return;
    }

    let mut engine = JsEngine::new();

    // Register native `print()` function as a convenient alias for console.log.
    engine.register_native("print", |vm, args| {
        let mut parts: alloc::vec::Vec<String> = alloc::vec::Vec::new();
        for arg in args {
            parts.push(arg.to_js_string());
        }
        let msg = parts.join(" ");
        vm.console_output.push(msg);
        libjs::JsValue::Undefined
    });

    // -e: evaluate expression.
    if let Some(code) = eval_code {
        run_string(&mut engine, code, false);
        return;
    }

    // -p: evaluate and print result.
    if let Some(code) = print_code {
        run_string(&mut engine, code, true);
        return;
    }

    // Execute file(s) passed as positional arguments.
    for i in 0..args.pos_count {
        run_file(&mut engine, args.positional[i]);
    }
}
