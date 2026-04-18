use alloc::format;
use alloc::string::String;

use crate::DnsService;

pub fn handle_requests(service: &mut DnsService, pipe_id: u32, buf: &mut [u8]) -> bool {
    let n = anyos_std::ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }

    let data = &buf[..n as usize];
    let mut line_start = 0usize;
    for i in 0..data.len() {
        if data[i] == b'\n' {
            if i > line_start {
                handle_single_request(service, &data[line_start..i]);
            }
            line_start = i + 1;
        }
    }
    if line_start < data.len() {
        handle_single_request(service, &data[line_start..]);
    }

    true
}

fn handle_single_request(service: &mut DnsService, line: &[u8]) {
    let tab_pos = match line.iter().position(|&b| b == b'\t') {
        Some(pos) => pos,
        None => return,
    };

    let tid_str = match core::str::from_utf8(&line[..tab_pos]) {
        Ok(s) => s,
        Err(_) => return,
    };
    let tid = match parse_u32(tid_str) {
        Some(v) => v,
        None => return,
    };
    let cmd = match core::str::from_utf8(&line[tab_pos + 1..]) {
        Ok(s) => s.trim(),
        Err(_) => return,
    };
    if cmd.is_empty() {
        return;
    }

    let response = dispatch(service, cmd);
    let reply_pipe_name = format!("dnsd-{}", tid);
    let reply_pipe = anyos_std::ipc::pipe_open(&reply_pipe_name);
    if reply_pipe != 0 {
        anyos_std::ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn dispatch(service: &mut DnsService, cmd: &str) -> String {
    let (verb, arg) = split_first_word(cmd);
    match verb {
        "RESOLVE" | "resolve" => cmd_resolve(service, arg),
        "FLUSH" | "flush" => cmd_flush(service),
        "RELOAD" | "reload" => cmd_reload(service),
        "STATUS" | "status" => service.status_response(),
        "STATS" | "stats" => service.stats_response(),
        _ => format!("ERR\tUnknown command: {}\n\n", verb),
    }
}

fn cmd_resolve(service: &mut DnsService, host: &str) -> String {
    if host.is_empty() {
        return String::from("ERR\tEmpty hostname\n\n");
    }
    match service.resolve(host) {
        Some(ip) => format!("OK\tA\t{}.{}.{}.{}\n\n", ip[0], ip[1], ip[2], ip[3]),
        None => String::from("ERR\tResolve failed\n\n"),
    }
}

fn cmd_flush(service: &mut DnsService) -> String {
    service.flush();
    String::from("OK\t0\nflushed\n\n")
}

fn cmd_reload(service: &mut DnsService) -> String {
    service.reload();
    String::from("OK\t0\nreloaded\n\n")
}

fn split_first_word(input: &str) -> (&str, &str) {
    if let Some(pos) = input.find(' ') {
        (&input[..pos], input[pos + 1..].trim())
    } else {
        (input, "")
    }
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut val = 0u32;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}
