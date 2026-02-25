//! Dialog layout constants, cleanup helper, and title formatter.

use anyos_std::ipc;
use anyos_std::process;

// ── Layout constants ──

pub const DIALOG_W: u32 = 380;
pub const PAD: i32 = 24;
pub const ROW_H: i32 = 48;
pub const BTN_W: u32 = 130;
pub const BTN_H: u32 = 34;
pub const BTN_PAD: i32 = 12;

/// Release the singleton lock pipe and exit with the given code.
pub fn cleanup(lock_pipe: u32, code: u32) -> ! {
    if lock_pipe != 0 && lock_pipe != u32::MAX {
        ipc::pipe_close(lock_pipe);
    }
    process::exit(code);
}

/// Format `"AppName" wants access to:` into `buf` using UTF-8 curly quotes.
/// Returns the number of bytes written.
pub fn format_title(app_name: &str, buf: &mut [u8]) -> usize {
    let mut pos = 0usize;
    let max = buf.len().saturating_sub(1);

    // Opening double quote (UTF-8: E2 80 9C)
    if pos + 2 < max {
        buf[pos] = 0xE2;
        buf[pos + 1] = 0x80;
        buf[pos + 2] = 0x9C;
        pos += 3;
    }

    for &b in app_name.as_bytes() {
        if pos >= max { break; }
        buf[pos] = b;
        pos += 1;
    }

    // Closing double quote (UTF-8: E2 80 9D)
    if pos + 2 < max {
        buf[pos] = 0xE2;
        buf[pos + 1] = 0x80;
        buf[pos + 2] = 0x9D;
        pos += 3;
    }

    for &b in b" wants access to:" {
        if pos >= max { break; }
        buf[pos] = b;
        pos += 1;
    }

    pos
}
