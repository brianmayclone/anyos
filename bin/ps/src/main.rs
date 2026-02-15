#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    // Thread info (cmd=1): 36-byte entries [tid:u32, prio:u8, state:u8, arch:u8, pad:u8, name:24bytes, cpu_ticks:u32]
    let mut thread_buf = [0u8; 36 * 64]; // max 64 threads
    let count = anyos_std::sys::sysinfo(1, &mut thread_buf);

    if count == u32::MAX || count == 0 {
        anyos_std::println!("No threads running.");
        return;
    }

    anyos_std::println!("{:<6} {:<6} {:<12} {}", "TID", "PRI", "STATE", "NAME");
    anyos_std::println!("{}", "--------------------------------------");

    for i in 0..count as usize {
        let entry = &thread_buf[i * 36..(i + 1) * 36];
        let tid = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let prio = entry[4];
        let state = entry[5];
        let name_bytes = &entry[8..32];
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(24);
        let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("???");

        let state_str = match state {
            0 => "Ready",
            1 => "Running",
            2 => "Blocked",
            3 => "Dead",
            _ => "Unknown",
        };

        anyos_std::println!("{:<6} {:<6} {:<12} {}", tid, prio, state_str, name);
    }

    anyos_std::println!("\nTotal: {} thread(s)", count);
}
