#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    anyos_std::println!(".anyOS System Information");
    anyos_std::println!("========================\n");

    // Time
    let mut time_buf = [0u8; 8];
    if anyos_std::sys::time(&mut time_buf) == 0 {
        let year = time_buf[0] as u16 | ((time_buf[1] as u16) << 8);
        let month = time_buf[2];
        let day = time_buf[3];
        let hour = time_buf[4];
        let min = time_buf[5];
        let sec = time_buf[6];
        anyos_std::println!(
            "Date/Time  : {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            year, month, day, hour, min, sec
        );
    }

    // Uptime
    let ticks = anyos_std::sys::uptime();
    let hz = anyos_std::sys::tick_hz();
    let secs = if hz > 0 { ticks / hz } else { 0 };
    let mins = secs / 60;
    anyos_std::println!("Uptime     : {}m {}s ({} ticks)", mins, secs % 60, ticks);

    // Memory info (cmd=0): [total_frames:u32, free_frames:u32, heap_used:u32, heap_total:u32]
    let mut mem_buf = [0u8; 16];
    if anyos_std::sys::sysinfo(0, &mut mem_buf) == 0 {
        let total = u32::from_le_bytes([mem_buf[0], mem_buf[1], mem_buf[2], mem_buf[3]]);
        let free = u32::from_le_bytes([mem_buf[4], mem_buf[5], mem_buf[6], mem_buf[7]]);
        let heap_used = u32::from_le_bytes([mem_buf[8], mem_buf[9], mem_buf[10], mem_buf[11]]);
        let heap_total = u32::from_le_bytes([mem_buf[12], mem_buf[13], mem_buf[14], mem_buf[15]]);

        let total_kb = total * 4; // 4K per frame
        let free_kb = free * 4;
        let used_kb = total_kb - free_kb;
        anyos_std::println!("\nMemory:");
        anyos_std::println!("  Physical : {} KiB total, {} KiB used, {} KiB free", total_kb, used_kb, free_kb);
        anyos_std::println!("  Heap     : {} KiB used / {} KiB total", heap_used / 1024, heap_total / 1024);
    }

    // CPU info (cmd=2): [cpu_count:u32]
    let mut cpu_buf = [0u8; 4];
    if anyos_std::sys::sysinfo(2, &mut cpu_buf) == 0 {
        let cpus = u32::from_le_bytes([cpu_buf[0], cpu_buf[1], cpu_buf[2], cpu_buf[3]]);
        anyos_std::println!("\nCPUs       : {}", cpus);
    }

    // Thread info (cmd=1): array of 32-byte entries [tid:u32, prio:u8, state:u8, pad:u16, name:24bytes]
    let mut thread_buf = [0u8; 32 * 32]; // max 32 threads
    let ret = anyos_std::sys::sysinfo(1, &mut thread_buf);
    if ret != u32::MAX {
        let count = ret;
        anyos_std::println!("\nThreads ({}):", count);
        anyos_std::println!("  {:<6} {:<6} {:<10} {}", "TID", "Prio", "State", "Name");
        anyos_std::println!("  {}", "--------------------------------------");

        for i in 0..count as usize {
            let entry = &thread_buf[i * 32..(i + 1) * 32];
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
                3 => "Terminated",
                4 => "Dead",
                _ => "Unknown",
            };

            anyos_std::println!("  {:<6} {:<6} {:<10} {}", tid, prio, state_str, name);
        }
    }

    anyos_std::println!("\nPID        : {}", anyos_std::process::getpid());
}
