#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut buf = [0u8; 256];
    let args = anyos_std::process::args(&mut buf);

    if args.contains("-a") || args.contains("--all") {
        // CPU count
        let mut cpu_buf = [0u8; 4];
        let cpus = if anyos_std::sys::sysinfo(2, &mut cpu_buf) == 0 {
            u32::from_le_bytes([cpu_buf[0], cpu_buf[1], cpu_buf[2], cpu_buf[3]])
        } else {
            1
        };
        anyos_std::println!(".anyOS {} x86_64 {} CPU(s)", env!("ANYOS_VERSION"), cpus);
    } else {
        anyos_std::println!(".anyOS");
    }
}
