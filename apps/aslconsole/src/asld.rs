use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{ipc, process};

const RESPONSE_TIMEOUT_TICKS: u32 = 200;
const RESPONSE_SLEEP_MS: u32 = 20;

pub struct AsldResponse {
    pub ok: bool,
    pub lines: Vec<String>,
    pub message: String,
}

pub fn request(command: &str) -> Result<AsldResponse, &'static str> {
    if command.is_empty() || command.contains('\n') || command.contains('\r') {
        return Err("invalid asld command");
    }

    let pid = process::getpid();
    let reply_name = format!("asld-{}", pid);
    let old_reply = ipc::pipe_open(&reply_name);
    if old_reply != 0 {
        let _ = ipc::pipe_close(old_reply);
    }

    let reply_pipe = ipc::pipe_create(&reply_name);
    if reply_pipe == 0 || reply_pipe == u32::MAX {
        return Err("failed to create asld reply pipe");
    }

    let request_pipe = ipc::pipe_open("asld");
    if request_pipe == 0 || request_pipe == u32::MAX {
        let _ = ipc::pipe_close(reply_pipe);
        return Err("asld is not running");
    }

    let request = format!("{}\t{}\n", pid, command);
    if ipc::pipe_write(request_pipe, request.as_bytes()) == u32::MAX {
        let _ = ipc::pipe_close(reply_pipe);
        return Err("failed to write asld request");
    }

    let mut raw = String::new();
    let mut buf = [0u8; 1024];
    for _ in 0..RESPONSE_TIMEOUT_TICKS {
        let n = ipc::pipe_read(reply_pipe, &mut buf);
        if n == u32::MAX {
            let _ = ipc::pipe_close(reply_pipe);
            return Err("failed to read asld response");
        }
        if n > 0 {
            let chunk = match core::str::from_utf8(&buf[..n as usize]) {
                Ok(text) => text,
                Err(_) => {
                    let _ = ipc::pipe_close(reply_pipe);
                    return Err("asld response was not valid UTF-8");
                }
            };
            raw.push_str(chunk);
            if raw.ends_with("\n\n") {
                let _ = ipc::pipe_close(reply_pipe);
                return Ok(parse_response(&raw));
            }
        }
        process::sleep(RESPONSE_SLEEP_MS);
    }

    let _ = ipc::pipe_close(reply_pipe);
    Err("timed out waiting for asld")
}

fn parse_response(raw: &str) -> AsldResponse {
    let trimmed = raw.trim_matches('\n');
    let mut lines = trimmed.split('\n');
    let header = lines.next().unwrap_or("");
    let mut header_parts = header.split('\t');
    match header_parts.next() {
        Some("OK") => AsldResponse {
            ok: true,
            lines: lines
                .filter(|line| !line.is_empty())
                .map(String::from)
                .collect(),
            message: String::new(),
        },
        Some("ERR") => {
            let code = header_parts.next().unwrap_or("unknown");
            let msg = join_tab_fields(&mut header_parts);
            AsldResponse {
                ok: false,
                lines: Vec::new(),
                message: if msg.is_empty() {
                    String::from(code)
                } else {
                    format!("{} {}", code, msg)
                },
            }
        }
        _ => AsldResponse {
            ok: false,
            lines: Vec::new(),
            message: String::from("invalid asld response"),
        },
    }
}

fn join_tab_fields<'a>(parts: &mut core::str::Split<'a, char>) -> String {
    let mut out = String::new();
    for part in parts {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}
