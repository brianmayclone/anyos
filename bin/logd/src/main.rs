#![no_std]
#![no_main]

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::fs::Write as FsWrite;

anyos_std::entry!(main);

// ─── Configuration ──────────────────────────────────────────────────────────

/// Default configuration values.
const DEFAULT_LOG_DIR: &str = "/System/logs";
const DEFAULT_MAX_SIZE: u32 = 1_048_576; // 1 MiB
const DEFAULT_MAX_FILES: u32 = 4;
const DEFAULT_FLUSH_INTERVAL: u32 = 5000; // ms
const CONFIG_PATH: &str = "/System/etc/logd.conf";

/// Named pipe for application log messages.
const LOG_PIPE_NAME: &str = "log";

/// Parsed daemon configuration.
struct Config {
    log_dir: String,
    max_size: u32,
    max_files: u32,
    kernel: bool,
    flush_interval: u32,
}

impl Config {
    /// Load configuration from file, falling back to defaults.
    fn load() -> Self {
        let mut cfg = Config {
            log_dir: String::from(DEFAULT_LOG_DIR),
            max_size: DEFAULT_MAX_SIZE,
            max_files: DEFAULT_MAX_FILES,
            kernel: true,
            flush_interval: DEFAULT_FLUSH_INTERVAL,
        };

        if let Ok(content) = anyos_std::fs::read_to_string(CONFIG_PATH) {
            for line in content.split('\n') {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }
                if let Some(val) = line.strip_prefix("log_dir=") {
                    cfg.log_dir = String::from(val.trim());
                } else if let Some(val) = line.strip_prefix("max_size=") {
                    cfg.max_size = parse_u32(val.trim(), DEFAULT_MAX_SIZE);
                } else if let Some(val) = line.strip_prefix("max_files=") {
                    cfg.max_files = parse_u32(val.trim(), DEFAULT_MAX_FILES);
                } else if let Some(val) = line.strip_prefix("kernel=") {
                    cfg.kernel = val.trim() == "true";
                } else if let Some(val) = line.strip_prefix("flush_interval=") {
                    cfg.flush_interval = parse_u32(val.trim(), DEFAULT_FLUSH_INTERVAL);
                }
            }
        }

        cfg
    }
}

/// Parse a decimal string into u32, returning `default` on failure.
fn parse_u32(s: &str, default: u32) -> u32 {
    let mut val = 0u32;
    let mut found = false;
    for b in s.bytes() {
        if b >= b'0' && b <= b'9' {
            val = val.saturating_mul(10).saturating_add((b - b'0') as u32);
            found = true;
        } else {
            break;
        }
    }
    if found { val } else { default }
}

// ─── Timestamp ──────────────────────────────────────────────────────────────

/// Format uptime in ms as `[HH:MM:SS.mmm]`.
fn format_timestamp(buf: &mut [u8; 16]) -> usize {
    let ms = anyos_std::sys::uptime_ms();
    let secs = ms / 1000;
    let millis = ms % 1000;
    let hours = secs / 3600;
    let mins = (secs % 3600) / 60;
    let s = secs % 60;

    buf[0] = b'[';
    buf[1] = b'0' + ((hours / 10) % 10) as u8;
    buf[2] = b'0' + (hours % 10) as u8;
    buf[3] = b':';
    buf[4] = b'0' + ((mins / 10) % 10) as u8;
    buf[5] = b'0' + (mins % 10) as u8;
    buf[6] = b':';
    buf[7] = b'0' + ((s / 10) % 10) as u8;
    buf[8] = b'0' + (s % 10) as u8;
    buf[9] = b'.';
    buf[10] = b'0' + ((millis / 100) % 10) as u8;
    buf[11] = b'0' + ((millis / 10) % 10) as u8;
    buf[12] = b'0' + (millis % 10) as u8;
    buf[13] = b']';
    buf[14] = b' ';
    15
}

// ─── Log Writer ─────────────────────────────────────────────────────────────

/// Manages log file output with rotation.
struct LogWriter {
    log_dir: String,
    max_size: u32,
    max_files: u32,
    current_size: u32,
    buffer: Vec<u8>,
}

impl LogWriter {
    fn new(cfg: &Config) -> Self {
        // Ensure log directory exists
        anyos_std::fs::mkdir(&cfg.log_dir);

        // Check current log file size
        let log_path = format!("{}/system.log", cfg.log_dir);
        let current_size = file_size(&log_path);

        LogWriter {
            log_dir: cfg.log_dir.clone(),
            max_size: cfg.max_size,
            max_files: cfg.max_files,
            current_size,
            buffer: Vec::new(),
        }
    }

    /// Append a formatted log line to the in-memory buffer.
    fn append(&mut self, line: &[u8]) {
        self.buffer.extend_from_slice(line);
        if !line.ends_with(b"\n") {
            self.buffer.push(b'\n');
        }
    }

    /// Flush the in-memory buffer to disk, rotating if needed.
    fn flush(&mut self) {
        if self.buffer.is_empty() {
            return;
        }

        let bytes_to_write = self.buffer.len() as u32;

        // Rotate before writing if we'd exceed max_size
        if self.current_size + bytes_to_write > self.max_size {
            self.rotate();
        }

        let log_path = format!("{}/system.log", self.log_dir);
        // Append to log file (open with write+append+create flags)
        let fd = anyos_std::fs::open(&log_path, 2 | 4); // append | create
        if fd != u32::MAX {
            anyos_std::fs::write(fd, &self.buffer);
            anyos_std::fs::close(fd);
            self.current_size += bytes_to_write;
        }

        self.buffer.clear();
    }

    /// Rotate log files: system.log -> .1 -> .2 -> ... -> delete oldest.
    fn rotate(&mut self) {
        // Delete the oldest file
        if self.max_files > 1 {
            let oldest = format!("{}/system.log.{}", self.log_dir, self.max_files - 1);
            anyos_std::fs::unlink(&oldest);
        }

        // Shift existing rotated files
        let mut i = self.max_files.saturating_sub(1);
        while i >= 2 {
            let from = format!("{}/system.log.{}", self.log_dir, i - 1);
            let to = format!("{}/system.log.{}", self.log_dir, i);
            anyos_std::fs::rename(&from, &to);
            i -= 1;
        }

        // Rename current to .1
        if self.max_files > 1 {
            let current = format!("{}/system.log", self.log_dir);
            let rotated = format!("{}/system.log.1", self.log_dir);
            anyos_std::fs::rename(&current, &rotated);
        } else {
            // Only 1 file allowed — truncate
            let current = format!("{}/system.log", self.log_dir);
            anyos_std::fs::truncate(&current);
        }

        self.current_size = 0;
    }
}

/// Get the size of a file, or 0 if it doesn't exist.
fn file_size(path: &str) -> u32 {
    let mut stat_buf = [0u32; 7];
    if anyos_std::fs::stat(path, &mut stat_buf) == 0 {
        stat_buf[1] // [type, size, flags, uid, gid, mode, mtime]
    } else {
        0
    }
}

// ─── Dmesg Tracker ──────────────────────────────────────────────────────────

/// Tracks how much of the kernel dmesg buffer we've already consumed.
struct DmesgTracker {
    last_offset: usize,
    buf: [u8; 8192],
}

impl DmesgTracker {
    fn new() -> Self {
        // Read current dmesg to establish baseline — don't re-log boot messages
        let mut buf = [0u8; 8192];
        let n = anyos_std::sys::dmesg(&mut buf) as usize;
        DmesgTracker {
            last_offset: n,
            buf,
        }
    }

    /// Poll for new kernel messages since last call.
    /// Returns a slice of new bytes, or empty if nothing new.
    fn poll_new(&mut self) -> &[u8] {
        let n = anyos_std::sys::dmesg(&mut self.buf) as usize;
        if n > self.last_offset {
            let new = &self.buf[self.last_offset..n];
            self.last_offset = n;
            new
        } else if n < self.last_offset {
            // Ring buffer wrapped — read everything as new
            self.last_offset = n;
            &self.buf[..n]
        } else {
            &[]
        }
    }
}

// ─── Pipe Message Parser ────────────────────────────────────────────────────

/// Format an application log message from pipe data.
/// Input format: `LEVEL|source|message`
/// Output format: `[HH:MM:SS.mmm] LEVEL source: message`
fn format_app_message(raw: &[u8], out: &mut Vec<u8>) {
    // Add timestamp
    let mut ts_buf = [0u8; 16];
    let ts_len = format_timestamp(&mut ts_buf);
    out.extend_from_slice(&ts_buf[..ts_len]);

    // Parse level|source|message
    let mut parts = [0usize; 3]; // start indices
    let mut part_lens = [0usize; 3];
    let mut current_part = 0;
    let mut start = 0;

    for (i, &b) in raw.iter().enumerate() {
        if b == b'|' && current_part < 2 {
            parts[current_part] = start;
            part_lens[current_part] = i - start;
            current_part += 1;
            start = i + 1;
        }
    }
    // Last part is the rest
    parts[current_part] = start;
    part_lens[current_part] = raw.len() - start;

    if current_part >= 2 {
        // Full format: LEVEL|source|message
        let level = &raw[parts[0]..parts[0] + part_lens[0]];
        let source = &raw[parts[1]..parts[1] + part_lens[1]];
        let message = &raw[parts[2]..parts[2] + part_lens[2]];

        // Pad level to 5 chars
        out.extend_from_slice(level);
        for _ in level.len()..5 {
            out.push(b' ');
        }
        out.push(b' ');
        out.extend_from_slice(source);
        out.extend_from_slice(b": ");
        out.extend_from_slice(message);
    } else {
        // Unparseable — log raw
        out.extend_from_slice(b"INFO  ");
        out.extend_from_slice(raw);
    }
    out.push(b'\n');
}

/// Format a kernel log line with timestamp prefix.
fn format_kernel_line(line: &[u8], out: &mut Vec<u8>) {
    let mut ts_buf = [0u8; 16];
    let ts_len = format_timestamp(&mut ts_buf);
    out.extend_from_slice(&ts_buf[..ts_len]);
    out.extend_from_slice(b"KERN  ");
    out.extend_from_slice(line);
    out.push(b'\n');
}

// ─── Main Loop ──────────────────────────────────────────────────────────────

fn main() {
    let cfg = Config::load();

    anyos_std::println!("logd: starting (log_dir={}, max_size={}, max_files={}, flush={}ms)",
        cfg.log_dir, cfg.max_size, cfg.max_files, cfg.flush_interval);

    // Create the log pipe for applications
    let pipe_id = anyos_std::ipc::pipe_create(LOG_PIPE_NAME);
    if pipe_id == 0 {
        anyos_std::println!("logd: failed to create '{}' pipe", LOG_PIPE_NAME);
        return;
    }

    let mut writer = LogWriter::new(&cfg);
    let mut dmesg = if cfg.kernel { Some(DmesgTracker::new()) } else { None };
    let mut pipe_buf = [0u8; 4096];
    let mut last_flush = anyos_std::sys::uptime_ms();

    // Write startup message
    let mut startup_line = Vec::new();
    let mut ts_buf = [0u8; 16];
    let ts_len = format_timestamp(&mut ts_buf);
    startup_line.extend_from_slice(&ts_buf[..ts_len]);
    startup_line.extend_from_slice(b"INFO  logd: logging daemon started\n");
    writer.append(&startup_line);
    writer.flush();

    loop {
        let mut got_data = false;

        // Poll application log pipe (non-blocking)
        let n = anyos_std::ipc::pipe_read(pipe_id, &mut pipe_buf);
        if n > 0 && n != u32::MAX {
            got_data = true;
            let data = &pipe_buf[..n as usize];
            // Process line by line
            let mut line_start = 0;
            for i in 0..data.len() {
                if data[i] == b'\n' {
                    if i > line_start {
                        let line = &data[line_start..i];
                        let mut formatted = Vec::new();
                        format_app_message(line, &mut formatted);
                        writer.append(&formatted);
                    }
                    line_start = i + 1;
                }
            }
            // Handle trailing data without newline
            if line_start < data.len() {
                let line = &data[line_start..];
                let mut formatted = Vec::new();
                format_app_message(line, &mut formatted);
                writer.append(&formatted);
            }
        }

        // Poll kernel dmesg for new messages
        if let Some(ref mut dmesg_tracker) = dmesg {
            let new_data = dmesg_tracker.poll_new();
            if !new_data.is_empty() {
                got_data = true;
                // Process kernel messages line by line
                let mut line_start = 0;
                for i in 0..new_data.len() {
                    if new_data[i] == b'\n' {
                        if i > line_start {
                            let line = &new_data[line_start..i];
                            let mut formatted = Vec::new();
                            format_kernel_line(line, &mut formatted);
                            writer.append(&formatted);
                        }
                        line_start = i + 1;
                    }
                }
                if line_start < new_data.len() {
                    let line = &new_data[line_start..];
                    let mut formatted = Vec::new();
                    format_kernel_line(line, &mut formatted);
                    writer.append(&formatted);
                }
            }
        }

        // Flush to disk periodically
        let now = anyos_std::sys::uptime_ms();
        if now.wrapping_sub(last_flush) >= cfg.flush_interval {
            writer.flush();
            last_flush = now;
        } else if got_data && writer.buffer.len() > 4096 {
            // Flush early if buffer is getting large
            writer.flush();
            last_flush = now;
        }

        // Sleep briefly to avoid busy-waiting
        if !got_data {
            anyos_std::process::sleep(100);
        } else {
            anyos_std::process::sleep(10);
        }
    }
}
