#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut buf = [0u8; 256];
    let len = anyos_std::fs::getcwd(&mut buf);
    if len > 0 {
        let path = core::str::from_utf8(&buf[..len as usize]).unwrap_or("/");
        anyos_std::println!("{}", path);
    } else {
        anyos_std::println!("/");
    }
}
