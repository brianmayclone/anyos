#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    anyos_std::i18n::init();
    let t = anyos_std::i18n::t;
    // Memory info (cmd=0): [total_frames:u32, free_frames:u32, heap_used:u32, heap_total:u32]
    let mut mem_buf = [0u8; 16];
    if anyos_std::sys::sysinfo(0, &mut mem_buf) != 0 {
        anyos_std::println!("{}", t("Failed to get memory info."));
        return;
    }

    let total = u32::from_le_bytes([mem_buf[0], mem_buf[1], mem_buf[2], mem_buf[3]]);
    let free = u32::from_le_bytes([mem_buf[4], mem_buf[5], mem_buf[6], mem_buf[7]]);
    let heap_used = u32::from_le_bytes([mem_buf[8], mem_buf[9], mem_buf[10], mem_buf[11]]);
    let heap_total = u32::from_le_bytes([mem_buf[12], mem_buf[13], mem_buf[14], mem_buf[15]]);

    let total_kb = total * 4;
    let free_kb = free * 4;
    let used_kb = total_kb - free_kb;

    anyos_std::println!("         {:>12} {:>11} {:>11}", t("total"), t("used"), t("free"));
    anyos_std::println!("{:<9}{:>8} KiB {:>8} KiB {:>8} KiB", t("Mem:"), total_kb, used_kb, free_kb);
    anyos_std::println!("{:<9}{:>8} KiB {:>8} KiB {:>8} KiB", t("Heap:"),
        heap_total / 1024,
        heap_used / 1024,
        (heap_total - heap_used) / 1024,
    );
}
