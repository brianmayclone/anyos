#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("free - Display memory usage\n\nUsage: free");
        return;
    }

    anyos_std::i18n::init();
    let t = anyos_std::i18n::t;
    // Memory info (cmd=0): u32 words [total, free, heap_used, heap_total,
    // swap_total, swap_free, swap_areas].
    let mut mem_buf = [0u8; 28];
    if anyos_std::sys::sysinfo(0, &mut mem_buf) != 0 {
        anyos_std::println!("{}", t("Failed to get memory info."));
        return;
    }

    let total = u32::from_le_bytes([mem_buf[0], mem_buf[1], mem_buf[2], mem_buf[3]]);
    let free = u32::from_le_bytes([mem_buf[4], mem_buf[5], mem_buf[6], mem_buf[7]]);
    let heap_used = u32::from_le_bytes([mem_buf[8], mem_buf[9], mem_buf[10], mem_buf[11]]);
    let heap_total = u32::from_le_bytes([mem_buf[12], mem_buf[13], mem_buf[14], mem_buf[15]]);
    let swap_total = u32::from_le_bytes([mem_buf[16], mem_buf[17], mem_buf[18], mem_buf[19]]);
    let swap_free = u32::from_le_bytes([mem_buf[20], mem_buf[21], mem_buf[22], mem_buf[23]]);

    let total_kb = total as u64 * 4;
    let free_kb = free as u64 * 4;
    let used_kb = total_kb.saturating_sub(free_kb);
    let swap_total_kb = swap_total as u64 * 4;
    let swap_free_kb = swap_free as u64 * 4;
    let swap_used_kb = swap_total_kb.saturating_sub(swap_free_kb);

    anyos_std::println!(
        "         {:>12} {:>11} {:>11}",
        t("total"),
        t("used"),
        t("free")
    );
    anyos_std::println!(
        "{:<9}{:>8} KiB {:>8} KiB {:>8} KiB",
        t("Mem:"),
        total_kb,
        used_kb,
        free_kb
    );
    anyos_std::println!(
        "{:<9}{:>8} KiB {:>8} KiB {:>8} KiB",
        t("Heap:"),
        heap_total / 1024,
        heap_used / 1024,
        heap_total.saturating_sub(heap_used) / 1024,
    );
    anyos_std::println!(
        "{:<9}{:>8} KiB {:>8} KiB {:>8} KiB",
        "Swap:",
        swap_total_kb,
        swap_used_kb,
        swap_free_kb
    );
}
