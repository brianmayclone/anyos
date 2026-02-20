//! System event helpers — process name unpacking and lookup.

use alloc::string::String;

/// System thread names that should NOT appear in the dock.
pub const SYSTEM_NAMES: &[&str] = &[
    "compositor", "dock", "cpu_monitor", "init", "netmon", "audiomon", "inputmon", "compositor/gpu",
];

/// Unpack a process name from EVT_PROCESS_SPAWNED words[2..5] (3 u32 LE packed).
pub fn unpack_event_name(w2: u32, w3: u32, w4: u32) -> String {
    let mut buf = [0u8; 12];
    for i in 0..4 { buf[i] = ((w2 >> (i * 8)) & 0xFF) as u8; }
    for i in 0..4 { buf[4 + i] = ((w3 >> (i * 8)) & 0xFF) as u8; }
    for i in 0..4 { buf[8 + i] = ((w4 >> (i * 8)) & 0xFF) as u8; }
    let len = buf.iter().position(|&b| b == 0).unwrap_or(12);
    String::from(core::str::from_utf8(&buf[..len]).unwrap_or(""))
}

/// Query thread name by TID via sysinfo(1,...).
pub fn query_thread_name(tid: u32) -> Option<String> {
    let mut buf = [0u8; 36 * 64];
    let count = anyos_std::sys::sysinfo(1, &mut buf);
    if count == u32::MAX {
        return None;
    }
    for i in 0..count as usize {
        let off = i * 36;
        if off + 36 > buf.len() { break; }
        let entry_tid = u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        if entry_tid == tid {
            let name_bytes = &buf[off + 8..off + 32];
            let name_len = name_bytes.iter().position(|&b| b == 0).unwrap_or(24);
            if name_len > 0 {
                return Some(String::from(
                    core::str::from_utf8(&name_bytes[..name_len]).unwrap_or(""),
                ));
            }
        }
    }
    None
}
