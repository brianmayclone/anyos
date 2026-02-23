//! ami — CLI client for querying the amid (Anywhere Management Interface) daemon.
//!
//! Interactive REPL that sends SQL queries to amid via named pipe and
//! displays results as formatted tables in the terminal.
//!
//! Usage:
//!   ami                  — enter interactive REPL mode
//!   ami "SELECT ..."     — execute a single query and exit

#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

anyos_std::entry!(main);

// ── Constants ────────────────────────────────────────────────────────────────

/// Named pipe created by amid for receiving queries.
const AMID_PIPE: &str = "ami";

/// Maximum line length for REPL input.
const MAX_INPUT: usize = 1024;

/// Maximum response size (64 KiB).
const MAX_RESPONSE: usize = 65536;

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    // Get our TID for the response pipe name
    let tid = anyos_std::process::getpid();

    // Check if amid is running by trying to open its pipe
    let amid_pipe = anyos_std::ipc::pipe_open(AMID_PIPE);
    if amid_pipe == 0 {
        anyos_std::println!("ami: amid daemon is not running");
        anyos_std::println!("  Start it with: svc start amid");
        return;
    }

    // Create our response pipe
    let reply_name = format!("ami-{}", tid);
    let reply_pipe = anyos_std::ipc::pipe_create(&reply_name);
    if reply_pipe == 0 {
        anyos_std::println!("ami: failed to create response pipe");
        return;
    }

    // Check for single-shot query from args
    // process::args() already strips argv[0], returns just the arguments
    let mut args_buf = [0u8; 256];
    let args_str = anyos_std::process::args(&mut args_buf);
    let query_from_args = strip_quotes(args_str);

    if !query_from_args.is_empty() {
        // Single-shot mode: execute one query and exit
        execute_and_print(amid_pipe, reply_pipe, tid, &query_from_args);
        anyos_std::ipc::pipe_close(reply_pipe);
        return;
    }

    // Interactive REPL mode
    anyos_std::println!("ami — Anywhere Management Interface client");
    anyos_std::println!("Connected to amid (pipe='{}')", AMID_PIPE);
    anyos_std::println!("Type SQL queries (SELECT only) or 'exit' to quit.\n");

    let mut input_buf = [0u8; MAX_INPUT];

    loop {
        anyos_std::print!("ami> ");
        let line = read_line(&mut input_buf);
        if line.is_empty() { continue; }

        // Check for exit commands
        if line.eq_ignore_ascii_case("exit") || line.eq_ignore_ascii_case("quit")
            || line.eq_ignore_ascii_case("\\q") {
            break;
        }

        // Show help
        if line.eq_ignore_ascii_case("help") || line == "?" {
            print_help();
            continue;
        }

        // Show tables
        if line.eq_ignore_ascii_case("tables") || line.eq_ignore_ascii_case("\\dt") {
            anyos_std::println!("Tables: hw, mem, cpu, threads, devices, disks, net, svc");
            continue;
        }

        execute_and_print(amid_pipe, reply_pipe, tid, line);
    }

    anyos_std::ipc::pipe_close(reply_pipe);
}

// ── Query Execution ──────────────────────────────────────────────────────────

/// Send a query to amid and print the formatted result.
fn execute_and_print(amid_pipe: u32, reply_pipe: u32, tid: u32, sql: &str) {
    // Send request: "{tid}\t{sql}\n"
    let request = format!("{}\t{}\n", tid, sql);
    let written = anyos_std::ipc::pipe_write(amid_pipe, request.as_bytes());
    if written == u32::MAX {
        anyos_std::println!("Error: amid pipe disconnected");
        return;
    }

    // Wait for response with timeout (poll up to 3 seconds)
    let mut resp_buf = alloc::vec![0u8; MAX_RESPONSE];
    let start = anyos_std::sys::uptime_ms();
    let mut total_read = 0usize;

    loop {
        let n = anyos_std::ipc::pipe_read(reply_pipe, &mut resp_buf[total_read..]);
        if n > 0 && n != u32::MAX {
            total_read += n as usize;
            // Check for end marker (double newline)
            if total_read >= 2
                && resp_buf[total_read - 1] == b'\n'
                && resp_buf[total_read - 2] == b'\n'
            {
                break;
            }
        }

        let elapsed = anyos_std::sys::uptime_ms().wrapping_sub(start);
        if elapsed > 3000 {
            anyos_std::println!("Error: timeout waiting for amid response");
            return;
        }
        anyos_std::process::sleep(5);
    }

    if total_read == 0 {
        anyos_std::println!("Error: empty response from amid");
        return;
    }

    let resp = &resp_buf[..total_read];
    parse_and_display(resp);
}

// ── Response Parsing ─────────────────────────────────────────────────────────

/// Parse amid's TSV response and display as formatted table.
fn parse_and_display(resp: &[u8]) {
    let text = match core::str::from_utf8(resp) {
        Ok(s) => s,
        Err(_) => {
            anyos_std::println!("Error: invalid UTF-8 in response");
            return;
        }
    };

    // Split into lines
    let mut lines: Vec<&str> = text.split('\n').collect();
    // Remove trailing empty lines
    while lines.last() == Some(&"") {
        lines.pop();
    }

    if lines.is_empty() {
        anyos_std::println!("Error: empty response");
        return;
    }

    // First line: "OK\t{col_count}\t{row_count}" or "ERR\t{message}"
    let status_line = lines[0];
    if status_line.starts_with("ERR\t") {
        anyos_std::println!("Error: {}", &status_line[4..]);
        return;
    }

    if !status_line.starts_with("OK\t") {
        anyos_std::println!("Error: unexpected response: {}", status_line);
        return;
    }

    // Parse OK header
    let parts: Vec<&str> = status_line.split('\t').collect();
    if parts.len() < 3 {
        anyos_std::println!("Error: malformed OK header");
        return;
    }
    let col_count = parse_usize(parts[1]);
    let row_count = parse_usize(parts[2]);

    if col_count == 0 {
        anyos_std::println!("(empty result set)");
        return;
    }

    // Line 1: column names
    if lines.len() < 2 {
        anyos_std::println!("(empty result set)");
        return;
    }
    let col_names: Vec<&str> = lines[1].split('\t').collect();

    // Lines 2+: row data
    let mut rows: Vec<Vec<&str>> = Vec::new();
    for i in 2..lines.len() {
        if lines[i].is_empty() { continue; }
        let cells: Vec<&str> = lines[i].split('\t').collect();
        rows.push(cells);
    }

    // Calculate column widths
    let mut widths = Vec::with_capacity(col_count);
    for c in 0..col_count {
        let header_w = col_names.get(c).map(|s| s.len()).unwrap_or(0);
        let mut max_w = header_w;
        for row in &rows {
            if let Some(cell) = row.get(c) {
                if cell.len() > max_w {
                    max_w = cell.len();
                }
            }
        }
        widths.push(max_w.max(1));
    }

    // Print header
    print_row(&col_names, &widths, col_count);
    print_separator(&widths, col_count);

    // Print rows
    for row in &rows {
        print_row(row, &widths, col_count);
    }

    // Print summary
    anyos_std::println!("({} row{})", row_count, if row_count == 1 { "" } else { "s" });
}

/// Print a row with padded columns.
fn print_row(cells: &[&str], widths: &[usize], col_count: usize) {
    let mut line = String::new();
    for c in 0..col_count {
        if c > 0 { line.push_str(" | "); }
        let cell = cells.get(c).copied().unwrap_or("");
        line.push_str(cell);
        // Pad with spaces
        for _ in cell.len()..widths[c] {
            line.push(' ');
        }
    }
    anyos_std::println!("{}", line);
}

/// Print a separator line (dashes).
fn print_separator(widths: &[usize], col_count: usize) {
    let mut line = String::new();
    for c in 0..col_count {
        if c > 0 { line.push_str("-+-"); }
        for _ in 0..widths[c] {
            line.push('-');
        }
    }
    anyos_std::println!("{}", line);
}

// ── Input Reading ────────────────────────────────────────────────────────────

/// Read a line from stdin (fd 0), one byte at a time.
/// Returns the trimmed line as a &str.
fn read_line(buf: &mut [u8; MAX_INPUT]) -> &str {
    let mut pos = 0;
    loop {
        let mut byte = [0u8; 1];
        let n = anyos_std::fs::read(0, &mut byte);
        if n == 0 {
            anyos_std::process::sleep(10);
            continue;
        }
        if n == u32::MAX {
            // EOF / pipe closed
            return "";
        }
        match byte[0] {
            b'\n' | b'\r' => {
                anyos_std::print!("\n");
                break;
            }
            8 | 127 => {
                // Backspace
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
    let s = core::str::from_utf8(&buf[..pos]).unwrap_or("");
    s.trim()
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Parse a decimal string into usize.
fn parse_usize(s: &str) -> usize {
    let mut val = 0usize;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            val = val.saturating_mul(10).saturating_add((b - b'0') as usize);
        } else {
            break;
        }
    }
    val
}

/// Strip surrounding quotes from args if present.
fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        return &s[1..s.len() - 1];
    }
    if s.len() >= 2 && s.starts_with('\'') && s.ends_with('\'') {
        return &s[1..s.len() - 1];
    }
    s
}

/// Print help text.
fn print_help() {
    anyos_std::println!("Available commands:");
    anyos_std::println!("  SELECT ...   Execute a SQL query against amid's database");
    anyos_std::println!("  tables       List available tables");
    anyos_std::println!("  help         Show this help");
    anyos_std::println!("  exit         Quit the REPL");
    anyos_std::println!("");
    anyos_std::println!("Tables: hw, mem, cpu, threads, devices, disks, net, svc");
    anyos_std::println!("");
    anyos_std::println!("Examples:");
    anyos_std::println!("  SELECT * FROM hw");
    anyos_std::println!("  SELECT * FROM cpu");
    anyos_std::println!("  SELECT tid, name, state FROM threads");
    anyos_std::println!("  SELECT * FROM net WHERE key = 'ip'");
}
