#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let args_buf = &mut [0u8; 256];
    let args_len = anyos_std::process::getargs(args_buf);
    let args = core::str::from_utf8(&args_buf[..args_len]).unwrap_or("");

    let path = if args.is_empty() { "/" } else { args.trim() };

    // Each entry is 64 bytes: [type:u8, name_len:u8, pad:u16, size:u32, name:56bytes]
    let mut buf = [0u8; 64 * 64]; // max 64 entries
    let count = anyos_std::fs::readdir(path, &mut buf);

    if count == u32::MAX {
        anyos_std::println!("ls: cannot access '{}': No such directory", path);
        return;
    }

    anyos_std::println!("Directory: {}", path);
    anyos_std::println!("{:<20} {:>8}  {}", "Name", "Size", "Type");
    anyos_std::println!("{}", "------------------------------------");

    for i in 0..count as usize {
        let entry = &buf[i * 64..(i + 1) * 64];
        let entry_type = entry[0];
        let name_len = entry[1] as usize;
        let size = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
        let name = core::str::from_utf8(&entry[8..8 + name_len]).unwrap_or("???");

        let type_str = match entry_type {
            0 => "FILE",
            1 => "DIR ",
            2 => "DEV ",
            _ => "??? ",
        };

        anyos_std::println!("{:<20} {:>8}  {}", name, size, type_str);
    }

    anyos_std::println!("\n{} entries", count);
}
