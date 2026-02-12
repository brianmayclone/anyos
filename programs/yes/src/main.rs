#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let text = args.trim();
    let msg = if text.is_empty() { "y" } else { text };

    // Print in a loop, but limited to avoid freezing (no Ctrl+C yet)
    for _ in 0..1000 {
        anyos_std::println!("{}", msg);
    }
}
