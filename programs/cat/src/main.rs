#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let args_buf = &mut [0u8; 256];
    let args_len = anyos_std::process::getargs(args_buf);
    let args = core::str::from_utf8(&args_buf[..args_len]).unwrap_or("");

    if args.is_empty() {
        anyos_std::println!("Usage: cat <file>");
        return;
    }

    let path = args.trim();

    // Open file
    let fd = anyos_std::fs::open(path, 0); // flags=0 for read
    if fd == u32::MAX {
        anyos_std::println!("cat: {}: No such file", path);
        return;
    }

    // Read and print in chunks
    let mut buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(fd, &mut buf);
        if n == 0 || n == u32::MAX {
            break;
        }
        if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
            anyos_std::print!("{}", text);
        } else {
            // Binary data — print hex
            for i in 0..n as usize {
                anyos_std::print!("{:02x} ", buf[i]);
            }
        }
    }

    anyos_std::fs::close(fd);
}
