//! IPC protocol for the `displayd` event channel.
//!
//! Channel name: `"displayd"`.
//!
//! Each request is a 5-tuple `[cmd, requester_sub, arg2, arg3, arg4]`.
//! The daemon emits the response back to the requester via
//! `evt_chan_emit_to(chan, requester_sub, ...)` so multiple in-flight
//! requests from different clients don't get tangled.
//!
//! ## CMD_LIST_OUTPUTS (0x7001)
//!
//! Returns the number of currently advertised outputs. Clients then
//! call `display::list()` (which reads `SYS_DISPLAY_LIST` directly —
//! no privilege needed) to fetch the array. displayd itself does not
//! re-marshal the output list because there's no security boundary
//! to cross.
//!
//!   evt[0] = 0x7001
//!   evt[1] = requester_sub
//!
//! Response: EVT_OUTPUT_COUNT
//!   evt[0] = 0x7002
//!   evt[1] = output count
//!
//! ## CMD_REAPPLY_LAYOUT (0x7003)
//!
//! Re-derive a layout from `display.conf` and atomically apply it.
//! Useful after a write to `display.conf` from a settings app.
//!
//!   evt[0] = 0x7003
//!   evt[1] = requester_sub
//!
//! Response: EVT_LAYOUT_CHANGED (also broadcast to all subscribers)
//!   evt[0] = 0x7004
//!   evt[1] = result code (0 = ok, !=0 = LayoutError or hard error)
//!
//! ## CMD_PROBE_HOTPLUG (0x7005)
//!
//! Force a hotplug check now (otherwise it's polled once a second).
//! Mainly for tests.
//!
//!   evt[0] = 0x7005
//!   evt[1] = requester_sub
//!
//! Response: EVT_HOTPLUG_DONE
//!   evt[0] = 0x7006
//!   evt[1] = 0

use anyos_std::{display, ipc};

pub const CMD_LIST_OUTPUTS: u32 = 0x7001;
pub const EVT_OUTPUT_COUNT: u32 = 0x7002;

pub const CMD_REAPPLY_LAYOUT: u32 = 0x7003;
pub const EVT_LAYOUT_CHANGED: u32 = 0x7004;

pub const CMD_PROBE_HOTPLUG: u32 = 0x7005;
pub const EVT_HOTPLUG_DONE: u32 = 0x7006;

pub const CMD_PUSH_LAYOUT: u32 = 0x7007;
pub const EVT_LAYOUT_PUSHED: u32 = 0x7008;

/// Set a configuration value for a single output, identified by EDID
/// hash, then re-apply the layout. Used by display-settings to commit
/// changes from the GUI (resolution, scale, orientation, …).
///
/// Wire: evt[2] = SHM id holding a [u8; SIZE_OF_OUTPUT_CONFIG_BLOB]
/// payload (see OutputConfigBlob below).
pub const CMD_SET_OUTPUT_CONFIG: u32 = 0x7009;
pub const EVT_OUTPUT_CONFIG_OK: u32 = 0x700A;

/// Set a global value (mirror_mode, primary_edid_hash). Wire payload
/// in the SHM is one GlobalConfigBlob.
pub const CMD_SET_GLOBAL_CONFIG: u32 = 0x700B;
pub const EVT_GLOBAL_CONFIG_OK: u32 = 0x700C;

/// Save the current live config under a named profile. evt[2] is an
/// SHM id holding a 32-byte UTF-8 profile name (null-padded).
pub const CMD_SAVE_PROFILE: u32 = 0x700D;
pub const EVT_PROFILE_SAVED: u32 = 0x700E;

/// Delete a saved profile (only removes the index entry; the per-
/// output keys remain in the registry for now). evt[2] = SHM with
/// the profile name.
pub const CMD_DELETE_PROFILE: u32 = 0x700F;
pub const EVT_PROFILE_DELETED: u32 = 0x7010;

/// Load a saved profile and re-apply the layout immediately. evt[2]
/// = SHM with the profile name. Used by display-settings to switch
/// profiles manually (e.g. user has 'home' saved but wants to test
/// the 'office' layout while still at home).
pub const CMD_LOAD_PROFILE: u32 = 0x7011;
pub const EVT_PROFILE_LOADED: u32 = 0x7012;

pub fn handle_request(evt: &[u32; 5]) -> [u32; 5] {
    match evt[0] {
        CMD_LIST_OUTPUTS => {
            let infos = display::list(16);
            [EVT_OUTPUT_COUNT, infos.len() as u32, 0, 0, 0]
        }
        CMD_REAPPLY_LAYOUT => {
            // Re-derive and apply. We use the same code path the boot
            // sequence does — see main::apply_persisted_layout.
            // Duplicated logic here is acceptable for a 30-line helper;
            // factoring it out is cosmetic until display.conf parsing
            // lands and the helper grows.
            let infos = display::list(16);
            let mut entries: anyos_std::Vec<display::LayoutEntry> =
                anyos_std::Vec::with_capacity(infos.len());
            let mut next_x: i32 = 0;
            let mut primary_assigned = false;
            for info in &infos {
                if !info.is_connected() {
                    continue;
                }
                let w = if info.current_w > 0 {
                    info.current_w
                } else {
                    info.preferred_w
                };
                let h = if info.current_h > 0 {
                    info.current_h
                } else {
                    info.preferred_h
                };
                if w == 0 || h == 0 {
                    continue;
                }
                let entry = if !primary_assigned {
                    primary_assigned = true;
                    display::LayoutEntry::primary(info.id, next_x, 0, w, h)
                } else {
                    display::LayoutEntry::secondary(info.id, next_x, 0, w, h)
                };
                entries.push(entry);
                next_x += w as i32;
            }
            let r = display::set_layout(&entries);
            [EVT_LAYOUT_CHANGED, r, entries.len() as u32, 0, 0]
        }
        CMD_PROBE_HOTPLUG => {
            // Drain whatever the kernel has queued; the main loop will
            // run apply_persisted_layout on a HotplugChanged event.
            // Just acks here.
            [EVT_HOTPLUG_DONE, 0, 0, 0, 0]
        }
        CMD_SET_OUTPUT_CONFIG => {
            // SHM payload = OutputConfigBlob (#[repr(C)], 96 bytes):
            //   u64  edid_hash       (0)
            //   u32  enabled         (8)
            //   u32  orientation     (12)
            //   u32  mode_w          (16)
            //   u32  mode_h          (20)
            //   u32  mode_refresh_mhz(24)
            //   u32  scale_percent   (28)
            //   u32  fractional_scale(32) // bool as u32
            //   i32  virtual_x       (36)
            //   i32  virtual_y       (40)
            //   u64  mirror_of_hash  (44, 0 = none)
            //   u8[44] friendly_name (52..96, null-padded)
            let shm_id = evt[2];
            let addr = ipc::shm_map(shm_id);
            if addr == 0 {
                return [EVT_OUTPUT_CONFIG_OK, u32::MAX, 0, 0, 0];
            }
            let r = unsafe { write_output_config(addr as u64) };
            ipc::shm_unmap(shm_id);
            // Persisted; now re-apply so the kernel sees the change.
            crate::apply_persisted_layout();
            [EVT_OUTPUT_CONFIG_OK, r, 0, 0, 0]
        }
        CMD_SET_GLOBAL_CONFIG => {
            // SHM payload = GlobalConfigBlob (#[repr(C)], 32 bytes):
            //   u32 mirror_mode (0)        // bool as u32
            //   u32 _reserved   (4)
            //   u64 primary_edid_hash (8)  // 0 = no preference
            //   u8[16] _reserved2 (16..32)
            let shm_id = evt[2];
            let addr = ipc::shm_map(shm_id);
            if addr == 0 {
                return [EVT_GLOBAL_CONFIG_OK, u32::MAX, 0, 0, 0];
            }
            let r = unsafe { write_global_config(addr as u64) };
            ipc::shm_unmap(shm_id);
            crate::apply_persisted_layout();
            [EVT_GLOBAL_CONFIG_OK, r, 0, 0, 0]
        }
        CMD_SAVE_PROFILE => {
            let name = unsafe { read_profile_name(evt[2]) };
            let ok = if name.is_empty() {
                false
            } else {
                crate::save_current_as_profile(&name)
            };
            [EVT_PROFILE_SAVED, if ok { 0 } else { u32::MAX }, 0, 0, 0]
        }
        CMD_DELETE_PROFILE => {
            let name = unsafe { read_profile_name(evt[2]) };
            let ok = if name.is_empty() {
                false
            } else {
                crate::delete_profile(&name)
            };
            [EVT_PROFILE_DELETED, if ok { 0 } else { u32::MAX }, 0, 0, 0]
        }
        CMD_LOAD_PROFILE => {
            let name = unsafe { read_profile_name(evt[2]) };
            let ok = if name.is_empty() {
                false
            } else {
                crate::load_profile_by_name(&name)
            };
            crate::apply_persisted_layout();
            [EVT_PROFILE_LOADED, if ok { 0 } else { u32::MAX }, 0, 0, 0]
        }
        CMD_PUSH_LAYOUT => {
            // Pushed by libdisplay_client::push_layout (in particular
            // vdagent forwarding a SPICE VD_AGENT_MONITORS_CONFIG).
            // evt[2] = SHM id with [LayoutEntry; entry_count] payload
            // evt[3] = entry_count
            let shm_id = evt[2];
            let n = evt[3] as usize;
            if n == 0 || n > 32 {
                return [EVT_LAYOUT_PUSHED, u32::MAX, 0, 0, 0];
            }
            let addr = ipc::shm_map(shm_id);
            if addr == 0 {
                return [EVT_LAYOUT_PUSHED, u32::MAX, 0, 0, 0];
            }
            // SAFETY: client wrote `n * size_of::<LayoutEntry>()` bytes
            // into shm before signalling us; the kernel SHM allocator
            // ensures the mapping covers it.
            let entries = unsafe {
                core::slice::from_raw_parts(addr as *const display::LayoutEntry, n)
            };
            let mut owned: anyos_std::Vec<display::LayoutEntry> =
                anyos_std::Vec::with_capacity(n);
            owned.extend_from_slice(entries);
            ipc::shm_unmap(shm_id);
            let r = display::set_layout(&owned);
            [EVT_LAYOUT_PUSHED, r, n as u32, 0, 0]
        }
        _ => [0, 0, 0, 0, 0],
    }
}

/// Read the SHM-resident OutputConfigBlob (see CMD_SET_OUTPUT_CONFIG)
/// and persist its fields under `services/displayd/config/output/<edid>/...`.
/// Returns 0 on success, non-zero on bad payload.
unsafe fn write_output_config(addr: u64) -> u32 {
    use crate::schema::{edid_hex, output_key, DISPLAYD_SCHEMA};
    let bytes = core::slice::from_raw_parts(addr as *const u8, 96);
    let read_u64 = |off: usize| -> u64 {
        u64::from_le_bytes([
            bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3],
            bytes[off + 4], bytes[off + 5], bytes[off + 6], bytes[off + 7],
        ])
    };
    let read_u32 = |off: usize| -> u32 {
        u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]])
    };
    let read_i32 = |off: usize| -> i32 { read_u32(off) as i32 };

    let edid_hash = read_u64(0);
    if edid_hash == 0 {
        return 1;
    }
    let hex = edid_hex(edid_hash);
    let _ = DISPLAYD_SCHEMA.write_bool(&output_key(&hex, "enabled"), read_u32(8) != 0);
    let _ = DISPLAYD_SCHEMA.write_i64(&output_key(&hex, "orientation"), read_u32(12) as i64);
    let _ = DISPLAYD_SCHEMA.write_i64(&output_key(&hex, "mode_w"), read_u32(16) as i64);
    let _ = DISPLAYD_SCHEMA.write_i64(&output_key(&hex, "mode_h"), read_u32(20) as i64);
    let _ = DISPLAYD_SCHEMA.write_i64(
        &output_key(&hex, "mode_refresh_mhz"),
        read_u32(24) as i64,
    );
    let _ = DISPLAYD_SCHEMA.write_i64(
        &output_key(&hex, "scale_percent"),
        read_u32(28) as i64,
    );
    let _ = DISPLAYD_SCHEMA.write_bool(
        &output_key(&hex, "fractional_scale"),
        read_u32(32) != 0,
    );
    let _ = DISPLAYD_SCHEMA.write_i64(&output_key(&hex, "virtual_x"), read_i32(36) as i64);
    let _ = DISPLAYD_SCHEMA.write_i64(&output_key(&hex, "virtual_y"), read_i32(40) as i64);
    let mirror_hash = read_u64(44);
    let mirror_str = if mirror_hash == 0 {
        anyos_std::String::new()
    } else {
        edid_hex(mirror_hash)
    };
    let _ = DISPLAYD_SCHEMA.write_string(&output_key(&hex, "mirror_of"), &mirror_str);

    // Friendly name: 44 bytes null-padded ASCII at offset 52.
    let name_end = (52..96)
        .find(|&i| bytes[i] == 0)
        .unwrap_or(96);
    if name_end > 52 {
        if let Ok(s) = core::str::from_utf8(&bytes[52..name_end]) {
            let _ = DISPLAYD_SCHEMA.write_string(&output_key(&hex, "friendly_name"), s);
        }
    }
    0
}

/// SHM-resident profile name: 32 bytes UTF-8, null-padded. Used by
/// CMD_{SAVE,DELETE,LOAD}_PROFILE.
unsafe fn read_profile_name(shm_id: u32) -> alloc::string::String {
    if shm_id == 0 {
        return alloc::string::String::new();
    }
    let addr = ipc::shm_map(shm_id);
    if addr == 0 {
        return alloc::string::String::new();
    }
    let bytes = core::slice::from_raw_parts(addr as *const u8, 32);
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(32);
    let s = core::str::from_utf8(&bytes[..end])
        .unwrap_or("")
        .trim()
        .into();
    ipc::shm_unmap(shm_id);
    s
}

/// Read a GlobalConfigBlob and persist `mirror_mode` and the
/// canonical primary EDID hex string.
unsafe fn write_global_config(addr: u64) -> u32 {
    use crate::schema::{edid_hex, DISPLAYD_SCHEMA};
    let bytes = core::slice::from_raw_parts(addr as *const u8, 32);
    let mirror = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) != 0;
    let primary_hash = u64::from_le_bytes([
        bytes[8], bytes[9], bytes[10], bytes[11],
        bytes[12], bytes[13], bytes[14], bytes[15],
    ]);
    let _ = DISPLAYD_SCHEMA.write_bool("config/global/mirror_mode", mirror);
    let primary_str = if primary_hash == 0 {
        anyos_std::String::new()
    } else {
        edid_hex(primary_hash)
    };
    let _ = DISPLAYD_SCHEMA.write_string("config/global/primary_edid_hash", &primary_str);
    0
}
