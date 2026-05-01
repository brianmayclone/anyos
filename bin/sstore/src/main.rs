//! sstore — write a key=value pair to the session store.
//!
//! Usage: sstore <key> <value>
//!        sstore            (list all stored keys and values)
//!
//! The session store is a plain text file at /tmp/.sstore.
//! Each line has the format:  KEY=VALUE
//! Keys may not contain '=' or newlines.

#![no_std]
#![no_main]

anyos_std::entry!(main);

const STORE_PATH: &str = "/tmp/.sstore";
const MAX_STORE: usize = 4096;

// ─── helpers ──────────────────────────────────────────────────────────────────

/// Read the entire store file into `buf`. Returns number of bytes read.
fn read_store(buf: &mut [u8; MAX_STORE]) -> usize {
    let fd = anyos_std::fs::open(STORE_PATH, 0);
    if fd == u32::MAX {
        return 0;
    }
    let n = anyos_std::fs::read(fd, buf);
    anyos_std::fs::close(fd);
    if n == u32::MAX {
        0
    } else {
        n as usize
    }
}

/// Write `data` to the store file (truncate + create).
fn write_store(data: &[u8]) {
    use anyos_std::fs::{O_CREATE, O_TRUNC, O_WRITE};
    let fd = anyos_std::fs::open(STORE_PATH, O_WRITE | O_CREATE | O_TRUNC);
    if fd == u32::MAX {
        anyos_std::println!("sstore: cannot write {}", STORE_PATH);
        return;
    }
    anyos_std::fs::write(fd, data);
    anyos_std::fs::close(fd);
}

// ─── main ─────────────────────────────────────────────────────────────────────

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if raw.contains("--help") {
        anyos_std::println!("sstore - Set session store value\n\nUsage: sstore [KEY VALUE]");
        return;
    }

    // No args: list all entries
    if args.pos_count == 0 {
        let mut store = [0u8; MAX_STORE];
        let n = read_store(&mut store);
        if n == 0 {
            anyos_std::println!("(session store is empty)");
        } else {
            let text = core::str::from_utf8(&store[..n]).unwrap_or("");
            anyos_std::print!("{}", text);
        }
        return;
    }

    if args.pos_count < 2 {
        anyos_std::println!("Usage: sstore <key> <value>");
        anyos_std::println!("       sstore              (list all)");
        return;
    }

    let key = args.positional[0];
    let value = args.positional[1];

    if key.contains('=') || key.contains('\n') {
        anyos_std::println!("sstore: key must not contain '=' or newline");
        return;
    }

    // Read existing store
    let mut store = [0u8; MAX_STORE];
    let n = read_store(&mut store);

    // Build new store content: copy lines that don't start with "KEY="
    let mut out = [0u8; MAX_STORE];
    let mut out_len: usize = 0;

    let text = core::str::from_utf8(&store[..n]).unwrap_or("");
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        // Check if this line belongs to our key
        let starts_with_key = line.len() > key.len()
            && &line[..key.len()] == key
            && line.as_bytes()[key.len()] == b'=';
        if starts_with_key {
            continue; // drop old entry
        }
        // Copy line + newline
        let lb = line.as_bytes();
        let remaining = MAX_STORE - out_len;
        if lb.len() + 1 > remaining {
            break;
        }
        out[out_len..out_len + lb.len()].copy_from_slice(lb);
        out_len += lb.len();
        out[out_len] = b'\n';
        out_len += 1;
    }

    // Append new KEY=VALUE\n
    let kb = key.as_bytes();
    let vb = value.as_bytes();
    let needed = kb.len() + 1 + vb.len() + 1;
    if out_len + needed <= MAX_STORE {
        out[out_len..out_len + kb.len()].copy_from_slice(kb);
        out_len += kb.len();
        out[out_len] = b'=';
        out_len += 1;
        out[out_len..out_len + vb.len()].copy_from_slice(vb);
        out_len += vb.len();
        out[out_len] = b'\n';
        out_len += 1;
    }

    write_store(&out[..out_len]);
}
