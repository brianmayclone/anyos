use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{fs, print, println, process};

use crate::config::WxeConfig;

macro_rules! log_ok {
    ($($arg:tt)*) => {
        println!("[OK]\t{}", alloc::format!($($arg)*))
    };
}

macro_rules! log_warn {
    ($($arg:tt)*) => {
        println!("[WARN]\t{}", alloc::format!($($arg)*))
    };
}

macro_rules! log_error {
    ($($arg:tt)*) => {
        println!("[ERROR]\t{}", alloc::format!($($arg)*))
    };
}

macro_rules! log_fatal {
    ($($arg:tt)*) => {
        println!("[FATAL]\t{}", alloc::format!($($arg)*))
    };
}

pub fn run_cli(raw: &str) {
    let config = WxeConfig::load();
    let argv: Vec<&str> = raw.split_ascii_whitespace().collect();

    match argv.first().copied() {
        None | Some("help") | Some("--help") | Some("-h") => usage(),
        Some("status") => status(&config),
        Some("init") => init(&config),
        Some("repair") => repair(&config),
        Some("inspect") => inspect(&config, &argv[1..]),
        Some("run") => run(&config, &argv[1..]),
        Some("shell") => shell(&config, &argv[1..]),
        Some("dlls") => dlls(&config),
        Some("import-ms") => import_ms(&config, &argv[1..]),
        Some(cmd) => {
            log_error!("wxe: unknown command '{}'", cmd);
            usage();
        }
    }
}

fn usage() {
    println!("wxe - Windows Experience Extension");
    println!();
    println!("Usage:");
    println!("  wxe status");
    println!("  wxe init");
    println!("  wxe repair");
    println!("  wxe inspect <windows-pe>");
    println!("  wxe run <windows-pe> [args...]");
    println!("  wxe shell");
    println!("  wxe dlls");
    println!("  wxe import-ms <source>");
}

fn status(config: &WxeConfig) {
    println!("wxe status");
    println!("  abi: windows-x86_64 skeleton");
    println!("  nt-profile: {}", config.nt_profile);
    println!("  root: {}", config.root);
    println!("  drive-c: {}", config.drive_c);
    println!(
        "  drive-z: {}",
        if config.drive_z.is_empty() {
            "<disabled>"
        } else {
            config.drive_z.as_str()
        }
    );
    println!("  system32: {}", config.system32());
    println!("  default-cwd: {}{}", config.default_drive, config.default_cwd);
    println!("  comspec: {}", config.comspec);
    println!("  spawn-syscall: SYS_WXE_SPAWN");
    println!("  loader: PE32+ console tier, imports blocked until DLL routing lands");
    println!("  microsoft-payloads: user-import only, no silent download");
}

fn init(config: &WxeConfig) {
    if crate::rootfs::ensure_rootfs_layout(config) {
        log_ok!("wxe root ready at {}", config.root);
        log_ok!("drive C: mapped to {}", config.drive_c);
        log_warn!("wxe DLL payloads are planned but not generated yet");
        log_warn!("wxe init does not download Microsoft binaries");
    } else {
        log_error!("wxe init: root layout incomplete");
    }
}

fn repair(config: &WxeConfig) {
    init(config);
}

fn inspect(config: &WxeConfig, args: &[&str]) {
    let Some(path) = args.first() else {
        log_error!("wxe inspect: missing PE path");
        return;
    };
    let native_path = resolve_windows_path(config, &config.default_drive, &config.default_cwd, path)
        .unwrap_or_else(|| String::from(*path));
    match fs::read_to_vec(&native_path) {
        Ok(data) => {
            print!("{}", crate::pe::diagnose(&data));
        }
        Err(_) => log_error!("wxe inspect: failed to read '{}'", native_path),
    }
}

fn run(config: &WxeConfig, args: &[&str]) {
    let Some(path) = args.first() else {
        log_error!("wxe run: missing PE path");
        return;
    };
    let native_path = resolve_windows_path(config, &config.default_drive, &config.default_cwd, path)
        .unwrap_or_else(|| String::from(*path));
    if let Ok(data) = fs::read_to_vec(&native_path) {
        print!("{}", crate::pe::diagnose(&data));
    } else {
        log_warn!("wxe run: could not inspect '{}'", native_path);
    }

    let child_args = join_args(&args[1..]);
    let tid = process::wxe_spawn(&native_path, &child_args);
    if tid == u32::MAX {
        log_fatal!("wxe run: failed to start '{}'", native_path);
        log_error!("wxe run: current tier rejects imported DLLs/TLS until DLL routing lands");
        return;
    }

    let code = process::waitpid(tid);
    println!("wxe run: process {} exited with {}", tid, code);
}

fn shell(config: &WxeConfig, args: &[&str]) {
    let _ = crate::rootfs::ensure_rootfs_layout(config);
    let force_builtin = args.iter().any(|arg| *arg == "--builtin");
    if !force_builtin && start_comspec(config) {
        return;
    }
    if !force_builtin {
        log_warn!("{} is not startable yet; using WXE bootstrap shell", config.comspec);
    }

    let mut drive = if config.default_drive.is_empty() {
        String::from("C:")
    } else {
        config.default_drive.clone()
    };
    let mut cwd = if config.default_cwd.is_empty() {
        String::from("\\")
    } else {
        normalize_windows_cwd("\\", &config.default_cwd)
    };

    println!("WXE Shell [{}]", config.nt_profile);
    println!("Type 'help' for builtins, 'exit' to leave.");

    let mut line_buf = [0u8; 512];
    loop {
        print!("{}{}> ", drive, cwd);
        let len = read_line(&mut line_buf);
        if len == 0 {
            continue;
        }
        let line = core::str::from_utf8(&line_buf[..len]).unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let tokens = anyos_std::shell::tokenize(line);
        if tokens.is_empty() {
            continue;
        }
        let cmd = tokens[0].as_str();

        if ascii_eq(cmd, "exit") {
            break;
        } else if ascii_eq(cmd, "help") {
            shell_help();
        } else if is_drive_switch(cmd) && tokens.len() == 1 {
            let next_drive = normalize_drive(cmd);
            if drive_root(config, &next_drive).is_some() {
                drive = next_drive;
            } else {
                log_error!("drive {} is not mapped", cmd);
            }
        } else if ascii_eq(cmd, "cd") || ascii_eq(cmd, "chdir") {
            if tokens.len() == 1 {
                println!("{}{}", drive, cwd);
            } else {
                shell_cd(config, &mut drive, &mut cwd, tokens[1].as_str());
            }
        } else if ascii_eq(cmd, "pwd") {
            println!("{}{}", drive, cwd);
        } else if ascii_eq(cmd, "dir") {
            let target = tokens.get(1).map(|s| s.as_str()).unwrap_or(".");
            shell_dir(config, &drive, &cwd, target);
        } else if ascii_eq(cmd, "inspect") {
            if let Some(target) = tokens.get(1) {
                shell_inspect(config, &drive, &cwd, target);
            } else {
                log_error!("inspect: missing PE path");
            }
        } else if ascii_eq(cmd, "run") {
            if let Some(target) = tokens.get(1) {
                shell_run(config, &drive, &cwd, target, &tokens[2..]);
            } else {
                log_error!("run: missing PE path");
            }
        } else {
            shell_run(config, &drive, &cwd, cmd, &tokens[1..]);
        }
    }
}

fn start_comspec(config: &WxeConfig) -> bool {
    let Some(native_path) = resolve_windows_path(config, "C:", "\\", &config.comspec) else {
        return false;
    };
    if fs::read_to_vec(&native_path).is_err() {
        return false;
    }

    let tid = process::wxe_spawn(&native_path, "");
    if tid == u32::MAX {
        log_warn!("wxe shell: found {}, but WXE could not start it yet", config.comspec);
        return false;
    }
    let code = process::waitpid(tid);
    println!("{} exited with {}", config.comspec, code);
    true
}

fn dlls(config: &WxeConfig) {
    println!("wxe dlls");
    println!("  system32: {}", config.system32());
    for dll in crate::rootfs::expected_dlls() {
        let path = crate::rootfs::installed_dll_path(config, dll);
        let state = if crate::rootfs::path_exists(&path) {
            "installed"
        } else {
            "missing"
        };
        println!("  {:<44} {}", dll, state);
    }
}

fn import_ms(_config: &WxeConfig, args: &[&str]) {
    let source = args.first().copied().unwrap_or("<missing>");
    println!("wxe import-ms {}", source);
    log_warn!("Microsoft payload import is planned but not implemented yet");
    log_warn!("imports will require explicit Microsoft license acceptance");
    log_warn!("wxe will not bundle, mirror, or silently download Microsoft OS binaries");
    println!("planned sources:");
    println!("  windows-media <path>     import from user-provided Windows install media");
    println!("  official-package <id>    fetch a Microsoft-published redistributable package");
    println!("  sysinternals <tool>      open/fetch from Microsoft's Sysinternals source");
}

fn join_args(args: &[&str]) -> String {
    let mut out = String::new();
    for (idx, arg) in args.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        out.push_str(arg);
    }
    out
}

fn join_string_args(args: &[String]) -> String {
    let mut out = String::new();
    for (idx, arg) in args.iter().enumerate() {
        if idx > 0 {
            out.push(' ');
        }
        out.push_str(arg);
    }
    out
}

fn shell_help() {
    println!("Builtins:");
    println!("  cd <path>        change WXE directory");
    println!("  C:               switch to drive C");
    println!("  dir [path]       list a WXE directory");
    println!("  inspect <exe>    inspect a PE32+ image");
    println!("  run <exe> [args] run a PE32+ console image");
    println!("  exit             leave WXE shell");
}

fn shell_cd(config: &WxeConfig, drive: &mut String, cwd: &mut String, target: &str) {
    let next_drive = drive_from_path(target).unwrap_or_else(|| drive.clone());
    if drive_root(config, &next_drive).is_none() {
        log_error!("cd: drive {} is not mapped", next_drive);
        return;
    }
    let next_cwd = normalize_windows_cwd(cwd, strip_drive(target));
    let native = resolve_windows_path(config, &next_drive, "\\", &next_cwd);
    if let Some(path) = native {
        if fs::read_dir(&path).is_err() {
            log_error!("cd: directory not found: {}{}", next_drive, next_cwd);
            return;
        }
        *drive = next_drive;
        *cwd = next_cwd;
    } else {
        log_error!("cd: unsupported drive");
    }
}

fn shell_dir(config: &WxeConfig, drive: &str, cwd: &str, target: &str) {
    let Some(native) = resolve_windows_path(config, drive, cwd, target) else {
        log_error!("dir: unsupported drive");
        return;
    };
    match fs::read_dir(&native) {
        Ok(mut entries) => {
            println!(" Directory of {}", display_windows_path(drive, cwd, target));
            while let Some(entry) = entries.next() {
                if entry.file_type == 1 {
                    println!("  <DIR>      {}", entry.name);
                } else {
                    println!("  {:>8}  {}", entry.size, entry.name);
                }
            }
        }
        Err(_) => log_error!("dir: failed to read '{}'", native),
    }
}

fn shell_inspect(config: &WxeConfig, drive: &str, cwd: &str, target: &str) {
    let Some(native) = resolve_windows_path(config, drive, cwd, target) else {
        log_error!("inspect: unsupported drive");
        return;
    };
    match fs::read_to_vec(&native) {
        Ok(data) => print!("{}", crate::pe::diagnose(&data)),
        Err(_) => log_error!("inspect: failed to read '{}'", native),
    }
}

fn shell_run(config: &WxeConfig, drive: &str, cwd: &str, target: &str, args: &[String]) {
    let executable = executable_name(target);
    let Some(native) = resolve_windows_path(config, drive, cwd, &executable) else {
        log_error!("run: unsupported drive");
        return;
    };
    let child_args = join_string_args(args);
    let tid = process::wxe_spawn(&native, &child_args);
    if tid == u32::MAX {
        log_error!("run: failed to start '{}'", native);
        return;
    }
    let code = process::waitpid(tid);
    println!("process {} exited with {}", tid, code);
}

fn executable_name(target: &str) -> String {
    if target.contains('.')
        || target.contains('\\')
        || target.contains('/')
        || target.as_bytes().get(1) == Some(&b':')
    {
        String::from(target)
    } else {
        alloc::format!("{}.exe", target)
    }
}

fn display_windows_path(drive: &str, cwd: &str, target: &str) -> String {
    if target == "." || target.is_empty() {
        alloc::format!("{}{}", drive, cwd)
    } else if is_absolute_windows_path(target) || drive_from_path(target).is_some() {
        String::from(target)
    } else if cwd == "\\" {
        alloc::format!("{}\\{}", drive, target)
    } else {
        alloc::format!("{}{}\\{}", drive, cwd, target)
    }
}

fn resolve_windows_path(
    config: &WxeConfig,
    current_drive: &str,
    current_cwd: &str,
    raw: &str,
) -> Option<String> {
    if raw.starts_with('/') {
        return Some(String::from(raw));
    }

    let drive = drive_from_path(raw).unwrap_or_else(|| normalize_drive(current_drive));
    let root = drive_root(config, &drive)?;
    let without_drive = strip_drive(raw);
    let win_path = if is_absolute_windows_path(without_drive) {
        normalize_windows_cwd("\\", without_drive)
    } else {
        normalize_windows_cwd(current_cwd, without_drive)
    };

    let mut native = String::from(root);
    if !native.ends_with('/') {
        native.push('/');
    }
    for (idx, part) in win_path
        .split(|c| c == '\\' || c == '/')
        .filter(|part| !part.is_empty())
        .enumerate()
    {
        if idx > 0 {
            native.push('/');
        }
        native.push_str(part);
    }
    Some(native)
}

fn normalize_windows_cwd(current: &str, raw: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !is_absolute_windows_path(raw) {
        push_path_parts(&mut parts, current);
    }
    push_path_parts(&mut parts, raw);

    if parts.is_empty() {
        return String::from("\\");
    }
    let mut out = String::from("\\");
    for (idx, part) in parts.iter().enumerate() {
        if idx > 0 {
            out.push('\\');
        }
        out.push_str(part);
    }
    out
}

fn push_path_parts(parts: &mut Vec<String>, raw: &str) {
    for part in strip_drive(raw).split(|c| c == '\\' || c == '/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            parts.pop();
        } else {
            parts.push(String::from(part));
        }
    }
}

fn drive_root<'a>(config: &'a WxeConfig, drive: &str) -> Option<&'a str> {
    if ascii_eq(drive, "C:") {
        Some(config.drive_c.as_str())
    } else if ascii_eq(drive, "Z:") && !config.drive_z.is_empty() {
        Some(config.drive_z.as_str())
    } else {
        None
    }
}

fn drive_from_path(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        Some(normalize_drive(&raw[..2]))
    } else {
        None
    }
}

fn strip_drive(raw: &str) -> &str {
    let bytes = raw.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        &raw[2..]
    } else {
        raw
    }
}

fn normalize_drive(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.is_empty() {
        return String::from("C:");
    }
    let letter = bytes[0].to_ascii_uppercase() as char;
    let mut out = String::new();
    out.push(letter);
    out.push(':');
    out
}

fn is_drive_switch(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.len() == 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

fn is_absolute_windows_path(raw: &str) -> bool {
    raw.starts_with('\\') || raw.starts_with('/')
}

fn ascii_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn read_line(buf: &mut [u8]) -> usize {
    let mut pos = 0usize;
    loop {
        let mut byte = [0u8; 1];
        let n = fs::read(0, &mut byte);
        if n == 0 {
            process::sleep(10);
            continue;
        }
        if n == u32::MAX {
            break;
        }
        match byte[0] {
            b'\n' | b'\r' => {
                print!("\n");
                break;
            }
            8 | 127 => {
                if pos > 0 {
                    pos -= 1;
                    print!("\x08 \x08");
                }
            }
            c if c >= b' ' => {
                if pos < buf.len() {
                    buf[pos] = c;
                    pos += 1;
                    print!("{}", c as char);
                }
            }
            _ => {}
        }
    }
    pos
}
