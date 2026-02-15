#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut buf = [0u8; 8];
    anyos_std::sys::time(&mut buf);
    let year = u16::from_le_bytes([buf[0], buf[1]]) as u32;
    let month = buf[2] as u32;
    let day = buf[3] as u32;
    let hour = buf[4] as u32;
    let min = buf[5] as u32;
    let sec = buf[6] as u32;
    anyos_std::println!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", year, month, day, hour, min, sec);
}
