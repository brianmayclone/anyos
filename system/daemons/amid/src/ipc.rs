//! Pipe-based AMI v1 protocol handler.

use alloc::format;
use alloc::string::{String, ToString};

use libdb_client::Database;

use crate::{schema, AmiState, AmiValue, StateEntry};

const MAX_PENDING_REQUEST_BYTES: usize = 16 * 1024;
const MAX_REQUEST_LINE_BYTES: usize = 4 * 1024;

pub fn handle_requests(db: &Database, state: &mut AmiState, pipe_id: u32, buf: &mut [u8]) -> bool {
    let n = anyos_std::ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }

    let data = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(text) => text,
        Err(_) => return true,
    };
    if state.pending_request.len().saturating_add(data.len()) > MAX_PENDING_REQUEST_BYTES {
        state.pending_request.clear();
        return true;
    }
    state.pending_request.push_str(data);

    while let Some(pos) = state.pending_request.find('\n') {
        let mut line = state.pending_request[..pos].to_string();
        state.pending_request.drain(..=pos);
        if line.len() > MAX_REQUEST_LINE_BYTES {
            continue;
        }
        if line.ends_with('\r') {
            line.pop();
        }
        if !line.is_empty() {
            handle_single_request(db, state, &line);
        }
    }

    true
}

fn handle_single_request(db: &Database, state: &mut AmiState, line: &str) {
    let Some(tab_pos) = line.find('\t') else {
        return;
    };
    let tid = match parse_u32(&line[..tab_pos]) {
        Some(v) => v,
        None => return,
    };
    let cmd = line[tab_pos + 1..].trim();
    if cmd.is_empty() {
        return;
    }

    dispatch(db, state, tid, cmd);
}

fn dispatch(db: &Database, state: &mut AmiState, tid: u32, cmd: &str) {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "HELLO" | "hello" => cmd_hello(state, tid, rest),
        "SET" | "set" => cmd_set(db, state, tid, rest),
        "GET" | "get" => cmd_get(state, tid, rest),
        "DEL" | "del" => cmd_del(db, state, tid, rest),
        "LIST" | "list" => cmd_list(state, tid, rest),
        "WATCH" | "watch" => cmd_watch(state, tid, rest),
        "UNWATCH" | "unwatch" => cmd_unwatch(state, tid, rest),
        "PING" | "ping" => {
            let _ = send_line(tid, "PONG");
        }
        _ => {
            let _ = send_line(tid, "ERR unknown_command");
        }
    }
}

fn cmd_hello(state: &mut AmiState, tid: u32, service: &str) {
    if !is_valid_service(service) {
        send_line(tid, "ERR invalid_service");
        return;
    }
    state.set_service(tid, service);
    let mut resp = String::from("OK hello ");
    resp.push_str(service);
    send_line(tid, &resp);
}

fn cmd_set(db: &Database, state: &mut AmiState, tid: u32, rest: &str) {
    let mut parts = rest.splitn(3, ' ');
    let Some(key) = parts.next() else {
        send_line(tid, "ERR invalid_set");
        return;
    };
    let Some(type_name) = parts.next() else {
        send_line(tid, "ERR invalid_set");
        return;
    };
    let Some(raw_value) = parts.next() else {
        send_line(tid, "ERR invalid_set");
        return;
    };

    if !is_valid_key(key) {
        send_line(tid, "ERR invalid_key");
        return;
    }

    let Some(service) = state.service_for(tid).map(String::from) else {
        send_line(tid, "ERR identify_required");
        return;
    };
    if !service_can_write(&service, key) {
        send_line(tid, "ERR forbidden");
        return;
    }

    let value = match decode_value(type_name, raw_value) {
        Some(value) => value,
        None => {
            send_line(tid, "ERR invalid_value");
            return;
        }
    };

    let now = anyos_std::sys::uptime_ms() as u64;
    let (entry, changed) = state.upsert_entry(key, value, &service, now);
    if changed {
        if schema::persist_entry(db, &entry).is_err() {
            send_line(tid, "ERR persist_failed");
            return;
        }
    }

    let mut resp = String::from("OK set ");
    resp.push_str(&entry.key);
    resp.push(' ');
    push_u64(&mut resp, entry.version);
    resp.push(' ');
    push_u64(&mut resp, entry.updated_at);
    send_line(tid, &resp);

    if changed {
        emit_set_events(state, &entry);
    }
}

fn cmd_get(state: &AmiState, tid: u32, key: &str) {
    if !is_valid_key(key) {
        send_line(tid, "ERR invalid_key");
        return;
    }
    let Some(entry) = state.find_entry(key) else {
        send_line(tid, "ERR not_found");
        return;
    };
    send_line(tid, &format_value_line("VALUE", entry));
}

fn cmd_del(db: &Database, state: &mut AmiState, tid: u32, key: &str) {
    if !is_valid_key(key) {
        send_line(tid, "ERR invalid_key");
        return;
    }

    let Some(service) = state.service_for(tid) else {
        send_line(tid, "ERR identify_required");
        return;
    };
    if !service_can_write(service, key) {
        send_line(tid, "ERR forbidden");
        return;
    }

    let Some(old_entry) = state.remove_entry(key) else {
        send_line(tid, "ERR not_found");
        return;
    };

    if schema::delete_entry(db, key).is_err() {
        state.entries.push(old_entry);
        send_line(tid, "ERR persist_failed");
        return;
    }

    let version = old_entry.version.saturating_add(1);
    let updated_at = anyos_std::sys::uptime_ms() as u64;

    let mut resp = String::from("OK del ");
    resp.push_str(key);
    resp.push(' ');
    push_u64(&mut resp, version);
    resp.push(' ');
    push_u64(&mut resp, updated_at);
    send_line(tid, &resp);

    emit_delete_events(state, key, version, updated_at);
}

fn cmd_list(state: &AmiState, tid: u32, prefix: &str) {
    if !is_valid_prefix(prefix) {
        send_line(tid, "ERR invalid_prefix");
        return;
    }
    let items = state.list_prefix(prefix);
    for entry in &items {
        send_line(tid, &format_value_line("ITEM", entry));
    }
    send_line(tid, "END");
}

fn cmd_watch(state: &mut AmiState, tid: u32, prefix: &str) {
    if !is_valid_prefix(prefix) {
        send_line(tid, "ERR invalid_prefix");
        return;
    }
    let watch_id = state.add_watch(tid, prefix);
    if watch_id == 0 {
        send_line(tid, "ERR watch_limit");
        return;
    }
    let mut resp = String::from("OK watch ");
    push_u32(&mut resp, watch_id);
    send_line(tid, &resp);
}

fn cmd_unwatch(state: &mut AmiState, tid: u32, raw_id: &str) {
    let Some(watch_id) = parse_u32(raw_id) else {
        send_line(tid, "ERR invalid_watch_id");
        return;
    };
    if !state.remove_watch(tid, watch_id) {
        send_line(tid, "ERR not_found");
        return;
    }
    let mut resp = String::from("OK unwatch ");
    push_u32(&mut resp, watch_id);
    send_line(tid, &resp);
}

fn emit_set_events(state: &mut AmiState, entry: &StateEntry) {
    let watchers = state.matching_watch_ids(&entry.key);
    if watchers.is_empty() {
        return;
    }

    let (type_name, value_str) = encode_value(&entry.value);
    for (tid, watch_id) in watchers {
        let mut msg = String::from("EVENT ");
        push_u32(&mut msg, watch_id);
        msg.push_str(" set ");
        msg.push_str(&entry.key);
        msg.push(' ');
        msg.push_str(type_name);
        msg.push(' ');
        msg.push_str(&value_str);
        msg.push(' ');
        push_u64(&mut msg, entry.version);
        msg.push(' ');
        push_u64(&mut msg, entry.updated_at);
        if !send_line(tid, &msg) {
            state.remove_client(tid);
        }
    }
}

fn emit_delete_events(state: &mut AmiState, key: &str, version: u64, updated_at: u64) {
    let watchers = state.matching_watch_ids(key);
    if watchers.is_empty() {
        return;
    }

    for (tid, watch_id) in watchers {
        let mut msg = String::from("EVENT ");
        push_u32(&mut msg, watch_id);
        msg.push_str(" delete ");
        msg.push_str(key);
        msg.push_str(" string - ");
        push_u64(&mut msg, version);
        msg.push(' ');
        push_u64(&mut msg, updated_at);
        if !send_line(tid, &msg) {
            state.remove_client(tid);
        }
    }
}

fn format_value_line(kind: &str, entry: &StateEntry) -> String {
    let (type_name, value_str) = encode_value(&entry.value);
    let mut line = String::from(kind);
    line.push(' ');
    line.push_str(&entry.key);
    line.push(' ');
    line.push_str(type_name);
    line.push(' ');
    line.push_str(&value_str);
    line.push(' ');
    push_u64(&mut line, entry.version);
    line.push(' ');
    push_u64(&mut line, entry.updated_at);
    line
}

fn send_line(tid: u32, line: &str) -> bool {
    let reply_pipe_name = format!("ami-{}", tid);
    let reply_pipe = anyos_std::ipc::pipe_open(&reply_pipe_name);
    if reply_pipe == 0 || reply_pipe == u32::MAX {
        return false;
    }
    let ok = anyos_std::ipc::pipe_write(reply_pipe, line.as_bytes()) != u32::MAX
        && anyos_std::ipc::pipe_write(reply_pipe, b"\n") != u32::MAX;
    ok
}

fn split_first_word(input: &str) -> (&str, &str) {
    if let Some(pos) = input.find(' ') {
        (&input[..pos], input[pos + 1..].trim())
    } else {
        (input, "")
    }
}

fn decode_value(type_name: &str, raw: &str) -> Option<AmiValue> {
    match type_name {
        "string" => Some(AmiValue::String(String::from(raw))),
        "int" => parse_i64(raw).map(AmiValue::Int),
        "bool" => match raw {
            "true" => Some(AmiValue::Bool(true)),
            "false" => Some(AmiValue::Bool(false)),
            _ => None,
        },
        _ => None,
    }
}

fn encode_value(value: &AmiValue) -> (&'static str, String) {
    match value {
        AmiValue::String(s) => ("string", String::from(s.as_str())),
        AmiValue::Int(v) => ("int", format!("{}", *v)),
        AmiValue::Bool(v) => (
            "bool",
            if *v {
                String::from("true")
            } else {
                String::from("false")
            },
        ),
    }
}

fn service_can_write(service: &str, key: &str) -> bool {
    let svc_key = format!("svc.{}.", service);
    if key.starts_with(&svc_key) {
        return true;
    }

    let prefixes: &[&str] = match service {
        "amid" => &["amid."],
        "compositor" => &["compositor."],
        "dnsd" => &["dns."],
        "fontd" => &["fontd."],
        "init" => &["system.", "init."],
        "networkd" => &["net."],
        "notifyd" => &["notify."],
        "searchd" => &["search."],
        "sessionhost" => &["session."],
        "svc" => &["svc."],
        "updater" => &["update."],
        _ => &[],
    };
    prefixes.iter().any(|prefix| key.starts_with(prefix))
}

fn is_valid_service(service: &str) -> bool {
    !service.is_empty()
        && service
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-'))
}

fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

fn is_valid_prefix(prefix: &str) -> bool {
    prefix.is_empty() || is_valid_key(prefix)
}

fn parse_u32(s: &str) -> Option<u32> {
    let mut val = 0u32;
    if s.is_empty() {
        return None;
    }
    for b in s.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(val)
}

fn parse_i64(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let (negative, start) = if bytes[0] == b'-' {
        (true, 1usize)
    } else {
        (false, 0usize)
    };
    if start >= bytes.len() {
        return None;
    }
    let mut val = 0i64;
    for &b in &bytes[start..] {
        if !b.is_ascii_digit() {
            return None;
        }
        val = val.checked_mul(10)?.checked_add((b - b'0') as i64)?;
    }
    Some(if negative { -val } else { val })
}

fn push_u32(out: &mut String, mut v: u32) {
    if v == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut n = 0usize;
    while v > 0 {
        buf[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        out.push(buf[n] as char);
    }
}

fn push_u64(out: &mut String, mut v: u64) {
    if v == 0 {
        out.push('0');
        return;
    }
    let mut buf = [0u8; 20];
    let mut n = 0usize;
    while v > 0 {
        buf[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    while n > 0 {
        n -= 1;
        out.push(buf[n] as char);
    }
}
