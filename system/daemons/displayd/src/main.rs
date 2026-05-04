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
use schema::{edid_hex, output_key, profile_key, profile_output_key, DISPLAYD_SCHEMA};

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

    // Auto-pick a saved monitor profile if one matches the currently
    // connected EDID set (home / office / mobile use cases). When a
    // matching profile is found, its per-output config is copied over
    // the live config/output/<edid>/* keys before the layout is built.
    // Falls through silently when no profile matches.
    auto_select_profile();

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
                    println!("[displayd] hotplug — re-evaluating profile");
                    auto_select_profile();
                    apply_persisted_layout();
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

// ── Monitor profiles ────────────────────────────────────────────────
//
// A profile is a named set of EDID hashes plus a per-output config
// snapshot. confd stores them under `config/profiles/<name>/...`. At
// boot and on every hot-plug event displayd computes the current
// connected-EDID set and looks for a profile whose stored EDID set
// matches; the first match wins. When found, the profile's per-output
// values are copied over the live `config/output/<edid>/...` keys so
// the regular apply_persisted_layout path picks them up without
// further special-casing.
//
// The list of profile names lives in confd as a comma-separated
// string at `config/profile_names`. We keep the list explicit (rather
// than doing a wildcard listing) because the libconf_schema API today
// is fetch-by-key — no enumeration. Saving / deleting a profile
// updates the index and the keys in lock-step.

/// Comma-separated list of all profile names known to this displayd.
fn list_profile_names() -> alloc::vec::Vec<alloc::string::String> {
    let raw = DISPLAYD_SCHEMA
        .read_string("config/profile_names")
        .unwrap_or_default();
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.into())
        .collect()
}

fn save_profile_index(names: &[alloc::string::String]) {
    let joined: alloc::string::String =
        names.iter().enumerate().fold(alloc::string::String::new(), |mut acc, (i, n)| {
            if i > 0 {
                acc.push(',');
            }
            acc.push_str(n);
            acc
        });
    let _ = DISPLAYD_SCHEMA.write_string("config/profile_names", &joined);
}

/// Sort + concatenate EDID hex strings into a canonical key for set
/// comparison. (Set, not list — order-independent.)
fn canonical_edid_set(hashes: &[u64]) -> alloc::string::String {
    let mut hex: alloc::vec::Vec<alloc::string::String> =
        hashes.iter().map(|&h| edid_hex(h)).collect();
    hex.sort();
    hex.join(",")
}

/// Detect which (if any) saved profile matches the current set of
/// connected outputs. Exact match preferred; falls back to "the
/// connected set is a superset of the profile's set" so a docked
/// laptop with extra ad-hoc monitors still picks up the closest
/// known profile.
fn detect_matching_profile() -> Option<alloc::string::String> {
    let infos = display::list(16);
    let mut connected: alloc::vec::Vec<u64> = infos
        .iter()
        .filter(|i| i.is_connected() && i.edid_hash != 0)
        .map(|i| i.edid_hash)
        .collect();
    if connected.is_empty() {
        return None;
    }
    connected.sort();
    let connected_canon = canonical_edid_set(&connected);

    // First pass: look for exact match.
    let mut best_match: Option<(alloc::string::String, usize)> = None;
    for name in list_profile_names() {
        let edids = DISPLAYD_SCHEMA
            .read_string(&profile_key(&name, "edids"))
            .unwrap_or_default();
        if edids.is_empty() {
            continue;
        }
        let mut profile_set: alloc::vec::Vec<&str> = edids
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        profile_set.sort();
        let profile_canon = profile_set.join(",");

        if profile_canon == connected_canon {
            return Some(name); // exact match — done
        }
        // Subset check: every profile EDID must be in the connected
        // set. Score = profile size (more matched outputs win).
        let connected_set: alloc::vec::Vec<alloc::string::String> =
            connected.iter().map(|&h| edid_hex(h)).collect();
        let is_subset = profile_set
            .iter()
            .all(|p| connected_set.iter().any(|c| c == p));
        if is_subset {
            let score = profile_set.len();
            match &best_match {
                Some((_, s)) if *s >= score => {}
                _ => best_match = Some((name, score)),
            }
        }
    }
    best_match.map(|(n, _)| n)
}

/// Auto-pick the matching profile (if any) and copy its per-output
/// values over the live `config/output/<edid>/*` keys so the regular
/// apply path uses them.
fn auto_select_profile() {
    let Some(profile) = detect_matching_profile() else {
        return;
    };
    println!("[displayd] activating monitor profile: {}", profile);
    let active = DISPLAYD_SCHEMA
        .read_string("config/active_profile")
        .unwrap_or_default();
    let _ = DISPLAYD_SCHEMA.write_string("config/active_profile", &profile);
    if active == profile {
        // Already active — no need to copy values again.
        return;
    }
    let edids = DISPLAYD_SCHEMA
        .read_string(&profile_key(&profile, "edids"))
        .unwrap_or_default();
    for hex in edids.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        for key in PROFILE_OUTPUT_KEYS {
            let from = profile_output_key(&profile, hex, key);
            let to = output_key(hex, key);
            // Type-aware copy.
            if let Some(v) = DISPLAYD_SCHEMA.read_i64(&from) {
                let _ = DISPLAYD_SCHEMA.write_i64(&to, v);
            } else if let Some(v) = DISPLAYD_SCHEMA.read_bool(&from) {
                let _ = DISPLAYD_SCHEMA.write_bool(&to, v);
            } else if let Some(v) = DISPLAYD_SCHEMA.read_string(&from) {
                let _ = DISPLAYD_SCHEMA.write_string(&to, &v);
            }
        }
    }
}

const PROFILE_OUTPUT_KEYS: &[&str] = &[
    "enabled",
    "orientation",
    "mode_w",
    "mode_h",
    "mode_refresh_mhz",
    "scale_percent",
    "fractional_scale",
    "virtual_x",
    "virtual_y",
    "mirror_of",
    "friendly_name",
];

/// Save the current live config as a named profile. Used by the
/// CMD_SAVE_PROFILE IPC. Captures every connected output's EDID hash
/// plus all per-output keys; profile becomes the active one on
/// the next layout apply.
pub(crate) fn save_current_as_profile(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let infos = display::list(16);
    let connected: alloc::vec::Vec<u64> = infos
        .iter()
        .filter(|i| i.is_connected() && i.edid_hash != 0)
        .map(|i| i.edid_hash)
        .collect();
    if connected.is_empty() {
        return false;
    }
    let edid_list: alloc::string::String = connected
        .iter()
        .map(|&h| edid_hex(h))
        .collect::<alloc::vec::Vec<_>>()
        .join(",");
    let _ = DISPLAYD_SCHEMA.write_string(&profile_key(name, "edids"), &edid_list);
    for hash in &connected {
        let hex = edid_hex(*hash);
        for key in PROFILE_OUTPUT_KEYS {
            let from = output_key(&hex, key);
            let to = profile_output_key(name, &hex, key);
            if let Some(v) = DISPLAYD_SCHEMA.read_i64(&from) {
                let _ = DISPLAYD_SCHEMA.write_i64(&to, v);
            } else if let Some(v) = DISPLAYD_SCHEMA.read_bool(&from) {
                let _ = DISPLAYD_SCHEMA.write_bool(&to, v);
            } else if let Some(v) = DISPLAYD_SCHEMA.read_string(&from) {
                let _ = DISPLAYD_SCHEMA.write_string(&to, &v);
            }
        }
    }
    // Update the index.
    let mut names = list_profile_names();
    if !names.iter().any(|n| n == name) {
        names.push(name.into());
        save_profile_index(&names);
    }
    let _ = DISPLAYD_SCHEMA.write_string("config/active_profile", name);
    true
}

/// Manually load a profile by name (CMD_LOAD_PROFILE). Sets it as
/// active and copies its values over the live config. The caller is
/// expected to follow up with apply_persisted_layout() — the IPC
/// handler does that automatically.
pub(crate) fn load_profile_by_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let names = list_profile_names();
    if !names.iter().any(|n| n == name) {
        return false;
    }
    // Force the copy regardless of current active_profile.
    let _ = DISPLAYD_SCHEMA.write_string("config/active_profile", "");
    let _ = DISPLAYD_SCHEMA.write_string("config/active_profile_pending", name);
    auto_select_profile_named(name);
    true
}

/// Internal helper used by load_profile_by_name. Skips the EDID-set
/// matching and just copies the named profile's values.
fn auto_select_profile_named(profile: &str) {
    let _ = DISPLAYD_SCHEMA.write_string("config/active_profile", profile);
    let edids = DISPLAYD_SCHEMA
        .read_string(&profile_key(profile, "edids"))
        .unwrap_or_default();
    for hex in edids.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        for key in PROFILE_OUTPUT_KEYS {
            let from = profile_output_key(profile, hex, key);
            let to = output_key(hex, key);
            if let Some(v) = DISPLAYD_SCHEMA.read_i64(&from) {
                let _ = DISPLAYD_SCHEMA.write_i64(&to, v);
            } else if let Some(v) = DISPLAYD_SCHEMA.read_bool(&from) {
                let _ = DISPLAYD_SCHEMA.write_bool(&to, v);
            } else if let Some(v) = DISPLAYD_SCHEMA.read_string(&from) {
                let _ = DISPLAYD_SCHEMA.write_string(&to, &v);
            }
        }
    }
}

/// Delete a profile (its per-output keys are not actively cleared —
/// the registry has no delete API today — but the index entry is
/// removed so detect_matching_profile can no longer find it).
pub(crate) fn delete_profile(name: &str) -> bool {
    let mut names = list_profile_names();
    let before = names.len();
    names.retain(|n| n != name);
    if names.len() == before {
        return false;
    }
    save_profile_index(&names);
    true
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
