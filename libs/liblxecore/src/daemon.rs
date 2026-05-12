use alloc::format;
use alloc::string::String;

use anyos_std::{ipc, process, sys};

use crate::config::LxeConfig;

const PIPE_NAME: &str = "lxed";
const REQUEST_TIMEOUT_MS: u32 = 8_000;
const LEASE_TIMEOUT_MS: u32 = 120_000;

pub enum LeaseKind {
    Run,
    Write,
}

pub struct RuntimeLease {
    kind: LeaseKind,
    active: bool,
}

pub struct RuntimeStatus {
    pub root: String,
    pub rootfs: String,
    pub active_runs: u32,
    pub active_processes: u32,
    pub writer: u32,
    pub syncs: u32,
}

impl RuntimeLease {
    pub fn release(mut self) {
        if !self.active {
            return;
        }
        let cmd = match self.kind {
            LeaseKind::Run => "END_RUN",
            LeaseKind::Write => "END_WRITE",
        };
        let _ = request(cmd, 2_000);
        self.active = false;
    }
}

impl Drop for RuntimeLease {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        let cmd = match self.kind {
            LeaseKind::Run => "END_RUN",
            LeaseKind::Write => "END_WRITE",
        };
        let _ = request(cmd, 1_000);
        self.active = false;
    }
}

pub fn acquire_run(config: &mut LxeConfig, label: &str) -> Result<RuntimeLease, String> {
    acquire(config, LeaseKind::Run, &format!("BEGIN_RUN {}", label))
}

pub fn acquire_write(config: &mut LxeConfig, label: &str) -> Result<RuntimeLease, String> {
    acquire(config, LeaseKind::Write, &format!("BEGIN_WRITE {}", label))
}

pub fn status() -> Result<RuntimeStatus, String> {
    let response = request("STATUS", REQUEST_TIMEOUT_MS)?;
    parse_status_response(&response).ok_or_else(|| String::from("lxed returned malformed status"))
}

pub fn track_process(tid: u32) -> Result<(), String> {
    let response = request(&format!("TRACK_PROCESS {}", tid), REQUEST_TIMEOUT_MS)?;
    if response.starts_with("OK") {
        Ok(())
    } else {
        Err(response)
    }
}

pub fn untrack_process(tid: u32) {
    let _ = request(&format!("UNTRACK_PROCESS {}", tid), 2_000);
}

pub fn sync_rootfs() {
    let _ = request("SYNC", REQUEST_TIMEOUT_MS);
}

fn acquire(config: &mut LxeConfig, kind: LeaseKind, command: &str) -> Result<RuntimeLease, String> {
    let deadline = sys::uptime_ms().wrapping_add(LEASE_TIMEOUT_MS);
    loop {
        match request(command, REQUEST_TIMEOUT_MS) {
            Ok(response) => {
                if response.starts_with("OK") {
                    apply_config_fields(config, &response);
                    return Ok(RuntimeLease { kind, active: true });
                }
                if !response.starts_with("BUSY") {
                    return Err(response);
                }
            }
            Err(err) => return Err(err),
        }
        if deadline_reached(sys::uptime_ms(), deadline) {
            return Err(String::from("timeout waiting for lxed runtime lease"));
        }
        process::sleep(100);
    }
}

fn request(command: &str, timeout_ms: u32) -> Result<String, String> {
    ensure_daemon()?;

    let tid = process::getpid();
    let reply_name = format!("lxed-{}", tid);
    let old_reply = ipc::pipe_open(&reply_name);
    if old_reply != 0 && old_reply != u32::MAX {
        let _ = ipc::pipe_close(old_reply);
    }
    let reply_pipe = ipc::pipe_create(&reply_name);
    if reply_pipe == 0 || reply_pipe == u32::MAX {
        return Err(String::from("failed to create lxed reply pipe"));
    }

    let result = request_with_reply_pipe(tid, command, reply_pipe, timeout_ms);
    let _ = ipc::pipe_close(reply_pipe);
    result
}

fn request_with_reply_pipe(
    tid: u32,
    command: &str,
    reply_pipe: u32,
    timeout_ms: u32,
) -> Result<String, String> {
    let daemon_pipe = ipc::pipe_open(PIPE_NAME);
    if daemon_pipe == 0 || daemon_pipe == u32::MAX {
        return Err(String::from("lxed pipe is not available"));
    }

    let line = format!("{}\t{}\n", tid, command);
    if ipc::pipe_write(daemon_pipe, line.as_bytes()) == u32::MAX {
        return Err(String::from("failed to write lxed request"));
    }

    let deadline = sys::uptime_ms().wrapping_add(timeout_ms);
    let mut buf = [0u8; 512];
    loop {
        let n = ipc::pipe_read(reply_pipe, &mut buf);
        if n != 0 && n != u32::MAX {
            return core::str::from_utf8(&buf[..n as usize])
                .map(|s| String::from(s.trim()))
                .map_err(|_| String::from("lxed returned non-utf8 response"));
        }
        if deadline_reached(sys::uptime_ms(), deadline) {
            return Err(String::from("timeout waiting for lxed response"));
        }
        process::sleep(20);
    }
}

fn ensure_daemon() -> Result<(), String> {
    if pipe_available() {
        return Ok(());
    }

    let tid = process::spawn("/System/bin/svc", "/System/bin/svc start lxed");
    if tid != 0 && tid != u32::MAX {
        let _ = process::waitpid(tid);
    }

    let deadline = sys::uptime_ms().wrapping_add(5_000);
    while !deadline_reached(sys::uptime_ms(), deadline) {
        if pipe_available() {
            return Ok(());
        }
        process::sleep(50);
    }
    Err(String::from("lxed is not running"))
}

fn pipe_available() -> bool {
    let pipe_id = ipc::pipe_open(PIPE_NAME);
    pipe_id != 0 && pipe_id != u32::MAX
}

fn apply_config_fields(config: &mut LxeConfig, response: &str) {
    for field in response.split('\t') {
        if let Some(value) = field.strip_prefix("root=") {
            if !value.is_empty() {
                config.root = String::from(value);
            }
        } else if let Some(value) = field.strip_prefix("rootfs=") {
            if !value.is_empty() {
                config.rootfs = String::from(value);
            }
        }
    }
}

fn parse_status_response(response: &str) -> Option<RuntimeStatus> {
    if !response.starts_with("OK") {
        return None;
    }
    let mut status = RuntimeStatus {
        root: String::new(),
        rootfs: String::new(),
        active_runs: 0,
        active_processes: 0,
        writer: 0,
        syncs: 0,
    };
    for field in response.split('\t') {
        if let Some(value) = field.strip_prefix("root=") {
            status.root = String::from(value);
        } else if let Some(value) = field.strip_prefix("rootfs=") {
            status.rootfs = String::from(value);
        } else if let Some(value) = field.strip_prefix("active_runs=") {
            status.active_runs = parse_u32(value).unwrap_or(0);
        } else if let Some(value) = field.strip_prefix("active_processes=") {
            status.active_processes = parse_u32(value).unwrap_or(0);
        } else if let Some(value) = field.strip_prefix("writer=") {
            status.writer = parse_u32(value).unwrap_or(0);
        } else if let Some(value) = field.strip_prefix("syncs=") {
            status.syncs = parse_u32(value).unwrap_or(0);
        }
    }
    Some(status)
}

fn parse_u32(raw: &str) -> Option<u32> {
    let mut value = 0u32;
    for b in raw.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(value)
}

fn deadline_reached(now: u32, deadline: u32) -> bool {
    now.wrapping_sub(deadline) < 0x8000_0000
}
