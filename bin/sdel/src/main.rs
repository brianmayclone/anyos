//! sdel — delete a key from the session store.
//!
//! Usage: sdel <key>
//!        sdel --all        (wipe the entire store)
//!
//! The session store is a plain text file at /tmp/.sstore.

#![no_std]
#![no_main]

anyos_std::entry!(main);

const STORE_PATH: &str = "/tmp/.sstore";
const MAX_STORE: usize = 4096;

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if raw.contains("--help") {
        anyos_std::println!("sdel - Delete session store key\n\nUsage: sdel KEY\n\nOptions:\n  --all          Delete all keys");
        return;
    }

    if args.pos_count == 0 {
        anyos_std::println!("Usage: sdel <key>");
        anyos_std::println!("       sdel --all   (clear entire store)");
        return;
    }

    let key = args.positional[0];

    // --all: delete the whole file
    if key == "--all" {
        anyos_std::fs::unlink(STORE_PATH);
        return;
    }

    // Read existing store
    let fd = anyos_std::fs::open(STORE_PATH, 0);
    if fd == u32::MAX {
        return; // nothing to delete
    }
    let mut store = [0u8; MAX_STORE];
    let n = anyos_std::fs::read(fd, &mut store);
    anyos_std::fs::close(fd);
    let n = if n == u32::MAX { return; } else { n as usize };

    // Rebuild without the target key
    let mut out = [0u8; MAX_STORE];
    let mut out_len: usize = 0;
    let mut found = false;

    let text = core::str::from_utf8(&store[..n]).unwrap_or("");
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        let is_key = line.len() > key.len()
            && &line[..key.len()] == key
            && line.as_bytes()[key.len()] == b'=';
        if is_key {
            found = true;
            continue; // drop it
        }
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

    if !found {
        return; // key wasn't there — no need to rewrite
    }

    // Write back (or unlink if empty)
    if out_len == 0 {
        anyos_std::fs::unlink(STORE_PATH);
    } else {
        use anyos_std::fs::{O_WRITE, O_CREATE, O_TRUNC};
        let fd = anyos_std::fs::open(STORE_PATH, O_WRITE | O_CREATE | O_TRUNC);
        if fd != u32::MAX {
            anyos_std::fs::write(fd, &out[..out_len]);
            anyos_std::fs::close(fd);
        }
    }
}
