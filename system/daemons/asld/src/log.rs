//! Simple append-only log at /System/var/asl/asld.log.
//!
//! Lines: "[uptime_ms] [level] [tag] message". Best-effort; file write
//! failures are silently dropped so logging never panics the daemon.

use alloc::format;

const LOG_PATH: &str = "/System/var/asl/asld.log";
const MAX_LINE_LEN: usize = 1024;

#[cfg(target_os = "linux")]
pub fn emit(level: &str, tag: &str, msg: &str) {
    // Host/test mode: route to stderr-style println so tests stay noise-free
    // but developers see output. No file I/O on Linux.
    anyos_std::println!("asld[{}][{}] {}", level, tag, msg);
}

#[cfg(not(target_os = "linux"))]
pub fn emit(level: &str, tag: &str, msg: &str) {
    use anyos_std::fs;

    let _ = fs::mkdir("/System/var");
    let _ = fs::mkdir("/System/var/asl");

    let ts = anyos_std::sys::uptime_ms();
    let mut line = format!("[{}] [{}] [{}] {}\n", ts, level, tag, msg);
    if tag != "ipc" {
        anyos_std::println!("[asld][{}][{}] {}", level, tag, msg);
    }
    if line.len() > MAX_LINE_LEN {
        line.truncate(MAX_LINE_LEN.saturating_sub(1));
        line.push('\n');
    }

    let fd = fs::open(LOG_PATH, fs::O_WRITE | fs::O_APPEND | fs::O_CREATE);
    if fd == u32::MAX || fd == 0 {
        return;
    }
    fs::write(fd, line.as_bytes());
    fs::close(fd);
}

pub fn info(tag: &str, msg: &str) {
    emit("info", tag, msg);
}

pub fn warn(tag: &str, msg: &str) {
    emit("warn", tag, msg);
}

pub fn error(tag: &str, msg: &str) {
    emit("error", tag, msg);
}

/// Read the last `limit` lines from the asld log, optionally filtered to
/// lines containing `needle` (case-sensitive substring match — typically
/// the distro name).
///
/// Returns lines in chronological order (oldest first within the
/// returned tail). On any I/O failure or missing log returns an empty
/// vec — best-effort to mirror the write path. Limit `0` means "no
/// cap" and returns all matching lines.
#[cfg(not(target_os = "linux"))]
pub fn read_tail(limit: usize, needle: Option<&str>) -> alloc::vec::Vec<alloc::string::String> {
    use anyos_std::fs;
    // flags=0 is the read-only mode in anyos_std::fs.
    let fd = fs::open(LOG_PATH, 0);
    if fd == 0 || fd == u32::MAX {
        return alloc::vec::Vec::new();
    }
    // 64 KiB tail buffer caps memory usage. The CLI is interactive — if
    // the log grows beyond that, the operator can use `aslctl logs`
    // multiple times after clearing or rotate. ADR-0010 forbids
    // unbounded read operations from a daemon.
    let mut buf = alloc::vec![0u8; 65536];
    let n = fs::read(fd, &mut buf);
    let _ = fs::close(fd);
    if n == u32::MAX {
        return alloc::vec::Vec::new();
    }
    let text = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(s) => s,
        Err(_) => return alloc::vec::Vec::new(),
    };
    tail_lines(text, limit, needle)
}

#[cfg(target_os = "linux")]
pub fn read_tail(_limit: usize, _needle: Option<&str>) -> alloc::vec::Vec<alloc::string::String> {
    // Host/test mode: there is no on-disk log (emit() prints to stdout).
    // Tests exercise `tail_lines` directly.
    alloc::vec::Vec::new()
}

/// Pure tail+filter operation. Splits `text` at '\n', drops the empty
/// trailing component if the input ends with '\n', optionally filters
/// by substring, and keeps the last `limit` matches. `limit == 0` means
/// "no cap".
///
/// Pure function — no I/O — so tests can pin behaviour with synthetic
/// log buffers.
pub fn tail_lines(
    text: &str,
    limit: usize,
    needle: Option<&str>,
) -> alloc::vec::Vec<alloc::string::String> {
    use alloc::string::ToString;
    let mut all: alloc::vec::Vec<&str> = text.lines().collect();
    if let Some(needle) = needle {
        all.retain(|line| line.contains(needle));
    }
    if limit > 0 && all.len() > limit {
        let skip = all.len() - limit;
        all.drain(..skip);
    }
    all.iter().map(|s| s.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn sample() -> String {
        // Three runtime lines, one ipc line. Simulates a real asld log.
        String::from(
            "[100] [info] [runtime] start: distro='ubuntu-dev'\n\
             [120] [info] [ipc] rx tid=1 verb=STATUS\n\
             [200] [warn] [runtime] vm exit reason=halt distro='debian-test'\n\
             [300] [info] [runtime] start: distro='ubuntu-dev'\n",
        )
    }

    #[test]
    fn tail_returns_all_when_below_limit() {
        let out = tail_lines(&sample(), 10, None);
        assert_eq!(out.len(), 4);
        assert!(out[0].contains("start: distro='ubuntu-dev'"));
    }

    #[test]
    fn tail_keeps_only_last_n() {
        let out = tail_lines(&sample(), 2, None);
        assert_eq!(out.len(), 2);
        assert!(out[0].contains("vm exit"));
        assert!(out[1].contains("start: distro='ubuntu-dev'"));
    }

    #[test]
    fn tail_filter_matches_substring() {
        let out = tail_lines(&sample(), 0, Some("debian-test"));
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("debian-test"));
    }

    #[test]
    fn tail_filter_then_limit() {
        let out = tail_lines(&sample(), 1, Some("ubuntu-dev"));
        // Two ubuntu-dev lines, limit=1 keeps the latest.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0], "[300] [info] [runtime] start: distro='ubuntu-dev'");
    }

    #[test]
    fn tail_zero_limit_is_no_cap() {
        let out = tail_lines(&sample(), 0, None);
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn tail_handles_empty_input() {
        let out = tail_lines("", 5, None);
        assert!(out.is_empty());
        let out2 = tail_lines("", 0, Some("anything"));
        assert!(out2.is_empty());
    }

    #[test]
    fn tail_handles_input_without_trailing_newline() {
        // `lines()` drops the empty trailing element automatically; we
        // pin that here so a partial last line doesn't get lost or
        // double-counted.
        let out = tail_lines("alpha\nbeta\ngamma", 2, None);
        assert_eq!(
            out,
            alloc::vec![String::from("beta"), String::from("gamma")]
        );
    }

    #[test]
    fn tail_filter_no_match_returns_empty() {
        let out = tail_lines(&sample(), 0, Some("nonexistent"));
        assert!(out.is_empty());
    }
}
