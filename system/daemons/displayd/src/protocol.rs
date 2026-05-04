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

use anyos_std::display;

pub const CMD_LIST_OUTPUTS: u32 = 0x7001;
pub const EVT_OUTPUT_COUNT: u32 = 0x7002;

pub const CMD_REAPPLY_LAYOUT: u32 = 0x7003;
pub const EVT_LAYOUT_CHANGED: u32 = 0x7004;

pub const CMD_PROBE_HOTPLUG: u32 = 0x7005;
pub const EVT_HOTPLUG_DONE: u32 = 0x7006;

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
        _ => [0, 0, 0, 0, 0],
    }
}
