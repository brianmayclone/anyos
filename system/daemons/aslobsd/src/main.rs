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

const PIPE_NAME: &str = "aslobsd";
const STATUS_PATH: &str = "/System/var/asl/aslobsd.status";

const DIRS: &[&str] = &["config"];
const DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_int("config/max_events", 512),
    default_bool("config/diagnostics_enabled", true),
];
const MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/aslobsd",
    RegistryScope::System,
    1,
    DIRS,
    DEFAULTS,
    &[],
);
const SCHEMA: ServiceSchema<'static> = ServiceSchema::new("aslobsd", &MANIFEST);

struct ObsState {
    events: u32,
    degraded_events: u32,
    last_event_ms: u32,
    last_event: String,
}

fn main() {
    println!("aslobsd: starting");
    let _ = SCHEMA.register();
    let mut lifecycle = ServiceLifecycle::connect("aslobsd").ok();
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
    let mut state = ObsState {
        events: 0,
        degraded_events: 0,
        last_event_ms: 0,
        last_event: String::new(),
    };
    write_status(&state);
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }
    println!("aslobsd: ready (pipe='{}')", PIPE_NAME);

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
    state: &mut ObsState,
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

fn handle_line(state: &mut ObsState, line: &str) {
    let Some(tab_pos) = line.find('\t') else {
        return;
    };
    let Some(tid) = parse_u32(&line[..tab_pos]) else {
        return;
    };
    let response = dispatch(state, line[tab_pos + 1..].trim());
    let reply_pipe = ipc::pipe_open(&format!("aslobsd-{}", tid));
    if reply_pipe != 0 {
        ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn dispatch(state: &mut ObsState, cmd: &str) -> String {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "STATUS" | "status" => ok_lines(alloc::vec![
            format!("events\t{}", state.events),
            format!("degraded_events\t{}", state.degraded_events),
            format!("last_event_ms\t{}", state.last_event_ms),
            format!(
                "last_event\t{}",
                if state.last_event.is_empty() {
                    "-"
                } else {
                    &state.last_event
                }
            ),
        ]),
        "EVENT" | "event" => {
            if rest.is_empty() || rest.contains('\n') || rest.contains('\r') {
                return err("invalid_event");
            }
            state.events = state.events.wrapping_add(1);
            if rest.contains("degraded") || rest.contains("failed") {
                state.degraded_events = state.degraded_events.wrapping_add(1);
            }
            state.last_event_ms = sys::uptime_ms();
            state.last_event = String::from(rest);
            write_status(state);
            ok_lines(alloc::vec![String::from("recorded\ttrue")])
        }
        "CLEAR" | "clear" => {
            state.events = 0;
            state.degraded_events = 0;
            state.last_event_ms = sys::uptime_ms();
            state.last_event.clear();
            write_status(state);
            ok_lines(alloc::vec![String::from("cleared\ttrue")])
        }
        _ => err("unknown_command"),
    }
}

fn write_status(state: &ObsState) {
    let _ = fs::mkdir("/System/var");
    let _ = fs::mkdir("/System/var/asl");
    let mut text = format!(
        "health=ready\nevents={}\ndegraded_events={}\nlast_event_ms={}\n",
        state.events, state.degraded_events, state.last_event_ms
    );
    if !state.last_event.is_empty() {
        text.push_str(&format!("last_event={}\n", state.last_event));
    }
    let _ = fs::write_bytes(STATUS_PATH, text.as_bytes());
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
