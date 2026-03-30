#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    let args = anyos_std::args::parse(raw, b"");

    if raw.contains("--help") {
        anyos_std::println!("touch - Create files or update timestamps\n\nUsage: touch FILE...");
        return;
    }

    if args.pos_count == 0 {
        anyos_std::println!("Usage: touch FILE...");
        return;
    }

    for i in 0..args.pos_count {
        let path = args.positional[i];
        let fd = anyos_std::fs::open(path, anyos_std::fs::O_CREATE | anyos_std::fs::O_WRITE);
        if fd == u32::MAX {
            anyos_std::println!("touch: cannot create '{}'", path);
        } else {
            anyos_std::fs::close(fd);
        }
    }
}
