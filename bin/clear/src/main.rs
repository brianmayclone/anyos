#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("clear - Clear the terminal screen\n\nUsage: clear");
        return;
    }

    // ANSI escape: clear screen + move cursor to top-left
    anyos_std::print!("\x1B[2J\x1B[H");
}
