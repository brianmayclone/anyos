//! ami — CLI client for the AMI v1 state service.

#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libami::{AmiClient, AmiError, AmiEvent, AmiEventKind, AmiItem, AmiValue};

anyos_std::entry!(main);

const MAX_INPUT: usize = 1024;

fn main() {
    let mut client = match AmiClient::connect("amid") {
        Ok(client) => client,
        Err(_) => {
            anyos_std::println!("ami: amid daemon is not running");
            return;
        }
    };

    let mut args_buf = [0u8; 256];
    let args_str = anyos_std::process::args(&mut args_buf);
    if args_str.contains("--help") {
        print_help();
        return;
    }

    let single = strip_quotes(args_str);
    if !single.is_empty() {
        run_command(&mut client, single);
        return;
    }

    anyos_std::println!("ami — Anywhere Management Interface client");
    anyos_std::println!("Commands: get, list, watch, namespaces, services, ping, help, exit\n");

    let mut input_buf = [0u8; MAX_INPUT];
    loop {
        anyos_std::print!("ami> ");
        let line = read_line(&mut input_buf);
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case("exit") || line.eq_ignore_ascii_case("quit") || line == "\\q" {
            break;
        }
        if line.eq_ignore_ascii_case("help") || line == "?" {
            print_help();
            continue;
        }
        run_command(&mut client, line);
    }
}

fn run_command(client: &mut AmiClient, line: &str) {
    let (verb, rest) = split_first_word(line.trim());
    match verb {
        "get" | "GET" => cmd_get(client, rest),
        "list" | "LIST" => cmd_list(client, rest),
        "watch" | "WATCH" => cmd_watch(client, rest),
        "namespaces" | "NAMESPACES" => cmd_namespaces(client),
        "services" | "SERVICES" => cmd_services(client),
        "ping" | "PING" => cmd_ping(client),
        "set" | "SET" => cmd_set(client, rest),
        "del" | "DEL" => cmd_del(client, rest),
        "" => {}
        _ => anyos_std::println!("Unknown command: {}", verb),
    }
}

fn cmd_get(client: &mut AmiClient, key: &str) {
    match client.get(key) {
        Ok(item) => print_item(&item),
        Err(err) => print_error(err),
    }
}

fn cmd_list(client: &mut AmiClient, prefix: &str) {
    match client.list(prefix) {
        Ok(items) => {
            if items.is_empty() {
                anyos_std::println!("(empty)");
                return;
            }
            for item in &items {
                print_item(item);
            }
        }
        Err(err) => print_error(err),
    }
}

fn cmd_watch(client: &mut AmiClient, prefix: &str) {
    let watch_id = match client.watch(prefix) {
        Ok(id) => id,
        Err(err) => {
            print_error(err);
            return;
        }
    };

    anyos_std::println!("Watching '{}' (id={})", prefix, watch_id);
    loop {
        match client.poll_event(1000) {
            Ok(Some(event)) => print_event(&event),
            Ok(None) => {}
            Err(err) => {
                print_error(err);
                return;
            }
        }
    }
}

fn cmd_ping(client: &mut AmiClient) {
    match client.ping() {
        Ok(()) => anyos_std::println!("PONG"),
        Err(err) => print_error(err),
    }
}

fn cmd_namespaces(client: &mut AmiClient) {
    match client.list("") {
        Ok(items) => {
            let mut roots = Vec::new();
            for item in &items {
                let root = first_segment(&item.key);
                if !root.is_empty() && !roots.iter().any(|v: &String| v == &root) {
                    roots.push(root);
                }
            }
            if roots.is_empty() {
                anyos_std::println!("(empty)");
                return;
            }
            for root in &roots {
                anyos_std::println!("{}.", root);
            }
        }
        Err(err) => print_error(err),
    }
}

fn cmd_services(client: &mut AmiClient) {
    match client.list("svc.") {
        Ok(items) => {
            let mut services = Vec::new();
            for item in &items {
                let name = service_name_from_key(&item.key);
                if !name.is_empty() && !services.iter().any(|v: &String| v == &name) {
                    services.push(name);
                }
            }
            if services.is_empty() {
                anyos_std::println!("(no service state published)");
                return;
            }
            for service in &services {
                let state = field_value(&items, service, "state").unwrap_or_else(|| String::from("unknown"));
                let ready = field_value(&items, service, "ready").unwrap_or_else(|| String::from("false"));
                let health = field_value(&items, service, "health").unwrap_or_else(|| String::new());
                if health.is_empty() {
                    anyos_std::println!("{}: state={} ready={}", service, state, ready);
                } else {
                    anyos_std::println!("{}: state={} ready={} health={}", service, state, ready, health);
                }
            }
        }
        Err(err) => print_error(err),
    }
}

fn cmd_set(client: &mut AmiClient, rest: &str) {
    let mut parts = rest.splitn(3, ' ');
    let Some(key) = parts.next() else {
        anyos_std::println!("Usage: set <key> <type> <value>");
        return;
    };
    let Some(ty) = parts.next() else {
        anyos_std::println!("Usage: set <key> <type> <value>");
        return;
    };
    let Some(raw) = parts.next() else {
        anyos_std::println!("Usage: set <key> <type> <value>");
        return;
    };

    let value = match parse_value(ty, raw) {
        Some(value) => value,
        None => {
            anyos_std::println!("Invalid value");
            return;
        }
    };

    match client.set(key, value) {
        Ok(item) => {
            anyos_std::println!("Updated {} (v{}, t={})", item.key, item.version, item.updated_at);
        }
        Err(err) => print_error(err),
    }
}

fn cmd_del(client: &mut AmiClient, key: &str) {
    match client.del(key) {
        Ok(()) => anyos_std::println!("Deleted {}", key),
        Err(err) => print_error(err),
    }
}

fn parse_value(ty: &str, raw: &str) -> Option<AmiValue> {
    match ty {
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

fn print_item(item: &AmiItem) {
    anyos_std::println!(
        "{} = {} (v{}, t={})",
        item.key,
        value_to_string(&item.value),
        item.version,
        item.updated_at
    );
}

fn print_event(event: &AmiEvent) {
    match event.kind {
        AmiEventKind::Set => {
            if let Some(value) = &event.value {
                anyos_std::println!(
                    "[watch {}] {} = {} (v{}, t={})",
                    event.watch_id,
                    event.key,
                    value_to_string(value),
                    event.version,
                    event.updated_at
                );
            }
        }
        AmiEventKind::Delete => {
            anyos_std::println!(
                "[watch {}] deleted {} (v{}, t={})",
                event.watch_id,
                event.key,
                event.version,
                event.updated_at
            );
        }
    }
}

fn print_error(err: AmiError) {
    match err {
        AmiError::Remote(msg) | AmiError::Protocol(msg) => anyos_std::println!("Error: {}", msg),
        AmiError::NotRunning => anyos_std::println!("Error: amid daemon is not running"),
        AmiError::PipeCreateFailed => anyos_std::println!("Error: failed to create reply pipe"),
        AmiError::Disconnected => anyos_std::println!("Error: amid pipe disconnected"),
        AmiError::Timeout => anyos_std::println!("Error: timeout waiting for amid"),
        AmiError::InvalidArgument(msg) => anyos_std::println!("Error: {}", msg),
    }
}

fn value_to_string(value: &AmiValue) -> String {
    match value {
        AmiValue::String(s) => String::from(s.as_str()),
        AmiValue::Int(v) => format!("{}", *v),
        AmiValue::Bool(v) => {
            if *v { String::from("true") } else { String::from("false") }
        }
    }
}

fn first_segment(key: &str) -> String {
    if let Some(pos) = key.find('.') {
        String::from(&key[..pos])
    } else {
        String::from(key)
    }
}

fn service_name_from_key(key: &str) -> String {
    if !key.starts_with("svc.") {
        return String::new();
    }
    let rest = &key[4..];
    if let Some(pos) = rest.find('.') {
        String::from(&rest[..pos])
    } else {
        String::new()
    }
}

fn field_value(items: &[AmiItem], service: &str, field: &str) -> Option<String> {
    let prefix = format!("svc.{}.{}", service, field);
    for item in items {
        if item.key == prefix {
            return Some(value_to_string(&item.value));
        }
    }
    None
}

fn split_first_word(input: &str) -> (&str, &str) {
    if let Some(pos) = input.find(' ') {
        (&input[..pos], input[pos + 1..].trim())
    } else {
        (input, "")
    }
}

fn parse_i64(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    let bytes = s.as_bytes();
    let (neg, start) = if bytes[0] == b'-' { (true, 1usize) } else { (false, 0usize) };
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
    Some(if neg { -val } else { val })
}

fn read_line(buf: &mut [u8; MAX_INPUT]) -> &str {
    let mut pos = 0usize;
    loop {
        let mut byte = [0u8; 1];
        let n = anyos_std::fs::read(0, &mut byte);
        if n == 0 {
            anyos_std::process::sleep(10);
            continue;
        }
        if n == u32::MAX {
            return "";
        }
        match byte[0] {
            b'\n' | b'\r' => {
                anyos_std::print!("\n");
                break;
            }
            8 | 127 => {
                if pos > 0 {
                    pos -= 1;
                    anyos_std::print!("\x08 \x08");
                }
            }
            b if b >= 0x20 && pos < MAX_INPUT - 1 => {
                buf[pos] = b;
                pos += 1;
                anyos_std::print!("{}", b as char);
            }
            _ => {}
        }
    }
    core::str::from_utf8(&buf[..pos]).unwrap_or("").trim()
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        &s[1..s.len() - 1]
    } else if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

fn print_help() {
    anyos_std::println!("ami - Anywhere Management Interface client");
    anyos_std::println!("");
    anyos_std::println!("Usage:");
    anyos_std::println!("  ami                   Start interactive mode");
    anyos_std::println!("  ami \"get dns.status\" Run a single command");
    anyos_std::println!("");
    anyos_std::println!("Commands:");
    anyos_std::println!("  get <key>             Read one key");
    anyos_std::println!("  list <prefix>         List keys below a prefix");
    anyos_std::println!("  watch <prefix>        Watch live changes");
    anyos_std::println!("  namespaces            List top-level prefixes");
    anyos_std::println!("  services              Summarize svc.* service state");
    anyos_std::println!("  ping                  Check liveness");
    anyos_std::println!("  set <key> <t> <v>     Debug write (subject to AMID policy)");
    anyos_std::println!("  del <key>             Debug delete (subject to AMID policy)");
    anyos_std::println!("  help                  Show help");
    anyos_std::println!("  exit                  Quit");
}
