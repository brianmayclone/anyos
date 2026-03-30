#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("dmesg - Print kernel messages\n\nUsage: dmesg");
        return;
    }

    let mut buf = [0u8; 32 * 1024]; // 32 KiB — matches kernel ring buffer size
    let n = anyos_std::sys::dmesg(&mut buf) as usize;
    if n > 0 {
        anyos_std::fs::write(1, &buf[..n]);
    }
}
