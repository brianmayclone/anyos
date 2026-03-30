#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("whoami - Print current username\n\nUsage: whoami");
        return;
    }

    let uid = anyos_std::process::getuid();
    let mut name_buf = [0u8; 32];
    let nlen = anyos_std::process::getusername(uid, &mut name_buf);
    if nlen != u32::MAX && nlen > 0 {
        let name = core::str::from_utf8(&name_buf[..nlen as usize]).unwrap_or("unknown");
        anyos_std::println!("{}", name);
    } else {
        anyos_std::println!("uid={}", uid);
    }
}
