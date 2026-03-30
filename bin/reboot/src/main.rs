#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::process;

fn main() -> u32 {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!("reboot - Reboot the system\n\nUsage: reboot");
        return 0;
    }
    // CAP_SYSTEM required; the kernel will reject the call for unprivileged users.
    process::reboot()
}
