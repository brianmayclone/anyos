//! anyOS NTP daemon — SNTP client for clock synchronization.
//!
//! # Architecture
//! - Reads `/System/etc/ntp/ntpd.conf` at startup and on "reload" control commands.
//! - Creates a named pipe "ntpd" for management commands (`reload`, `stop`, `sync`, `status`).
//! - Periodically queries configured NTP servers via UDP port 123.
//! - Maintains current clock offset and sync state.
//! - Writes status to `/System/etc/ntp/ntpd.status` for the `ntp` command to read.
//!
//! # Control pipe commands
//! - `stop`   — shut down the daemon
//! - `reload` — reload configuration
//! - `sync`   — force an immediate synchronization
//! - `status` — write current status to the status file

#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{fs, ipc, net, println, process, sys};

mod config;
mod ntp;

const PIPE_NAME: &str = "ntpd";
const STATUS_PATH: &str = "/System/etc/ntp/ntpd.status";

/// Current synchronization state.
struct SyncState {
    /// Last measured offset in milliseconds (positive = clock behind).
    offset_ms: i64,
    /// Last measured round-trip delay in milliseconds.
    delay_ms: i64,
    /// Stratum of the server we synced from (0 = not synced).
    stratum: u8,
    /// Index of the server we last synced from.
    server_idx: usize,
    /// Whether we have successfully synced at least once.
    synced: bool,
    /// System uptime_ms at last sync.
    last_sync_uptime: u32,
}

impl SyncState {
    fn new() -> Self {
        SyncState {
            offset_ms: 0,
            delay_ms: 0,
            stratum: 0,
            server_idx: 0,
            synced: false,
            last_sync_uptime: 0,
        }
    }
}

fn print_config(cfg: &config::NtpdConfig) {
    println!("ntpd: --- config ---");
    println!(
        "ntpd:   enabled       = {}",
        if cfg.enabled { "yes" } else { "no" }
    );
    println!("ntpd:   poll_interval = {}s", cfg.poll_interval);
    println!("ntpd:   servers ({}):", cfg.server_count);
    for i in 0..cfg.server_count {
        println!("ntpd:     {}", cfg.servers[i].hostname_str());
    }
    println!("ntpd: ---");
}

/// Resolve hostnames to IP addresses.
fn resolve_servers(cfg: &mut config::NtpdConfig) {
    for i in 0..cfg.server_count {
        if cfg.servers[i].resolved {
            continue;
        }
        // Copy hostname to a local buffer to avoid borrow conflict.
        let mut name_buf = [0u8; config::MAX_HOSTNAME_LEN];
        let name_len = cfg.servers[i].hostname_len;
        name_buf[..name_len].copy_from_slice(&cfg.servers[i].hostname[..name_len]);
        let hostname = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("");

        if let Some(ip) = parse_ip(hostname) {
            cfg.servers[i].ip = ip;
            cfg.servers[i].resolved = true;
            println!(
                "ntpd: server {} -> {}.{}.{}.{}",
                hostname, ip[0], ip[1], ip[2], ip[3]
            );
        } else {
            let mut resolved = [0u8; 4];
            if net::dns(hostname, &mut resolved) == 0 {
                cfg.servers[i].ip = resolved;
                cfg.servers[i].resolved = true;
                println!(
                    "ntpd: server {} -> {}.{}.{}.{}",
                    hostname, resolved[0], resolved[1], resolved[2], resolved[3]
                );
            } else {
                println!("ntpd: failed to resolve {}", hostname);
            }
        }
    }
}

/// Try to sync with configured NTP servers.
fn do_sync(cfg: &config::NtpdConfig, state: &mut SyncState) {
    for i in 0..cfg.server_count {
        if !cfg.servers[i].resolved {
            continue;
        }
        let ip = &cfg.servers[i].ip;
        match ntp::query_server(ip) {
            Some((offset, delay, stratum)) => {
                state.offset_ms = offset;
                state.delay_ms = delay;
                state.stratum = stratum;
                state.server_idx = i;
                state.synced = true;
                state.last_sync_uptime = sys::uptime_ms();

                let abs_offset = if offset < 0 { -offset } else { offset };
                let sign = if offset < 0 { '-' } else { '+' };
                println!(
                    "ntpd: synced with {} (stratum {}): offset {}{}ms, delay {}ms",
                    cfg.servers[i].hostname_str(),
                    stratum,
                    sign,
                    abs_offset,
                    delay
                );

                // Set system clock if offset exceeds 100ms.
                if abs_offset > 100 {
                    let mut time_buf = ntp::corrected_time_buf(offset);
                    let rc = sys::set_time(&time_buf);
                    if rc == 0 {
                        println!("ntpd: system clock adjusted by {}{}ms", sign, abs_offset);
                    } else {
                        println!("ntpd: failed to set system time");
                    }
                }

                write_status(cfg, state);
                return;
            }
            None => {
                println!("ntpd: query to {} failed", cfg.servers[i].hostname_str());
            }
        }
    }
    println!("ntpd: sync failed — no server reachable");
    write_status(cfg, state);
}

/// Write status file for the `ntp` command.
fn write_status(cfg: &config::NtpdConfig, state: &SyncState) {
    let mut out = [0u8; 1024];
    let mut pos = 0;

    macro_rules! append {
        ($s:expr) => {{
            let s: &[u8] = $s;
            let n = s.len().min(out.len() - pos);
            out[pos..pos + n].copy_from_slice(&s[..n]);
            pos += n;
        }};
    }
    macro_rules! append_i64 {
        ($v:expr) => {{
            let mut v = $v;
            if v < 0 {
                append!(b"-");
                v = -v;
            }
            let mut nbuf = [0u8; 20];
            let s = fmt_u64(v as u64, &mut nbuf);
            append!(s);
        }};
    }

    append!(if state.synced {
        b"synced=yes\n"
    } else {
        b"synced=no\n"
    });
    append!(b"offset_ms=");
    append_i64!(state.offset_ms);
    append!(b"\n");
    append!(b"delay_ms=");
    append_i64!(state.delay_ms);
    append!(b"\n");
    append!(b"stratum=");
    let mut nb = [0u8; 20];
    append!(fmt_u64(state.stratum as u64, &mut nb));
    append!(b"\n");
    if state.synced && state.server_idx < cfg.server_count {
        append!(b"server=");
        let h = cfg.servers[state.server_idx].hostname_str();
        append!(h.as_bytes());
        append!(b"\n");
    }
    append!(b"last_sync_uptime=");
    append!(fmt_u64(state.last_sync_uptime as u64, &mut nb));
    append!(b"\n");
    append!(b"poll_interval=");
    append!(fmt_u64(cfg.poll_interval as u64, &mut nb));
    append!(b"\n");

    // Ensure directory exists.
    ensure_dir_exists("/System/etc/ntp");

    let fd = fs::open(STATUS_PATH, fs::O_WRITE | fs::O_CREATE | fs::O_TRUNC);
    if fd != u32::MAX {
        fs::write(fd, &out[..pos]);
        fs::close(fd);
    }
}

fn fmt_u64(mut v: u64, buf: &mut [u8; 20]) -> &[u8] {
    if v == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut i = 20usize;
    while v > 0 {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    &buf[i..]
}

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut parts = [0u8; 4];
    let mut idx = 0;
    let mut num: u32 = 0;
    let mut has_digit = false;
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                num = num * 10 + (b - b'0') as u32;
                if num > 255 {
                    return None;
                }
                has_digit = true;
            }
            b'.' => {
                if !has_digit || idx >= 3 {
                    return None;
                }
                parts[idx] = num as u8;
                idx += 1;
                num = 0;
                has_digit = false;
            }
            _ => return None,
        }
    }
    if !has_digit || idx != 3 {
        return None;
    }
    parts[3] = num as u8;
    Some(parts)
}

fn ensure_dir_exists(path: &str) {
    let mut st = [0u32; 7];
    if fs::stat(path, &mut st) != u32::MAX {
        return;
    }
    let mut cur = alloc::string::String::new();
    for part in path.split('/') {
        if part.is_empty() {
            cur.push('/');
            continue;
        }
        if !cur.ends_with('/') {
            cur.push('/');
        }
        cur.push_str(part);
        if fs::stat(&cur, &mut st) == u32::MAX {
            fs::mkdir(&cur);
        }
    }
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("ntpd - NTP time sync daemon\n\nUsage: ntpd");
        return;
    }

    println!("ntpd: starting");

    config::register_manifest();
    let mut cfg = config::load();
    print_config(&cfg);

    let pipe_id = ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        println!("ntpd: failed to create control pipe");
        return;
    }

    if !cfg.enabled {
        println!("ntpd: disabled (enabled=no in config)");
    }

    // Bind UDP port for NTP communication.
    let bind_result = net::udp_bind(ntp::LOCAL_PORT);
    if bind_result != 0 {
        println!("ntpd: failed to bind UDP port {}", ntp::LOCAL_PORT);
        ipc::pipe_close(pipe_id);
        return;
    }
    // Set receive timeout to 5 seconds.
    net::udp_set_opt(ntp::LOCAL_PORT, 2, 5000);

    let mut state = SyncState::new();

    // Resolve server hostnames.
    if cfg.enabled && cfg.server_count > 0 {
        resolve_servers(&mut cfg);
    }

    // Initial sync.
    if cfg.enabled {
        do_sync(&cfg, &mut state);
    }

    let mut last_poll_uptime = sys::uptime_ms();

    loop {
        // ── Poll control pipe ─────────────────────────────────────────────
        let mut cmd_buf = [0u8; 64];
        let n = ipc::pipe_read(pipe_id, &mut cmd_buf);
        if n > 0 && n != u32::MAX {
            if let Ok(cmd) = core::str::from_utf8(&cmd_buf[..n as usize]) {
                let cmd = cmd.trim_end_matches('\n').trim();
                match cmd {
                    "stop" => {
                        println!("ntpd: stopping");
                        net::udp_unbind(ntp::LOCAL_PORT);
                        ipc::pipe_close(pipe_id);
                        return;
                    }
                    "reload" => {
                        println!("ntpd: reloading config");
                        cfg = config::load();
                        print_config(&cfg);
                        if cfg.enabled {
                            resolve_servers(&mut cfg);
                        }
                    }
                    "sync" => {
                        if cfg.enabled {
                            println!("ntpd: forced sync");
                            // Re-resolve in case DNS changed.
                            resolve_servers(&mut cfg);
                            do_sync(&cfg, &mut state);
                            last_poll_uptime = sys::uptime_ms();
                        }
                    }
                    "status" => {
                        write_status(&cfg, &state);
                    }
                    _ => {}
                }
            }
        }

        // ── Periodic sync ─────────────────────────────────────────────────
        if cfg.enabled && cfg.server_count > 0 {
            let now = sys::uptime_ms();
            let elapsed = now.wrapping_sub(last_poll_uptime);
            if elapsed >= cfg.poll_interval * 1000 {
                // Re-resolve unresolved servers.
                resolve_servers(&mut cfg);
                do_sync(&cfg, &mut state);
                last_poll_uptime = now;
            }
        }

        process::sleep(500);
    }
}
