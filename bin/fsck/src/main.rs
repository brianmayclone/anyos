#![no_std]
#![no_main]

use anyos_std::{args, println, process, String};

anyos_std::entry!(main);

fn main() -> u32 {
    let mut buf = [0u8; 512];
    let raw = process::args(&mut buf);
    let argv = args::tokenize(raw);

    if argv.is_empty() || has_help(&argv) {
        usage();
        return 0;
    }

    let Some(fs_type) = fs_type_arg(&argv) else {
        println!("fsck: filesystem type required; use -t exfat or -t corefs");
        usage();
        return 1;
    };

    let target = match fs_type.as_str() {
        "exfat" | "fat" | "vfat" => "/System/sbin/fsck.exfat",
        "corefs" => "/System/sbin/fsck.corefs",
        other => {
            println!("fsck: unsupported filesystem type '{}'", other);
            usage();
            return 1;
        }
    };

    let forwarded = forward_args(&argv);
    let rc = process::exec(target, &forwarded);
    if rc == u32::MAX {
        println!("fsck: failed to exec {}", target);
        1
    } else {
        rc
    }
}

fn usage() {
    println!("Usage: fsck -t TYPE [checker options]");
    println!("Types: exfat, corefs");
    println!("Examples:");
    println!("  fsck -t exfat --device /dev/sda2");
    println!("  fsck -t corefs --device 2 --capacity 1073741824");
}

fn has_help(argv: &[String]) -> bool {
    argv.iter()
        .any(|arg| arg == "--help" || arg == "-h" || arg == "help")
}

fn fs_type_arg(argv: &[String]) -> Option<String> {
    let mut i = 0usize;
    while i < argv.len() {
        match argv[i].as_str() {
            "-t" | "--type" | "--fstype" => {
                if i + 1 < argv.len() {
                    return Some(argv[i + 1].clone());
                }
                return None;
            }
            arg if arg.starts_with("--type=") => return Some(String::from(&arg[7..])),
            arg if arg.starts_with("--fstype=") => return Some(String::from(&arg[9..])),
            "exfat" | "fat" | "vfat" | "corefs" => return Some(argv[i].clone()),
            _ => {}
        }
        i += 1;
    }
    None
}

fn forward_args(argv: &[String]) -> String {
    let mut out = String::new();
    let mut i = 0usize;
    while i < argv.len() {
        let arg = argv[i].as_str();
        if arg == "-t" || arg == "--type" || arg == "--fstype" {
            i += 2;
            continue;
        }
        if arg.starts_with("--type=") || arg.starts_with("--fstype=") {
            i += 1;
            continue;
        }
        if arg == "exfat" || arg == "fat" || arg == "vfat" || arg == "corefs" {
            i += 1;
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        append_shell_arg(&mut out, arg);
        i += 1;
    }
    out
}

fn append_shell_arg(out: &mut String, arg: &str) {
    if arg
        .bytes()
        .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
    {
        out.push_str(arg);
        return;
    }
    out.push('"');
    for b in arg.bytes() {
        if b == b'"' || b == b'\\' {
            out.push('\\');
        }
        out.push(b as char);
    }
    out.push('"');
}
