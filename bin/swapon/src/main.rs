#![no_std]
#![no_main]

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!("swapon - Enable a swap file");
    anyos_std::println!("");
    anyos_std::println!("Usage: swapon FILE");
}

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf).trim();

    if args.is_empty() || args == "--help" {
        usage();
        return;
    }

    let mut tokens = args.split_whitespace();
    let Some(path) = tokens.next() else {
        usage();
        return;
    };
    if tokens.next().is_some() {
        usage();
        anyos_std::process::exit(1);
    }

    if anyos_std::sys::swapon(path, 0) == u32::MAX {
        anyos_std::println!("swapon: failed to enable {}", path);
        anyos_std::process::exit(1);
    }
}
