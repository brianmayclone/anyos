#![cfg_attr(not(target_os = "linux"), no_std)]
#![cfg_attr(not(target_os = "linux"), no_main)]

#[cfg(target_os = "linux")]
extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::{fs, ipc, println, process, sys};
use libconf_schema::{default_bool, default_int, manifest, RegistryScope, ServiceSchema};
use libsvc::ServiceLifecycle;

#[cfg(not(target_os = "linux"))]
anyos_std::entry!(main);

const PIPE_NAME: &str = "aslconsoled";
const STATUS_PATH: &str = "/System/var/asl/aslconsoled.status";

const DIRS: &[&str] = &["config"];
const DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/max_sessions", 128),
    default_bool("config/fallback_console_enabled", true),
];
const MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/aslconsoled",
    RegistryScope::System,
    1,
    DIRS,
    DEFAULTS,
    &[],
);
const SCHEMA: ServiceSchema<'static> = ServiceSchema::new("aslconsoled", &MANIFEST);

#[derive(Clone)]
struct ConsoleSession {
    distro: String,
    session_id: String,
    mode: String,
    console_pipe: String,
    stdin_pipe: String,
}

struct ConsoleState {
    sessions: Vec<ConsoleSession>,
    attach_count: u32,
    rejected_count: u32,
    write_count: u32,
    output_bytes: u64,
    last_attach_ms: u32,
    last_write_ms: u32,
}

fn main() {
    println!("aslconsoled: starting");
    let _ = SCHEMA.register();
    let mut lifecycle = ServiceLifecycle::connect("aslconsoled").ok();
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_starting();
        let _ = lifecycle.set_health("starting");
    }
    let old_pipe = ipc::pipe_open(PIPE_NAME);
    if old_pipe != 0 {
        ipc::pipe_close(old_pipe);
    }
    let pipe_id = ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 || pipe_id == u32::MAX {
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("pipe_create_failed");
        }
        return;
    }
    let mut state = ConsoleState {
        sessions: Vec::new(),
        attach_count: 0,
        rejected_count: 0,
        write_count: 0,
        output_bytes: 0,
        last_attach_ms: 0,
        last_write_ms: 0,
    };
    write_status(&state);
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }
    println!("aslconsoled: ready (pipe='{}')", PIPE_NAME);

    let mut pending = String::new();
    let mut buf = [0u8; 1024];
    loop {
        if handle_requests(pipe_id, &mut pending, &mut state, &mut buf) {
            process::sleep(20);
        } else {
            process::sleep(100);
        }
    }
}

fn handle_requests(
    pipe_id: u32,
    pending: &mut String,
    state: &mut ConsoleState,
    buf: &mut [u8],
) -> bool {
    let n = ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }
    let Ok(text) = core::str::from_utf8(&buf[..n as usize]) else {
        return true;
    };
    pending.push_str(text);
    while let Some(pos) = pending.find('\n') {
        let line = pending[..pos].trim().to_string();
        pending.drain(..=pos);
        if !line.is_empty() {
            handle_line(state, &line);
        }
    }
    true
}

fn handle_line(state: &mut ConsoleState, line: &str) {
    let Some(tab_pos) = line.find('\t') else {
        return;
    };
    let Some(tid) = parse_u32(&line[..tab_pos]) else {
        return;
    };
    let response = dispatch(state, line[tab_pos + 1..].trim());
    let reply_pipe = ipc::pipe_open(&format!("aslconsoled-{}", tid));
    if reply_pipe != 0 {
        ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn dispatch(state: &mut ConsoleState, cmd: &str) -> String {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "STATUS" | "status" => ok_lines(alloc::vec![
            format!("sessions\t{}", state.sessions.len()),
            format!("attach_count\t{}", state.attach_count),
            format!("rejected_count\t{}", state.rejected_count),
            format!("write_count\t{}", state.write_count),
            format!("output_bytes\t{}", state.output_bytes),
            format!("last_attach_ms\t{}", state.last_attach_ms),
            format!("last_write_ms\t{}", state.last_write_ms),
            format!(
                "last_session\t{}",
                state
                    .sessions
                    .last()
                    .map(format_session)
                    .unwrap_or_else(|| String::from("-"))
            ),
        ]),
        "ATTACH" | "attach" => {
            let Some(session) = parse_attach(rest) else {
                state.rejected_count = state.rejected_count.wrapping_add(1);
                return err("invalid_session");
            };
            state.sessions.retain(|existing| {
                !(existing.distro == session.distro && existing.session_id == session.session_id)
            });
            state.sessions.push(session);
            state.attach_count = state.attach_count.wrapping_add(1);
            state.last_attach_ms = sys::uptime_ms();
            write_status(state);
            ok_lines(alloc::vec![String::from("attached\ttrue")])
        }
        "WRITE" | "write" => {
            let Some((distro, bytes)) = parse_write(rest) else {
                state.rejected_count = state.rejected_count.wrapping_add(1);
                return err("invalid_write");
            };
            let delivered = deliver_console_bytes(state, distro, &bytes);
            state.write_count = state.write_count.wrapping_add(1);
            state.output_bytes = state.output_bytes.wrapping_add(bytes.len() as u64);
            state.last_write_ms = sys::uptime_ms();
            write_status(state);
            ok_lines(alloc::vec![
                format!("delivered\t{}", delivered),
                format!("bytes\t{}", bytes.len()),
            ])
        }
        "CLEAR" | "clear" => {
            let distro = rest.trim();
            state.sessions.retain(|session| session.distro != distro);
            write_status(state);
            ok_lines(alloc::vec![format!("sessions\t{}", state.sessions.len())])
        }
        _ => err("unknown_command"),
    }
}

fn write_status(state: &ConsoleState) {
    let _ = fs::mkdir("/System/var");
    let _ = fs::mkdir("/System/var/asl");
    let mut text = format!(
        "health=ready\nsessions={}\nattach_count={}\nrejected_count={}\nwrite_count={}\noutput_bytes={}\nlast_attach_ms={}\nlast_write_ms={}\n",
        state.sessions.len(),
        state.attach_count,
        state.rejected_count,
        state.write_count,
        state.output_bytes,
        state.last_attach_ms,
        state.last_write_ms
    );
    if let Some(session) = state.sessions.last() {
        text.push_str(&format!("last_session={}\n", format_session(session)));
    }
    let _ = fs::write_bytes(STATUS_PATH, text.as_bytes());
}

fn parse_write(rest: &str) -> Option<(&str, Vec<u8>)> {
    let fields = split_tab_fields(rest);
    if fields.len() != 2 || !valid_token(fields[0]) {
        return None;
    }
    Some((fields[0], decode_hex(fields[1])?))
}

fn deliver_console_bytes(state: &ConsoleState, distro: &str, bytes: &[u8]) -> u32 {
    let mut delivered = 0u32;
    for session in state
        .sessions
        .iter()
        .filter(|session| session.distro == distro)
    {
        if session.console_pipe == "-" {
            continue;
        }
        let pipe = ipc::pipe_open(&session.console_pipe);
        if pipe == 0 || pipe == u32::MAX {
            continue;
        }
        let written = ipc::pipe_write(pipe, bytes);
        ipc::pipe_close(pipe);
        if written != 0 && written != u32::MAX {
            delivered = delivered.wrapping_add(1);
        }
    }
    delivered
}

fn parse_attach(rest: &str) -> Option<ConsoleSession> {
    let fields = split_tab_fields(rest);
    if fields.len() >= 5 {
        let session = ConsoleSession {
            distro: String::from(fields[0]),
            session_id: String::from(fields[1]),
            mode: String::from(fields[2]),
            console_pipe: String::from(fields[3]),
            stdin_pipe: String::from(fields[4]),
        };
        return valid_session(&session).then_some(session);
    }
    if rest.is_empty() || rest.contains("..") {
        return None;
    }
    let session = ConsoleSession {
        distro: String::from("unknown"),
        session_id: String::from(rest),
        mode: String::from("unknown"),
        console_pipe: String::from("-"),
        stdin_pipe: String::from("-"),
    };
    valid_session(&session).then_some(session)
}

fn valid_session(session: &ConsoleSession) -> bool {
    valid_token(&session.distro)
        && valid_token(&session.session_id)
        && matches!(
            session.mode.as_str(),
            "agent" | "fallback-console" | "unknown"
        )
        && valid_pipe(&session.console_pipe)
        && valid_pipe(&session.stdin_pipe)
}

fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && !value.contains("..")
        && value
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_'))
}

fn valid_pipe(value: &str) -> bool {
    value == "-"
        || (!value.is_empty()
            && !value.contains("..")
            && value.bytes().all(|b| {
                matches!(
                    b,
                    b'a'..=b'z'
                        | b'A'..=b'Z'
                        | b'0'..=b'9'
                        | b'-'
                        | b'_'
                        | b'.'
                        | b':'
                )
            }))
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        let high = hex_value(bytes[index])?;
        let low = hex_value(bytes[index + 1])?;
        out.push((high << 4) | low);
        index += 2;
    }
    Some(out)
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn format_session(session: &ConsoleSession) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        session.distro, session.session_id, session.mode, session.console_pipe, session.stdin_pipe
    )
}

fn ok_lines(lines: Vec<String>) -> String {
    let mut out = format!("OK\t{}\n", lines.len());
    for line in lines {
        out.push_str(&line);
        out.push('\n');
    }
    out.push('\n');
    out
}

fn err(code: &str) -> String {
    format!("ERR\t{}\t{}\n\n", code, code)
}

fn split_first_word(s: &str) -> (&str, &str) {
    let trimmed = s.trim();
    if let Some(pos) = trimmed.find(char::is_whitespace) {
        (&trimmed[..pos], trimmed[pos + 1..].trim())
    } else {
        (trimmed, "")
    }
}

fn split_tab_fields(rest: &str) -> Vec<&str> {
    rest.split('\t').collect()
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut value = 0u32;
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(value)
}
