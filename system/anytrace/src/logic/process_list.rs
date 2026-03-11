//! Process/thread enumeration via SYS_SYSINFO.

use alloc::vec::Vec;
use alloc::string::String;

/// Information about a single thread.
#[derive(Clone)]
pub struct ProcessEntry {
    pub tid: u32,
    pub name: String,
    pub state: u8,
    pub priority: u8,
    pub cpu_ticks: u32,
    pub user_pages: u32,
}

/// Poll the system for the current process list.
///
/// Uses `anyos_std::sys::sysinfo(1, buf)` which returns the **count** of
/// thread entries written into `buf`.  Each entry is 60 bytes.
pub fn poll_processes() -> Vec<ProcessEntry> {
    let mut buf = [0u8; 60 * 128]; // room for 128 threads
    let count = anyos_std::sys::sysinfo(1, &mut buf);
    if count == 0 || count == u32::MAX {
        return Vec::new();
    }

    parse_sysinfo(&buf, count as usize)
}

/// Parse the sysinfo buffer into process entries.
///
/// Kernel 60-byte entry layout (all LE):
///   +0   u32   tid
///   +4   u8    priority
///   +5   u8    state  (0=ready, 1=running, 2=blocked, 3=dead)
///   +6   u8    arch_mode
///   +7   u8    pad
///   +8   [u8;24] name (null-terminated C-string)
///   +32  u32   user_pages
///   +36  u32   cpu_ticks
///   +40  u64   io_read_bytes
///   +48  u64   io_write_bytes
///   +56  u16   uid
///   +58  u16   pad
fn parse_sysinfo(buf: &[u8], count: usize) -> Vec<ProcessEntry> {
    const ENTRY_SIZE: usize = 60;
    let mut entries = Vec::new();

    for i in 0..count {
        let off = i * ENTRY_SIZE;
        if off + ENTRY_SIZE > buf.len() {
            break;
        }

        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let priority = buf[off + 4];
        let state = buf[off + 5];

        // Skip dead threads
        if state > 2 {
            continue;
        }

        let name_bytes = &buf[off + 8..off + 32];
        let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(24);
        let name = String::from(core::str::from_utf8(&name_bytes[..name_len]).unwrap_or("?"));

        let user_pages = u32::from_le_bytes([buf[off + 32], buf[off + 33], buf[off + 34], buf[off + 35]]);
        let cpu_ticks = u32::from_le_bytes([buf[off + 36], buf[off + 37], buf[off + 38], buf[off + 39]]);

        entries.push(ProcessEntry {
            tid,
            name,
            state,
            priority,
            cpu_ticks,
            user_pages,
        });
    }

    entries
}
