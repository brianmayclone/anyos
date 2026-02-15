#![no_std]
#![no_main]

anyos_std::entry!(main);

fn driver_type_str(t: u8) -> &'static str {
    match t {
        0 => "Block",
        1 => "Char",
        2 => "Network",
        3 => "Display",
        4 => "Input",
        5 => "Audio",
        6 => "Output",
        7 => "Sensor",
        8 => "Bus",
        _ => "Unknown",
    }
}

fn main() {
    let mut buf = [0u8; 64 * 32]; // up to 32 devices
    let count = anyos_std::sys::devlist(&mut buf);

    if count == 0 {
        anyos_std::println!("No devices registered.");
        return;
    }

    anyos_std::println!("{:<32} {:<24} {}", "Path", "Driver", "Type");
    anyos_std::println!("{}", "--------------------------------------------------------------");

    for i in 0..count as usize {
        if (i + 1) * 64 > buf.len() { break; }
        let entry = &buf[i * 64..(i + 1) * 64];

        // Path [0..32]
        let path_bytes = &entry[0..32];
        let path_len = path_bytes.iter().position(|&b| b == 0).unwrap_or(32);
        let path = core::str::from_utf8(&path_bytes[..path_len]).unwrap_or("???");

        // Driver name [32..56]
        let name_bytes = &entry[32..56];
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(24);
        let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("???");

        // Driver type [56]
        let dtype = entry[56];

        anyos_std::println!("{:<32} {:<24} {}", path, name, driver_type_str(dtype));
    }

    anyos_std::println!("\nTotal: {} device(s)", count);
}
