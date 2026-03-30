#![no_std]
#![no_main]

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("pwd - Print current working directory\n\nUsage: pwd");
        return;
    }

    let mut buf = [0u8; 256];
    let len = anyos_std::fs::getcwd(&mut buf);
    if len > 0 {
        let path = core::str::from_utf8(&buf[..len as usize]).unwrap_or("/");
        anyos_std::println!("{}", path);
    } else {
        anyos_std::println!("/");
    }
}
