//! Simple append-only log at /System/var/asl/asld.log.
//!
//! Lines: "[uptime_ms] [level] [tag] message". Best-effort; write failures
//! are silently dropped so logging never blocks or panics the daemon.

use alloc::format;

const LOG_PATH: &str = "/System/var/asl/asld.log";
const MAX_LINE_LEN: usize = 1024;

#[cfg(target_os = "linux")]
pub fn emit(level: &str, tag: &str, msg: &str) {
    // Host/test mode: route to stderr-style println so tests stay noise-free
    // but developers see output. No file I/O on Linux.
    anyos_std::println!("asld[{}][{}] {}", level, tag, msg);
}

#[cfg(not(target_os = "linux"))]
pub fn emit(level: &str, tag: &str, msg: &str) {
    use anyos_std::fs;

    let _ = fs::mkdir("/System/var");
    let _ = fs::mkdir("/System/var/asl");

    let ts = anyos_std::sys::uptime_ms();
    let mut line = format!("[{}] [{}] [{}] {}\n", ts, level, tag, msg);
    if line.len() > MAX_LINE_LEN {
        line.truncate(MAX_LINE_LEN.saturating_sub(1));
        line.push('\n');
    }

    let fd = fs::open(LOG_PATH, fs::O_WRITE | fs::O_APPEND | fs::O_CREATE);
    if fd == u32::MAX || fd == 0 {
        return;
    }
    fs::write(fd, line.as_bytes());
    fs::close(fd);
}

pub fn info(tag: &str, msg: &str) {
    emit("info", tag, msg);
}

pub fn warn(tag: &str, msg: &str) {
    emit("warn", tag, msg);
}

pub fn error(tag: &str, msg: &str) {
    emit("error", tag, msg);
}
