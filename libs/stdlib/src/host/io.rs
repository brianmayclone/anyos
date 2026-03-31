//! Host-mode I/O — delegates to std.

use core::fmt::{self, Write as FmtWrite};

pub struct Stdout;

impl FmtWrite for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        use std::io::Write;
        let _ = std::io::stdout().write_all(s.as_bytes());
        Ok(())
    }
}

pub fn _print(args: fmt::Arguments) {
    let _ = Stdout.write_fmt(args);
}

pub fn _print_panic(info: &core::panic::PanicInfo) {
    eprintln!("PANIC: {}", info);
}

pub fn _print_str(s: &str) {
    use std::io::Write;
    let _ = std::io::stdout().write_all(s.as_bytes());
}
