#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let args = anyos_std::process::args(&mut args_buf);
    let path = args.trim();

    if path.is_empty() {
        anyos_std::println!("Usage: stat FILE");
        return;
    }

    let mut stat_buf = [0u32; 2];
    if anyos_std::fs::stat(path, &mut stat_buf) != 0 {
        anyos_std::println!("stat: cannot stat '{}'", path);
        return;
    }

    let file_type = stat_buf[0];
    let size = stat_buf[1];

    anyos_std::println!("  File: {}", path);
    if file_type == 1 {
        anyos_std::println!("  Type: directory");
        anyos_std::println!("  Entries: {}", size);
    } else {
        anyos_std::println!("  Type: regular file");
        anyos_std::println!("  Size: {} bytes", size);
        if size >= 1024 {
            anyos_std::println!("        ({} KiB)", size / 1024);
        }
    }
}
