#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("sysinfo - Display system information\n\nUsage: sysinfo");
        return;
    }

    anyos_std::i18n::init();
    let t = anyos_std::i18n::t;
    anyos_std::println!("{}", t(".anyOS System Information"));
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
            "{:<11}: {:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            t("Date/Time"),
            year,
            month,
            day,
            hour,
            min,
            sec
        );
    }

    // Uptime
    let ticks = anyos_std::sys::uptime();
    let hz = anyos_std::sys::tick_hz();
    let secs = if hz > 0 { ticks / hz } else { 0 };
    let mins = secs / 60;
    anyos_std::println!(
        "{:<11}: {}m {}s ({} ticks)",
        t("Uptime"),
        mins,
        secs % 60,
        ticks
    );

    // Memory info (cmd=0): u32 words [total, free, heap_used, heap_total,
    // swap_total, swap_free, swap_areas].
    let mut mem_buf = [0u8; 28];
    if anyos_std::sys::sysinfo(0, &mut mem_buf) == 0 {
        let total = u32::from_le_bytes([mem_buf[0], mem_buf[1], mem_buf[2], mem_buf[3]]);
        let free = u32::from_le_bytes([mem_buf[4], mem_buf[5], mem_buf[6], mem_buf[7]]);
        let heap_used = u32::from_le_bytes([mem_buf[8], mem_buf[9], mem_buf[10], mem_buf[11]]);
        let heap_total = u32::from_le_bytes([mem_buf[12], mem_buf[13], mem_buf[14], mem_buf[15]]);
        let swap_total = u32::from_le_bytes([mem_buf[16], mem_buf[17], mem_buf[18], mem_buf[19]]);
        let swap_free = u32::from_le_bytes([mem_buf[20], mem_buf[21], mem_buf[22], mem_buf[23]]);
        let swap_areas = u32::from_le_bytes([mem_buf[24], mem_buf[25], mem_buf[26], mem_buf[27]]);

        let total_kb = total * 4; // 4K per frame
        let free_kb = free * 4;
        let used_kb = total_kb - free_kb;
        let swap_total_kb = swap_total * 4;
        let swap_free_kb = swap_free * 4;
        anyos_std::println!("\n{}:", t("Memory"));
        anyos_std::println!(
            "  {:<9}: {} KiB {}, {} KiB {}, {} KiB {}",
            t("Physical"),
            total_kb,
            t("total"),
            used_kb,
            t("used"),
            free_kb,
            t("free")
        );
        anyos_std::println!(
            "  {:<9}: {} KiB {} / {} KiB {}",
            t("Heap"),
            heap_used / 1024,
            t("used"),
            heap_total / 1024,
            t("total")
        );
        anyos_std::println!(
            "  {:<9}: {} KiB {}, {} KiB {}, {} {}",
            "Swap",
            swap_total_kb,
            t("total"),
            swap_free_kb,
            t("free"),
            swap_areas,
            "areas"
        );
    }

    // CPU info (cmd=2): [cpu_count:u32]
    let mut cpu_buf = [0u8; 4];
    if anyos_std::sys::sysinfo(2, &mut cpu_buf) == 0 {
        let cpus = u32::from_le_bytes([cpu_buf[0], cpu_buf[1], cpu_buf[2], cpu_buf[3]]);
        anyos_std::println!("\n{:<11}: {}", t("CPUs"), cpus);
    }

    // Thread info (cmd=1): 80-byte entries
    let mut thread_buf = [0u8; 80 * 64]; // max 64 threads
    let ret = anyos_std::sys::sysinfo(1, &mut thread_buf);
    if ret != u32::MAX {
        let count = ret;
        anyos_std::println!("\n{} ({}):", t("Threads"), count);
        anyos_std::println!(
            "  {:<6} {:<6} {:<10} {}",
            "TID",
            t("Prio"),
            t("State"),
            t("Name")
        );
        anyos_std::println!("  {}", "--------------------------------------");

        for i in 0..count as usize {
            let entry = &thread_buf[i * 80..(i + 1) * 80];
            let tid = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
            let prio = entry[4];
            let state = entry[5];
            let name_bytes = &entry[8..32];
            let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(24);
            let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("???");

            let state_str = match state {
                0 => t("Ready"),
                1 => t("Running"),
                2 => t("Blocked"),
                3 => t("Dead"),
                _ => t("Unknown"),
            };

            anyos_std::println!("  {:<6} {:<6} {:<10} {}", tid, prio, state_str, name);
        }
    }

    anyos_std::println!("\nPID        : {}", anyos_std::process::getpid());
}
