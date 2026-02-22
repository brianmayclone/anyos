#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use anyos_std::fs::Write;

anyos_std::entry!(main);

const SVC_DIR: &str = "/tmp/svc";
const STILL_RUNNING: u32 = 0xFFFFFFFE;

/// Known services and their binary paths.
fn service_path(name: &str) -> Option<&'static str> {
    match name {
        "sshd"       => Some("/System/bin/sshd"),
        "echoserver" => Some("/bin/echoserver"),
        _ => None,
    }
}

/// Read the stored TID for a service, or 0 if not found / not running.
fn read_pid(name: &str) -> u32 {
    let path = format!("{}/{}.pid", SVC_DIR, name);
    match anyos_std::fs::read_to_string(&path) {
        Ok(s) => {
            let trimmed = s.trim();
            let mut val = 0u32;
            for c in trimmed.bytes() {
                if c >= b'0' && c <= b'9' {
                    val = val * 10 + (c - b'0') as u32;
                } else {
                    break;
                }
            }
            val
        }
        Err(_) => 0,
    }
}

/// Write the TID for a service.
fn write_pid(name: &str, tid: u32) {
    anyos_std::fs::mkdir(SVC_DIR);
    let path = format!("{}/{}.pid", SVC_DIR, name);
    let content = format!("{}", tid);
    if let Ok(mut f) = anyos_std::fs::File::create(&path) {
        let _ = f.write(content.as_bytes());
    }
}

/// Remove the PID file for a service.
fn remove_pid(name: &str) {
    let path = format!("{}/{}.pid", SVC_DIR, name);
    anyos_std::fs::unlink(&path);
}

/// Check if a TID is still running.
fn is_running(tid: u32) -> bool {
    if tid == 0 { return false; }
    anyos_std::process::try_waitpid(tid) == STILL_RUNNING
}

fn cmd_start(name: &str, extra_args: &str) {
    // Check if already running
    let old_tid = read_pid(name);
    if old_tid != 0 && is_running(old_tid) {
        anyos_std::println!("{}: already running (TID {})", name, old_tid);
        return;
    }

    let bin_path = match service_path(name) {
        Some(p) => String::from(p),
        None => {
            // Try /System/bin/<name> then /bin/<name>
            let sys_path = format!("/System/bin/{}", name);
            if anyos_std::fs::File::open(&sys_path).is_ok() {
                sys_path
            } else {
                let bp = format!("/bin/{}", name);
                if anyos_std::fs::File::open(&bp).is_ok() {
                    bp
                } else {
                    anyos_std::println!("svc: unknown service '{}'", name);
                    return;
                }
            }
        }
    };

    let args = if extra_args.is_empty() {
        format!("{}", bin_path)
    } else {
        format!("{} {}", bin_path, extra_args)
    };

    let tid = anyos_std::process::spawn(&bin_path, &args);
    if tid == 0 || tid == u32::MAX {
        anyos_std::println!("svc: failed to start {}", name);
        return;
    }

    write_pid(name, tid);
    anyos_std::println!("{}: started (TID {})", name, tid);
}

fn cmd_stop(name: &str) {
    let tid = read_pid(name);
    if tid == 0 {
        anyos_std::println!("{}: not running (no PID file)", name);
        return;
    }

    if !is_running(tid) {
        anyos_std::println!("{}: already stopped", name);
        remove_pid(name);
        return;
    }

    anyos_std::process::kill(tid);
    remove_pid(name);
    anyos_std::println!("{}: stopped (TID {})", name, tid);
}

fn cmd_status(name: &str) {
    let tid = read_pid(name);
    if tid == 0 {
        anyos_std::println!("{}: stopped", name);
        return;
    }

    if is_running(tid) {
        anyos_std::println!("{}: running (TID {})", name, tid);
    } else {
        anyos_std::println!("{}: stopped (stale PID {})", name, tid);
        remove_pid(name);
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
    if parts.len() < 2 {
        anyos_std::println!("Usage: svc <start|stop|status|restart> <service> [args...]");
        anyos_std::println!("");
        anyos_std::println!("Known services: sshd, echoserver");
        anyos_std::println!("");
        anyos_std::println!("Examples:");
        anyos_std::println!("  svc start sshd");
        anyos_std::println!("  svc start sshd -p 2222");
        anyos_std::println!("  svc stop sshd");
        anyos_std::println!("  svc status sshd");
        anyos_std::println!("  svc restart echoserver");
        return;
    }

    let cmd = parts[0];
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
            anyos_std::println!("svc: unknown command '{}' (use start/stop/status/restart)", cmd);
        }
    }
}
