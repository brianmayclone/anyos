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
mod schema;

use anyos_std::{display, ipc, println};
use schema::{edid_hex, output_key, DISPLAYD_SCHEMA};

anyos_std::entry!(main);

const CHANNEL_NAME: &str = "displayd";

/// Broadcast event so subscribers know displayd is ready.
pub const EVT_DISPLAYD_READY: u32 = 0x7000;

fn main() {
    println!("[displayd] starting");

    // Register the confd schema (keys + defaults). Idempotent — re-runs
    // simply confirm the schema version. Has to come before any read,
    // otherwise the defaults would not be visible to first-time
    // queries.
    let _ = DISPLAYD_SCHEMA.register();

    // Register as the authoritative display-layout owner. Without this
    // SYS_DISPLAY_SET_LAYOUT rejects our calls (only the compositor
    // and a registered display owner are allowed to write layouts).
    // First-caller-wins; if some other process already grabbed the
    // slot we keep running but our set_layout calls will silently
    // fail — caller can recover by killing the squatter and restarting.
    let r = display::register_owner();
    if r == 0 {
        println!("[displayd] registered as display-layout owner");
    } else {
        println!("[displayd] WARNING — display-layout owner already registered (someone else got there first)");
    }

    // Apply any /System/etc/displayd-seed.conf the image build (e.g.
    // run.sh --displays 1280x720,1920x1080) left for us. Idempotent:
    // entries land in confd keyed by EDID hash on the first boot,
    // subsequent boots skip what's already there.
    apply_seed_file();

    // First-pass layout: read confd, derive a layout for the outputs
    // the kernel currently reports as connected. Single-output setups
    // already have a sane layout active (the kernel sets up output 0
    // itself at GPU init), so this only matters for N >= 2.
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

/// Read `/System/etc/displayd-seed.conf` (if present) and write any
/// `output <id> mode <w> <h>` lines into the confd entries for the
/// matching connected output's EDID hash. The seed is build-image
/// data shipped by `run.sh --displays …`; once the corresponding
/// confd entry exists subsequent boots ignore the seed for that
/// output (so a user who later changes the resolution in the GUI
/// doesn't get overwritten on every reboot).
fn apply_seed_file() {
    use crate::schema::{edid_hex, output_key, DISPLAYD_SCHEMA};
    let path = "/System/etc/displayd-seed.conf";
    let fd = anyos_std::fs::open(path, 0);
    if fd == u32::MAX {
        return;
    }
    let mut buf = [0u8; 4096];
    let n = anyos_std::fs::read(fd, &mut buf);
    anyos_std::fs::close(fd);
    if n == 0 || n == u32::MAX {
        return;
    }
    let text = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(s) => s,
        Err(_) => return,
    };
    let infos = display::list(16);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // Format: "output <id> mode <w> <h>"
        let mut parts = line.split_ascii_whitespace();
        if parts.next() != Some("output") {
            continue;
        }
        let id_str = parts.next().unwrap_or("");
        let id: u32 = match id_str.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        if parts.next() != Some("mode") {
            continue;
        }
        let w: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let h: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        if w == 0 || h == 0 {
            continue;
        }
        let info = match infos.iter().find(|i| i.id == id && i.is_connected()) {
            Some(i) => i,
            None => continue,
        };
        if info.edid_hash == 0 {
            continue;
        }
        let hex = edid_hex(info.edid_hash);
        // Only write if not yet present, so the user's later GUI edits
        // win over the build-time seed on subsequent boots.
        if DISPLAYD_SCHEMA
            .read_i64(&output_key(&hex, "mode_w"))
            .is_none()
        {
            let _ = DISPLAYD_SCHEMA.write_i64(&output_key(&hex, "mode_w"), w as i64);
            let _ = DISPLAYD_SCHEMA.write_i64(&output_key(&hex, "mode_h"), h as i64);
            println!(
                "[displayd] seeded output {} (edid={}) mode {}x{}",
                id, hex, w, h
            );
        }
    }
}

/// Apply the layout derived from confd if any, falling back to a sane
/// default ("primary at 0,0, secondaries stacked to the right at
/// preferred mode") for outputs we have no saved entry for.
///
/// Persistence layer is now confd (`services/displayd/config/...`).
/// A fresh boot with no entries still works — the per-output queries
/// just return None and we use the kernel's reported preferred mode.
pub(crate) fn apply_persisted_layout() {
    use anyos_std::display::LayoutEntry;
    let infos = display::list(16);
    if infos.is_empty() {
        return;
    }

    // Globals.
    let mirror_mode = DISPLAYD_SCHEMA
        .read_bool("config/global/mirror_mode")
        .unwrap_or(false);
    let primary_hash_pref = DISPLAYD_SCHEMA
        .read_string("config/global/primary_edid_hash")
        .unwrap_or_default();

    // Pick the source output for mirror mode (or the first connected
    // when no preference exists). For extended mode the primary
    // determines who owns (0, 0) in the virtual desktop.
    let mut primary_idx: Option<usize> = None;
    if !primary_hash_pref.is_empty() {
        for (i, info) in infos.iter().enumerate() {
            if info.is_connected() && edid_hex(info.edid_hash) == primary_hash_pref {
                primary_idx = Some(i);
                break;
            }
        }
    }
    if primary_idx.is_none() {
        for (i, info) in infos.iter().enumerate() {
            if info.is_connected() {
                primary_idx = Some(i);
                break;
            }
        }
    }
    let Some(primary_idx) = primary_idx else {
        println!("[displayd] no connected outputs — skipping layout apply");
        return;
    };

    let mut entries: anyos_std::Vec<LayoutEntry> = anyos_std::Vec::with_capacity(infos.len());
    let mut next_x: i32 = 0;

    // Helper: read per-output config, fall back to kernel-reported values.
    let resolve = |info: &display::DisplayInfo| -> Option<(u32, u32, u32, u16, bool)> {
        let hex = edid_hex(info.edid_hash);
        // enabled?
        let enabled = DISPLAYD_SCHEMA
            .read_bool(&output_key(&hex, "enabled"))
            .unwrap_or(true);
        if !enabled {
            return None;
        }
        let w = DISPLAYD_SCHEMA
            .read_i64(&output_key(&hex, "mode_w"))
            .filter(|&v| v > 0)
            .map(|v| v as u32)
            .or_else(|| {
                if info.current_w > 0 {
                    Some(info.current_w)
                } else if info.preferred_w > 0 {
                    Some(info.preferred_w)
                } else {
                    None
                }
            })?;
        let h = DISPLAYD_SCHEMA
            .read_i64(&output_key(&hex, "mode_h"))
            .filter(|&v| v > 0)
            .map(|v| v as u32)
            .or_else(|| {
                if info.current_h > 0 {
                    Some(info.current_h)
                } else if info.preferred_h > 0 {
                    Some(info.preferred_h)
                } else {
                    None
                }
            })?;
        let refresh = DISPLAYD_SCHEMA
            .read_i64(&output_key(&hex, "mode_refresh_mhz"))
            .filter(|&v| v > 0)
            .map(|v| v as u32)
            .unwrap_or(if info.refresh_mhz > 0 {
                info.refresh_mhz
            } else {
                60_000
            });
        let scale = DISPLAYD_SCHEMA
            .read_i64(&output_key(&hex, "scale_percent"))
            .filter(|&v| (50..=400).contains(&v))
            .map(|v| v as u16)
            .unwrap_or(100);
        let frac = DISPLAYD_SCHEMA
            .read_bool(&output_key(&hex, "fractional_scale"))
            .unwrap_or(false);
        let _ = frac;
        Some((w, h, refresh, scale, true))
    };

    if mirror_mode {
        // Mirror: every connected output mirrors the primary. Source
        // owns its own framebuffer; everyone else points to it via
        // mirror_of.
        let primary = &infos[primary_idx];
        let (pw, ph, prefresh, pscale, _) = match resolve(primary) {
            Some(v) => v,
            None => return,
        };
        let mut e = LayoutEntry::primary(primary.id, 0, 0, pw, ph);
        e.mode_refresh_mhz = prefresh;
        e.scale = pscale as u32;
        entries.push(e);
        for info in &infos {
            if info.id == primary.id || !info.is_connected() {
                continue;
            }
            // Even mirrors need an entry so the kernel knows about them.
            let mut e = LayoutEntry::secondary(info.id, 0, 0, pw, ph);
            e.mode_refresh_mhz = prefresh;
            e.scale = pscale as u32;
            e.mirror_of = primary.id;
            entries.push(e);
        }
    } else {
        // Extended: primary at virtual (0, 0), other outputs stacked
        // according to either the per-output saved virtual_x/y or the
        // default attach side.
        let primary = &infos[primary_idx];
        let (pw, ph, prefresh, pscale, _) = match resolve(primary) {
            Some(v) => v,
            None => return,
        };
        let mut e = LayoutEntry::primary(primary.id, 0, 0, pw, ph);
        e.mode_refresh_mhz = prefresh;
        e.scale = pscale as u32;
        entries.push(e);
        next_x = pw as i32;

        for info in &infos {
            if info.id == primary.id || !info.is_connected() {
                continue;
            }
            let Some((w, h, refresh, scale, _)) = resolve(info) else {
                continue;
            };
            let hex = edid_hex(info.edid_hash);
            let saved_x = DISPLAYD_SCHEMA.read_i64(&output_key(&hex, "virtual_x"));
            let saved_y = DISPLAYD_SCHEMA.read_i64(&output_key(&hex, "virtual_y"));
            let (vx, vy) = match (saved_x, saved_y) {
                (Some(x), Some(y)) => (x as i32, y as i32),
                _ => {
                    // Default: stack to the right of the previous one.
                    let v = (next_x, 0);
                    next_x += w as i32;
                    v
                }
            };
            // mirror_of: an EDID hash that points at another connected
            // output. Resolve to that output's id.
            let mirror_of_hex = DISPLAYD_SCHEMA
                .read_string(&output_key(&hex, "mirror_of"))
                .unwrap_or_default();
            let mut e = LayoutEntry::secondary(info.id, vx, vy, w, h);
            e.mode_refresh_mhz = refresh;
            e.scale = scale as u32;
            if !mirror_of_hex.is_empty() {
                if let Some(target) = infos
                    .iter()
                    .find(|o| o.is_connected() && edid_hex(o.edid_hash) == mirror_of_hex)
                {
                    e.mirror_of = target.id;
                }
            }
            entries.push(e);
        }
    }

    if entries.is_empty() {
        println!("[displayd] no enabled outputs — skipping layout apply");
        return;
    }

    let r = display::set_layout(&entries);
    if r == 0 {
        println!(
            "[displayd] applied layout with {} entries (mirror={})",
            entries.len(),
            mirror_mode
        );
    } else if r == u32::MAX {
        println!("[displayd] set_layout failed (hard error)");
    } else {
        println!("[displayd] set_layout rejected (LayoutError code {})", r);
    }
}
