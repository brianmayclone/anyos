#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut buf = [0u8; 32 * 1024]; // 32 KiB — matches kernel ring buffer size
    let n = anyos_std::sys::dmesg(&mut buf) as usize;
    if n > 0 {
        anyos_std::fs::write(1, &buf[..n]);
    }
}
