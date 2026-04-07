// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! IPC client for the searchd daemon.

use anyos_std::{String, Vec};

const PIPE_NAME: &str = "searchd";

/// A single search result from searchd.
pub struct FileResult {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size: u32,
}

/// Query searchd for files matching `query`. Returns an empty Vec if searchd
/// is not running or no results are found.
pub fn search(query: &str) -> Vec<FileResult> {
    if query.len() < 2 {
        return Vec::new();
    }

    let tid = anyos_std::process::getpid();
    let mut tbuf = [0u8; 16];
    let tlen = fmt_u32(tid, &mut tbuf);
    let tid_str = core::str::from_utf8(&tbuf[..tlen]).unwrap_or("0");

    let mut resp_name = String::from("searchd-");
    resp_name.push_str(tid_str);

    let main_pipe = anyos_std::ipc::pipe_open(PIPE_NAME);
    if main_pipe == 0 {
        return Vec::new();
    }

    let resp_pipe = anyos_std::ipc::pipe_create(&resp_name);
    if resp_pipe == 0 {
        return Vec::new();
    }

    // Send request: "{tid}\tSEARCH {query}\n"
    let mut req = String::from(tid_str);
    req.push('\t');
    req.push_str("SEARCH ");
    req.push_str(query);
    req.push('\n');
    anyos_std::ipc::pipe_write(main_pipe, req.as_bytes());

    // Read response — growable buffer, multiple attempts
    let mut data = Vec::new();
    let mut chunk = [0u8; 4096];
    for _ in 0..30 {
        let n = anyos_std::ipc::pipe_read(resp_pipe, &mut chunk);
        if n > 0 && n != u32::MAX {
            data.extend_from_slice(&chunk[..n as usize]);
            if data.len() >= 2 && data[data.len() - 1] == b'\n' && data[data.len() - 2] == b'\n' {
                break;
            }
        } else {
            anyos_std::process::sleep(15);
        }
    }

    anyos_std::ipc::pipe_close(resp_pipe);

    if data.is_empty() {
        return Vec::new();
    }

    parse_response(&data)
}

fn parse_response(data: &[u8]) -> Vec<FileResult> {
    let text = match core::str::from_utf8(data) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let mut lines = text.split('\n');
    let header = match lines.next() {
        Some(h) => h,
        None => return Vec::new(),
    };
    if !header.starts_with("OK\t") {
        return Vec::new();
    }

    let mut results = Vec::new();
    for line in lines {
        if line.is_empty() { continue; }
        let mut parts = line.splitn(4, '\t');
        let path = match parts.next() { Some(s) => s, None => continue };
        let name = match parts.next() { Some(s) => s, None => continue };
        let kind = match parts.next() { Some(s) => s, None => continue };
        let size_str = match parts.next() { Some(s) => s, None => continue };
        results.push(FileResult {
            path: String::from(path),
            name: String::from(name),
            kind: String::from(kind),
            size: parse_u32(size_str),
        });
    }
    results
}

fn parse_u32(s: &str) -> u32 {
    let mut val = 0u32;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            val = val.wrapping_mul(10).wrapping_add((b - b'0') as u32);
        } else { break; }
    }
    val
}

fn fmt_u32(mut val: u32, buf: &mut [u8]) -> usize {
    if val == 0 { buf[0] = b'0'; return 1; }
    let mut tmp = [0u8; 10];
    let mut i = 0;
    while val > 0 {
        tmp[i] = b'0' + (val % 10) as u8;
        val /= 10;
        i += 1;
    }
    for j in 0..i { buf[j] = tmp[i - 1 - j]; }
    i
}

/// Format file size as human-readable string.
pub fn fmt_size(size: u32) -> String {
    if size >= 1_048_576 {
        let mb = size / 1_048_576;
        let frac = (size % 1_048_576) * 10 / 1_048_576;
        let mut s = String::new();
        push_u32(&mut s, mb);
        s.push('.');
        push_u32(&mut s, frac);
        s.push_str(" MB");
        s
    } else if size >= 1024 {
        let mut s = String::new();
        push_u32(&mut s, size / 1024);
        s.push_str(" KB");
        s
    } else {
        let mut s = String::new();
        push_u32(&mut s, size);
        s.push_str(" B");
        s
    }
}

fn push_u32(s: &mut String, val: u32) {
    let mut buf = [0u8; 10];
    let len = fmt_u32(val, &mut buf);
    if let Ok(t) = core::str::from_utf8(&buf[..len]) { s.push_str(t); }
}
