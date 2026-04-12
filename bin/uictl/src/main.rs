//! uictl — UI automation / inspection tool for anyOS GUI applications.
//!
//! Usage:
//!   uictl windows                       List all GUI apps with accessibility support
//!   uictl tree <pid>                    Dump full control tree of an app
//!   uictl props <pid> <id>              Show one control's properties
//!   uictl click <pid> <id>              Fire a click event on a control
//!   uictl get-text <pid> <id>           Get text content of a control
//!   uictl set-text <pid> <id> <text>    Set text content of a control
//!   uictl get-state <pid> <id>          Get state of a control (0/1 etc.)
//!   uictl set-state <pid> <id> <val>    Set state of a control
//!   uictl find-text <pid> <text>        Find controls whose text contains <text>
//!   uictl resize <pid> <w> <h>          Resize window to w×h (logical pixels)
//!   uictl move <pid> <x> <y>            Move window to screen position x,y
//!   uictl focus <pid>                   Bring window to foreground
//!
//! The tool speaks the libanyui accessibility pipe protocol.
//! Each running GUI app exposes a named pipe "uiacc-<pid>".

#![no_std]
#![no_main]

use anyos_std::{String, Vec, format};

anyos_std::entry!(main);

// ── Protocol constants ────────────────────────────────────────────────────────

/// Max bytes per response read cycle.
const RSP_BUF: usize = 8192;
/// Milliseconds to wait for each response chunk.
const READ_TIMEOUT_MS: u32 = 200;
/// Total retry attempts while waiting for a response line.
const MAX_RETRIES: u32 = 25; // 25 × 200ms = 5 seconds

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if args.pos_count == 0 || raw.contains("--help") || raw.contains("-h") {
        print_usage();
        return;
    }

    let cmd = args.positional[0];

    match cmd {
        "windows" => cmd_windows(),
        "tree" => {
            if args.pos_count < 2 {
                anyos_std::println!("Usage: uictl tree <pid>");
                return;
            }
            if let Some(pid) = parse_u32(args.positional[1]) {
                cmd_send_recv(pid, "TREE\n", |line| {
                    if line == "TREE_END" { return false; }
                    if line.starts_with("CTRL\t") { anyos_std::println!("{}", fmt_ctrl(line)); }
                    else if line.starts_with("ERR\t") { anyos_std::println!("{}", &line[4..]); return false; }
                    true
                });
            } else {
                anyos_std::println!("Invalid pid: {}", args.positional[1]);
            }
        }
        "props" => {
            if args.pos_count < 3 {
                anyos_std::println!("Usage: uictl props <pid> <id>");
                return;
            }
            let (pid, id) = match (parse_u32(args.positional[1]), parse_u32(args.positional[2])) {
                (Some(p), Some(i)) => (p, i),
                _ => { anyos_std::println!("Invalid args"); return; }
            };
            let req = format!("PROPS\t{}\n", id);
            cmd_send_recv(pid, &req, |line| {
                if line.starts_with("CTRL\t") { anyos_std::println!("{}", fmt_ctrl_verbose(line)); }
                else { anyos_std::println!("{}", line); }
                false
            });
        }
        "click" => {
            if args.pos_count < 3 {
                anyos_std::println!("Usage: uictl click <pid> <id>");
                return;
            }
            let (pid, id) = match (parse_u32(args.positional[1]), parse_u32(args.positional[2])) {
                (Some(p), Some(i)) => (p, i),
                _ => { anyos_std::println!("Invalid args"); return; }
            };
            let req = format!("CLICK\t{}\n", id);
            cmd_send_recv_one(pid, &req);
        }
        "get-text" => {
            if args.pos_count < 3 {
                anyos_std::println!("Usage: uictl get-text <pid> <id>");
                return;
            }
            let (pid, id) = match (parse_u32(args.positional[1]), parse_u32(args.positional[2])) {
                (Some(p), Some(i)) => (p, i),
                _ => { anyos_std::println!("Invalid args"); return; }
            };
            let req = format!("GET_TEXT\t{}\n", id);
            cmd_send_recv(pid, &req, |line| {
                if let Some(text) = line.strip_prefix("TEXT\t") {
                    anyos_std::println!("{}", unescape_tab(text));
                } else {
                    anyos_std::println!("{}", line);
                }
                false
            });
        }
        "set-text" => {
            // Usage: uictl set-text <pid> <id> [--enter] <text>
            // --enter fires EVENT_SUBMIT after setting the text (simulates Enter key)
            if args.pos_count < 4 {
                anyos_std::println!("Usage: uictl set-text <pid> <id> [--enter] <text>");
                return;
            }
            let (pid, id) = match (parse_u32(args.positional[1]), parse_u32(args.positional[2])) {
                (Some(p), Some(i)) => (p, i),
                _ => { anyos_std::println!("Invalid args"); return; }
            };
            // Check for optional --enter flag before the text argument
            let (press_enter, text) = if args.pos_count >= 5 && args.positional[3] == "--enter" {
                (true, args.positional[4])
            } else {
                (false, args.positional[3])
            };
            let req = format!("SET_TEXT\t{}\t{}\n", id, text);
            cmd_send_recv_one(pid, &req);
            if press_enter {
                let submit_req = format!("SUBMIT\t{}\n", id);
                cmd_send_recv_one(pid, &submit_req);
            }
        }
        "submit" => {
            if args.pos_count < 3 {
                anyos_std::println!("Usage: uictl submit <pid> <id>");
                return;
            }
            let (pid, id) = match (parse_u32(args.positional[1]), parse_u32(args.positional[2])) {
                (Some(p), Some(i)) => (p, i),
                _ => { anyos_std::println!("Invalid args"); return; }
            };
            let req = format!("SUBMIT\t{}\n", id);
            cmd_send_recv_one(pid, &req);
        }
        "get-state" => {
            if args.pos_count < 3 {
                anyos_std::println!("Usage: uictl get-state <pid> <id>");
                return;
            }
            let (pid, id) = match (parse_u32(args.positional[1]), parse_u32(args.positional[2])) {
                (Some(p), Some(i)) => (p, i),
                _ => { anyos_std::println!("Invalid args"); return; }
            };
            let req = format!("GET_STATE\t{}\n", id);
            cmd_send_recv(pid, &req, |line| {
                if let Some(val) = line.strip_prefix("STATE\t") {
                    anyos_std::println!("{}", val);
                } else {
                    anyos_std::println!("{}", line);
                }
                false
            });
        }
        "set-state" => {
            if args.pos_count < 4 {
                anyos_std::println!("Usage: uictl set-state <pid> <id> <val>");
                return;
            }
            let (pid, id, val) = match (
                parse_u32(args.positional[1]),
                parse_u32(args.positional[2]),
                parse_u32(args.positional[3]),
            ) {
                (Some(p), Some(i), Some(v)) => (p, i, v),
                _ => { anyos_std::println!("Invalid args"); return; }
            };
            let req = format!("SET_STATE\t{}\t{}\n", id, val);
            cmd_send_recv_one(pid, &req);
        }
        "find-text" => {
            if args.pos_count < 3 {
                anyos_std::println!("Usage: uictl find-text <pid> <text>");
                return;
            }
            let pid = match parse_u32(args.positional[1]) {
                Some(p) => p,
                None => { anyos_std::println!("Invalid pid"); return; }
            };
            let text = args.positional[2];
            let req = format!("FIND_TEXT\t{}\n", text);
            cmd_send_recv(pid, &req, |line| {
                if line == "MATCH_END" { return false; }
                if let Some(rest) = line.strip_prefix("MATCH\t") {
                    let parts: Vec<&str> = rest.splitn(3, '\t').collect();
                    if parts.len() == 3 {
                        anyos_std::println!("  id={:<6} kind={:<24} text={}", parts[0], parts[1], unescape_tab(parts[2]));
                    }
                } else if line.starts_with("ERR\t") {
                    anyos_std::println!("{}", &line[4..]);
                    return false;
                }
                true
            });
        }
        "resize" => {
            if args.pos_count < 4 {
                anyos_std::println!("Usage: uictl resize <pid> <w> <h>");
                return;
            }
            let (pid, w, h) = match (
                parse_u32(args.positional[1]),
                parse_u32(args.positional[2]),
                parse_u32(args.positional[3]),
            ) {
                (Some(p), Some(w), Some(h)) => (p, w, h),
                _ => { anyos_std::println!("Invalid args"); return; }
            };
            let req = format!("RESIZE\t{}\t{}\n", w, h);
            cmd_send_recv_one(pid, &req);
        }
        "move" => {
            if args.pos_count < 4 {
                anyos_std::println!("Usage: uictl move <pid> <x> <y>");
                return;
            }
            let (pid, x, y) = match (
                parse_u32(args.positional[1]),
                parse_i32(args.positional[2]),
                parse_i32(args.positional[3]),
            ) {
                (Some(p), Some(x), Some(y)) => (p, x, y),
                _ => { anyos_std::println!("Invalid args"); return; }
            };
            let req = format!("MOVE\t{}\t{}\n", x, y);
            cmd_send_recv_one(pid, &req);
        }
        "focus" => {
            if args.pos_count < 2 {
                anyos_std::println!("Usage: uictl focus <pid>");
                return;
            }
            let pid = match parse_u32(args.positional[1]) {
                Some(p) => p,
                None => { anyos_std::println!("Invalid pid"); return; }
            };
            cmd_send_recv_one(pid, "FOCUS\n");
        }
        _ => {
            anyos_std::println!("Unknown command: {}", cmd);
            print_usage();
        }
    }
}

// ── Command: windows ──────────────────────────────────────────────────────────

fn cmd_windows() {
    anyos_std::println!("{:<8} {:<6} {:<20} {:>5} {:>5}  {}", "PID", "PPID", "Process", "W", "H", "Title");
    anyos_std::println!("{}", "─".repeat(65));

    // Enumerate all threads and probe for accessibility pipes.
    const ENTRY_SIZE: usize = 80;
    const MAX_THREADS: usize = 256;
    let mut buf = [0u8; ENTRY_SIZE * MAX_THREADS];
    let count = anyos_std::sys::sysinfo(1, &mut buf);
    if count == u32::MAX || count == 0 {
        anyos_std::println!("(no processes)");
        return;
    }
    let n = (count as usize).min(MAX_THREADS);

    let my_pid = anyos_std::process::getpid();
    let rsp_name = format!("uiacc-rsp-{}", my_pid);
    let rsp_id = anyos_std::ipc::pipe_create(&rsp_name);
    if rsp_id == 0 {
        anyos_std::println!("Error: could not create response pipe");
        return;
    }

    for i in 0..n {
        let off = i * ENTRY_SIZE;
        if off + ENTRY_SIZE > buf.len() { break; }
        let tid = u32::from_le_bytes([buf[off], buf[off+1], buf[off+2], buf[off+3]]);
        let ppid = u32::from_le_bytes([buf[off+60], buf[off+61], buf[off+62], buf[off+63]]);
        let name_bytes = &buf[off + 8..off + 32];
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(24);
        let proc_name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("?");

        // Try to open the accessibility request pipe for this process.
        let req_name = format!("uiacc-{}", tid);
        let req_id = anyos_std::ipc::pipe_open(&req_name);
        if req_id == 0 {
            continue; // Not a GUI app with accessibility
        }

        // Send HELLO with our response pipe name.
        let hello = format!("HELLO\t{}\n", rsp_name);
        anyos_std::ipc::pipe_write(req_id, hello.as_bytes());

        // Wait for READY response.
        let mut title = String::from("?");
        let mut w = 0u32;
        let mut h = 0u32;
        let mut rsp_buf = [0u8; 512];
        let mut accumulated = String::new();
        for _ in 0..10 {
            let n = anyos_std::ipc::pipe_read(rsp_id, &mut rsp_buf);
            if n > 0 && n != u32::MAX {
                if let Ok(s) = core::str::from_utf8(&rsp_buf[..n as usize]) {
                    accumulated.push_str(s);
                }
            }
            // Try to find a READY line
            if let Some(line_end) = accumulated.find('\n') {
                let line = accumulated[..line_end].trim_end_matches('\r');
                if let Some(rest) = line.strip_prefix("READY\t") {
                    let parts: Vec<&str> = rest.splitn(3, '\t').collect();
                    if parts.len() >= 3 {
                        title = String::from(parts[0]);
                        w = parse_u32(parts[1]).unwrap_or(0);
                        h = parse_u32(parts[2]).unwrap_or(0);
                    } else if parts.len() >= 1 {
                        title = String::from(parts[0]);
                    }
                    break;
                }
                break;
            }
            anyos_std::process::sleep(READ_TIMEOUT_MS / 10);
        }

        // Close the session.
        anyos_std::ipc::pipe_write(req_id, b"BYE\n");
        anyos_std::ipc::pipe_close(req_id);

        anyos_std::println!("{:<8} {:<6} {:<20} {:>5} {:>5}  {}", tid, ppid, proc_name, w, h, title);
    }

    anyos_std::ipc::pipe_close(rsp_id);
}

// ── Session helpers ───────────────────────────────────────────────────────────

/// Open a session with the target app and send a command.
/// `on_line` is called for each response line; return true to keep reading.
fn cmd_send_recv<F: FnMut(&str) -> bool>(pid: u32, request: &str, mut on_line: F) {
    let my_pid = anyos_std::process::getpid();
    let rsp_name = format!("uiacc-rsp-{}", my_pid);
    let rsp_id = anyos_std::ipc::pipe_create(&rsp_name);
    if rsp_id == 0 {
        anyos_std::println!("Error: could not create response pipe");
        return;
    }

    let req_name = format!("uiacc-{}", pid);
    let req_id = anyos_std::ipc::pipe_open(&req_name);
    if req_id == 0 {
        anyos_std::println!("Error: app pid={} not found or has no accessibility pipe", pid);
        anyos_std::ipc::pipe_close(rsp_id);
        return;
    }

    // Handshake
    let hello = format!("HELLO\t{}\n", rsp_name);
    anyos_std::ipc::pipe_write(req_id, hello.as_bytes());

    // Wait for READY
    if !wait_for_ready(rsp_id) {
        anyos_std::println!("Error: no READY response from pid={}", pid);
        anyos_std::ipc::pipe_close(req_id);
        anyos_std::ipc::pipe_close(rsp_id);
        return;
    }

    // Send the actual command
    anyos_std::ipc::pipe_write(req_id, request.as_bytes());

    // Read all response lines
    drain_responses(rsp_id, |line| on_line(line));

    anyos_std::ipc::pipe_write(req_id, b"BYE\n");
    anyos_std::ipc::pipe_close(req_id);
    anyos_std::ipc::pipe_close(rsp_id);
}

/// Like `cmd_send_recv` but reads exactly one response line (OK or ERR).
fn cmd_send_recv_one(pid: u32, request: &str) {
    cmd_send_recv(pid, request, |line| {
        anyos_std::println!("{}", line);
        false
    });
}

/// Wait for a READY line in the response pipe. Returns true if received.
fn wait_for_ready(rsp_id: u32) -> bool {
    let mut accumulated = [0u8; 512];
    let mut acc_len = 0usize;
    for _ in 0..MAX_RETRIES {
        let mut tmp = [0u8; 256];
        let n = anyos_std::ipc::pipe_read(rsp_id, &mut tmp);
        if n > 0 && n != u32::MAX {
            let to_copy = (n as usize).min(512 - acc_len);
            accumulated[acc_len..acc_len + to_copy].copy_from_slice(&tmp[..to_copy]);
            acc_len += to_copy;
        }
        // Scan for newline
        for i in 0..acc_len {
            if accumulated[i] == b'\n' {
                let line = core::str::from_utf8(&accumulated[..i]).unwrap_or("")
                    .trim_end_matches('\r');
                if line.starts_with("READY") {
                    return true;
                }
                return false;
            }
        }
        anyos_std::process::sleep(READ_TIMEOUT_MS);
    }
    false
}

/// Drain response lines from the pipe, calling `on_line` for each.
/// Stops when on_line returns false or after a read timeout.
fn drain_responses<F: FnMut(&str) -> bool>(rsp_id: u32, mut on_line: F) {
    let mut acc = [0u8; RSP_BUF];
    let mut acc_len = 0usize;
    let mut idle_count = 0u32;

    loop {
        let mut tmp = [0u8; RSP_BUF / 2];
        let n = anyos_std::ipc::pipe_read(rsp_id, &mut tmp);
        if n == 0 || n == u32::MAX {
            idle_count += 1;
            if idle_count > MAX_RETRIES {
                break;
            }
            anyos_std::process::sleep(READ_TIMEOUT_MS);
            continue;
        }
        idle_count = 0;

        let to_copy = (n as usize).min(RSP_BUF - acc_len);
        acc[acc_len..acc_len + to_copy].copy_from_slice(&tmp[..to_copy]);
        acc_len += to_copy;

        // Process all complete lines
        let mut start = 0usize;
        loop {
            let slice = &acc[start..acc_len];
            match slice.iter().position(|&b| b == b'\n') {
                None => break,
                Some(end) => {
                    let line_bytes = &slice[..end];
                    let line = core::str::from_utf8(line_bytes).unwrap_or("")
                        .trim_end_matches('\r');
                    let keep = on_line(line);
                    start += end + 1;
                    if !keep {
                        return;
                    }
                }
            }
        }
        // Shift remaining bytes
        if start > 0 {
            let remaining = acc_len - start;
            for i in 0..remaining {
                acc[i] = acc[start + i];
            }
            acc_len = remaining;
        }
    }
}

// ── Display helpers ───────────────────────────────────────────────────────────

/// Format a CTRL line for compact tree view.
/// CTRL\t<id>\t<parent>\t<kind>\t<ax>\t<ay>\t<w>\t<h>\t<vis>\t<dis>\t<state>\t<text>
fn fmt_ctrl(line: &str) -> String {
    let parts: Vec<&str> = line.splitn(13, '\t').collect();
    if parts.len() < 12 {
        return String::from(line);
    }
    let id    = parts[1];
    let kind  = parts[3];
    let x     = parts[4];
    let y     = parts[5];
    let w     = parts[6];
    let h     = parts[7];
    let vis   = if parts[8] == "1" { "" } else { " [hidden]" };
    let dis   = if parts[9] == "1" { " [disabled]" } else { "" };
    let text  = if parts.len() > 12 { parts[12] } else { parts.get(11).copied().unwrap_or("") };
    let text_part = if text.is_empty() { String::new() } else { format!("  \"{}\"", unescape_tab(text)) };
    format!("  id={:<6} {:>4},{:>4} {:>4}×{:>4}  {:<24}{}{}{}", id, x, y, w, h, kind, text_part, vis, dis)
}

/// Format a CTRL line for verbose (props) view.
fn fmt_ctrl_verbose(line: &str) -> String {
    let parts: Vec<&str> = line.splitn(13, '\t').collect();
    if parts.len() < 12 {
        return String::from(line);
    }
    let id    = parts[1];
    let parent = parts[2];
    let kind  = parts[3];
    let x     = parts[4];
    let y     = parts[5];
    let w     = parts[6];
    let h     = parts[7];
    let vis   = parts[8];
    let dis   = parts[9];
    let state = parts[10];
    let text  = if parts.len() > 12 { parts[12] } else { parts.get(11).copied().unwrap_or("") };
    format!(
        "id:     {}\nparent: {}\nkind:   {}\npos:    {},{}\nsize:   {}×{}\nvis:    {}\ndis:    {}\nstate:  {}\ntext:   {}",
        id, parent, kind, x, y, w, h, vis, dis, state, unescape_tab(text)
    )
}

// ── String utilities ──────────────────────────────────────────────────────────

fn parse_u32(s: &str) -> Option<u32> {
    s.trim().parse().ok()
}

fn parse_i32(s: &str) -> Option<i32> {
    s.trim().parse().ok()
}

fn unescape_tab(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'t') {
            chars.next();
            out.push('\t');
        } else {
            out.push(c);
        }
    }
    out
}

fn print_usage() {
    anyos_std::println!("uictl — UI automation tool for anyOS GUI apps\n");
    anyos_std::println!("Usage:");
    anyos_std::println!("  uictl windows                        List running GUI apps");
    anyos_std::println!("  uictl tree <pid>                     Dump control tree");
    anyos_std::println!("  uictl props <pid> <id>               Show control properties");
    anyos_std::println!("  uictl click <pid> <id>               Click a control");
    anyos_std::println!("  uictl get-text <pid> <id>            Get text of a control");
    anyos_std::println!("  uictl set-text <pid> <id> [--enter] <text>  Set text; optionally trigger Enter");
    anyos_std::println!("  uictl get-state <pid> <id>           Get state of a control");
    anyos_std::println!("  uictl set-state <pid> <id> <val>     Set state of a control");
    anyos_std::println!("  uictl find-text <pid> <text>         Find controls by text");
    anyos_std::println!("  uictl submit <pid> <id>              Fire Enter/submit on a control");
    anyos_std::println!("  uictl resize <pid> <w> <h>           Resize window");
    anyos_std::println!("  uictl move <pid> <x> <y>             Move window on screen");
    anyos_std::println!("  uictl focus <pid>                    Focus/raise window");
}
