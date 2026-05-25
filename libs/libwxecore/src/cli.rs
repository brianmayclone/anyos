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
        Some("init") => init(&config, &argv[1..]),
        Some("repair") => repair(&config),
        Some("bootstrap-ms") => bootstrap_ms(&config, &argv[1..]),
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
    println!("  wxe init [--bootstrap-ms --accept-microsoft-licenses]");
    println!("  wxe repair");
    println!("  wxe bootstrap-ms --accept-microsoft-licenses");
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
    println!(
        "  default-cwd: {}{}",
        config.default_drive, config.default_cwd
    );
    println!("  comspec: {}", config.comspec);
    println!("  spawn-syscall: SYS_WXE_SPAWN");
    println!("  loader: PE32+ console tier, WXE DLL imports and console file APIs");
    println!(
        "  routes: {} NT services, {} DLL routes, {} UI routes, {} libanyui bindings",
        crate::rootfs::nt_service_count(),
        crate::rootfs::dll_route_count(),
        crate::rootfs::ui_route_count(),
        crate::rootfs::anyui_binding_count()
    );
    println!("  microsoft-payloads: explicit bootstrap/import only");
}

fn init(config: &WxeConfig, args: &[&str]) {
    if crate::rootfs::ensure_rootfs_layout(config) {
        log_ok!("wxe root ready at {}", config.root);
        log_ok!("drive C: mapped to {}", config.drive_c);
        log_ok!("ntdll, Win32, UI and libanyui binding manifests installed");
        log_ok!("generated WXE PE DLL payloads installed in System32");
        if has_arg(args, "--bootstrap-ms") {
            log_warn!("Microsoft payload bootstrap requested explicitly");
            let _ = crate::bootstrap::bootstrap_microsoft(
                config,
                has_arg(args, "--accept-microsoft-licenses"),
            );
        } else {
            log_warn!("wxe init does not download Microsoft binaries");
        }
    } else {
        log_error!("wxe init: root layout incomplete");
    }
}

fn repair(config: &WxeConfig) {
    init(config, &[]);
}

fn bootstrap_ms(config: &WxeConfig, args: &[&str]) {
    let _ =
        crate::bootstrap::bootstrap_microsoft(config, has_arg(args, "--accept-microsoft-licenses"));
}

fn inspect(config: &WxeConfig, args: &[&str]) {
    let Some(path) = args.first() else {
        log_error!("wxe inspect: missing PE path");
        return;
    };
    let native_path =
        resolve_windows_path(config, &config.default_drive, &config.default_cwd, path)
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
    let native_path =
        resolve_windows_path(config, &config.default_drive, &config.default_cwd, path)
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
        log_error!("wxe run: check PE subsystem, TLS callbacks, imports and WXE DLL status");
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
        log_warn!(
            "{} is not startable yet; using WXE bootstrap shell",
            config.comspec
        );
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
        } else if ascii_eq(cmd, "echo") {
            shell_echo(&tokens[1..]);
        } else if ascii_eq(cmd, "type") {
            shell_type(config, &drive, &cwd, &tokens[1..]);
        } else if ascii_eq(cmd, "copy") {
            shell_copy(config, &drive, &cwd, &tokens[1..]);
        } else if ascii_eq(cmd, "del") || ascii_eq(cmd, "erase") {
            shell_delete(config, &drive, &cwd, &tokens[1..]);
        } else if ascii_eq(cmd, "mkdir") || ascii_eq(cmd, "md") {
            shell_mkdir(config, &drive, &cwd, &tokens[1..]);
        } else if ascii_eq(cmd, "cls") {
            print!("\x1b[2J\x1b[H");
        } else if ascii_eq(cmd, "ver") {
            println!("WXE Command Shell [{}]", config.nt_profile);
        } else if ascii_eq(cmd, "path") {
            shell_path(config, &drive, &cwd);
        } else if ascii_eq(cmd, "where") {
            shell_where(config, &drive, &cwd, &tokens[1..]);
        } else if ascii_eq(cmd, "set") {
            shell_set(config, &drive, &cwd, &tokens[1..]);
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
        log_warn!(
            "wxe shell: found {}, but WXE could not start it yet",
            config.comspec
        );
        return false;
    }
    let code = process::waitpid(tid);
    println!("{} exited with {}", config.comspec, code);
    true
}

fn dlls(config: &WxeConfig) {
    println!("wxe dlls");
    println!("  system32: {}", config.system32());
    println!("  manifests:");
    println!(
        "    {} ({} services)",
        crate::rootfs::nt_services_path(config),
        crate::rootfs::nt_service_count()
    );
    println!(
        "    {} ({} routes)",
        crate::rootfs::dll_routes_path(config),
        crate::rootfs::dll_route_count()
    );
    println!(
        "    {} ({} UI routes)",
        crate::rootfs::ui_routes_path(config),
        crate::rootfs::ui_route_count()
    );
    println!(
        "    {} ({} libanyui bindings)",
        crate::rootfs::anyui_bindings_path(config),
        crate::rootfs::anyui_binding_count()
    );
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

fn import_ms(config: &WxeConfig, args: &[&str]) {
    let source = args.first().copied().unwrap_or("<missing>");
    if source == "bootstrap" || source == "sysinternals" || source == "edit" {
        let _ = crate::bootstrap::bootstrap_microsoft(
            config,
            has_arg(args, "--accept-microsoft-licenses"),
        );
        return;
    }
    println!("wxe import-ms {}", source);
    log_warn!("Microsoft payload import is planned but not implemented yet");
    log_warn!("imports will require explicit Microsoft license acceptance");
    log_warn!("wxe will not bundle, mirror, or silently download Microsoft OS binaries");
    println!("planned sources:");
    println!("  windows-media <path>     import from user-provided Windows install media");
    println!("  official-package <id>    fetch a Microsoft-published redistributable package");
    println!("  sysinternals <tool>      open/fetch from Microsoft's Sysinternals source");
}

fn has_arg(args: &[&str], needle: &str) -> bool {
    args.iter().any(|arg| *arg == needle)
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
    println!("  copy <src> <dst> copy a file");
    println!("  del <path>       delete a file");
    println!("  echo [text]      print text");
    println!("  mkdir <path>     create a directory");
    println!("  type <path>      print a file");
    println!("  cls              clear the screen");
    println!("  path             show WXE executable search path");
    println!("  where <cmd>      locate an executable");
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

fn shell_echo(args: &[String]) {
    for (idx, arg) in args.iter().enumerate() {
        if idx > 0 {
            print!(" ");
        }
        print!("{}", arg);
    }
    println!();
}

fn shell_type(config: &WxeConfig, drive: &str, cwd: &str, args: &[String]) {
    if args.is_empty() {
        log_error!("type: missing file");
        return;
    }
    for target in args {
        let Some(native) = resolve_windows_path(config, drive, cwd, target) else {
            log_error!("type: unsupported drive");
            continue;
        };
        match fs::read_to_vec(&native) {
            Ok(data) => match core::str::from_utf8(&data) {
                Ok(text) => print!("{}", text),
                Err(_) => {
                    for byte in data {
                        match byte {
                            b'\n' | b'\r' | b'\t' => print!("{}", byte as char),
                            b' '..=b'~' => print!("{}", byte as char),
                            _ => print!("."),
                        }
                    }
                    println!();
                }
            },
            Err(_) => log_error!("type: failed to read '{}'", native),
        }
    }
}

fn shell_copy(config: &WxeConfig, drive: &str, cwd: &str, args: &[String]) {
    if args.len() < 2 {
        log_error!("copy: usage: copy <source> <target>");
        return;
    }
    let Some(src) = resolve_windows_path(config, drive, cwd, &args[0]) else {
        log_error!("copy: unsupported source drive");
        return;
    };
    let Some(mut dst) = resolve_windows_path(config, drive, cwd, &args[1]) else {
        log_error!("copy: unsupported target drive");
        return;
    };
    if fs::read_dir(&dst).is_ok() {
        dst = join_native(&dst, windows_basename(&args[0]));
    }
    if copy_native_file(&src, &dst) {
        println!("        1 file(s) copied.");
    } else {
        log_error!("copy: failed to copy '{}' to '{}'", src, dst);
    }
}

fn shell_delete(config: &WxeConfig, drive: &str, cwd: &str, args: &[String]) {
    if args.is_empty() {
        log_error!("del: missing file");
        return;
    }
    for target in args {
        let Some(native) = resolve_windows_path(config, drive, cwd, target) else {
            log_error!("del: unsupported drive");
            continue;
        };
        if fs::unlink(&native) == 0 {
            println!("Deleted {}", target);
        } else {
            log_error!("del: failed to delete '{}'", native);
        }
    }
}

fn shell_mkdir(config: &WxeConfig, drive: &str, cwd: &str, args: &[String]) {
    if args.is_empty() {
        log_error!("mkdir: missing directory");
        return;
    }
    for target in args {
        let Some(native) = resolve_windows_path(config, drive, cwd, target) else {
            log_error!("mkdir: unsupported drive");
            continue;
        };
        if fs::mkdir(&native) != 0 && fs::read_dir(&native).is_err() {
            log_error!("mkdir: failed to create '{}'", native);
        }
    }
}

fn shell_path(config: &WxeConfig, drive: &str, cwd: &str) {
    println!("{}{}", drive, cwd);
    for path in executable_search_paths() {
        println!("{}", path);
    }
    println!("C:\\Program Files\\Sysinternals");
    println!("C:\\Program Files\\Microsoft\\Edit");
    println!("C:\\ProgramData\\WXE\\Installers");
    println!("System32 native: {}", config.system32());
}

fn shell_where(config: &WxeConfig, drive: &str, cwd: &str, args: &[String]) {
    if args.is_empty() {
        log_error!("where: missing command");
        return;
    }
    for target in args {
        match resolve_executable_path(config, drive, cwd, target) {
            Some(native) => println!("{}", native),
            None => log_error!("where: '{}' not found", target),
        }
    }
}

fn shell_set(config: &WxeConfig, drive: &str, cwd: &str, args: &[String]) {
    if args.is_empty() {
        println!("COMSPEC={}", config.comspec);
        println!("SystemDrive=C:");
        println!("SystemRoot=C:\\Windows");
        println!("USERPROFILE=C:\\Users\\Default");
        println!("CD={}{}", drive, cwd);
        print!("PATH={}{}", drive, cwd);
        for path in executable_search_paths() {
            print!(";C:{}", path);
        }
        println!();
        return;
    }
    log_warn!("set: environment mutation is not persisted yet");
    for arg in args {
        println!("{}", arg);
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
    let Some(native) = resolve_executable_path(config, drive, cwd, target) else {
        log_error!("run: '{}' not found", target);
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

fn resolve_executable_path(
    config: &WxeConfig,
    drive: &str,
    cwd: &str,
    target: &str,
) -> Option<String> {
    if has_path_component(target) {
        return first_existing_candidate(config, drive, cwd, target);
    }

    if let Some(native) = first_existing_candidate(config, drive, cwd, target) {
        return Some(native);
    }

    for search_cwd in executable_search_paths() {
        if let Some(native) = first_existing_candidate(config, "C:", search_cwd, target) {
            return Some(native);
        }
    }

    let file_name = executable_file_name(target);
    for search_cwd in recursive_executable_search_paths() {
        let Some(root) = resolve_windows_path(config, "C:", "\\", search_cwd) else {
            continue;
        };
        if let Some(native) = find_executable_recursive(&root, &file_name, 4) {
            return Some(native);
        }
    }
    None
}

fn first_existing_candidate(
    config: &WxeConfig,
    drive: &str,
    cwd: &str,
    target: &str,
) -> Option<String> {
    let mut candidate = String::from(target);
    if let Some(native) = existing_windows_path(config, drive, cwd, &candidate) {
        return Some(native);
    }
    if !has_extension(target) {
        candidate.push_str(".exe");
        return existing_windows_path(config, drive, cwd, &candidate);
    }
    None
}

fn existing_windows_path(
    config: &WxeConfig,
    drive: &str,
    cwd: &str,
    target: &str,
) -> Option<String> {
    let native = resolve_windows_path(config, drive, cwd, target)?;
    if crate::rootfs::path_exists(&native) {
        Some(native)
    } else {
        None
    }
}

fn executable_search_paths() -> &'static [&'static str] {
    &[
        "\\Windows\\System32",
        "\\Windows",
        "\\Program Files\\Sysinternals",
        "\\Program Files\\Microsoft\\Edit",
        "\\ProgramData\\WXE\\Installers",
    ]
}

fn recursive_executable_search_paths() -> &'static [&'static str] {
    &["\\Program Files\\Microsoft\\Edit"]
}

fn executable_file_name(target: &str) -> String {
    let base = windows_basename(target);
    if has_extension(base) {
        String::from(base)
    } else {
        alloc::format!("{}.exe", base)
    }
}

fn find_executable_recursive(native_dir: &str, file_name: &str, depth: u32) -> Option<String> {
    let Ok(mut entries) = fs::read_dir(native_dir) else {
        return None;
    };
    while let Some(entry) = entries.next() {
        let path = join_native(native_dir, &entry.name);
        if entry.file_type == 1 {
            if depth > 0 {
                if let Some(found) = find_executable_recursive(&path, file_name, depth - 1) {
                    return Some(found);
                }
            }
        } else if ascii_eq(&entry.name, file_name) {
            return Some(path);
        }
    }
    None
}

fn has_path_component(target: &str) -> bool {
    target.contains('\\') || target.contains('/') || drive_from_path(target).is_some()
}

fn has_extension(target: &str) -> bool {
    windows_basename(target).contains('.')
}

fn windows_basename(path: &str) -> &str {
    path.rsplit(|c| c == '\\' || c == '/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
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

fn join_native(base: &str, leaf: &str) -> String {
    if base.ends_with('/') {
        alloc::format!("{}{}", base, leaf)
    } else {
        alloc::format!("{}/{}", base, leaf)
    }
}

fn copy_native_file(src: &str, dst: &str) -> bool {
    let in_fd = fs::open(src, 0);
    if in_fd == u32::MAX {
        return false;
    }

    let out_fd = fs::open(dst, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if out_fd == u32::MAX {
        let _ = fs::close(in_fd);
        return false;
    }

    let mut ok = true;
    let mut buf = [0u8; 4096];
    loop {
        let n = fs::read(in_fd, &mut buf);
        if n == u32::MAX {
            ok = false;
            break;
        }
        if n == 0 {
            break;
        }
        let mut written = 0usize;
        while written < n as usize {
            let m = fs::write(out_fd, &buf[written..n as usize]);
            if m == u32::MAX || m == 0 {
                ok = false;
                break;
            }
            written += m as usize;
        }
        if !ok {
            break;
        }
    }

    let _ = fs::close(in_fd);
    let _ = fs::close(out_fd);
    ok
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
