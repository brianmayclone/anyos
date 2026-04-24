#![cfg_attr(not(target_os = "linux"), no_std)]
#![cfg_attr(not(target_os = "linux"), no_main)]

#[cfg(target_os = "linux")]
extern crate alloc;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::{fs, ipc, println, process, sys};
use libconf_schema::{
    default_bool, default_int, default_string, manifest, RegistryScope, ServiceSchema,
};
use libsvc::ServiceLifecycle;

#[cfg(not(target_os = "linux"))]
anyos_std::entry!(main);

const PIPE_NAME: &str = "aslnetd";
const STATUS_PATH: &str = "/System/var/asl/aslnetd.status";

const ASLNETD_DIRS: &[&str] = &["config"];
const ASLNETD_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_bool("config/nat_enabled", true),
    default_bool("config/dns_broker_enabled", true),
    default_string("config/default_listen_address", "127.0.0.1"),
    default_int("config/max_forward_rules", 256),
];
const ASLNETD_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "services/aslnetd",
    RegistryScope::System,
    1,
    ASLNETD_DIRS,
    ASLNETD_DEFAULTS,
    &[],
);
const ASLNETD_SCHEMA: ServiceSchema<'static> = ServiceSchema::new("aslnetd", &ASLNETD_MANIFEST);

#[derive(Clone)]
struct ForwardRule {
    distro: String,
    id: String,
    listen_address: String,
    listen_port: u16,
    guest_port: u16,
    protocol: String,
}

struct BrokerState {
    rules: Vec<ForwardRule>,
    apply_count: u32,
    rejected_count: u32,
    last_apply_ms: u32,
}

impl BrokerState {
    fn new() -> Self {
        Self {
            rules: Vec::new(),
            apply_count: 0,
            rejected_count: 0,
            last_apply_ms: 0,
        }
    }
}

fn main() {
    println!("aslnetd: starting");
    let _ = ASLNETD_SCHEMA.register();

    let mut lifecycle = ServiceLifecycle::connect("aslnetd").ok();
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
        println!("aslnetd: failed to create '{}' pipe", PIPE_NAME);
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("pipe_create_failed");
        }
        return;
    }

    let mut state = BrokerState::new();
    write_status(&state, "ready");
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }
    println!("aslnetd: ready (pipe='{}')", PIPE_NAME);

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
    state: &mut BrokerState,
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
        let mut line = pending[..pos].to_string();
        pending.drain(..=pos);
        if line.ends_with('\r') {
            line.pop();
        }
        if !line.is_empty() {
            handle_line(state, &line);
        }
    }
    true
}

fn handle_line(state: &mut BrokerState, line: &str) {
    let Some(tab_pos) = line.find('\t') else {
        return;
    };
    let Some(tid) = parse_u32(&line[..tab_pos]) else {
        return;
    };
    let cmd = line[tab_pos + 1..].trim();
    let response = dispatch(state, cmd);
    let reply_name = format!("aslnetd-{}", tid);
    let reply_pipe = ipc::pipe_open(&reply_name);
    if reply_pipe != 0 {
        ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn dispatch(state: &mut BrokerState, cmd: &str) -> String {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "STATUS" | "status" => status_response(state),
        "CLEAR" | "clear" => {
            let distro = rest.trim();
            if distro.is_empty() {
                return err("invalid_distro");
            }
            let before = state.rules.len();
            state.rules.retain(|rule| rule.distro != distro);
            state.apply_count = state.apply_count.wrapping_add(1);
            state.last_apply_ms = sys::uptime_ms();
            write_status(state, "ready");
            ok_lines(alloc::vec![
                format!("distro\t{}", distro),
                format!("removed\t{}", before.saturating_sub(state.rules.len())),
            ])
        }
        "APPLY" | "apply" => apply_rule(state, rest),
        "VALIDATE" | "validate" => validate_response(rest),
        _ => err("unknown_command"),
    }
}

fn apply_rule(state: &mut BrokerState, rest: &str) -> String {
    let fields = split_tab_fields(rest);
    if fields.len() < 6 {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_apply");
    }
    let Some(listen_port) = parse_u16(fields[3]) else {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_listen_port");
    };
    let Some(guest_port) = parse_u16(fields[4]) else {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("invalid_guest_port");
    };
    if let Err(message) = validate_rule(fields[1], fields[2], listen_port, guest_port, fields[5]) {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err(message);
    }

    let new_rule = ForwardRule {
        distro: String::from(fields[0]),
        id: String::from(fields[1]),
        listen_address: String::from(fields[2]),
        listen_port,
        guest_port,
        protocol: String::from(fields[5]),
    };
    if state.rules.iter().any(|rule| conflicts(rule, &new_rule)) {
        state.rejected_count = state.rejected_count.wrapping_add(1);
        return err("listener_conflict");
    }
    state
        .rules
        .retain(|rule| !(rule.distro == new_rule.distro && rule.id == new_rule.id));
    state.rules.push(new_rule);
    state.apply_count = state.apply_count.wrapping_add(1);
    state.last_apply_ms = sys::uptime_ms();
    write_status(state, "ready");
    ok_lines(alloc::vec![
        format!("rules\t{}", state.rules.len()),
        String::from("applied\ttrue"),
    ])
}

fn validate_response(rest: &str) -> String {
    let fields = split_tab_fields(rest);
    if fields.len() < 5 {
        return err("invalid_validate");
    }
    let Some(listen_port) = parse_u16(fields[2]) else {
        return err("invalid_listen_port");
    };
    let Some(guest_port) = parse_u16(fields[3]) else {
        return err("invalid_guest_port");
    };
    match validate_rule(fields[0], fields[1], listen_port, guest_port, fields[4]) {
        Ok(()) => ok_lines(alloc::vec![String::from("valid\ttrue")]),
        Err(message) => ok_lines(alloc::vec![
            String::from("valid\tfalse"),
            format!("message\t{}", message),
        ]),
    }
}

fn status_response(state: &BrokerState) -> String {
    let mut lines = alloc::vec![
        String::from("mode\tnat"),
        String::from("dns\thost-broker"),
        format!("rules\t{}", state.rules.len()),
        format!("apply_count\t{}", state.apply_count),
        format!("rejected_count\t{}", state.rejected_count),
        format!("last_apply_ms\t{}", state.last_apply_ms),
    ];
    if let Some(rule) = state.rules.last() {
        lines.push(format!(
            "last_rule\t{}\t{}\t{}:{}->{}\t{}",
            rule.distro,
            rule.id,
            rule.listen_address,
            rule.listen_port,
            rule.guest_port,
            rule.protocol
        ));
    }
    ok_lines(lines)
}

fn validate_rule(
    id: &str,
    listen_address: &str,
    listen_port: u16,
    guest_port: u16,
    protocol: &str,
) -> Result<(), &'static str> {
    if id.is_empty() || !id.bytes().all(valid_id_byte) {
        return Err("invalid_rule_id");
    }
    if !valid_listen_address(listen_address) {
        return Err("invalid_listen_address");
    }
    if listen_port == 0 || guest_port == 0 {
        return Err("invalid_port");
    }
    if protocol != "tcp" {
        return Err("invalid_protocol");
    }
    Ok(())
}

fn conflicts(left: &ForwardRule, right: &ForwardRule) -> bool {
    if left.protocol != right.protocol || left.listen_port != right.listen_port {
        return false;
    }
    let left_addr = normalize_listen_address(&left.listen_address);
    let right_addr = normalize_listen_address(&right.listen_address);
    left_addr == right_addr
        || (left_addr == "0.0.0.0" && valid_ipv4(right_addr))
        || (right_addr == "0.0.0.0" && valid_ipv4(left_addr))
}

fn normalize_listen_address(address: &str) -> &str {
    match address {
        "localhost" => "127.0.0.1",
        "*" => "0.0.0.0",
        other => other,
    }
}

fn valid_listen_address(address: &str) -> bool {
    matches!(address, "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "*") || valid_ipv4(address)
}

fn valid_ipv4(address: &str) -> bool {
    let mut parts = 0usize;
    for part in address.split('.') {
        parts += 1;
        if parts > 4 || part.is_empty() || part.len() > 3 {
            return false;
        }
        let mut value = 0u16;
        for b in part.bytes() {
            if !b.is_ascii_digit() {
                return false;
            }
            value = match value
                .checked_mul(10)
                .and_then(|v| v.checked_add((b - b'0') as u16))
            {
                Some(value) => value,
                None => return false,
            };
        }
        if value > 255 {
            return false;
        }
    }
    parts == 4
}

fn valid_id_byte(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_')
}

fn write_status(state: &BrokerState, health: &str) {
    let _ = fs::mkdir("/System/var");
    let _ = fs::mkdir("/System/var/asl");
    let mut text = format!(
        "health={}\nmode=nat\ndns=host-broker\nrules={}\napply_count={}\nrejected_count={}\nlast_apply_ms={}\n",
        health,
        state.rules.len(),
        state.apply_count,
        state.rejected_count,
        state.last_apply_ms
    );
    if let Some(rule) = state.rules.last() {
        text.push_str(&format!(
            "last_rule={}:{}:{}:{}->{}:{}\n",
            rule.distro,
            rule.id,
            rule.listen_address,
            rule.listen_port,
            rule.guest_port,
            rule.protocol
        ));
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

fn parse_u16(s: &str) -> Option<u16> {
    let value = parse_u32(s)?;
    u16::try_from(value).ok()
}
