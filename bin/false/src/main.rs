#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("false - Exit with failure status\n\nUsage: false");
        return;
    }

    anyos_std::process::exit(1);
}
