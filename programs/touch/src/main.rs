#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let args_buf = &mut [0u8; 256];
    let args_len = anyos_std::process::getargs(args_buf);
    let args = core::str::from_utf8(&args_buf[..args_len]).unwrap_or("");
    let path = args.trim();

    if path.is_empty() {
        anyos_std::println!("Usage: touch <path>");
        return;
    }

    // Open with O_CREATE | O_WRITE — creates file if it doesn't exist
    let fd = anyos_std::fs::open(path, anyos_std::fs::O_CREATE | anyos_std::fs::O_WRITE);
    if fd == u32::MAX {
        anyos_std::println!("touch: cannot create '{}'", path);
        return;
    }
    anyos_std::fs::close(fd);
}
