#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    let force = args.has(b'f');

    if args.pos_count == 0 {
        anyos_std::println!("Usage: rm [-f] FILE...");
        return;
    }

    for i in 0..args.pos_count {
        let path = args.positional[i];
        if anyos_std::fs::unlink(path) == u32::MAX && !force {
            anyos_std::println!("rm: cannot remove '{}': No such file or directory", path);
        }
    }
}
