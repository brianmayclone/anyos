//! Basic console I/O helpers.
//!
//! Thin wrappers around `sys::con_write` / `sys::con_read_*` so the rest of
//! the codebase does not have to care about the newline convention or the
//! unsafe FFI details.

use anyos_std::sys;
use anyos_std::String;

// ─── Output ──────────────────────────────────────────────────────────────────

pub fn print(s: &str) {
    sys::con_write(s);
}
pub fn println(s: &str) {
    sys::con_write(s);
    sys::con_write("\n");
}

// ─── Input ───────────────────────────────────────────────────────────────────

/// Read one line of plain text (echo on).  Used for the login username prompt.
pub fn read_line() -> String {
    let mut buf = [0u8; 256];
    let n = sys::con_read_line(&mut buf);
    let s = core::str::from_utf8(&buf[..n]).unwrap_or("").trim_end();
    String::from(s)
}

/// Read one line without echo.  Used for the password prompt.
pub fn read_password() -> String {
    let mut buf = [0u8; 128];
    let n = sys::con_read_password(&mut buf);
    let s = core::str::from_utf8(&buf[..n]).unwrap_or("");
    String::from(s)
}
