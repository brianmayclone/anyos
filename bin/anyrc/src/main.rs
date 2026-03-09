use anyrc::driver::{compile, CompileOptions, EmitKind, CrateType};

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let mut input = None;
    let mut output = String::from("a.out");
    let mut emit = EmitKind::Exe;
    let mut opt_level = 0u32;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                if i < args.len() {
                    output = args[i].clone();
                }
            }
            "--emit" => {
                i += 1;
                if i < args.len() {
                    emit = match args[i].as_str() {
                        "exe" => EmitKind::Exe,
                        "obj" => EmitKind::Obj,
                        "mir" => EmitKind::Mir,
                        "hir" => EmitKind::Hir,
                        "asm" => EmitKind::Asm,
                        other => {
                            eprintln!("anyrc: unknown emit type: {}", other);
                            std::process::exit(1);
                        }
                    };
                }
            }
            "--opt-level" | "-O" => {
                i += 1;
                if i < args.len() {
                    opt_level = args[i].parse().unwrap_or(0);
                }
            }
            "--version" => {
                println!("anyrc 0.1.0");
                return;
            }
            "--help" | "-h" => {
                println!("Usage: anyrc [OPTIONS] <INPUT>");
                println!();
                println!("Options:");
                println!("  -o <FILE>          Output file (default: a.out)");
                println!("  --emit <TYPE>      Output type: exe, obj, mir, hir, asm");
                println!("  --opt-level <N>    Optimization level (0-3)");
                println!("  --version          Print version");
                println!("  -h, --help         Print help");
                return;
            }
            arg if !arg.starts_with('-') => {
                input = Some(arg.to_string());
            }
            other => {
                eprintln!("anyrc: unknown option: {}", other);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let input_file = input.unwrap_or_else(|| {
        eprintln!("anyrc: error: no input file");
        std::process::exit(1);
    });

    let source = std::fs::read_to_string(&input_file).unwrap_or_else(|e| {
        eprintln!("anyrc: error: {}: {}", input_file, e);
        std::process::exit(1);
    });

    let options = CompileOptions {
        input: input_file.clone(),
        output: output.clone(),
        emit,
        opt_level,
        crate_type: CrateType::Bin,
        crate_name: None,
    };

    match compile(&source, &input_file, &options) {
        Ok(bytes) => {
            std::fs::write(&output, &bytes).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = std::fs::metadata(&output).unwrap().permissions();
                perms.set_mode(0o755);
                std::fs::set_permissions(&output, perms).unwrap();
            }
        }
        Err(errors) => {
            let source_map = anyrc::diagnostics::SourceMap::new(input_file, source);
            for err in &errors {
                eprintln!("{}", err.render(&source_map));
            }
            std::process::exit(1);
        }
    }
}
