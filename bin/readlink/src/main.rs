#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let path = anyos_std::process::args(&mut args_buf).trim();

    if path.is_empty() {
        anyos_std::println!("Usage: readlink FILE");
        return;
    }

    let mut buf = [0u8; 256];
    let n = anyos_std::fs::readlink(path, &mut buf);
    if n == u32::MAX {
        anyos_std::println!("readlink: {}: Not a symbolic link", path);
        return;
    }

    let target = core::str::from_utf8(&buf[..n as usize]).unwrap_or("?");
    anyos_std::println!("{}", target);
}
