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
    pub fn push_layout(
        &self,
        entries: &[anyos_std::display::LayoutEntry],
    ) -> Option<u32> {
        if entries.is_empty() || entries.len() > 32 {
            return Some(u32::MAX);
        }
        let bytes = entries.len()
            * core::mem::size_of::<anyos_std::display::LayoutEntry>();
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
                entries.as_ptr() as *const u8,
                addr as *mut u8,
                bytes,
            );
        }
        let req = [CMD_PUSH_LAYOUT, self.sub, shm, entries.len() as u32, 0];
        ipc::evt_chan_emit(self.chan, &req);
        let result = self.wait_response(EVT_LAYOUT_PUSHED).map(|e| e[1]);
        ipc::shm_unmap(shm);
        ipc::shm_destroy(shm);
        result
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
