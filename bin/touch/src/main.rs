#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
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
