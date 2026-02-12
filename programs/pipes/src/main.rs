#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut buf = [0u8; 80 * 64]; // up to 64 pipes
    let count = anyos_std::sys::pipe_list(&mut buf);

    if count == 0 {
        anyos_std::println!("No open pipes.");
        return;
    }

    anyos_std::println!("{:<6} {:<10} {}", "ID", "Buffered", "Name");
    anyos_std::println!("{}", "-------------------------------");

    for i in 0..count as usize {
        let entry = &buf[i * 80..(i + 1) * 80];
        let id = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
        let buffered = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);
        let name_bytes = &entry[8..72];
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(64);
        let name = core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("???");
        anyos_std::println!("{:<6} {:<10} {}", id, buffered, name);
    }

    anyos_std::println!("\nTotal: {} pipe(s)", count);
}
