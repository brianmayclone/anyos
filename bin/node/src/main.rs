#![cfg_attr(not(feature = "host"), no_std)]
#![cfg_attr(not(feature = "host"), no_main)]

use alloc::string::String;
use alloc::vec::Vec;

mod cli;

#[cfg(feature = "host")]
extern crate alloc;

anyos_std::entry!(node_main);

fn node_main() -> u32 {
    let mut args_buf = [0u8; 1024];
    let raw = anyos_std::process::args(&mut args_buf);
    let cli = match cli::parse(raw) {
        Ok(cli) => cli,
        Err(err) => {
            anyos_std::println!("node: {}", err);
            return 9;
        }
    };

    let mut options = libnode::NodeOptions::default();
    options.exec_argv = cli.exec_argv.clone();
    options.cwd = current_dir();
    options.argv = argv_for(&cli.mode, &cli.argv_tail);

    let mut runtime = libnode::NodeRuntime::new(options);
    for preload in &cli.preloads {
        runtime.eval(&alloc::format!("require({:?});", preload));
    }

    match &cli.mode {
        cli::NodeMode::Version => {
            anyos_std::println!("v{}", libnode::VERSION);
            return 0;
        }
        cli::NodeMode::Help => {
            print_usage();
            return 0;
        }
        cli::NodeMode::Eval { source, print } => {
            let value = runtime.eval(source);
            if finish_runtime(&mut runtime) != 0 {
                return 1;
            }
            if *print && !matches!(value, libjs::JsValue::Undefined) {
                anyos_std::println!("{}", value.to_js_string());
            }
        }
        cli::NodeMode::Check { script } => {
            let source = match anyos_std::fs::read_to_string(script) {
                Ok(source) => source,
                Err(_) => {
                    anyos_std::println!("node: Could not read script");
                    return 1;
                }
            };
            let value = runtime.run_script(script, &source);
            if let Some(exception) = runtime.engine().last_exception() {
                anyos_std::println!("{}", exception.to_js_string());
                return 1;
            }
            let _ = value;
        }
        cli::NodeMode::Script { script } => match runtime.run_file(script) {
            Ok(_) => {
                if finish_runtime(&mut runtime) != 0 {
                    return 1;
                }
            }
            Err(err) => {
                anyos_std::println!("node: {}", err);
                return 1;
            }
        },
        cli::NodeMode::Stdin => {
            let source = read_stdin_to_string();
            runtime.run_script("[stdin]", &source);
            if finish_runtime(&mut runtime) != 0 {
                return 1;
            }
        }
        cli::NodeMode::Repl => run_repl(&mut runtime),
    }
    0
}

fn finish_runtime(runtime: &mut libnode::NodeRuntime) -> u32 {
    runtime.run_event_loop();
    flush_console(runtime);
    if let Some(exception) = runtime.engine().last_exception() {
        anyos_std::println!("{}", exception.to_js_string());
        return 1;
    }
    0
}

fn flush_console(runtime: &mut libnode::NodeRuntime) {
    for msg in runtime.engine().console_output() {
        anyos_std::println!("{}", msg);
    }
    runtime.engine().clear_console();
}

fn run_repl(runtime: &mut libnode::NodeRuntime) {
    let stdin = read_stdin_to_string();
    anyos_std::println!("Welcome to anyOS Node.js v{}.", libnode::VERSION);
    if stdin.trim().is_empty() {
        anyos_std::println!("Type .exit to leave the REPL.");
        anyos_std::print!("> ");
        return;
    }
    for line in stdin.lines() {
        let line = line.trim();
        if line == ".exit" || line == ".quit" {
            break;
        }
        if line.is_empty() {
            continue;
        }
        let value = runtime.eval(line);
        runtime.run_event_loop();
        flush_console(runtime);
        if let Some(exception) = runtime.engine().last_exception() {
            anyos_std::println!("{}", exception.to_js_string());
            runtime.engine().clear_last_exception();
        } else if !matches!(value, libjs::JsValue::Undefined) {
            anyos_std::println!("{}", value.to_js_string());
        }
    }
}

fn argv_for(mode: &cli::NodeMode, tail: &[String]) -> Vec<String> {
    let mut argv = Vec::new();
    argv.push(String::from("node"));
    match mode {
        cli::NodeMode::Script { script } | cli::NodeMode::Check { script } => {
            argv.push(script.clone());
        }
        _ => {}
    }
    argv.extend(tail.iter().cloned());
    argv
}

fn current_dir() -> String {
    let mut buf = [0u8; 512];
    let len = anyos_std::fs::getcwd(&mut buf);
    if len == u32::MAX {
        return String::from(".");
    }
    let len = (len as usize).min(buf.len());
    String::from(core::str::from_utf8(&buf[..len]).unwrap_or("."))
}

#[cfg(feature = "host")]
fn read_stdin_to_string() -> String {
    use std::io::Read;
    let mut out = String::new();
    let _ = std::io::stdin().read_to_string(&mut out);
    out
}

#[cfg(not(feature = "host"))]
fn read_stdin_to_string() -> String {
    let mut out = String::new();
    let mut buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(0, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
            out.push_str(text);
        }
    }
    out
}

fn print_usage() {
    anyos_std::println!("node {}", libnode::VERSION);
    anyos_std::println!("Usage:");
    anyos_std::println!("  node [options] [script.js] [arguments]");
    anyos_std::println!("  node");
    anyos_std::println!("  node -e, --eval <source>");
    anyos_std::println!("  node -p, --print <source>");
    anyos_std::println!("  node -r, --require <module>");
    anyos_std::println!("  node -v, --version");
}
