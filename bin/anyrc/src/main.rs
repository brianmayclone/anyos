#![no_std]
#![no_main]

use anyos_std::{String, Vec};
use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);

    // Split raw argument string into individual args
    let args: Vec<&str> = raw.split_whitespace().collect();

    let mut input = None;
    let mut output = "a.out";
    let mut emit = EmitKind::Exe;
    let mut opt_level = 0u32;

    let mut i = 1;
    while i < args.len() {
        match args[i] {
            "-o" => {
                i += 1;
                if i < args.len() {
                    output = args[i];
                }
            }
            "--emit" => {
                i += 1;
                if i < args.len() {
                    emit = match args[i] {
                        "exe" => EmitKind::Exe,
                        "obj" => EmitKind::Obj,
                        "mir" => EmitKind::Mir,
                        "hir" => EmitKind::Hir,
                        "asm" => EmitKind::Asm,
                        other => {
                            anyos_std::println!("anyrc: unknown emit type: {}", other);
                            anyos_std::process::exit(1);
                        }
                    };
                }
            }
            "--opt-level" | "-O" => {
                i += 1;
                if i < args.len() {
                    opt_level = parse_u32(args[i]);
                }
            }
            "--version" => {
                anyos_std::println!("anyrc 0.1.0");
                return;
            }
            "--help" | "-h" => {
                anyos_std::println!("Usage: anyrc [OPTIONS] <INPUT>");
                anyos_std::println!();
                anyos_std::println!("Options:");
                anyos_std::println!("  -o <FILE>          Output file (default: a.out)");
                anyos_std::println!("  --emit <TYPE>      Output type: exe, obj, mir, hir, asm");
                anyos_std::println!("  --opt-level <N>    Optimization level (0-3)");
                anyos_std::println!("  --version          Print version");
                anyos_std::println!("  -h, --help         Print help");
                return;
            }
            arg if !arg.starts_with('-') => {
                input = Some(arg);
            }
            other => {
                anyos_std::println!("anyrc: unknown option: {}", other);
                anyos_std::process::exit(1);
            }
        }
        i += 1;
    }

    let input_file = match input {
        Some(f) => f,
        None => {
            anyos_std::println!("anyrc: error: no input file");
            anyos_std::process::exit(1);
        }
    };

    // Read source file
    let source = match read_file_to_string(input_file) {
        Some(s) => s,
        None => {
            anyos_std::println!("anyrc: error: cannot read {}", input_file);
            anyos_std::process::exit(1);
        }
    };

    let options = CompileOptions {
        input: String::from(input_file),
        output: String::from(output),
        emit,
        opt_level,
        crate_type: CrateType::Bin,
        crate_name: None,
    };

    match compile(&source, input_file, &options) {
        Ok(bytes) => {
            write_file(output, &bytes);
        }
        Err(errors) => {
            let source_map = anyrc::diagnostics::SourceMap::new(
                String::from(input_file),
                source,
            );
            for err in &errors {
                anyos_std::println!("{}", err.render(&source_map));
            }
            anyos_std::process::exit(1);
        }
    }
}

fn read_file_to_string(path: &str) -> Option<String> {
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return None;
    }
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        data.extend_from_slice(&buf[..n as usize]);
    }
    anyos_std::fs::close(fd);
    String::from_utf8(data).ok()
}

fn write_file(path: &str, data: &[u8]) {
    let fd = anyos_std::fs::open(path, anyos_std::fs::O_WRITE | anyos_std::fs::O_CREATE | anyos_std::fs::O_TRUNC);
    if fd == u32::MAX {
        anyos_std::println!("anyrc: error: cannot create {}", path);
        anyos_std::process::exit(1);
    }
    anyos_std::fs::write(fd, data);
    anyos_std::fs::close(fd);
}

fn parse_u32(s: &str) -> u32 {
    let mut n = 0u32;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            n = n * 10 + (b - b'0') as u32;
        }
    }
    n
}
