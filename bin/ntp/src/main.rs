//! anyOS NTP client — query time servers and manage ntpd.
//!
//! # Usage
//! ```
//! ntp                     Show current sync status (from ntpd)
//! ntp status              Same as above
//! ntp sync                Force ntpd to synchronize now
//! ntp query <host>        One-shot NTP query to a specific server
//! ntp reload              Tell ntpd to reload its configuration
//! ntp stop                Stop the NTP daemon
//! ```

#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::{fs, ipc, net, sys, println};

const NTP_PORT: u16 = 123;
const NTP_PACKET_SIZE: usize = 48;
const NTP_EPOCH_OFFSET: u64 = 2_208_988_800;
const STATUS_PATH: &str = "/System/etc/ntp/ntpd.status";
const PIPE_NAME: &str = "ntpd";

fn main() {


    let mut args_buf = [0u8; 256];
    let args_raw = anyos_std::process::args(&mut args_buf);

    if args_raw.contains("--help") {
        anyos_std::println!("ntp - NTP time sync client\n\nUsage: ntp [status|sync|reload|stop|query HOST]");
        return;
    }

    let args = args_raw.trim();

    if args.is_empty() || args == "status" {
        show_status();
    } else if args == "sync" {
        send_command("sync");
    } else if args == "reload" {
        send_command("reload");
    } else if args == "stop" {
        send_command("stop");
    } else if args.starts_with("query ") {
        let host = args["query ".len()..].trim();
        if host.is_empty() {
            println!("Usage: ntp query <host>");
        } else {
            query_server(host);
        }
    } else {
        println!("Usage: ntp [status|sync|reload|stop|query <host>]");
        println!("");
        println!("  status   Show current sync status (default)");
        println!("  sync     Force immediate synchronization");
        println!("  query    One-shot NTP query to a server");
        println!("  reload   Reload ntpd configuration");
        println!("  stop     Stop the NTP daemon");
    }
}

fn show_status() {
    let mut buf = [0u8; 1024];
    let fd = fs::open(STATUS_PATH, 0);
    if fd == u32::MAX {
        println!("ntpd is not running or has not synced yet.");
        return;
    }
    let n = fs::read(fd, &mut buf) as usize;
    fs::close(fd);
    if n == 0 || n > buf.len() {
        println!("No status available.");
        return;
    }

    // Parse key=value pairs.
    let text = core::str::from_utf8(&buf[..n]).unwrap_or("");
    let mut synced = false;
    let mut offset = 0i64;
    let mut delay = 0i64;
    let mut stratum = 0u8;
    let mut server = "";
    let mut last_uptime = 0u32;
    let mut poll_interval = 0u32;

    for line in text.split('\n') {
        let line = line.trim();
        if line.is_empty() { continue; }
        if let Some(eq) = line.find('=') {
            let key = &line[..eq];
            let val = &line[eq + 1..];
            match key {
                "synced" => synced = val == "yes",
                "offset_ms" => offset = parse_i64(val),
                "delay_ms" => delay = parse_i64(val),
                "stratum" => stratum = parse_i64(val) as u8,
                "server" => server = val,
                "last_sync_uptime" => last_uptime = parse_i64(val) as u32,
                "poll_interval" => poll_interval = parse_i64(val) as u32,
                _ => {}
            }
        }
    }

    if synced {
        let now = sys::uptime_ms();
        let ago_ms = now.wrapping_sub(last_uptime);
        let ago_s = ago_ms / 1000;

        let abs_offset = if offset < 0 { -offset } else { offset };
        let sign = if offset < 0 { "-" } else { "+" };

        println!("NTP synchronized:");
        println!("  Server:   {}", server);
        println!("  Stratum:  {}", stratum);
        println!("  Offset:   {}{}ms", sign, abs_offset);
        println!("  Delay:    {}ms", delay);
        println!("  Last sync: {}s ago", ago_s);
        println!("  Poll:     {}s", poll_interval);
    } else {
        println!("NTP not synchronized.");
    }
}

fn send_command(cmd: &str) {
    let pipe_id = ipc::pipe_open(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        println!("ntpd is not running.");
        return;
    }
    let written = ipc::pipe_write(pipe_id, cmd.as_bytes());
    ipc::pipe_close(pipe_id);
    if written == u32::MAX {
        println!("Failed to send command to ntpd.");
    } else {
        println!("Sent '{}' to ntpd.", cmd);
    }
}

fn query_server(host: &str) {
    // Resolve hostname.
    let ip = match parse_ip(host) {
        Some(ip) => ip,
        None => {
            let mut resolved = [0u8; 4];
            if net::dns(host, &mut resolved) == 0 {
                println!("Resolved {} -> {}.{}.{}.{}",
                    host, resolved[0], resolved[1], resolved[2], resolved[3]);
                resolved
            } else {
                println!("Cannot resolve host: {}", host);
                return;
            }
        }
    };

    // Bind a temporary UDP port.
    let local_port: u16 = 1124;
    if net::udp_bind(local_port) != 0 {
        println!("Failed to bind UDP port.");
        return;
    }
    net::udp_set_opt(local_port, 2, 5000); // 5s timeout

    // Build request.
    let mut request = [0u8; NTP_PACKET_SIZE];
    for b in request.iter_mut() { *b = 0; }
    request[0] = 0x23; // LI=0, VN=4, Mode=3
    let ts = get_system_ntp_time();
    request[40] = (ts.0 >> 24) as u8;
    request[41] = (ts.0 >> 16) as u8;
    request[42] = (ts.0 >> 8) as u8;
    request[43] = ts.0 as u8;

    println!("Querying {}.{}.{}.{}...", ip[0], ip[1], ip[2], ip[3]);
    let sent = net::udp_sendto(&ip, NTP_PORT, local_port, &request, 0);
    if sent == u32::MAX {
        println!("Failed to send NTP request.");
        net::udp_unbind(local_port);
        return;
    }

    let mut recv_buf = [0u8; 128];
    let n = net::udp_recvfrom(local_port, &mut recv_buf);
    net::udp_unbind(local_port);

    if n == 0 || n == u32::MAX || (n as usize) < 8 + NTP_PACKET_SIZE {
        println!("No response (timeout).");
        return;
    }

    // Parse response.
    let pkt = &recv_buf[8..8 + NTP_PACKET_SIZE];
    let mode = pkt[0] & 0x07;
    if mode != 4 && mode != 5 {
        println!("Invalid response (mode={}).", mode);
        return;
    }
    let stratum = pkt[1];
    let tx_secs = u32::from_be_bytes([pkt[40], pkt[41], pkt[42], pkt[43]]);
    let rx_secs = u32::from_be_bytes([pkt[32], pkt[33], pkt[34], pkt[35]]);
    let orig_secs = u32::from_be_bytes([pkt[24], pkt[25], pkt[26], pkt[27]]);

    // Calculate offset: simplified (T2 - T1) as one-way estimate.
    let t1 = ts.0 as u64 * 1000;
    let t2 = rx_secs as u64 * 1000;
    let t3 = tx_secs as u64 * 1000;

    let local_now = get_system_ntp_time();
    let t4 = local_now.0 as u64 * 1000;

    let offset = ((t2.wrapping_sub(t1) as i64) + (t3.wrapping_sub(t4) as i64)) / 2;
    let delay = (t4.wrapping_sub(t1) as i64) - (t3.wrapping_sub(t2) as i64);

    let abs_offset = if offset < 0 { -offset } else { offset };
    let sign = if offset < 0 { "-" } else { "+" };

    // Convert NTP timestamp to readable time.
    let unix_secs = (tx_secs as u64).wrapping_sub(NTP_EPOCH_OFFSET);
    let (year, month, day, hour, min, sec) = unix_to_date(unix_secs);

    println!("Response from {}.{}.{}.{}:", ip[0], ip[1], ip[2], ip[3]);
    println!("  Stratum:    {}", stratum);
    println!("  Server time: {}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        year, month, day, hour, min, sec);
    println!("  Offset:     {}{}ms", sign, abs_offset);
    println!("  Delay:      {}ms", delay);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_ip(s: &str) -> Option<[u8; 4]> {
    let mut parts = [0u8; 4];
    let mut idx = 0;
    let mut num: u32 = 0;
    let mut has_digit = false;
    for b in s.bytes() {
        match b {
            b'0'..=b'9' => {
                num = num * 10 + (b - b'0') as u32;
                if num > 255 { return None; }
                has_digit = true;
            }
            b'.' => {
                if !has_digit || idx >= 3 { return None; }
                parts[idx] = num as u8;
                idx += 1;
                num = 0;
                has_digit = false;
            }
            _ => return None,
        }
    }
    if !has_digit || idx != 3 { return None; }
    parts[3] = num as u8;
    Some(parts)
}

fn parse_i64(s: &str) -> i64 {
    let bytes = s.as_bytes();
    if bytes.is_empty() { return 0; }
    let (neg, start) = if bytes[0] == b'-' { (true, 1) } else { (false, 0) };
    let mut val: i64 = 0;
    for &b in &bytes[start..] {
        if b >= b'0' && b <= b'9' {
            val = val * 10 + (b - b'0') as i64;
        } else {
            break;
        }
    }
    if neg { -val } else { val }
}

/// Get system time as (NTP seconds, fraction).
fn get_system_ntp_time() -> (u32, u32) {
    let mut time_buf = [0u8; 8];
    sys::time(&mut time_buf);
    let year = time_buf[0] as u32 | ((time_buf[1] as u32) << 8);
    let month = time_buf[2] as u32;
    let day = time_buf[3] as u32;
    let hour = time_buf[4] as u32;
    let min = time_buf[5] as u32;
    let sec = time_buf[6] as u32;
    let unix_secs = date_to_unix(year, month, day, hour, min, sec);
    let ntp_secs = unix_secs + NTP_EPOCH_OFFSET;
    (ntp_secs as u32, 0)
}

fn date_to_unix(year: u32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> u64 {
    let mut days: u64 = 0;
    for y in 1970..year {
        days += if is_leap(y) { 366 } else { 365 };
    }
    let month_days: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        let d = month_days[(m - 1) as usize];
        days += d as u64;
        if m == 2 && is_leap(year) {
            days += 1;
        }
    }
    days += (day as u64).saturating_sub(1);
    days * 86400 + hour as u64 * 3600 + min as u64 * 60 + sec as u64
}

fn is_leap(year: u32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn unix_to_date(mut secs: u64) -> (u32, u32, u32, u32, u32, u32) {
    let sec = (secs % 60) as u32; secs /= 60;
    let min = (secs % 60) as u32; secs /= 60;
    let hour = (secs % 24) as u32; secs /= 24;
    // secs is now days since 1970-01-01.
    let mut year = 1970u32;
    loop {
        let days_in_year = if is_leap(year) { 366u64 } else { 365 };
        if secs < days_in_year { break; }
        secs -= days_in_year;
        year += 1;
    }
    let month_days: [u32; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u32;
    for m in 0..12 {
        let mut d = month_days[m] as u64;
        if m == 1 && is_leap(year) { d += 1; }
        if secs < d { break; }
        secs -= d;
        month += 1;
    }
    let day = secs as u32 + 1;
    (year, month, day, hour, min, sec)
}
