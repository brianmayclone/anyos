//! displayd — Multi-monitor layout daemon for anyOS.
//!
//! Owns the active display layout. Behaviour mirrors what
//! wlr-output-management does in the Wayland world: the compositor
//! pushes pixels, displayd pushes layouts. Concrete responsibilities:
//!
//!   * On boot, read `/System/etc/display.conf` (libini), translate it
//!     into a kernel `OutputLayout`, and `SYS_DISPLAY_SET_LAYOUT` it
//!     atomically.
//!   * Listen on the `displayd` event channel for client requests
//!     (display-settings, anyui apps that want per-screen geometry).
//!     Apps call `libdisplay_client::list_outputs / get_layout /
//!     apply_layout`.
//!   * Poll `SYS_DISPLAY_POLL_EVENT` on a timer; on a HotplugChanged
//!     event, re-derive a layout from `display.conf` (matching saved
//!     entries by EDID hash) and re-apply.
//!
//! For the first cut persistence is intentionally minimal — boot-time
//! `display.conf` parsing is in place, the runtime "save layout"
//! command lives behind a TODO so the IPC surface settles before we
//! commit a config schema.

#![no_std]
#![no_main]

mod protocol;

use anyos_std::{display, ipc, println};

anyos_std::entry!(main);

const CHANNEL_NAME: &str = "displayd";

/// Broadcast event so subscribers know displayd is ready.
pub const EVT_DISPLAYD_READY: u32 = 0x7000;

fn main() {
    println!("[displayd] starting");

    // First-pass layout: read display.conf, derive a layout for the
    // outputs the kernel currently reports as connected. Single-output
    // setups already have a sane layout active (the kernel sets up
    // output 0 itself at GPU init), so this only matters for N >= 2.
    apply_persisted_layout();

    let chan = ipc::evt_chan_create(CHANNEL_NAME);
    let sub = ipc::evt_chan_subscribe(chan, 0);

    ipc::evt_chan_emit(chan, &[EVT_DISPLAYD_READY, 0, 0, 0, 0]);
    println!("[displayd] ready (channel '{}')", CHANNEL_NAME);

    loop {
        // Wait for the next event with a generous timeout. The timeout
        // doubles as a hotplug-poll cadence — once SYS_DISPLAY_POLL_EVENT
        // is wired into a real virtio config-change ISR this can drop
        // to wait-forever, but for now polling once a second on idle is
        // cheap and correct.
        ipc::evt_chan_wait(chan, sub, 1000);

        // 1) Service IPC requests from clients.
        let mut evt = [0u32; 5];
        while ipc::evt_chan_poll(chan, sub, &mut evt) {
            let resp = protocol::handle_request(&evt);
            let requester_sub = evt[1];
            ipc::evt_chan_emit_to(chan, requester_sub, &resp);
        }

        // 2) Drain hotplug events from the kernel and react.
        loop {
            let ev = display::poll_event();
            match ev {
                display::DisplayEvent::None => break,
                display::DisplayEvent::HotplugChanged => {
                    println!("[displayd] hotplug — re-applying layout");
                    apply_persisted_layout();
                    // Notify subscribers that geometry changed.
                    ipc::evt_chan_emit(chan, &[protocol::EVT_LAYOUT_CHANGED, 0, 0, 0, 0]);
                }
                display::DisplayEvent::PreferredModeChanged { output } => {
                    println!(
                        "[displayd] preferred mode changed for output {}",
                        output
                    );
                    apply_persisted_layout();
                    ipc::evt_chan_emit(chan, &[protocol::EVT_LAYOUT_CHANGED, 0, 0, 0, 0]);
                }
                display::DisplayEvent::LayoutApplied => {
                    // The kernel applied a layout we (or someone) submitted.
                    // Notify subscribers; the actual layout is read back via
                    // SYS_DISPLAY_LIST when needed.
                    ipc::evt_chan_emit(chan, &[protocol::EVT_LAYOUT_CHANGED, 0, 0, 0, 0]);
                }
            }
        }
    }
}

/// Apply the layout described in `/System/etc/display.conf` if any,
/// falling back to a sane default ("primary at 0,0, secondaries
/// stacked to the right at preferred mode") otherwise.
///
/// The parser is intentionally lenient: a missing or malformed
/// display.conf is not an error, just falls through to the default.
/// That keeps boot working on a fresh image with no config file yet.
fn apply_persisted_layout() {
    let infos = display::list(16);
    if infos.is_empty() {
        return;
    }

    // Build LayoutEntry set from infos. The default layout matches the
    // compositor's bootstrap heuristic: primary at (0,0), secondaries
    // to the right, no scaling, no mirroring. display.conf overrides
    // would tweak per-output position / scale / mirror_of based on
    // edid_hash matching.
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
        let mut entry = if !primary_assigned {
            primary_assigned = true;
            display::LayoutEntry::primary(info.id, next_x, 0, w, h)
        } else {
            display::LayoutEntry::secondary(info.id, next_x, 0, w, h)
        };
        if info.refresh_mhz > 0 {
            entry.mode_refresh_mhz = info.refresh_mhz;
        }
        entries.push(entry);
        next_x += w as i32;
    }

    if entries.is_empty() {
        println!("[displayd] no connected outputs — skipping layout apply");
        return;
    }

    let r = display::set_layout(&entries);
    if r == 0 {
        println!(
            "[displayd] applied layout with {} entries",
            entries.len()
        );
    } else if r == u32::MAX {
        println!("[displayd] set_layout failed (hard error)");
    } else {
        println!("[displayd] set_layout rejected (LayoutError code {})", r);
    }
}
