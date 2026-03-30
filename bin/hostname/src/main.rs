#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("hostname - Print system hostname\n\nUsage: hostname");
        return;
    }

    anyos_std::println!(".anyOS");
}
