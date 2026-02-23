//! Pipe-based SQL query IPC handler.
//!
//! Protocol:
//! - ami creates named pipe `"ami"` at startup
//! - Client creates response pipe `"ami-{tid}"`, sends `{tid}\t{sql}\n` to `"ami"`
//! - ami parses request, executes SELECT via libdb, writes TSV result to `"ami-{tid}"`
//!
//! Request format:  `{tid}\t{sql}\n`
//! Response (SELECT): `OK\t{col_count}\t{row_count}\n{col1}\t{col2}\n{v1}\t{v2}\n...\n\n`
//! Response (error):  `ERR\t{message}\n\n`

use alloc::format;
use alloc::string::String;
use libdb_client::Database;

/// Handle incoming pipe requests. Non-blocking — returns immediately if no data.
///
/// Reads from the ami pipe, parses requests, executes queries, and writes
/// responses back to per-client response pipes.
pub fn handle_requests(db: &Database, pipe_id: u32, buf: &mut [u8]) -> bool {
    let n = anyos_std::ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }

    let data = &buf[..n as usize];

    // Process line by line (multiple requests may arrive in one read)
    let mut line_start = 0;
    for i in 0..data.len() {
        if data[i] == b'\n' {
            if i > line_start {
                handle_single_request(db, &data[line_start..i]);
            }
            line_start = i + 1;
        }
    }
    // Handle trailing data without newline
    if line_start < data.len() {
        handle_single_request(db, &data[line_start..]);
    }

    true
}

/// Parse and execute a single request line: `{tid}\t{sql}`.
fn handle_single_request(db: &Database, line: &[u8]) {
    // Find tab separator
    let tab_pos = match line.iter().position(|&b| b == b'\t') {
        Some(pos) => pos,
        None => return, // Malformed — no tab separator
    };

    // Parse TID
    let tid_str = match core::str::from_utf8(&line[..tab_pos]) {
        Ok(s) => s,
        Err(_) => return,
    };
    let tid: u32 = match parse_u32(tid_str) {
        Some(v) => v,
        None => return,
    };

    // Extract SQL
    let sql = match core::str::from_utf8(&line[tab_pos + 1..]) {
        Ok(s) => s.trim(),
        Err(_) => return,
    };
    if sql.is_empty() { return; }

    // Security: only allow SELECT queries via pipe (read-only access)
    let sql_upper = sql.as_bytes();
    let is_select = sql_upper.len() >= 6
        && (sql_upper[0] == b'S' || sql_upper[0] == b's')
        && (sql_upper[1] == b'E' || sql_upper[1] == b'e')
        && (sql_upper[2] == b'L' || sql_upper[2] == b'l')
        && (sql_upper[3] == b'E' || sql_upper[3] == b'e')
        && (sql_upper[4] == b'C' || sql_upper[4] == b'c')
        && (sql_upper[5] == b'T' || sql_upper[5] == b't');

    let response = if !is_select {
        String::from("ERR\tOnly SELECT queries are allowed\n\n")
    } else {
        execute_query(db, sql)
    };

    // Open client's response pipe and write result
    let reply_pipe_name = format!("ami-{}", tid);
    let reply_pipe = anyos_std::ipc::pipe_open(&reply_pipe_name);
    if reply_pipe != 0 {
        anyos_std::ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

/// Execute a SELECT query and format the result as TSV.
fn execute_query(db: &Database, sql: &str) -> String {
    match db.query(sql) {
        Ok(result) => {
            let col_count = result.col_count();
            let row_count = result.row_count();

            let mut resp = format!("OK\t{}\t{}\n", col_count, row_count);

            // Column names
            for c in 0..col_count {
                if c > 0 { resp.push('\t'); }
                resp.push_str(&result.col_name(c));
            }
            resp.push('\n');

            // Row data
            for r in 0..row_count {
                for c in 0..col_count {
                    if c > 0 { resp.push('\t'); }
                    if result.is_null(r, c) {
                        resp.push_str("NULL");
                    } else if let Some(i) = result.get_int(r, c) {
                        push_i64(&mut resp, i);
                    } else if let Some(ref t) = result.get_text(r, c) {
                        // Escape tabs and newlines in text values
                        for ch in t.chars() {
                            match ch {
                                '\t' => resp.push_str("\\t"),
                                '\n' => resp.push_str("\\n"),
                                _ => resp.push(ch),
                            }
                        }
                    } else {
                        resp.push_str("NULL");
                    }
                }
                resp.push('\n');
            }
            // End marker
            resp.push('\n');
            resp
        }
        Err(e) => {
            format!("ERR\t{}\n\n", e)
        }
    }
}

/// Append an i64 as decimal to a string (no_std-friendly).
fn push_i64(s: &mut String, v: i64) {
    if v < 0 {
        s.push('-');
        // Handle i64::MIN carefully
        if v == i64::MIN {
            s.push_str("9223372036854775808");
            return;
        }
        push_u64(s, (-v) as u64);
    } else {
        push_u64(s, v as u64);
    }
}

/// Append a u64 as decimal to a string.
fn push_u64(s: &mut String, v: u64) {
    if v == 0 {
        s.push('0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut pos = 20;
    let mut val = v;
    while val > 0 {
        pos -= 1;
        digits[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    for &d in &digits[pos..] {
        s.push(d as char);
    }
}

/// Parse a decimal string into u32.
fn parse_u32(s: &str) -> Option<u32> {
    let mut val = 0u32;
    let mut found = false;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            val = val.checked_mul(10)?.checked_add((b - b'0') as u32)?;
            found = true;
        } else {
            break;
        }
    }
    if found { Some(val) } else { None }
}
