use core::fmt::{self, Write};

struct Stdout;

impl Write for Stdout {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        crate::fs::write(1, s.as_bytes());
        Ok(())
    }
}

/// Internal: print formatted arguments to stdout.
pub fn _print(args: fmt::Arguments) {
    let _ = Stdout.write_fmt(args);
}

/// Internal: print a panic message to stdout.
pub fn _print_panic(info: &core::panic::PanicInfo) {
    let _ = write!(Stdout, "PANIC: {}\n", info);
}

/// Internal: print a raw string to stdout.
pub fn _print_str(s: &str) {
    crate::fs::write(1, s.as_bytes());
}

/// Print formatted text to stdout (no newline).
#[macro_export]
macro_rules! print {
    ($($arg:tt)*) => ($crate::io::_print(format_args!($($arg)*)));
}

/// Print formatted text to stdout with a trailing newline.
#[macro_export]
macro_rules! println {
    () => ($crate::print!("\n"));
    ($($arg:tt)*) => ($crate::print!("{}\n", format_args!($($arg)*)));
}
