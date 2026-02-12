#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    // Get memory info (sysinfo cmd=0 returns [total_kb:u32, used_kb:u32, free_kb:u32])
    let mut mem_buf = [0u8; 64];
    anyos_std::sys::sysinfo(0, &mut mem_buf);
    let total_kb = u32::from_le_bytes([mem_buf[0], mem_buf[1], mem_buf[2], mem_buf[3]]);
    let used_kb = u32::from_le_bytes([mem_buf[4], mem_buf[5], mem_buf[6], mem_buf[7]]);
    let free_kb = u32::from_le_bytes([mem_buf[8], mem_buf[9], mem_buf[10], mem_buf[11]]);

    // Display disk-like info for our RAM disk / FAT16 partition
    anyos_std::println!("Filesystem     Size    Used    Avail   Use%  Mounted on");
    anyos_std::println!("{:<15}{:>4}K {:>7}K {:>7}K  {:>3}%  {}",
        "/dev/hda",
        total_kb,
        used_kb,
        free_kb,
        if total_kb > 0 { used_kb * 100 / total_kb } else { 0 },
        "/",
    );
}
