#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    // Thread info (cmd=1): 60-byte entries
    // [tid:u32, prio:u8, state:u8, arch:u8, pad:u8, name:24bytes,
    //  user_pages:u32, cpu_ticks:u32, io_read:u64, io_write:u64, uid:u16, pad:u16]
    let mut thread_buf = [0u8; 60 * 64]; // max 64 threads
    let count = anyos_std::sys::sysinfo(1, &mut thread_buf);

    if count == u32::MAX || count == 0 {
        anyos_std::println!("No threads running.");
        return;
    }

    // Pre-resolve uid→username for all unique uids
    let mut uid_cache: [(u16, [u8; 16], u8); 16] = [(0, [0u8; 16], 0); 16];
    let mut uid_cache_len = 0usize;

    for i in 0..count as usize {
        let off = i * 60;
        if off + 60 > thread_buf.len() { break; }
        let uid = u16::from_le_bytes([thread_buf[off + 56], thread_buf[off + 57]]);
        // Check if already cached
        let found = uid_cache[..uid_cache_len].iter().any(|e| e.0 == uid);
        if !found && uid_cache_len < 16 {
            let mut name_buf = [0u8; 16];
            let nlen = anyos_std::process::getusername(uid, &mut name_buf);
            let len = if nlen != u32::MAX && nlen > 0 { (nlen as u8).min(15) } else { 0 };
            uid_cache[uid_cache_len] = (uid, name_buf, len);
            uid_cache_len += 1;
        }
    }

    anyos_std::println!("{:<6} {:<10} {:<6} {:<12} {}", "TID", "USER", "PRI", "STATE", "NAME");
    anyos_std::println!("{}", "------------------------------------------------");

    for i in 0..count as usize {
        let off = i * 60;
        if off + 60 > thread_buf.len() { break; }
        let tid = u32::from_le_bytes([thread_buf[off], thread_buf[off+1], thread_buf[off+2], thread_buf[off+3]]);
        let prio = thread_buf[off + 4];
        let state = thread_buf[off + 5];
        let name_bytes = &thread_buf[off + 8..off + 32];
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(24);
        let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("???");
        let uid = u16::from_le_bytes([thread_buf[off + 56], thread_buf[off + 57]]);

        let state_str = match state {
            0 => "Ready",
            1 => "Running",
            2 => "Blocked",
            3 => "Dead",
            _ => "Unknown",
        };

        // Look up username from cache
        let username = uid_cache[..uid_cache_len]
            .iter()
            .find(|e| e.0 == uid)
            .and_then(|e| {
                if e.2 > 0 {
                    core::str::from_utf8(&e.1[..e.2 as usize]).ok()
                } else {
                    None
                }
            })
            .unwrap_or("?");

        anyos_std::println!("{:<6} {:<10} {:<6} {:<12} {}", tid, username, prio, state_str, name);
    }

    anyos_std::println!("\nTotal: {} thread(s)", count);
}
