#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("true - Exit with success status\n\nUsage: true");
        return;
    }

    // Exit successfully (exit code 0 is default)
}
