//! lxe command line entry point.

#![no_std]
#![no_main]

use anyos_std::process;

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = process::args(&mut args_buf);
    liblxecore::run_cli(raw);
}
