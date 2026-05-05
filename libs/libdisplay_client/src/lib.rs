//! Client wrapper for talking to `displayd`.
//!
//! Apps that want to *read* the current outputs can call
//! [`anyos_std::display::list`] directly — `SYS_DISPLAY_LIST` is open
//! to every process. This crate is for the cases where displayd's
//! ownership of the layout matters:
//!
//!   * forcing a re-apply after writing `display.conf`
//!   * triggering a hotplug probe from a test
//!   * subscribing to layout-changed broadcasts
//!
//! Internal protocol matches `system/daemons/displayd/src/protocol.rs`
//! 1:1; the constants are duplicated here so libdisplay_client doesn't
//! pull displayd in as a dependency (cleaner build graph and the
//! constants are tiny).

#![no_std]

extern crate alloc;

use anyos_std::ipc;

const CHANNEL_NAME: &str = "displayd";

pub const CMD_LIST_OUTPUTS: u32 = 0x7001;
pub const EVT_OUTPUT_COUNT: u32 = 0x7002;
pub const CMD_REAPPLY_LAYOUT: u32 = 0x7003;
pub const EVT_LAYOUT_CHANGED: u32 = 0x7004;
pub const CMD_PROBE_HOTPLUG: u32 = 0x7005;
pub const EVT_HOTPLUG_DONE: u32 = 0x7006;

/// Push a host-driven layout to displayd. Used by `vdagent` when SPICE
/// sends `VD_AGENT_MONITORS_CONFIG` so we can adapt the guest geometry
/// to whatever the SPICE client requested (resize, add monitor, …).
///
/// The layout payload travels through a small SHM buffer because event
/// channel events are limited to 5*u32. evt[2] carries the SHM id,
/// evt[3] the entry count.
pub const CMD_PUSH_LAYOUT: u32 = 0x7007;
pub const EVT_LAYOUT_PUSHED: u32 = 0x7008;
pub const CMD_SET_OUTPUT_CONFIG: u32 = 0x7009;
pub const EVT_OUTPUT_CONFIG_OK: u32 = 0x700A;
pub const CMD_SET_GLOBAL_CONFIG: u32 = 0x700B;
pub const EVT_GLOBAL_CONFIG_OK: u32 = 0x700C;

/// Set a friendly name (cosmetic) for the currently-active monitor
/// setup. Layouts are auto-keyed by the connected EDID set; the name
/// is only a label the GUI shows so the user can tell setups apart
/// at a glance ("home", "office", "mobile-only").
pub const CMD_SET_SETUP_NAME: u32 = 0x700D;
pub const EVT_SETUP_NAME_OK: u32 = 0x700E;

// Backwards-compatibility aliases (apps built against the older
// profile-based API still link). Map onto the auto-keyed model:
// "save profile" really means "name the current setup",
// "load profile" / "delete profile" are no-ops in the auto-keyed
// world since the layout is determined by the EDID set.
pub const CMD_SAVE_PROFILE: u32 = CMD_SET_SETUP_NAME;
pub const EVT_PROFILE_SAVED: u32 = EVT_SETUP_NAME_OK;
pub const CMD_DELETE_PROFILE: u32 = 0x700F;
pub const EVT_PROFILE_DELETED: u32 = 0x7010;
pub const CMD_LOAD_PROFILE: u32 = 0x7011;
pub const EVT_PROFILE_LOADED: u32 = 0x7012;

/// Per-output settings as persisted in confd. Marshalled to displayd
/// via SHM (96 bytes), see CMD_SET_OUTPUT_CONFIG.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct OutputConfig {
    pub edid_hash: u64,
    pub enabled: u32,
    pub orientation: u32,
    pub mode_w: u32,
    pub mode_h: u32,
    pub mode_refresh_mhz: u32,
    pub scale_percent: u32,
    pub fractional_scale: u32,
    pub virtual_x: i32,
    pub virtual_y: i32,
    pub mirror_of_hash: u64,
    pub friendly_name: [u8; 44],
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            edid_hash: 0,
            enabled: 1,
            orientation: 0,
            mode_w: 0,
            mode_h: 0,
            mode_refresh_mhz: 60_000,
            scale_percent: 100,
            fractional_scale: 0,
            virtual_x: 0,
            virtual_y: 0,
            mirror_of_hash: 0,
            friendly_name: [0u8; 44],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct GlobalConfig {
    pub mirror_mode: u32,
    pub _reserved: u32,
    pub primary_edid_hash: u64,
    pub _reserved2: [u8; 16],
}

/// Handle returned by [`connect`]. Callers hold this for the lifetime
/// of their displayd interaction; on drop we don't auto-unsubscribe
/// (the kernel cleans up on process exit) but a `disconnect()` helper
/// is provided for long-running apps that want to release the slot.
pub struct DisplaydClient {
    chan: u32,
    sub: u32,
}

impl DisplaydClient {
    /// Subscribe to the displayd channel. Returns `None` if the
    /// channel doesn't exist yet (displayd hasn't started). Callers
    /// commonly retry after a short delay or wait on `EVT_DISPLAYD_READY`
    /// — see the bootstrap sequence in the compositor or display-settings.
    pub fn connect() -> Option<Self> {
        let chan = ipc::evt_chan_create(CHANNEL_NAME);
        if chan == u32::MAX {
            return None;
        }
        let sub = ipc::evt_chan_subscribe(chan, 0);
        if sub == u32::MAX {
            return None;
        }
        Some(Self { chan, sub })
    }

    pub fn disconnect(self) {
        ipc::evt_chan_unsubscribe(self.chan, self.sub);
    }

    /// Number of advertised outputs. Equivalent to
    /// `display::list(16).len()`; the round-trip through displayd
    /// exists so a test can use a single channel to observe both
    /// the count and any subsequent EVT_LAYOUT_CHANGED broadcasts.
    pub fn list_outputs(&self) -> Option<u32> {
        let req = [CMD_LIST_OUTPUTS, self.sub, 0, 0, 0];
        ipc::evt_chan_emit(self.chan, &req);
        self.wait_response(EVT_OUTPUT_COUNT).map(|e| e[1])
    }

    /// Force displayd to re-derive a layout from `display.conf` and
    /// atomically re-apply it. Returns `0` on success, a `LayoutError`
    /// code on validation failure, or `u32::MAX` on hard error.
    pub fn reapply_layout(&self) -> Option<u32> {
        let req = [CMD_REAPPLY_LAYOUT, self.sub, 0, 0, 0];
        ipc::evt_chan_emit(self.chan, &req);
        self.wait_response(EVT_LAYOUT_CHANGED).map(|e| e[1])
    }

    /// Force a hotplug check now (otherwise polled once a second).
    pub fn probe_hotplug(&self) -> Option<()> {
        let req = [CMD_PROBE_HOTPLUG, self.sub, 0, 0, 0];
        ipc::evt_chan_emit(self.chan, &req);
        self.wait_response(EVT_HOTPLUG_DONE).map(|_| ())
    }

    /// Push a fully-specified layout to displayd, which validates and
    /// applies it via SYS_DISPLAY_SET_LAYOUT. Returns `0` on success,
    /// a non-zero LayoutError code on validation failure, or
    /// `u32::MAX` on hard error (SHM marshalling failed, etc.).
    ///
    /// Marshalling: each `LayoutEntry` (`#[repr(C)]`, 36 bytes) is
    /// memcpy'd into a fresh SHM buffer; the SHM id and entry count
    /// travel via the event channel. displayd reads the buffer back,
    /// reconstructs the slice, and forwards to the kernel.
    pub fn push_layout(&self, entries: &[anyos_std::display::LayoutEntry]) -> Option<u32> {
        if entries.is_empty() || entries.len() > 32 {
            return Some(u32::MAX);
        }
        let bytes = entries.len() * core::mem::size_of::<anyos_std::display::LayoutEntry>();
        let shm = ipc::shm_create(bytes as u32);
        if shm == u32::MAX {
            return Some(u32::MAX);
        }
        let addr = ipc::shm_map(shm);
        if addr == 0 {
            ipc::shm_destroy(shm);
            return Some(u32::MAX);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(entries.as_ptr() as *const u8, addr as *mut u8, bytes);
        }
        let req = [CMD_PUSH_LAYOUT, self.sub, shm, entries.len() as u32, 0];
        ipc::evt_chan_emit(self.chan, &req);
        let result = self.wait_response(EVT_LAYOUT_PUSHED).map(|e| e[1]);
        ipc::shm_unmap(shm);
        ipc::shm_destroy(shm);
        result
    }

    /// Persist per-output configuration in confd and trigger a layout
    /// re-apply. Used by display-settings to commit a Resolution /
    /// Refresh / Scale / Orientation change.
    pub fn set_output_config(&self, cfg: &OutputConfig) -> Option<u32> {
        let bytes = core::mem::size_of::<OutputConfig>();
        let shm = ipc::shm_create(bytes as u32);
        if shm == u32::MAX {
            return Some(u32::MAX);
        }
        let addr = ipc::shm_map(shm);
        if addr == 0 {
            ipc::shm_destroy(shm);
            return Some(u32::MAX);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                cfg as *const OutputConfig as *const u8,
                addr as *mut u8,
                bytes,
            );
        }
        let req = [CMD_SET_OUTPUT_CONFIG, self.sub, shm, 0, 0];
        ipc::evt_chan_emit(self.chan, &req);
        let result = self.wait_response(EVT_OUTPUT_CONFIG_OK).map(|e| e[1]);
        ipc::shm_unmap(shm);
        ipc::shm_destroy(shm);
        result
    }

    /// Persist global display config (mirror mode, primary monitor)
    /// and re-apply the layout.
    pub fn set_global_config(&self, cfg: &GlobalConfig) -> Option<u32> {
        let bytes = core::mem::size_of::<GlobalConfig>();
        let shm = ipc::shm_create(bytes as u32);
        if shm == u32::MAX {
            return Some(u32::MAX);
        }
        let addr = ipc::shm_map(shm);
        if addr == 0 {
            ipc::shm_destroy(shm);
            return Some(u32::MAX);
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                cfg as *const GlobalConfig as *const u8,
                addr as *mut u8,
                bytes,
            );
        }
        let req = [CMD_SET_GLOBAL_CONFIG, self.sub, shm, 0, 0];
        ipc::evt_chan_emit(self.chan, &req);
        let result = self.wait_response(EVT_GLOBAL_CONFIG_OK).map(|e| e[1]);
        ipc::shm_unmap(shm);
        ipc::shm_destroy(shm);
        result
    }

    fn send_named_profile(&self, cmd: u32, want: u32, name: &str) -> Option<u32> {
        if name.is_empty() || name.len() > 31 {
            return Some(u32::MAX);
        }
        let shm = ipc::shm_create(32);
        if shm == u32::MAX {
            return Some(u32::MAX);
        }
        let addr = ipc::shm_map(shm);
        if addr == 0 {
            ipc::shm_destroy(shm);
            return Some(u32::MAX);
        }
        unsafe {
            // Zero, then copy name bytes — leaves null padding.
            core::ptr::write_bytes(addr as *mut u8, 0, 32);
            core::ptr::copy_nonoverlapping(name.as_ptr(), addr as *mut u8, name.len());
        }
        let req = [cmd, self.sub, shm, 0, 0];
        ipc::evt_chan_emit(self.chan, &req);
        let result = self.wait_response(want).map(|e| e[1]);
        ipc::shm_unmap(shm);
        ipc::shm_destroy(shm);
        result
    }

    /// Set a cosmetic friendly name on the currently-active setup.
    /// The layout is always saved under the auto-derived setup hash;
    /// this just adds a human-readable label like "home" or "office".
    /// Apps re-purpose the older save_profile name for backwards
    /// compatibility.
    pub fn set_setup_name(&self, name: &str) -> Option<u32> {
        self.send_named_profile(CMD_SET_SETUP_NAME, EVT_SETUP_NAME_OK, name)
    }

    /// Backwards-compat alias for set_setup_name. Older apps used
    /// this to "save the current layout under a named profile";
    /// in the auto-keyed model the layout is auto-saved on every
    /// edit, so this just attaches the supplied name as the
    /// friendly_name of the current setup hash.
    pub fn save_profile(&self, name: &str) -> Option<u32> {
        self.set_setup_name(name)
    }

    /// Backwards-compat no-op: in the auto-keyed model the active
    /// layout is determined by the connected EDID set, not chosen
    /// by name. We still issue the IPC so the daemon can re-apply
    /// the layout (handy after an external config edit).
    pub fn load_profile(&self, name: &str) -> Option<u32> {
        self.send_named_profile(CMD_LOAD_PROFILE, EVT_PROFILE_LOADED, name)
    }

    /// Backwards-compat no-op (the registry has no delete API and
    /// the setup hash is the identity). Returns success.
    pub fn delete_profile(&self, name: &str) -> Option<u32> {
        self.send_named_profile(CMD_DELETE_PROFILE, EVT_PROFILE_DELETED, name)
    }

    /// Block until the next event with a matching `evt[0]` arrives.
    /// Times out after roughly 5 seconds to avoid hanging an app on
    /// a stuck displayd.
    fn wait_response(&self, want: u32) -> Option<[u32; 5]> {
        const TIMEOUT_MS: u32 = 5_000;
        let mut elapsed = 0u32;
        loop {
            ipc::evt_chan_wait(self.chan, self.sub, 100);
            elapsed = elapsed.saturating_add(100);
            let mut evt = [0u32; 5];
            while ipc::evt_chan_poll(self.chan, self.sub, &mut evt) {
                if evt[0] == want {
                    return Some(evt);
                }
            }
            if elapsed >= TIMEOUT_MS {
                return None;
            }
        }
    }
}
