use alloc::format;
use alloc::string::String;

use anyos_std::{ipc, process};

use crate::state::RuntimeState;

pub(crate) const PIPE_NAME: &str = "lxed";

pub(crate) fn create_or_open_pipe() -> Option<u32> {
    let pipe_id = ipc::pipe_open(PIPE_NAME);
    if pipe_id != 0 && pipe_id != u32::MAX {
        return Some(pipe_id);
    }
    let pipe_id = ipc::pipe_create(PIPE_NAME);
    if pipe_id != 0 && pipe_id != u32::MAX {
        Some(pipe_id)
    } else {
        None
    }
}

pub(crate) fn handle_requests(runtime: &mut RuntimeState, pipe_id: u32, buf: &mut [u8]) -> bool {
    let n = ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        runtime.cleanup_dead_leases();
        return false;
    }

    let data = &buf[..n as usize];
    let mut handled = false;
    let mut line_start = 0usize;
    for idx in 0..data.len() {
        if data[idx] == b'\n' {
            if idx > line_start {
                handle_single_request(runtime, &data[line_start..idx]);
                handled = true;
            }
            line_start = idx + 1;
        }
    }
    if line_start < data.len() {
        handle_single_request(runtime, &data[line_start..]);
        handled = true;
    }
    handled
}

fn handle_single_request(runtime: &mut RuntimeState, line: &[u8]) {
    let Ok(line) = core::str::from_utf8(line) else {
        return;
    };
    let Some(tab) = line.find('\t') else {
        return;
    };
    let Some(tid) = parse_u32(&line[..tab]) else {
        return;
    };
    let command = line[tab + 1..].trim();
    runtime.record_request();

    let response = dispatch(runtime, tid, command);
    send_reply(tid, &response);
}

fn dispatch(runtime: &mut RuntimeState, tid: u32, command: &str) -> String {
    let verb = command.split_ascii_whitespace().next().unwrap_or("");
    match verb {
        "PING" => String::from("OK\tpong\n"),
        "STATUS" => runtime.ok(),
        "RELOAD" => {
            runtime.reload();
            runtime.ok()
        }
        "BEGIN_RUN" => runtime.begin_run(tid),
        "END_RUN" => runtime.end_run(tid),
        "BEGIN_WRITE" => runtime.begin_write(tid),
        "END_WRITE" => runtime.end_write(tid),
        "TRACK_PROCESS" => match command.split_ascii_whitespace().nth(1).and_then(parse_u32) {
            Some(process_tid) => runtime.track_process(tid, process_tid),
            None => String::from("ERR\tmissing process tid\n"),
        },
        "UNTRACK_PROCESS" => match command.split_ascii_whitespace().nth(1).and_then(parse_u32) {
            Some(process_tid) => runtime.untrack_process(process_tid),
            None => String::from("ERR\tmissing process tid\n"),
        },
        "SYNC" => {
            runtime.sync_rootfs();
            runtime.ok()
        }
        "STOP" => {
            process::exit(0);
        }
        _ => format!("ERR\tunknown command '{}'\n", verb),
    }
}

fn send_reply(tid: u32, response: &str) {
    let reply_name = format!("lxed-{}", tid);
    let reply_pipe = ipc::pipe_open(&reply_name);
    if reply_pipe == 0 || reply_pipe == u32::MAX {
        return;
    }
    let _ = ipc::pipe_write(reply_pipe, response.as_bytes());
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
