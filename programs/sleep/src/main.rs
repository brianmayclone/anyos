#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let args_buf = &mut [0u8; 256];
    let args_len = anyos_std::process::getargs(args_buf);
    let args = core::str::from_utf8(&args_buf[..args_len]).unwrap_or("");
    let args = args.trim();

    if args.is_empty() {
        anyos_std::println!("Usage: sleep <milliseconds>");
        return;
    }

    // Parse number
    let mut ms = 0u32;
    for b in args.bytes() {
        if b >= b'0' && b <= b'9' {
            ms = ms * 10 + (b - b'0') as u32;
        } else {
            anyos_std::println!("sleep: invalid number '{}'", args);
            return;
        }
    }

    anyos_std::process::sleep(ms);
}
