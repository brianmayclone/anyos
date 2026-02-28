//! killall — kill all processes matching a name.
//!
//! Uses `sysinfo(1)` to enumerate threads and `kill()` to terminate matches.
//! Supports exact name matching and optional `-v` (verbose) flag.

#![no_std]
#![no_main]

anyos_std::entry!(main);

/// Size of one thread-info entry returned by sysinfo(1).
const THREAD_ENTRY_SIZE: usize = 60;
/// Maximum number of threads we can enumerate in one call.
const MAX_THREADS: usize = 128;
/// Offset of the null-terminated name within a thread entry.
const NAME_OFFSET: usize = 8;
/// Length of the name field in bytes.
const NAME_LEN: usize = 24;

/// Extract a little-endian u32 from a byte slice at the given offset.
fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// Extract the thread name from a sysinfo entry.
fn thread_name(buf: &[u8], entry_off: usize) -> &str {
    let name_bytes = &buf[entry_off + NAME_OFFSET..entry_off + NAME_OFFSET + NAME_LEN];
    let len = name_bytes.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
    core::str::from_utf8(&name_bytes[..len]).unwrap_or("???")
}

fn main() {
    let mut arg_buf = [0u8; 256];
    let raw_args = anyos_std::process::args(&mut arg_buf);

    // Parse flags and collect the process name pattern.
    let mut verbose = false;
    let mut pattern = "";
    for token in raw_args.split_whitespace() {
        if token == "-v" || token == "--verbose" {
            verbose = true;
        } else if pattern.is_empty() {
            pattern = token;
        }
    }

    if pattern.is_empty() {
        anyos_std::println!("Usage: killall [-v] <name>");
        return;
    }

    // Enumerate all threads via sysinfo(1).
    let mut info_buf = [0u8; THREAD_ENTRY_SIZE * MAX_THREADS];
    let count = anyos_std::sys::sysinfo(1, &mut info_buf) as usize;

    let mut killed = 0u32;
    let mut failed = 0u32;

    for i in 0..count {
        let off = i * THREAD_ENTRY_SIZE;
        let name = thread_name(&info_buf, off);
        if name != pattern {
            continue;
        }
        let tid = read_u32(&info_buf, off);
        let result = anyos_std::process::kill(tid);
        if result == 0 {
            killed += 1;
            if verbose {
                anyos_std::println!("Killed {} (tid {})", name, tid);
            }
        } else {
            failed += 1;
            if verbose {
                anyos_std::println!("Failed to kill {} (tid {})", name, tid);
            }
        }
    }

    if killed == 0 && failed == 0 {
        anyos_std::println!("No process found: {}", pattern);
    } else if !verbose {
        anyos_std::println!("Killed {} process(es)", killed);
    }
}
