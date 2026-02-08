#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let args_buf = &mut [0u8; 256];
    let args_len = anyos_std::process::getargs(args_buf);
    let args = core::str::from_utf8(&args_buf[..args_len]).unwrap_or("");

    // Parse "src dst" from args
    let args = args.trim();
    let mut parts = args.splitn(2, ' ');
    let src = parts.next().unwrap_or("");
    let dst = parts.next().unwrap_or("").trim();

    if src.is_empty() || dst.is_empty() {
        anyos_std::println!("Usage: cp <source> <destination>");
        return;
    }

    // Open source for reading
    let src_fd = anyos_std::fs::open(src, 0);
    if src_fd == u32::MAX {
        anyos_std::println!("cp: cannot open '{}'", src);
        return;
    }

    // Open destination for writing (create + truncate)
    let dst_fd = anyos_std::fs::open(dst, anyos_std::fs::O_WRITE | anyos_std::fs::O_CREATE | anyos_std::fs::O_TRUNC);
    if dst_fd == u32::MAX {
        anyos_std::println!("cp: cannot create '{}'", dst);
        anyos_std::fs::close(src_fd);
        return;
    }

    // Copy in chunks
    let mut buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(src_fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        anyos_std::fs::write(dst_fd, &buf[..n as usize]);
    }

    anyos_std::fs::close(src_fd);
    anyos_std::fs::close(dst_fd);
}
