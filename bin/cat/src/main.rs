#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);

    if args.is_empty() {
        // No file argument — read from stdin (pipe input)
        read_and_print(0);
        return;
    }

    let path = args.trim();

    // Open file
    let fd = anyos_std::fs::open(path, 0); // flags=0 for read
    if fd == u32::MAX {
        anyos_std::println!("cat: {}: No such file", path);
        return;
    }

    read_and_print(fd);
    anyos_std::fs::close(fd);
}

fn read_and_print(fd: u32) {
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
}
