#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;

anyos_std::entry!(main);

/// Service configuration directory.
const SVC_CONFIG_DIR: &str = "/System/etc/svc";

/// Thread list entry size from sysinfo cmd=1.
const THREAD_ENTRY_SIZE: usize = 60;
/// Max threads to query.
const MAX_THREADS: usize = 256;

/// Max dependency chain depth to prevent circular dependencies.
const MAX_DEPEND_DEPTH: usize = 8;

/// Parsed service configuration from `/System/etc/svc/<name>`.
struct ServiceConfig {
    exec: String,
    args: String,
    depends: String,
}

/// Read and parse a service config file.
/// Format: key=value lines, supports `exec=`, `args=`, and `depends=`.
fn read_config(name: &str) -> Option<ServiceConfig> {
    let path = format!("{}/{}", SVC_CONFIG_DIR, name);
    let content = anyos_std::fs::read_to_string(&path).ok()?;
    let mut exec = String::new();
    let mut args = String::new();
    let mut depends = String::new();
    for line in content.split('\n') {
        let line = line.trim();
        if line.is_empty() { continue; }
        if let Some(val) = line.strip_prefix("exec=") {
            exec = String::from(val.trim());
        } else if let Some(val) = line.strip_prefix("args=") {
            args = String::from(val.trim());
        } else if let Some(val) = line.strip_prefix("depends=") {
            depends = String::from(val.trim());
        }
    }
    if exec.is_empty() {
        return None;
    }
    Some(ServiceConfig { exec, args, depends })
}

/// Ensure all dependencies for a service are running.
/// Recursively starts dependencies up to MAX_DEPEND_DEPTH levels.
/// Returns false if a dependency could not be started.
fn ensure_dependencies(name: &str, depth: usize) -> bool {
    if depth >= MAX_DEPEND_DEPTH {
        anyos_std::println!("svc: dependency chain too deep at '{}' (circular?)", name);
        return false;
    }
    let config = match read_config(name) {
        Some(c) => c,
        None => return true, // no config = no deps to check
    };
    if config.depends.is_empty() {
        return true;
    }
    for dep in config.depends.split(',') {
        let dep = dep.trim();
        if dep.is_empty() { continue; }
        // Recursively ensure this dependency's own dependencies first
        if !ensure_dependencies(dep, depth + 1) {
            return false;
        }
        // Start the dependency if not already running
        if find_thread_by_name(dep) == 0 {
            anyos_std::println!("{}: starting dependency '{}'", name, dep);
            start_service(dep, "");
            if find_thread_by_name(dep) == 0 {
                anyos_std::println!("svc: failed to start dependency '{}' for '{}'", dep, name);
                return false;
            }
        }
    }
    true
}

/// Start a service by name (used by both cmd_start and dependency resolution).
fn start_service(name: &str, extra_args: &str) {
    let config = match read_config(name) {
        Some(c) => c,
        None => {
            anyos_std::println!("svc: unknown service '{}' (no config in {})", name, SVC_CONFIG_DIR);
            return;
        }
    };

    // Build args: binary path + config args + command-line extra args
    let mut args = String::from(config.exec.as_str());
    if !config.args.is_empty() {
        args.push(' ');
        args.push_str(&config.args);
    }
    if !extra_args.is_empty() {
        args.push(' ');
        args.push_str(extra_args);
    }

    let tid = anyos_std::process::spawn(&config.exec, &args);
    if tid == 0 || tid == u32::MAX {
        anyos_std::println!("svc: failed to start {} ({})", name, config.exec);
        return;
    }

    anyos_std::println!("{}: started (TID {})", name, tid);
}

/// Find a live thread by name via sysinfo thread listing.
/// Returns the TID of the first non-terminated thread whose name matches,
/// or 0 if no match is found.
fn find_thread_by_name(name: &str) -> u32 {
    let mut buf = [0u8; THREAD_ENTRY_SIZE * MAX_THREADS];
    let count = anyos_std::sys::sysinfo(1, &mut buf) as usize;
    let name_bytes = name.as_bytes();
    for i in 0..count {
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() {
            break;
        }
        // Thread name at offset 8, null-terminated, max 23 bytes
        let name_start = off + 8;
        let mut len = 0;
        for j in 0..23 {
            if buf[name_start + j] == 0 { break; }
            len += 1;
        }
        if len == name_bytes.len() && &buf[name_start..name_start + len] == name_bytes {
            let state = buf[off + 5];
            // 0=ready, 1=running, 2=blocked — skip 3=dead
            if state <= 2 {
                return u32::from_le_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]]);
            }
        }
    }
    0
}

/// Read all service names from the config directory.
/// Calls `f` for each valid service name.
fn for_each_service(mut f: impl FnMut(&str)) {
    let mut buf = [0u8; 8192];
    let count = anyos_std::fs::readdir(SVC_CONFIG_DIR, &mut buf);
    // readdir entries: 64 bytes each [type:u8, name_len:u8, flags:u8, pad:u8, size:u32, name:56]
    for i in 0..count as usize {
        let off = i * 64;
        if off + 64 > buf.len() { break; }
        let entry_type = buf[off];
        if entry_type != 0 { continue; } // 0 = regular file
        let name_len = buf[off + 1] as usize;
        let name_start = off + 8;
        if name_len == 0 || name_start + name_len > buf.len() { continue; }
        if let Ok(name) = core::str::from_utf8(&buf[name_start..name_start + name_len]) {
            if !name.starts_with('.') {
                f(name);
            }
        }
    }
}

/// List available services from the config directory.
fn cmd_list() {
    let mut found = false;
    anyos_std::println!("{:<16} {:<8} {}", "SERVICE", "STATUS", "EXEC");
    anyos_std::println!("{:<16} {:<8} {}", "-------", "------", "----");
    for_each_service(|name| {
        found = true;
        let tid = find_thread_by_name(name);
        let status = if tid != 0 { "running" } else { "stopped" };
        let exec_str = match read_config(name) {
            Some(cfg) => cfg.exec,
            None => String::from("(invalid config)"),
        };
        anyos_std::println!("{:<16} {:<8} {}", name, status, exec_str);
    });
    if !found {
        anyos_std::println!("No services configured in {}", SVC_CONFIG_DIR);
    }
}

/// Start all configured services that are not already running.
fn cmd_start_all() {
    let mut started = 0u32;
    let mut already = 0u32;
    // Collect names first to avoid borrowing issues with the readdir buffer
    let mut names = alloc::vec::Vec::new();
    for_each_service(|name| {
        names.push(String::from(name));
    });
    for name in &names {
        if find_thread_by_name(name) != 0 {
            already += 1;
            continue;
        }
        if !ensure_dependencies(name, 0) {
            continue;
        }
        start_service(name, "");
        if find_thread_by_name(name) != 0 {
            started += 1;
        }
    }
    anyos_std::println!("svc: {} started, {} already running", started, already);
}

fn cmd_start(name: &str, extra_args: &str) {
    if read_config(name).is_none() {
        anyos_std::println!("svc: unknown service '{}' (no config in {})", name, SVC_CONFIG_DIR);
        return;
    }

    // Check if already running via thread name lookup
    let existing = find_thread_by_name(name);
    if existing != 0 {
        anyos_std::println!("{}: already running (TID {})", name, existing);
        return;
    }

    // Ensure dependencies are running first
    if !ensure_dependencies(name, 0) {
        return;
    }

    start_service(name, extra_args);
}

fn cmd_stop(name: &str) {
    let tid = find_thread_by_name(name);
    if tid == 0 {
        anyos_std::println!("{}: not running", name);
        return;
    }

    anyos_std::process::kill(tid);
    anyos_std::println!("{}: stopped (TID {})", name, tid);
}

fn cmd_status(name: &str) {
    let tid = find_thread_by_name(name);
    if tid != 0 {
        anyos_std::println!("{}: running (TID {})", name, tid);
    } else {
        anyos_std::println!("{}: stopped", name);
    }
}

fn cmd_restart(name: &str, extra_args: &str) {
    cmd_stop(name);
    cmd_start(name, extra_args);
}

fn main() {
    let mut arg_buf = [0u8; 256];
    let args_str = anyos_std::process::args(&mut arg_buf);
    let parts: alloc::vec::Vec<&str> = args_str.split_whitespace().collect();

    // args() strips argv[0], so parts[0] = command, parts[1] = service name, parts[2..] = extra
    if parts.is_empty() || (parts.len() < 2 && !matches!(parts[0], "list" | "start-all")) {
        anyos_std::println!("Usage: svc <command> [service] [args...]");
        anyos_std::println!("");
        anyos_std::println!("Commands:");
        anyos_std::println!("  start <service> [args]   Start a service");
        anyos_std::println!("  stop <service>           Stop a running service");
        anyos_std::println!("  status <service>         Check if a service is running");
        anyos_std::println!("  restart <service> [args] Restart a service");
        anyos_std::println!("  list                     List all configured services");
        anyos_std::println!("  start-all                Start all configured services");
        anyos_std::println!("");
        anyos_std::println!("Services are configured in {}/", SVC_CONFIG_DIR);
        return;
    }

    let cmd = parts[0];

    if cmd == "list" {
        cmd_list();
        return;
    }
    if cmd == "start-all" {
        cmd_start_all();
        return;
    }

    let name = parts[1];

    // Collect extra args (parts[2..])
    let extra = if parts.len() > 2 {
        let mut s = String::new();
        for i in 2..parts.len() {
            if !s.is_empty() { s.push(' '); }
            s.push_str(parts[i]);
        }
        s
    } else {
        String::new()
    };

    match cmd {
        "start"   => cmd_start(name, &extra),
        "stop"    => cmd_stop(name),
        "status"  => cmd_status(name),
        "restart" => cmd_restart(name, &extra),
        _ => {
            anyos_std::println!("svc: unknown command '{}' (use start/stop/status/restart/list)", cmd);
        }
    }
}
