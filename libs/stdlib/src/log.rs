//! Structured logging API for anyOS user programs.
//!
//! Messages are sent to the central `logd` daemon via the "log" named pipe.
//! The pipe is opened lazily on first use and cached for the process lifetime.
//!
//! Wire format: `LEVEL|source|message\n`
//!
//! # Example
//! ```ignore
//! anyos_std::log_info!("server started on port {}", 8080);
//! anyos_std::log_warn!("connection timeout");
//! anyos_std::log_error!("failed to open file: {}", path);
//! ```

use core::sync::atomic::{AtomicU32, Ordering};

/// Pipe ID cache. 0 = not yet opened, u32::MAX = open failed.
static LOG_PIPE: AtomicU32 = AtomicU32::new(0);

/// Log severity levels matching logd protocol.
pub const LEVEL_INFO: &str = "INFO";
pub const LEVEL_WARN: &str = "WARN";
pub const LEVEL_ERROR: &str = "ERROR";
pub const LEVEL_DEBUG: &str = "DEBUG";

/// Open (or return cached) pipe to the logd daemon.
/// Returns pipe_id > 0 on success, 0 on failure.
fn get_pipe() -> u32 {
    let cached = LOG_PIPE.load(Ordering::Relaxed);
    if cached != 0 {
        return if cached == u32::MAX { 0 } else { cached };
    }
    let id = crate::ipc::pipe_open("log");
    if id == 0 {
        LOG_PIPE.store(u32::MAX, Ordering::Relaxed);
        0
    } else {
        LOG_PIPE.store(id, Ordering::Relaxed);
        id
    }
}

/// Derive a source name from the program's argv[0].
/// Returns the filename portion (after last '/'), truncated to 31 chars.
fn source_name(buf: &mut [u8; 32]) -> usize {
    let mut args = [0u8; 256];
    let len = crate::process::getargs(&mut args);
    if len == 0 {
        let fallback = b"unknown";
        buf[..fallback.len()].copy_from_slice(fallback);
        return fallback.len();
    }
    // Find end of argv[0] (first space or end)
    let mut end = 0;
    while end < len && args[end] != b' ' {
        end += 1;
    }
    // Find last '/' for filename extraction
    let mut start = 0;
    for i in 0..end {
        if args[i] == b'/' {
            start = i + 1;
        }
    }
    let name_len = (end - start).min(31);
    buf[..name_len].copy_from_slice(&args[start..start + name_len]);
    name_len
}

/// Send a log message to logd. This is the core function used by the macros.
/// Format: `LEVEL|source|message\n`
pub fn log_msg(level: &str, args: core::fmt::Arguments) {
    let pipe = get_pipe();
    if pipe == 0 {
        return; // logd not running, silently drop
    }

    // Build the message in a stack buffer to avoid heap allocation
    let mut buf = [0u8; 512];
    let mut pos = 0;

    // Level
    let lb = level.as_bytes();
    let ll = lb.len().min(8);
    buf[pos..pos + ll].copy_from_slice(&lb[..ll]);
    pos += ll;
    buf[pos] = b'|';
    pos += 1;

    // Source
    let mut name_buf = [0u8; 32];
    let name_len = source_name(&mut name_buf);
    buf[pos..pos + name_len].copy_from_slice(&name_buf[..name_len]);
    pos += name_len;
    buf[pos] = b'|';
    pos += 1;

    // Message — use a small fmt::Write adapter on the remaining buffer
    struct BufWriter<'a> {
        buf: &'a mut [u8],
        pos: usize,
    }
    impl<'a> core::fmt::Write for BufWriter<'a> {
        fn write_str(&mut self, s: &str) -> core::fmt::Result {
            let bytes = s.as_bytes();
            let avail = self.buf.len() - self.pos;
            let n = bytes.len().min(avail);
            self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
            self.pos += n;
            Ok(())
        }
    }

    let mut writer = BufWriter { buf: &mut buf[pos..511], pos: 0 };
    let _ = core::fmt::write(&mut writer, args);
    pos += writer.pos;

    // Newline terminator
    if pos < 512 {
        buf[pos] = b'\n';
        pos += 1;
    }

    crate::ipc::pipe_write(pipe, &buf[..pos]);
}

/// Log an informational message to the system log.
#[macro_export]
macro_rules! log_info {
    ($($arg:tt)*) => {
        $crate::log::log_msg($crate::log::LEVEL_INFO, format_args!($($arg)*))
    };
}

/// Log a warning message to the system log.
#[macro_export]
macro_rules! log_warn {
    ($($arg:tt)*) => {
        $crate::log::log_msg($crate::log::LEVEL_WARN, format_args!($($arg)*))
    };
}

/// Log an error message to the system log.
#[macro_export]
macro_rules! log_error {
    ($($arg:tt)*) => {
        $crate::log::log_msg($crate::log::LEVEL_ERROR, format_args!($($arg)*))
    };
}

/// Log a debug message to the system log.
#[macro_export]
macro_rules! log_debug {
    ($($arg:tt)*) => {
        $crate::log::log_msg($crate::log::LEVEL_DEBUG, format_args!($($arg)*))
    };
}
