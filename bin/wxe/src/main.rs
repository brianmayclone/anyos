//! wxe command line entry point.

#![no_std]
#![no_main]

use anyos_std::process;

anyos_std::entry!(main);

fn main() {
    let mut args_buf = [0u8; 512];
    let raw = process::args(&mut args_buf);
    libwxecore::run_cli(raw);
}
