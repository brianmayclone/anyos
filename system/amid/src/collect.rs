//! Data collectors for the ami database.
//!
//! Each function collects system information via syscalls, clears the
//! corresponding table, and inserts fresh rows. Uses the same binary
//! formats as `system/taskmanager/src/data.rs` and `bin/devlist`.

use alloc::format;
use libdb_client::Database;

// ── Constants ────────────────────────────────────────────────────────────────

const THREAD_ENTRY_SIZE: usize = 60;
const MAX_THREADS: usize = 256;
const MAX_CPUS: usize = 16;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Extract a null-terminated ASCII string from a byte slice.
fn str_from_bytes(buf: &[u8]) -> &str {
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    core::str::from_utf8(&buf[..len]).unwrap_or("")
}

/// Format an IPv4 address from 4 bytes.
fn format_ip(b: &[u8]) -> alloc::string::String {
    format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3])
}

/// Format a MAC address from 6 bytes.
fn format_mac(b: &[u8]) -> alloc::string::String {
    format!("{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            b[0], b[1], b[2], b[3], b[4], b[5])
}

/// Escape single quotes in a text value for safe SQL insertion.
fn escape_sql(s: &str) -> alloc::string::String {
    if !s.contains('\'') {
        return alloc::string::String::from(s);
    }
    let mut out = alloc::string::String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        if ch == '\'' {
            out.push('\'');
        }
        out.push(ch);
    }
    out
}

/// Insert a key/value text pair into a table.
fn insert_kv_text(db: &Database, table: &str, key: &str, value: &str) {
    let sql = format!("INSERT INTO {} (key, value) VALUES ('{}', '{}')",
                      table, key, escape_sql(value));
    let _ = db.exec(&sql);
}

/// Insert a key/value integer pair into the mem table.
fn insert_kv_int(db: &Database, key: &str, value: u32) {
    let sql = format!("INSERT INTO mem (key, value) VALUES ('{}', {})", key, value);
    let _ = db.exec(&sql);
}

// ── Static: Hardware Info ────────────────────────────────────────────────────

/// Collect hardware information (sysinfo cmd 4, 96 bytes).
/// Populates the `hw` table with key/value pairs.
pub fn collect_hw(db: &Database) {
    let _ = db.exec("DELETE FROM hw");

    let mut buf = [0u8; 96];
    anyos_std::sys::sysinfo(4, &mut buf);

    let brand = str_from_bytes(&buf[0..48]);
    let vendor = str_from_bytes(&buf[48..64]);
    let tsc_mhz = u32::from_le_bytes([buf[64], buf[65], buf[66], buf[67]]);
    let cpu_count = u32::from_le_bytes([buf[68], buf[69], buf[70], buf[71]]);
    let boot_mode = u32::from_le_bytes([buf[72], buf[73], buf[74], buf[75]]);
    let total_mem = u32::from_le_bytes([buf[76], buf[77], buf[78], buf[79]]);
    let fb_w = u32::from_le_bytes([buf[84], buf[85], buf[86], buf[87]]);
    let fb_h = u32::from_le_bytes([buf[88], buf[89], buf[90], buf[91]]);
    let fb_bpp = u32::from_le_bytes([buf[92], buf[93], buf[94], buf[95]]);

    insert_kv_text(db, "hw", "cpu_brand", brand);
    insert_kv_text(db, "hw", "cpu_vendor", vendor);
    insert_kv_text(db, "hw", "tsc_mhz", &format!("{}", tsc_mhz));
    insert_kv_text(db, "hw", "cpu_count", &format!("{}", cpu_count));
    insert_kv_text(db, "hw", "boot_mode", match boot_mode {
        0 => "BIOS",
        1 => "UEFI",
        _ => "Unknown",
    });
    insert_kv_text(db, "hw", "total_mem_mib", &format!("{}", total_mem));
    insert_kv_text(db, "hw", "fb_width", &format!("{}", fb_w));
    insert_kv_text(db, "hw", "fb_height", &format!("{}", fb_h));
    insert_kv_text(db, "hw", "fb_bpp", &format!("{}", fb_bpp));
}

// ── Fast: Memory Stats ───────────────────────────────────────────────────────

/// Collect memory statistics (sysinfo cmd 0, 16 bytes).
pub fn collect_mem(db: &Database) {
    let _ = db.exec("DELETE FROM mem");

    let mut buf = [0u8; 16];
    if anyos_std::sys::sysinfo(0, &mut buf) != 0 {
        return;
    }

    let total_frames = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let free_frames = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let heap_used = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let heap_total = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    // Derive MiB: each frame = 4 KiB
    let free_mib = free_frames / 256;

    insert_kv_int(db, "total_frames", total_frames);
    insert_kv_int(db, "free_frames", free_frames);
    insert_kv_int(db, "heap_used", heap_used);
    insert_kv_int(db, "heap_total", heap_total);
    insert_kv_int(db, "free_mem_mib", free_mib);
}

// ── Fast: CPU Load ───────────────────────────────────────────────────────────

/// Per-CPU state for delta-based load calculation.
pub struct CpuState {
    pub prev_total: u32,
    pub prev_idle: u32,
    pub prev_core_total: [u32; MAX_CPUS],
    pub prev_core_idle: [u32; MAX_CPUS],
}

impl CpuState {
    pub fn new() -> Self {
        CpuState {
            prev_total: 0,
            prev_idle: 0,
            prev_core_total: [0; MAX_CPUS],
            prev_core_idle: [0; MAX_CPUS],
        }
    }
}

/// Collect per-core CPU load (sysinfo cmd 3).
/// Requires mutable `CpuState` for delta computation between calls.
pub fn collect_cpu(db: &Database, state: &mut CpuState) {
    let _ = db.exec("DELETE FROM cpu");

    let mut buf = [0u8; 16 + 8 * MAX_CPUS];
    anyos_std::sys::sysinfo(3, &mut buf);

    let total = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let idle = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let ncpu = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let ncpu = ncpu.max(1).min(MAX_CPUS as u32) as usize;

    // Overall load (core = -1 encoded as large int; we use -1)
    let dt = total.wrapping_sub(state.prev_total);
    let di = idle.wrapping_sub(state.prev_idle);
    let overall = if dt > 0 {
        100u32.saturating_sub(di.saturating_mul(100) / dt)
    } else {
        0
    };
    state.prev_total = total;
    state.prev_idle = idle;

    // Use -1 as i64 for "overall" row
    let sql = format!("INSERT INTO cpu (core, load_pct) VALUES (-1, {})", overall);
    let _ = db.exec(&sql);

    // Per-core load
    for i in 0..ncpu {
        let off = 16 + i * 8;
        if off + 8 > buf.len() { break; }
        let ct = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let ci = u32::from_le_bytes([buf[off + 4], buf[off + 5], buf[off + 6], buf[off + 7]]);
        let dct = ct.wrapping_sub(state.prev_core_total[i]);
        let dci = ci.wrapping_sub(state.prev_core_idle[i]);
        let pct = if dct > 0 {
            100u32.saturating_sub(dci.saturating_mul(100) / dct)
        } else {
            0
        };
        state.prev_core_total[i] = ct;
        state.prev_core_idle[i] = ci;

        let sql = format!("INSERT INTO cpu (core, load_pct) VALUES ({}, {})", i, pct);
        let _ = db.exec(&sql);
    }
}

// ── Fast: Thread List ────────────────────────────────────────────────────────

/// Collect thread list (sysinfo cmd 1, 60-byte entries).
pub fn collect_threads(db: &Database) {
    let _ = db.exec("DELETE FROM threads");

    let mut buf = [0u8; THREAD_ENTRY_SIZE * MAX_THREADS];
    let count = anyos_std::sys::sysinfo(1, &mut buf);
    if count == u32::MAX { return; }

    for i in 0..count as usize {
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() { break; }

        let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        let prio = buf[off + 4];
        let state = buf[off + 5];
        let arch = buf[off + 6];
        let name = str_from_bytes(&buf[off + 8..off + 32]);
        let uid = u16::from_le_bytes([buf[off + 56], buf[off + 57]]);
        let user_pages = u32::from_le_bytes([buf[off + 32], buf[off + 33], buf[off + 34], buf[off + 35]]);
        let cpu_ticks = u32::from_le_bytes([buf[off + 36], buf[off + 37], buf[off + 38], buf[off + 39]]);

        let sql = format!(
            "INSERT INTO threads (tid, name, state, prio, arch, uid, pages, ticks) VALUES ({}, '{}', {}, {}, {}, {}, {}, {})",
            tid, escape_sql(name), state, prio, arch, uid, user_pages, cpu_ticks
        );
        let _ = db.exec(&sql);
    }
}

// ── Slow: Device List ────────────────────────────────────────────────────────

/// Collect device list (SYS_DEVLIST, 64-byte entries).
pub fn collect_devices(db: &Database) {
    let _ = db.exec("DELETE FROM devices");

    let mut buf = [0u8; 64 * 32]; // up to 32 devices
    let count = anyos_std::sys::devlist(&mut buf);
    if count == 0 { return; }

    for i in 0..count as usize {
        if (i + 1) * 64 > buf.len() { break; }
        let entry = &buf[i * 64..(i + 1) * 64];

        let path = str_from_bytes(&entry[0..32]);
        let driver = str_from_bytes(&entry[32..56]);
        let dtype = entry[56] as u32;

        let sql = format!(
            "INSERT INTO devices (path, driver, dtype) VALUES ('{}', '{}', {})",
            escape_sql(path), escape_sql(driver), dtype
        );
        let _ = db.exec(&sql);
    }
}

// ── Slow: Disk List ──────────────────────────────────────────────────────────

/// Collect block device list (SYS_DISK_LIST, 32-byte entries).
pub fn collect_disks(db: &Database) {
    let _ = db.exec("DELETE FROM disks");

    let mut buf = [0u8; 32 * 16]; // up to 16 block devices
    let count = anyos_std::sys::disk_list(&mut buf);
    if count == 0 || count == u32::MAX { return; }

    for i in 0..count as usize {
        let off = i * 32;
        if off + 32 > buf.len() { break; }

        let id = buf[off] as u32;
        let disk_id = buf[off + 1] as u32;
        let part = buf[off + 2] as i32; // 0xFF = whole disk
        let start_lba = u64::from_le_bytes([
            buf[off + 8], buf[off + 9], buf[off + 10], buf[off + 11],
            buf[off + 12], buf[off + 13], buf[off + 14], buf[off + 15],
        ]);
        let size_sect = u64::from_le_bytes([
            buf[off + 16], buf[off + 17], buf[off + 18], buf[off + 19],
            buf[off + 20], buf[off + 21], buf[off + 22], buf[off + 23],
        ]);

        let sql = format!(
            "INSERT INTO disks (id, disk_id, part, start_lba, size_sect) VALUES ({}, {}, {}, {}, {})",
            id, disk_id, part, start_lba as i64, size_sect as i64
        );
        let _ = db.exec(&sql);
    }
}

// ── Slow: Network Config ─────────────────────────────────────────────────────

/// Collect network configuration (SYS_NET_CONFIG cmd 0, 24 bytes).
pub fn collect_net(db: &Database) {
    let _ = db.exec("DELETE FROM net");

    let mut buf = [0u8; 24];
    anyos_std::net::get_config(&mut buf);

    // [ip:4, mask:4, gw:4, dns:4, mac:6, link:1, pad:1]
    insert_kv_text(db, "net", "ip", &format_ip(&buf[0..4]));
    insert_kv_text(db, "net", "mask", &format_ip(&buf[4..8]));
    insert_kv_text(db, "net", "gateway", &format_ip(&buf[8..12]));
    insert_kv_text(db, "net", "dns", &format_ip(&buf[12..16]));
    insert_kv_text(db, "net", "mac", &format_mac(&buf[16..22]));
    insert_kv_text(db, "net", "link", if buf[22] != 0 { "up" } else { "down" });

    let nic_enabled = anyos_std::net::is_nic_enabled();
    let nic_available = anyos_std::net::is_nic_available();
    insert_kv_text(db, "net", "nic_enabled", if nic_enabled { "true" } else { "false" });
    insert_kv_text(db, "net", "nic_available", if nic_available { "true" } else { "false" });
}

// ── Slow: Service List ───────────────────────────────────────────────────────

/// Collect service status by scanning `/System/etc/svc/` directory
/// and checking for running threads with matching names.
pub fn collect_svc(db: &Database) {
    let _ = db.exec("DELETE FROM svc");

    // Read service config directory
    let mut dir_buf = [0u8; 4096];
    let dir_len = anyos_std::fs::readdir("/System/etc/svc", &mut dir_buf);
    if dir_len == 0 || dir_len == u32::MAX { return; }

    // Pre-fetch thread list for status lookup
    let mut thread_buf = [0u8; THREAD_ENTRY_SIZE * MAX_THREADS];
    let thread_count = anyos_std::sys::sysinfo(1, &mut thread_buf);

    // Parse directory entries (newline-separated names)
    let dir_data = &dir_buf[..dir_len as usize];
    for entry in dir_data.split(|&b| b == b'\n') {
        if entry.is_empty() { continue; }
        let name = match core::str::from_utf8(entry) {
            Ok(s) => s.trim(),
            Err(_) => continue,
        };
        if name.is_empty() || name.starts_with('.') { continue; }

        // Check if a thread with this name is running
        let (status, tid) = find_thread_status(name, &thread_buf, thread_count);

        let sql = format!(
            "INSERT INTO svc (name, status, tid) VALUES ('{}', '{}', {})",
            escape_sql(name), status, tid
        );
        let _ = db.exec(&sql);
    }
}

/// Search the thread list for a thread matching `name`.
/// Returns ("running", tid) or ("stopped", 0).
fn find_thread_status(name: &str, buf: &[u8], count: u32) -> (&'static str, u32) {
    if count == u32::MAX { return ("unknown", 0); }
    let name_bytes = name.as_bytes();

    for i in 0..count as usize {
        let off = i * THREAD_ENTRY_SIZE;
        if off + THREAD_ENTRY_SIZE > buf.len() { break; }

        // Thread name at offset 8, null-terminated, max 23 bytes
        let name_start = off + 8;
        let mut len = 0;
        for j in 0..23 {
            if buf[name_start + j] == 0 { break; }
            len += 1;
        }
        if len == name_bytes.len() && &buf[name_start..name_start + len] == name_bytes {
            let state = buf[off + 5];
            // 0=ready, 1=running, 2=blocked — alive; 3=dead
            if state <= 2 {
                let tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
                return ("running", tid);
            }
        }
    }
    ("stopped", 0)
}
