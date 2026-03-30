//! sget — read a value from the session store.
//!
//! Usage: sget <key>
//!
//! Prints the value for <key> to stdout (suitable for command substitution).
//! Exits silently (no output) if the key does not exist.
//!
//! Example:
//!   sstore token abc123
//!   TOKEN=$(sget token)
//!   echo $TOKEN    # abc123

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
        anyos_std::println!("sget - Get session store value\n\nUsage: sget KEY");
        return;
    }

    if args.pos_count == 0 {
        anyos_std::println!("Usage: sget <key>");
        return;
    }

    let key = args.positional[0];

    let fd = anyos_std::fs::open(STORE_PATH, 0);
    if fd == u32::MAX {
        return; // store doesn't exist yet — no output
    }

    let mut store = [0u8; MAX_STORE];
    let n = anyos_std::fs::read(fd, &mut store);
    anyos_std::fs::close(fd);
    let n = if n == u32::MAX { return; } else { n as usize };

    let text = core::str::from_utf8(&store[..n]).unwrap_or("");
    for line in text.split('\n') {
        if line.is_empty() {
            continue;
        }
        if line.len() > key.len()
            && &line[..key.len()] == key
            && line.as_bytes()[key.len()] == b'='
        {
            let value = &line[key.len() + 1..];
            anyos_std::println!("{}", value);
            return;
        }
    }
    // Key not found — silent exit (consistent with env var behaviour)
}
