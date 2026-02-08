#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let args_buf = &mut [0u8; 256];
    let args_len = anyos_std::process::getargs(args_buf);
    let args = core::str::from_utf8(&args_buf[..args_len]).unwrap_or("");
    let path = args.trim();

    if path.is_empty() {
        anyos_std::println!("Usage: mkdir <path>");
        return;
    }

    if anyos_std::fs::mkdir(path) == u32::MAX {
        anyos_std::println!("mkdir: cannot create directory '{}'", path);
    }
}
