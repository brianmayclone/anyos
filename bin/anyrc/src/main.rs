#![no_std]
#![no_main]

use anyos_std::{String, Vec};
use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType, ExternCrateSpec};

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
    let mut crate_type = CrateType::Bin;
    let mut crate_name: Option<String> = None;
    let mut src_dir: Option<String> = None;
    let mut extern_crates: Vec<ExternCrateSpec> = Vec::new();
    let mut cfg_flags: Vec<String> = Vec::new();
    let mut linker_script: Option<String> = None;
    let mut link_args: Vec<String> = Vec::new();
    let mut env_vars: Vec<(String, String)> = Vec::new();
    let mut features: Vec<String> = Vec::new();

    let mut i = 0;
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
                        "rlib" => EmitKind::Rlib,
                        "mir" => EmitKind::Mir,
                        "hir" => EmitKind::Hir,
                        "asm" => EmitKind::Asm,
                        other => {
                            anyos_std::println!("crust: unknown emit type: {}", other);
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
            "--crate-type" => {
                i += 1;
                if i < args.len() {
                    crate_type = match args[i] {
                        "bin" => CrateType::Bin,
                        "lib" => CrateType::Lib,
                        "staticlib" => CrateType::StaticLib,
                        other => {
                            anyos_std::println!("crust: unknown crate type: {}", other);
                            anyos_std::process::exit(1);
                        }
                    };
                }
            }
            "--crate-name" => {
                i += 1;
                if i < args.len() {
                    crate_name = Some(String::from(args[i]));
                }
            }
            "--src-dir" => {
                i += 1;
                if i < args.len() {
                    src_dir = Some(String::from(args[i]));
                }
            }
            "--extern" => {
                // Format: --extern name=path.rlib
                i += 1;
                if i < args.len() {
                    if let Some(eq_pos) = args[i].find('=') {
                        let name = &args[i][..eq_pos];
                        let path = &args[i][eq_pos + 1..];
                        extern_crates.push(ExternCrateSpec {
                            name: String::from(name),
                            rlib_path: String::from(path),
                        });
                    } else {
                        anyos_std::println!("crust: --extern expects name=path.rlib");
                        anyos_std::process::exit(1);
                    }
                }
            }
            "--cfg" => {
                // --cfg 'target_arch="x86_64"' or --cfg feature="kunit"
                i += 1;
                if i < args.len() {
                    cfg_flags.push(String::from(args[i]));
                }
            }
            "-T" | "--linker-script" => {
                i += 1;
                if i < args.len() {
                    linker_script = Some(String::from(args[i]));
                }
            }
            "--link-arg" | "-l" => {
                i += 1;
                if i < args.len() {
                    link_args.push(String::from(args[i]));
                }
            }
            "--env" => {
                // --env KEY=VALUE
                i += 1;
                if i < args.len() {
                    if let Some(eq_pos) = args[i].find('=') {
                        let key = &args[i][..eq_pos];
                        let val = &args[i][eq_pos + 1..];
                        env_vars.push((String::from(key), String::from(val)));
                    }
                }
            }
            "--feature" => {
                i += 1;
                if i < args.len() {
                    features.push(String::from(args[i]));
                }
            }
            "--version" => {
                anyos_std::println!("crust 0.3.0");
                return;
            }
            "--help" | "-h" => {
                anyos_std::println!("Usage: crust [OPTIONS] <INPUT>");
                anyos_std::println!();
                anyos_std::println!("Options:");
                anyos_std::println!("  -o <FILE>              Output file (default: a.out)");
                anyos_std::println!("  --emit <TYPE>          Output type: exe, obj, rlib, mir, hir, asm");
                anyos_std::println!("  --opt-level <N>        Optimization level (0-3)");
                anyos_std::println!("  --crate-type <TYPE>    Crate type: bin, lib, staticlib");
                anyos_std::println!("  --crate-name <NAME>    Crate name");
                anyos_std::println!("  --src-dir <DIR>        Source directory for module resolution");
                anyos_std::println!("  --extern <name=path>   Link against extern crate .rlib");
                anyos_std::println!("  --cfg <SPEC>           Set cfg flag (e.g. target_arch=\"x86_64\")");
                anyos_std::println!("  -T <SCRIPT>            Linker script path");
                anyos_std::println!("  --link-arg <ARG>       Additional linker argument (.o file)");
                anyos_std::println!("  --env <KEY=VALUE>      Set compile-time environment variable");
                anyos_std::println!("  --feature <NAME>       Enable feature gate");
                anyos_std::println!("  --version              Print version");
                anyos_std::println!("  -h, --help             Print help");
                return;
            }
            arg if !arg.starts_with('-') => {
                input = Some(arg);
            }
            other => {
                anyos_std::println!("crust: unknown option: {}", other);
                anyos_std::process::exit(1);
            }
        }
        i += 1;
    }

    let input_file = match input {
        Some(f) => f,
        None => {
            anyos_std::println!("crust: error: no input file");
            anyos_std::process::exit(1);
        }
    };

    // Read source file
    let source = match read_file_to_string(input_file) {
        Some(s) => s,
        None => {
            anyos_std::println!("crust: error: cannot read {}", input_file);
            anyos_std::process::exit(1);
        }
    };

    let options = CompileOptions {
        input: String::from(input_file),
        output: String::from(output),
        emit,
        opt_level,
        crate_type,
        crate_name,
        src_dir,
        extern_crates,
        cfg_flags,
        linker_script,
        link_args,
        env_vars,
        features,
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
        anyos_std::println!("crust: error: cannot create {}", path);
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
