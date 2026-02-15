#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let path = args.trim();

    if path.is_empty() {
        anyos_std::println!("Usage: rev FILE");
        return;
    }

    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        anyos_std::println!("rev: cannot open '{}'", path);
        return;
    }

    let mut file_buf = anyos_std::vec![0u8; 64 * 1024];
    let mut total: usize = 0;
    let mut read_buf = [0u8; 512];
    loop {
        let n = anyos_std::fs::read(fd, &mut read_buf);
        if n == 0 || n == u32::MAX { break; }
        let n = n as usize;
        if total + n > file_buf.len() { break; }
        file_buf[total..total + n].copy_from_slice(&read_buf[..n]);
        total += n;
    }
    anyos_std::fs::close(fd);

    let text = core::str::from_utf8(&file_buf[..total]).unwrap_or("");
    for line in text.lines() {
        let reversed: alloc::string::String = line.chars().rev().collect();
        anyos_std::println!("{}", reversed);
    }
}
