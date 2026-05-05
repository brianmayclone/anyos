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
use schema::{
    compute_setup_hash, edid_hex, output_key, setup_key, setup_output_key, DISPLAYD_SCHEMA,
};

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

    // Compute the canonical setup hash from the connected monitors
    // and load the saved layout for that combination (or seed a fresh
    // one if this exact set has never been seen). Same set of
    // monitors at home and at the office produces the same hash and
    // therefore the same layout, automatically.
    activate_current_setup();

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
        //
        // Critical: don't call apply_persisted_layout from
        // LayoutApplied handlers. apply_persisted_layout itself
        // emits SYS_DISPLAY_SET_LAYOUT which produces a fresh
        // LayoutApplied event the next poll round — feeding back
        // through here would be an infinite write loop and crashes
        // confd within seconds. Only HotplugChanged means "the
        // physical situation changed, re-evaluate the setup".
        loop {
            let ev = display::poll_event();
            match ev {
                display::DisplayEvent::None => break,
                display::DisplayEvent::HotplugChanged => {
                    println!("[displayd] hotplug — re-evaluating setup hash");
                    activate_current_setup();
                    apply_persisted_layout();
                    ipc::evt_chan_emit(chan, &[protocol::EVT_LAYOUT_CHANGED, 0, 0, 0, 0]);
                }
                display::DisplayEvent::PreferredModeChanged { output } => {
                    println!(
                        "[displayd] preferred mode changed for output {}",
                        output
                    );
                    // Mode-change is host-driven (vdagent monitors-config
                    // resize, EDID refresh) — re-derive layout once.
                    apply_persisted_layout();
                    ipc::evt_chan_emit(chan, &[protocol::EVT_LAYOUT_CHANGED, 0, 0, 0, 0]);
                }
                display::DisplayEvent::LayoutApplied => {
                    // Confirmation that *some* layout was applied
                    // (could be ours, could be the compositor's
                    // boot-time setup). Notify subscribers so the
                    // GUI can refresh, but DO NOT re-apply — that
                    // would be an infinite loop.
                    ipc::evt_chan_emit(chan, &[protocol::EVT_LAYOUT_CHANGED, 0, 0, 0, 0]);
                }
            }
        }
    }
}

// ── Auto-keyed monitor setups ───────────────────────────────────────
//
// The display layout is keyed by a hash of the currently connected
// EDID set. Same set of monitors → same hash → same layout, plug-and-
// play. There is no manual "save profile" step: any change made
// through CMD_SET_OUTPUT_CONFIG is written under the active setup
// hash, so re-connecting that exact monitor combination later
// restores the layout automatically.
//
// A different combination (a third monitor plugged in, the dock
// removed) produces a different hash and therefore lives in its own
// slot — the previous setup's layout stays intact.

const SETUP_OUTPUT_KEYS: &[&str] = &[
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

/// EDID hashes of currently connected outputs, in stable order.
fn connected_edid_hashes() -> alloc::vec::Vec<u64> {
    display::list(16)
        .into_iter()
        .filter(|i| i.is_connected() && i.edid_hash != 0)
        .map(|i| i.edid_hash)
        .collect()
}

/// Setup hash for the current connected set. Returns "" when no
/// monitors are connected (boot-without-displays guard).
fn current_setup_hash() -> alloc::string::String {
    compute_setup_hash(&connected_edid_hashes())
}

/// Activate the layout for the current EDID set. If a saved entry
/// for this hash exists, copy its per-output values onto the live
/// `config/output/<edid>/*` keys; otherwise seed a fresh entry from
/// whatever's already in the live config (effectively a snapshot of
/// the kernel-derived defaults). Updates `config/active_setup`.
pub(crate) fn activate_current_setup() {
    let connected = connected_edid_hashes();
    if connected.is_empty() {
        return;
    }
    let hash = compute_setup_hash(&connected);
    println!(
        "[displayd] active setup: {} ({} monitor(s))",
        hash,
        connected.len()
    );
    let _ = DISPLAYD_SCHEMA.write_string("config/active_setup", &hash);

    // Ensure the EDID list is recorded — used for verification and as
    // a stable index when iterating saved setups.
    let edid_list: alloc::string::String = connected
        .iter()
        .map(|&h| edid_hex(h))
        .collect::<alloc::vec::Vec<_>>()
        .join(",");
    let _ = DISPLAYD_SCHEMA.write_string(&setup_key(&hash, "edids"), &edid_list);

    // Determine direction: load saved values if the setup has any
    // mode_w entries; otherwise seed it from the live config.
    let probe = SETUP_OUTPUT_KEYS.iter().any(|k| {
        connected
            .iter()
            .any(|h| {
                DISPLAYD_SCHEMA
                    .read_i64(&setup_output_key(&hash, &edid_hex(*h), k))
                    .is_some()
                    || DISPLAYD_SCHEMA
                        .read_bool(&setup_output_key(&hash, &edid_hex(*h), k))
                        .is_some()
                    || DISPLAYD_SCHEMA
                        .read_string(&setup_output_key(&hash, &edid_hex(*h), k))
                        .is_some()
            })
    });

    for h in &connected {
        let hex = edid_hex(*h);
        for k in SETUP_OUTPUT_KEYS {
            let setup_path = setup_output_key(&hash, &hex, k);
            let live_path = output_key(&hex, k);
            if probe {
                // Saved setup → load it onto the live keys.
                if let Some(v) = DISPLAYD_SCHEMA.read_i64(&setup_path) {
                    let _ = DISPLAYD_SCHEMA.write_i64(&live_path, v);
                } else if let Some(v) = DISPLAYD_SCHEMA.read_bool(&setup_path) {
                    let _ = DISPLAYD_SCHEMA.write_bool(&live_path, v);
                } else if let Some(v) = DISPLAYD_SCHEMA.read_string(&setup_path) {
                    let _ = DISPLAYD_SCHEMA.write_string(&live_path, &v);
                }
            } else {
                // Fresh setup → snapshot the live value into the
                // setup so the next layout edit has somewhere to
                // accumulate.
                if let Some(v) = DISPLAYD_SCHEMA.read_i64(&live_path) {
                    let _ = DISPLAYD_SCHEMA.write_i64(&setup_path, v);
                } else if let Some(v) = DISPLAYD_SCHEMA.read_bool(&live_path) {
                    let _ = DISPLAYD_SCHEMA.write_bool(&setup_path, v);
                } else if let Some(v) = DISPLAYD_SCHEMA.read_string(&live_path) {
                    let _ = DISPLAYD_SCHEMA.write_string(&setup_path, &v);
                }
            }
        }
    }
}

/// Persist a single OutputConfig under both the live keys (so the
/// next apply_persisted_layout picks it up) and the active setup's
/// per-output keys (so re-plugging the same set later restores the
/// change).
pub(crate) fn write_output_to_live_and_active_setup(
    edid_hash: u64,
    enabled: bool,
    orientation: i64,
    mode_w: i64,
    mode_h: i64,
    refresh_mhz: i64,
    scale_percent: i64,
    fractional: bool,
    virtual_x: i64,
    virtual_y: i64,
    mirror_of: &str,
    friendly_name: &str,
) {
    let hex = edid_hex(edid_hash);
    let live = |k: &str| output_key(&hex, k);
    let _ = DISPLAYD_SCHEMA.write_bool(&live("enabled"), enabled);
    let _ = DISPLAYD_SCHEMA.write_i64(&live("orientation"), orientation);
    let _ = DISPLAYD_SCHEMA.write_i64(&live("mode_w"), mode_w);
    let _ = DISPLAYD_SCHEMA.write_i64(&live("mode_h"), mode_h);
    let _ = DISPLAYD_SCHEMA.write_i64(&live("mode_refresh_mhz"), refresh_mhz);
    let _ = DISPLAYD_SCHEMA.write_i64(&live("scale_percent"), scale_percent);
    let _ = DISPLAYD_SCHEMA.write_bool(&live("fractional_scale"), fractional);
    let _ = DISPLAYD_SCHEMA.write_i64(&live("virtual_x"), virtual_x);
    let _ = DISPLAYD_SCHEMA.write_i64(&live("virtual_y"), virtual_y);
    let _ = DISPLAYD_SCHEMA.write_string(&live("mirror_of"), mirror_of);
    let _ = DISPLAYD_SCHEMA.write_string(&live("friendly_name"), friendly_name);

    // Mirror to the active setup so re-plugging the same combination
    // restores this exact value.
    let setup = DISPLAYD_SCHEMA
        .read_string("config/active_setup")
        .unwrap_or_default();
    if !setup.is_empty() {
        let s = |k: &str| setup_output_key(&setup, &hex, k);
        let _ = DISPLAYD_SCHEMA.write_bool(&s("enabled"), enabled);
        let _ = DISPLAYD_SCHEMA.write_i64(&s("orientation"), orientation);
        let _ = DISPLAYD_SCHEMA.write_i64(&s("mode_w"), mode_w);
        let _ = DISPLAYD_SCHEMA.write_i64(&s("mode_h"), mode_h);
        let _ = DISPLAYD_SCHEMA.write_i64(&s("mode_refresh_mhz"), refresh_mhz);
        let _ = DISPLAYD_SCHEMA.write_i64(&s("scale_percent"), scale_percent);
        let _ = DISPLAYD_SCHEMA.write_bool(&s("fractional_scale"), fractional);
        let _ = DISPLAYD_SCHEMA.write_i64(&s("virtual_x"), virtual_x);
        let _ = DISPLAYD_SCHEMA.write_i64(&s("virtual_y"), virtual_y);
        let _ = DISPLAYD_SCHEMA.write_string(&s("mirror_of"), mirror_of);
        let _ = DISPLAYD_SCHEMA.write_string(&s("friendly_name"), friendly_name);
    }
}

/// Set a friendly name ("home", "office", …) for the *current* setup
/// hash. Lives at `config/setups/<hash>/friendly_name`. Optional:
/// purely cosmetic, the GUI shows it as a label in the title bar.
pub(crate) fn set_active_setup_name(name: &str) -> bool {
    let hash = current_setup_hash();
    if hash.is_empty() {
        return false;
    }
    let _ = DISPLAYD_SCHEMA.write_string(&setup_key(&hash, "friendly_name"), name);
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

// Common display modes used as a safe fallback for mirror groups. The
// kernel still performs exact validation against each output's mode list.
const DISPLAY_COMMON_MODES: &[(u32, u32)] = &[
    (640, 480),
    (800, 600),
    (1024, 768),
    (1152, 864),
    (1280, 720),
    (1280, 1024),
    (1440, 900),
    (1600, 900),
    (1600, 1200),
    (1920, 1080),
];

fn is_portrait_orientation(orientation: i64) -> bool {
    orientation == 1 || orientation == 3
}

fn output_cap(info: &display::DisplayInfo) -> (u32, u32) {
    if info.preferred_w > 0 && info.preferred_h > 0 {
        (info.preferred_w, info.preferred_h)
    } else if info.current_w > 0 && info.current_h > 0 {
        (info.current_w, info.current_h)
    } else {
        (u32::MAX, u32::MAX)
    }
}

fn mode_fits_output(info: &display::DisplayInfo, w: u32, h: u32) -> bool {
    let (cap_w, cap_h) = output_cap(info);
    w <= cap_w && h <= cap_h
}

fn choose_global_mirror_mode(
    infos: &[display::DisplayInfo],
    preferred_w: u32,
    preferred_h: u32,
) -> (u32, u32) {
    if infos
        .iter()
        .filter(|info| info.is_connected())
        .all(|info| mode_fits_output(info, preferred_w, preferred_h))
    {
        return (preferred_w, preferred_h);
    }

    let mut cap_w = u32::MAX;
    let mut cap_h = u32::MAX;
    for info in infos.iter().filter(|info| info.is_connected()) {
        let (w, h) = output_cap(info);
        cap_w = cap_w.min(w);
        cap_h = cap_h.min(h);
    }

    let mut best = None;
    for &(w, h) in DISPLAY_COMMON_MODES {
        if w <= cap_w && h <= cap_h {
            best = Some((w, h));
        }
    }
    best.unwrap_or((cap_w.min(1024), cap_h.min(768)))
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
    let mut next_x: i32;

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
        let mut w = DISPLAYD_SCHEMA
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
        let mut h = DISPLAYD_SCHEMA
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
        let orientation = DISPLAYD_SCHEMA
            .read_i64(&output_key(&hex, "orientation"))
            .unwrap_or(0);
        if is_portrait_orientation(orientation) {
            core::mem::swap(&mut w, &mut h);
        }
        let _ = frac;
        Some((w, h, refresh, scale, true))
    };

    if mirror_mode {
        // Mirror: every connected output mirrors the primary. Source
        // owns its own framebuffer; everyone else points to it via
        // mirror_of.
        let primary = &infos[primary_idx];
        let (mut pw, mut ph, prefresh, pscale, _) = match resolve(primary) {
            Some(v) => v,
            None => return,
        };
        (pw, ph) = choose_global_mirror_mode(&infos, pw, ph);
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
            // output. Resolve to that output's id. Mirror entries must
            // use the source mode; the kernel validator rejects mode
            // mismatches because a mirror shares one framebuffer resource.
            let mirror_of_hex = DISPLAYD_SCHEMA
                .read_string(&output_key(&hex, "mirror_of"))
                .unwrap_or_default();
            let mut mirror_target_id = None;
            let mut mode_w = w;
            let mut mode_h = h;
            let mut mode_refresh = refresh;
            let mut mode_scale = scale;
            if !mirror_of_hex.is_empty() {
                if let Some(target) = infos
                    .iter()
                    .find(|o| o.is_connected() && edid_hex(o.edid_hash) == mirror_of_hex)
                {
                    if let Some((tw, th, tr, ts, _)) = resolve(target) {
                        mirror_target_id = Some(target.id);
                        mode_w = tw;
                        mode_h = th;
                        mode_refresh = tr;
                        mode_scale = ts;
                    }
                }
            }
            let mut e = LayoutEntry::secondary(info.id, vx, vy, mode_w, mode_h);
            e.mode_refresh_mhz = mode_refresh;
            e.scale = mode_scale as u32;
            if let Some(target_id) = mirror_target_id {
                e.mirror_of = target_id;
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
