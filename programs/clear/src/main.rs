#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    // ANSI escape: clear screen + move cursor to top-left
    anyos_std::print!("\x1B[2J\x1B[H");
}
