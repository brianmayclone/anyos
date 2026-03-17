#![no_std]
#![no_main]

anyos_std::entry!(main);

use anyos_std::process;

fn main() -> u32 {
    // CAP_SYSTEM required; the kernel will reject the call for unprivileged users.
    process::reboot()
}
