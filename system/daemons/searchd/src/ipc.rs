//! Pipe-based search API for searchd.
//!
//! Protocol:
//! - searchd creates named pipe `"searchd"` at startup
//! - Client creates response pipe `"searchd-{tid}"`, sends `{tid}\t{command}\n`
//! - searchd parses command, executes search, writes results to `"searchd-{tid}"`
//!
//! Commands:
//!   SEARCH {query}          — Freetext search across filenames and content
//!   FIND {filename}         — Search by filename (substring match)
//!   KIND {kind}             — List files of a specific kind (text, image, etc.)
//!   RECENT {count}          — Most recently indexed files
//!   STATS                   — Index statistics
//!   REINDEX                 — Trigger a full re-index
//!
//! Response format:
//!   OK\t{row_count}\n
//!   {path}\t{name}\t{kind}\t{size}\n
//!   ...
//!   \n
//!
//! Error format:
//!   ERR\t{message}\n\n

use alloc::format;
use alloc::string::String;
use libdb_client::Database;

/// Handle incoming pipe requests. Non-blocking — returns true if work was done.
pub fn handle_requests(
    db: &Database,
    pipe_id: u32,
    buf: &mut [u8],
    reindex_flag: &mut bool,
) -> bool {
    let n = anyos_std::ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }

    let data = &buf[..n as usize];

    let mut line_start = 0;
    for i in 0..data.len() {
        if data[i] == b'\n' {
            if i > line_start {
                handle_single_request(db, &data[line_start..i], reindex_flag);
            }
            line_start = i + 1;
        }
    }
    if line_start < data.len() {
        handle_single_request(db, &data[line_start..], reindex_flag);
    }

    true
}

fn handle_single_request(db: &Database, line: &[u8], reindex_flag: &mut bool) {
    // Parse: {tid}\t{command}
    let tab_pos = match line.iter().position(|&b| b == b'\t') {
        Some(pos) => pos,
        None => return,
    };

    let tid_str = match core::str::from_utf8(&line[..tab_pos]) {
        Ok(s) => s,
        Err(_) => return,
    };
    let tid: u32 = match parse_u32(tid_str) {
        Some(v) => v,
        None => return,
    };

    let cmd = match core::str::from_utf8(&line[tab_pos + 1..]) {
        Ok(s) => s.trim(),
        Err(_) => return,
    };
    if cmd.is_empty() {
        return;
    }

    anyos_std::println!("searchd: [query] tid={} cmd='{}'", tid, cmd);
    let response = dispatch(db, cmd, reindex_flag);
    // Log first line only (OK/ERR + count)
    let first_line = response.split('\n').next().unwrap_or("");
    anyos_std::println!("searchd: [result] {}", first_line);

    let reply_pipe_name = format!("searchd-{}", tid);
    let reply_pipe = anyos_std::ipc::pipe_open(&reply_pipe_name);
    if reply_pipe != 0 {
        anyos_std::ipc::pipe_write(reply_pipe, response.as_bytes());
    }
}

fn dispatch(db: &Database, cmd: &str, reindex_flag: &mut bool) -> String {
    let (verb, arg) = split_first_word(cmd);

    match verb {
        "SEARCH" | "search" => cmd_search(db, arg),
        "FIND" | "find" => cmd_find(db, arg),
        "KIND" | "kind" => cmd_kind(db, arg),
        "RECENT" | "recent" => cmd_recent(db, arg),
        "STATS" | "stats" => cmd_stats(db),
        "REINDEX" | "reindex" => {
            *reindex_flag = true;
            String::from("OK\t0\nReindex scheduled\n\n")
        }
        _ => format!("ERR\tUnknown command: {}\n\n", verb),
    }
}

// ── Search commands ─────────────────────────────────────────────────────────

/// Freetext search: searches both filenames and content.
fn cmd_search(db: &Database, query: &str) -> String {
    if query.is_empty() {
        return String::from("ERR\tEmpty query\n\n");
    }

    let q_escaped = sql_escape(query);
    let mut results = String::new();
    let mut count: u32 = 0;

    // Phase 1: Search filenames using LIKE
    let sql = format!(
        "SELECT path, name, kind, size FROM files WHERE name LIKE '%{}%' LIMIT 100",
        q_escaped
    );
    anyos_std::println!("searchd: [sql] {}", sql);
    let query_result = db.query(&sql);
    if let Err(ref e) = query_result {
        anyos_std::println!("searchd: [sql-error] {}", e);
    }
    if let Ok(result) = query_result {
        for row in 0..result.row_count() {
            let path = result.get_text(row, 0).unwrap_or_default();
            let name = result.get_text(row, 1).unwrap_or_default();
            let kind = result.get_text(row, 2).unwrap_or_default();
            let size = result.get_int(row, 3).unwrap_or(0);
            append_result(&mut results, &path, &name, &kind, size);
            count += 1;
        }
    }

    // Phase 2: Search content using LIKE
    if count < 100 {
        let remaining = 100 - count;
        let content_sql = format!(
            "SELECT DISTINCT path FROM content WHERE body LIKE '%{}%' LIMIT {}",
            q_escaped, remaining
        );
        if let Ok(result) = db.query(&content_sql) {
            for row in 0..result.row_count() {
                let path = result.get_text(row, 0).unwrap_or_default();
                if results.contains(&path) {
                    continue;
                }
                let file_sql = format!(
                    "SELECT name, kind, size FROM files WHERE path = '{}'",
                    sql_escape(&path)
                );
                if let Ok(fres) = db.query(&file_sql) {
                    if fres.row_count() > 0 {
                        let name = fres.get_text(0, 0).unwrap_or_default();
                        let kind = fres.get_text(0, 1).unwrap_or_default();
                        let size = fres.get_int(0, 2).unwrap_or(0);
                        append_result(&mut results, &path, &name, &kind, size);
                        count += 1;
                    }
                }
            }
        }
    }

    format!("OK\t{}\n{}\n", count, results)
}

/// Search by filename using LIKE.
fn cmd_find(db: &Database, pattern: &str) -> String {
    if pattern.is_empty() {
        return String::from("ERR\tEmpty pattern\n\n");
    }

    let p_escaped = sql_escape(pattern);
    let sql = format!(
        "SELECT path, name, kind, size FROM files WHERE name LIKE '%{}%' LIMIT 100",
        p_escaped
    );
    let mut results = String::new();
    let mut count: u32 = 0;

    if let Ok(result) = db.query(&sql) {
        for row in 0..result.row_count() {
            let path = result.get_text(row, 0).unwrap_or_default();
            let name = result.get_text(row, 1).unwrap_or_default();
            let kind = result.get_text(row, 2).unwrap_or_default();
            let size = result.get_int(row, 3).unwrap_or(0);
            append_result(&mut results, &path, &name, &kind, size);
            count += 1;
        }
    }

    format!("OK\t{}\n{}\n", count, results)
}

/// List files of a specific kind.
fn cmd_kind(db: &Database, kind: &str) -> String {
    if kind.is_empty() {
        return String::from("ERR\tEmpty kind\n\n");
    }

    let k_escaped = sql_escape(kind);
    let sql = format!(
        "SELECT path, name, kind, size FROM files WHERE kind = '{}' ORDER BY name LIMIT 100",
        k_escaped
    );
    let mut results = String::new();
    let mut count: u32 = 0;

    if let Ok(result) = db.query(&sql) {
        for row in 0..result.row_count() {
            let path = result.get_text(row, 0).unwrap_or_default();
            let name = result.get_text(row, 1).unwrap_or_default();
            let kind = result.get_text(row, 2).unwrap_or_default();
            let size = result.get_int(row, 3).unwrap_or(0);
            append_result(&mut results, &path, &name, &kind, size);
            count += 1;
        }
    }

    format!("OK\t{}\n{}\n", count, results)
}

/// Most recently indexed files.
fn cmd_recent(db: &Database, arg: &str) -> String {
    let max = parse_u32(arg).unwrap_or(20);
    let max = if max > 100 { 100 } else { max };

    let sql = format!(
        "SELECT path, name, kind, size FROM files WHERE kind != 'directory' ORDER BY modified DESC LIMIT {}",
        max
    );
    let mut results = String::new();
    let mut count: u32 = 0;

    if let Ok(result) = db.query(&sql) {
        for row in 0..result.row_count() {
            let path = result.get_text(row, 0).unwrap_or_default();
            let name = result.get_text(row, 1).unwrap_or_default();
            let kind = result.get_text(row, 2).unwrap_or_default();
            let size = result.get_int(row, 3).unwrap_or(0);
            append_result(&mut results, &path, &name, &kind, size);
            count += 1;
        }
    }

    format!("OK\t{}\n{}\n", count, results)
}

/// Index statistics.
fn cmd_stats(db: &Database) -> String {
    let mut file_count: u32 = 0;
    let mut content_chunks: u32 = 0;
    let mut dir_count: u32 = 0;

    if let Ok(r) = db.query("SELECT kind FROM files") {
        file_count = r.row_count();
        for row in 0..r.row_count() {
            if let Some(k) = r.get_text(row, 0) {
                if k == "directory" {
                    dir_count += 1;
                }
            }
        }
    }
    if let Ok(r) = db.query("SELECT chunk FROM content") {
        content_chunks = r.row_count();
    }

    format!(
        "OK\t3\nfiles\t{}\ndirectories\t{}\ncontent_chunks\t{}\n\n",
        file_count, dir_count, content_chunks
    )
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn append_result(buf: &mut String, path: &str, name: &str, kind: &str, size: i64) {
    buf.push_str(path);
    buf.push('\t');
    buf.push_str(name);
    buf.push('\t');
    buf.push_str(kind);
    buf.push('\t');
    push_i64(buf, size);
    buf.push('\n');
}

fn split_first_word(s: &str) -> (&str, &str) {
    let s = s.trim();
    if let Some(space) = s.find(' ') {
        (&s[..space], s[space + 1..].trim())
    } else {
        (s, "")
    }
}

fn sql_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if ch == '\'' {
            out.push('\'');
            out.push('\'');
        } else {
            out.push(ch);
        }
    }
    out
}

fn to_lower(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b >= b'A' && b <= b'Z' {
            out.push((b + 32) as char);
        } else {
            out.push(b as char);
        }
    }
    out
}

fn push_i64(s: &mut String, v: i64) {
    if v < 0 {
        s.push('-');
        if v == i64::MIN {
            s.push_str("9223372036854775808");
            return;
        }
        push_u64(s, (-v) as u64);
    } else {
        push_u64(s, v as u64);
    }
}

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
    if found {
        Some(val)
    } else {
        None
    }
}
