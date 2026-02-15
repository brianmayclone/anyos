#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    // Check USER env var first, fall back to "root"
    let mut buf = [0u8; 64];
    let len = anyos_std::env::get("USER", &mut buf);
    if len != u32::MAX && len > 0 {
        let name = core::str::from_utf8(&buf[..len as usize]).unwrap_or("root");
        anyos_std::println!("{}", name);
    } else {
        anyos_std::println!("root");
    }
}
