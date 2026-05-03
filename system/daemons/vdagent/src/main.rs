// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! SPICE vdagent daemon for anyOS.
//!
//! Bridges the SPICE vdagent clipboard protocol between the QEMU host and the
//! anyOS compositor, enabling bidirectional clipboard synchronization.
//!
//! Communicates with the host via a VirtIO serial port (`/dev/vport0`) using
//! the VDAgent protocol, and with the compositor via SHM-based IPC.
//!
//! # Usage
//!
//! QEMU must be started with SPICE and virtio-serial:
//! ```
//! qemu-system-x86_64 ... \
//!   -spice port=5930,disable-ticketing=on \
//!   -device virtio-serial \
//!   -chardev spicevmc,id=vdagent,debug=0,name=vdagent \
//!   -device virtserialport,chardev=vdagent,name=com.redhat.spice.0
//! ```

#![no_std]
#![no_main]

anyos_std::entry!(main);

mod config;

use alloc::string::String;
use anyos_std::{fs, ipc, process, Vec};
use config::{LogLevel, VdAgentConfig};
use core::sync::atomic::AtomicU32;
use libsvc::ServiceLifecycle;

static MOUSE_LOG_COUNT: AtomicU32 = AtomicU32::new(0);

// ── Logging helpers ──────────────────────────────────────────────────────────

static mut CURRENT_LOG_LEVEL: LogLevel = LogLevel::Info;

fn log_level() -> LogLevel {
    unsafe { CURRENT_LOG_LEVEL }
}

fn set_log_level(lvl: LogLevel) {
    unsafe {
        CURRENT_LOG_LEVEL = lvl;
    }
}

macro_rules! vd_log {
    ($lvl:expr, $tag:expr, $($arg:tt)*) => {
        if log_level().enabled($lvl) {
            anyos_std::println!("vdagent[{}]: {}", $tag, format_args!($($arg)*));
        }
    };
}
macro_rules! vd_error { ($($a:tt)*) => { vd_log!(LogLevel::Error, "ERROR", $($a)*) } }
macro_rules! vd_warn  { ($($a:tt)*) => { vd_log!(LogLevel::Warn,  "WARN",  $($a)*) } }
macro_rules! vd_info  { ($($a:tt)*) => { vd_log!(LogLevel::Info,  "INFO",  $($a)*) } }
macro_rules! vd_debug { ($($a:tt)*) => { vd_log!(LogLevel::Debug, "DEBUG", $($a)*) } }

// ── VDAgent Protocol Constants ───────────────────────────────────────────────

/// VDAgent protocol version.
const VD_AGENT_PROTOCOL: u32 = 1;

/// VDI port numbers.
const _VDP_CLIENT_PORT: u32 = 1;
const VDP_SERVER_PORT: u32 = 2;

/// VDAgent message types.
const VD_AGENT_MOUSE_STATE: u32 = 1;
const VD_AGENT_MONITORS_CONFIG: u32 = 2;
const VD_AGENT_REPLY: u32 = 3;
const VD_AGENT_CLIPBOARD: u32 = 4;
const VD_AGENT_ANNOUNCE_CAPABILITIES: u32 = 6;
const VD_AGENT_CLIPBOARD_GRAB: u32 = 7;
const VD_AGENT_CLIPBOARD_REQUEST: u32 = 8;
const VD_AGENT_CLIPBOARD_RELEASE: u32 = 9;

/// Clipboard data types.
const VD_AGENT_CLIPBOARD_UTF8_TEXT: u32 = 1;

/// Agent capability bit indices, per upstream `spice-protocol/spice/vd_agent.h`.
/// Earlier versions of this file used wrong values (2/3) for the clipboard
/// caps, which caused QEMU's SPICE server to think we couldn't do clipboard
/// at all (bit 2 is REPLY, bit 3 is the deprecated legacy CLIPBOARD).
#[allow(dead_code)] const VD_AGENT_CAP_MOUSE_STATE: u32 = 0;
#[allow(dead_code)] const VD_AGENT_CAP_MONITORS_CONFIG: u32 = 1;
#[allow(dead_code)] const VD_AGENT_CAP_REPLY: u32 = 2;
#[allow(dead_code)] const VD_AGENT_CAP_CLIPBOARD: u32 = 3; // legacy, deprecated
#[allow(dead_code)] const VD_AGENT_CAP_DISPLAY_CONFIG: u32 = 4;
#[allow(dead_code)] const VD_AGENT_CAP_CLIPBOARD_BY_DEMAND: u32 = 5;
#[allow(dead_code)] const VD_AGENT_CAP_CLIPBOARD_SELECTION: u32 = 6;
#[allow(dead_code)] const VD_AGENT_CAP_SPARSE_MONITORS_CONFIG: u32 = 7;
#[allow(dead_code)] const VD_AGENT_CAP_GUEST_LINEEND_LF: u32 = 8;
#[allow(dead_code)] const VD_AGENT_CAP_GUEST_LINEEND_CRLF: u32 = 9;
#[allow(dead_code)] const VD_AGENT_CAP_MAX_CLIPBOARD: u32 = 10;

// QEMU's SPICE server actually serialises mouse buttons as `1 << SpiceMouseButton`,
// not `SpiceMouseButtonMask`: LEFT (enum=1) -> bit 1, MIDDLE (2) -> bit 2,
// RIGHT (3) -> bit 3, wheel-UP (4) -> bit 4, wheel-DOWN (5) -> bit 5. The
// `SpiceMouseButtonMask` typedef in spice/enums.h reads bit-for-bit RFB-like,
// but tracing real packets shows that QEMU does not use those mask values.
// The compositor's CMD_INJECT_POINTER follows the RFB convention (LEFT bit 0,
// MIDDLE bit 1, RIGHT bit 2, wheel-up bit 3, wheel-down bit 4), so we shift
// the SPICE mask right by one bit before forwarding.

// ── Compositor IPC Constants ─────────────────────────────────────────────────

const CMD_SET_CLIPBOARD: u32 = 0x1011;
const CMD_GET_CLIPBOARD: u32 = 0x1012;
const CMD_REGISTER_SUB: u32 = 0x100C;
const CMD_INJECT_POINTER: u32 = 0x1023;
const RESP_CLIPBOARD_DATA: u32 = 0x2010;
/// Hard upper bound regardless of config (sanity ceiling).
const MAX_CLIPBOARD_SIZE: usize = 4 * 1024 * 1024;

// ── Wire Format Structures ───────────────────────────────────────────────────

// VDI chunk header: [port:u32, size:u32] = 8 bytes
// VDAgent message header: [protocol:u32, type:u32, opaque:u64, size:u32] = 20 bytes

const VDI_CHUNK_HDR_SIZE: usize = 8; // port(4) + size(4)
const VD_AGENT_MSG_HDR_SIZE: usize = 20; // protocol(4) + type(4) + opaque(8) + size(4)

// ── Clipboard IPC (same as vncd/clipboard.rs) ────────────────────────────────

fn set_compositor_clipboard(comp_chan: u32, text: &[u8]) {
    if text.is_empty() || text.len() > MAX_CLIPBOARD_SIZE {
        return;
    }
    let data_len = text.len() as u32;
    let shm_id = ipc::shm_create(data_len);
    if shm_id == 0 {
        return;
    }
    let shm_addr = ipc::shm_map(shm_id);
    if shm_addr == 0 {
        ipc::shm_destroy(shm_id);
        return;
    }
    unsafe {
        core::ptr::copy_nonoverlapping(text.as_ptr(), shm_addr as *mut u8, text.len());
    }
    let cmd: [u32; 5] = [CMD_SET_CLIPBOARD, shm_id, data_len, 0, 0];
    ipc::evt_chan_emit(comp_chan, &cmd);
    process::sleep(32);
    ipc::shm_unmap(shm_id);
    ipc::shm_destroy(shm_id);
}

fn get_compositor_clipboard(comp_chan: u32, reply_chan: u32, sub_id: u32, buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }
    let cap = buf.len().min(MAX_CLIPBOARD_SIZE) as u32;
    let tid = process::getpid();
    // shm_id field is 0 — compositor provides its own persistent SHM.
    let cmd: [u32; 5] = [CMD_GET_CLIPBOARD, 0, cap, tid, 0];
    ipc::evt_chan_emit(comp_chan, &cmd);

    let mut response = [0u32; 5];
    for _ in 0..50 {
        while ipc::evt_chan_poll(reply_chan, sub_id, &mut response) {
            if response[0] == RESP_CLIPBOARD_DATA && response[4] == tid {
                let comp_shm_id = response[1];
                let result_len = response[2] as usize;
                let copy_len = result_len.min(buf.len());
                if copy_len > 0 && comp_shm_id != 0 {
                    // Map compositor-owned SHM, copy, unmap (do NOT destroy).
                    let shm_addr = ipc::shm_map(comp_shm_id);
                    if shm_addr != 0 {
                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                shm_addr as *const u8,
                                buf.as_mut_ptr(),
                                copy_len,
                            );
                        }
                        ipc::shm_unmap(comp_shm_id);
                    } else {
                        return 0;
                    }
                }
                return copy_len;
            }
        }
        process::sleep(5);
    }
    0
}

// ── VDAgent Message Construction ─────────────────────────────────────────────

/// Build a complete VDI chunk + VDAgent message with the given payload.
fn build_message(msg_type: u32, payload: &[u8]) -> Vec<u8> {
    let agent_size = VD_AGENT_MSG_HDR_SIZE + payload.len();
    let total = VDI_CHUNK_HDR_SIZE + agent_size;
    let mut buf = Vec::with_capacity(total);

    // VDI chunk header
    buf.extend_from_slice(&VDP_SERVER_PORT.to_le_bytes());
    buf.extend_from_slice(&(agent_size as u32).to_le_bytes());

    // VDAgent message header
    buf.extend_from_slice(&VD_AGENT_PROTOCOL.to_le_bytes());
    buf.extend_from_slice(&msg_type.to_le_bytes());
    buf.extend_from_slice(&0u64.to_le_bytes()); // opaque
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());

    // Payload
    buf.extend_from_slice(payload);
    buf
}

/// Send an ANNOUNCE_CAPABILITIES message to the host.
fn send_capabilities(fd: u32) {
    // Capabilities bitmap: request=1, then capability bits.
    // Bit layout: u32 request flag, then u32[] capability words.
    let mut payload = Vec::with_capacity(8);
    // request = 1 (we are announcing, not replying)
    payload.extend_from_slice(&1u32.to_le_bytes());
    // Capability word 0: clipboard + input. MOUSE_STATE / MONITORS_CONFIG must
    // be advertised before the SPICE host will route mouse events through the
    // vdagent channel — without these bits the host falls back to delivering
    // pointer events via the SPICE inputs channel (vmmouse / virtio-mouse-pci).
    // Note: deliberately NOT advertising VD_AGENT_CAP_CLIPBOARD_SELECTION.
    // QEMU's spice-server (8.2) often only offers BY_DEMAND (cap0=0x35077 in
    // tests). If we claim SELECTION, the host expects a selection-byte prefix
    // on every clipboard message (PRIMARY/CLIPBOARD) — but our send_clipboard_*
    // helpers emit the simpler BY_DEMAND format without that prefix, so QEMU
    // silently drops them. Advertising only BY_DEMAND aligns wire format.
    let cap_word: u32 = (1 << VD_AGENT_CAP_MOUSE_STATE)
        | (1 << VD_AGENT_CAP_MONITORS_CONFIG)
        | (1 << VD_AGENT_CAP_REPLY)
        | (1 << VD_AGENT_CAP_CLIPBOARD_BY_DEMAND)
        | (1 << VD_AGENT_CAP_CLIPBOARD_SELECTION)
        | (1 << VD_AGENT_CAP_MAX_CLIPBOARD);
    payload.extend_from_slice(&cap_word.to_le_bytes());

    let msg = build_message(VD_AGENT_ANNOUNCE_CAPABILITIES, &payload);
    fs::write(fd, &msg);
}

/// Send ANNOUNCE_CAPABILITIES with request=0 — used when the host's own
/// announce arrived with request=1 and is waiting for our reply.
fn send_capabilities_reply(fd: u32) {
    let mut payload = Vec::with_capacity(8);
    payload.extend_from_slice(&0u32.to_le_bytes()); // request=0 (reply)
    let cap_word: u32 = (1 << VD_AGENT_CAP_MOUSE_STATE)
        | (1 << VD_AGENT_CAP_MONITORS_CONFIG)
        | (1 << VD_AGENT_CAP_REPLY)
        | (1 << VD_AGENT_CAP_CLIPBOARD_BY_DEMAND)
        | (1 << VD_AGENT_CAP_CLIPBOARD_SELECTION)
        | (1 << VD_AGENT_CAP_MAX_CLIPBOARD);
    payload.extend_from_slice(&cap_word.to_le_bytes());
    let msg = build_message(VD_AGENT_ANNOUNCE_CAPABILITIES, &payload);
    fs::write(fd, &msg);
}

// VDAgent selection IDs (when CLIPBOARD_SELECTION cap is active).
const VD_AGENT_CLIPBOARD_SELECTION_CLIPBOARD: u8 = 0;
#[allow(dead_code)] const VD_AGENT_CLIPBOARD_SELECTION_PRIMARY: u8 = 1;
#[allow(dead_code)] const VD_AGENT_CLIPBOARD_SELECTION_SECONDARY: u8 = 2;

/// 4-byte selection prefix used when CLIPBOARD_SELECTION is negotiated:
/// [u8 selection][u8 reserved[3]].
fn selection_prefix(sel: u8) -> [u8; 4] {
    [sel, 0, 0, 0]
}

/// Send a CLIPBOARD_GRAB to tell the host we have new clipboard data.
fn send_clipboard_grab(fd: u32) {
    let mut payload = Vec::with_capacity(8);
    payload.extend_from_slice(&selection_prefix(VD_AGENT_CLIPBOARD_SELECTION_CLIPBOARD));
    payload.extend_from_slice(&VD_AGENT_CLIPBOARD_UTF8_TEXT.to_le_bytes());
    let msg = build_message(VD_AGENT_CLIPBOARD_GRAB, &payload);
    fs::write(fd, &msg);
}

/// Send clipboard data to the host.
fn send_clipboard_data(fd: u32, data: &[u8]) {
    let mut payload = Vec::with_capacity(8 + data.len());
    payload.extend_from_slice(&selection_prefix(VD_AGENT_CLIPBOARD_SELECTION_CLIPBOARD));
    payload.extend_from_slice(&VD_AGENT_CLIPBOARD_UTF8_TEXT.to_le_bytes());
    payload.extend_from_slice(data);
    let msg = build_message(VD_AGENT_CLIPBOARD, &payload);
    fs::write(fd, &msg);
}

/// Send a CLIPBOARD_RELEASE to release clipboard ownership.
#[allow(dead_code)]
fn send_clipboard_release(fd: u32) {
    let msg = build_message(VD_AGENT_CLIPBOARD_RELEASE, &[]);
    fs::write(fd, &msg);
}

/// Acknowledge a request from the host. SPICE expects a `VD_AGENT_REPLY` after
/// `VD_AGENT_MONITORS_CONFIG` and similar configuration messages so it knows
/// the agent is alive and the request was consumed.
fn send_reply(fd: u32, reply_to: u32, error: u32) {
    let mut payload = Vec::with_capacity(8);
    payload.extend_from_slice(&reply_to.to_le_bytes());
    payload.extend_from_slice(&error.to_le_bytes());
    let msg = build_message(VD_AGENT_REPLY, &payload);
    fs::write(fd, &msg);
}

/// Inject an absolute-positioned pointer event. We shift the SPICE button
/// mask right by one bit so QEMU's `1 << SpiceMouseButton` encoding lines up
/// with the compositor's RFB-style mask (LEFT bit 0, etc.). Wheel ticks are
/// edge-triggered — a bit toggle within one frame is one scroll notch.
fn inject_mouse(comp_chan: u32, x: u32, y: u32, buttons: u32) {
    let mask = (buttons >> 1) & 0x1F;
    let evt = [CMD_INJECT_POINTER, x, y, mask, 0];
    ipc::evt_chan_emit(comp_chan, &evt);
}

// ── Message Parsing ──────────────────────────────────────────────────────────

fn read_u32_le(buf: &[u8], offset: usize) -> u32 {
    if offset + 4 > buf.len() {
        return 0;
    }
    u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

fn read_u64_le(buf: &[u8], offset: usize) -> u64 {
    if offset + 8 > buf.len() {
        return 0;
    }
    u64::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
        buf[offset + 4],
        buf[offset + 5],
        buf[offset + 6],
        buf[offset + 7],
    ])
}

/// Tracks the last absolute pointer state we forwarded to the compositor so
/// we can skip redundant identical samples (SPICE sometimes streams the same
/// position multiple times when the wire latency is low).
struct PointerState {
    x: u32,
    y: u32,
    buttons: u32,
    initialized: bool,
}

/// Process a complete VDAgent message from the receive buffer.
fn handle_agent_message(
    fd: u32,
    comp_chan: u32,
    reply_chan: u32,
    sub_id: u32,
    msg_type: u32,
    payload: &[u8],
    host_has_clipboard: &mut bool,
    last_pointer: &mut PointerState,
) {
    match msg_type {
        VD_AGENT_ANNOUNCE_CAPABILITIES => {
            // payload: u32 request, then u32 cap_word_0 (and possibly more).
            // request=1 means the host wants us to reply with our own caps
            // (with request=0). If we don't reply, SPICE eventually drops us
            // with CLIENT_DISCONNECTED and reconnects in an infinite loop.
            let request = if payload.len() >= 4 {
                read_u32_le(payload, 0)
            } else {
                0
            };
            let cap0 = if payload.len() >= 8 {
                read_u32_le(payload, 4)
            } else {
                0
            };
            let has_clip_demand = (cap0 & (1 << VD_AGENT_CAP_CLIPBOARD_BY_DEMAND)) != 0;
            let has_clip_sel = (cap0 & (1 << VD_AGENT_CAP_CLIPBOARD_SELECTION)) != 0;
            vd_debug!(
                "host caps: payload={}B request={} cap0=0x{:x} by_demand={} sel={}",
                payload.len(),
                request,
                cap0,
                has_clip_demand,
                has_clip_sel
            );
            if request == 1 {
                send_capabilities_reply(fd);
                vd_debug!("sent caps reply");
            }
        }

        VD_AGENT_CLIPBOARD_GRAB => {
            // With CLIPBOARD_SELECTION negotiated, payload starts with
            // [u8 selection][u8 reserved[3]], then list of u32 types.
            *host_has_clipboard = true;
            vd_debug!("host grabbed clipboard (payload={}B)", payload.len());

            // Reply with CLIPBOARD_REQUEST (UTF8_TEXT) — same selection prefix.
            let mut req_payload = Vec::with_capacity(8);
            req_payload.extend_from_slice(&selection_prefix(
                VD_AGENT_CLIPBOARD_SELECTION_CLIPBOARD,
            ));
            req_payload.extend_from_slice(&VD_AGENT_CLIPBOARD_UTF8_TEXT.to_le_bytes());
            let msg = build_message(VD_AGENT_CLIPBOARD_REQUEST, &req_payload);
            fs::write(fd, &msg);
        }

        VD_AGENT_CLIPBOARD_REQUEST => {
            vd_debug!("host requests clipboard data");
            let mut clip_buf = [0u8; 4096];
            let clip_len = get_compositor_clipboard(comp_chan, reply_chan, sub_id, &mut clip_buf);
            if clip_len > 0 {
                send_clipboard_data(fd, &clip_buf[..clip_len]);
            } else {
                send_clipboard_data(fd, &[]);
            }
        }

        VD_AGENT_CLIPBOARD => {
            // With CLIPBOARD_SELECTION negotiated, payload starts with the
            // 4-byte selection prefix, then [u32 type][data...].
            if payload.len() >= 8 {
                let _selection = payload[0];
                let _clip_type = read_u32_le(payload, 4);
                let data = &payload[8..];
                if !data.is_empty() {
                    vd_debug!("received clipboard data ({} bytes) <- host", data.len());
                    set_compositor_clipboard(comp_chan, data);
                }
            }
            *host_has_clipboard = false;
        }

        VD_AGENT_CLIPBOARD_RELEASE => {
            *host_has_clipboard = false;
            vd_debug!("host released clipboard");
        }

        VD_AGENT_MOUSE_STATE => {
            // VDAgentMouseState: { u32 x, u32 y, u32 buttons, u8 display_id }
            if payload.len() >= 12 {
                let x = read_u32_le(payload, 0);
                let y = read_u32_le(payload, 4);
                let buttons = read_u32_le(payload, 8);

                // Dedup identical samples; emit on every position OR button
                // change. Always log button transitions at info level so the
                // path stays observable in production.
                let same = last_pointer.initialized
                    && last_pointer.x == x
                    && last_pointer.y == y
                    && last_pointer.buttons == buttons;
                if same {
                    return;
                }
                let button_change = !last_pointer.initialized
                    || last_pointer.buttons != buttons;
                if button_change {
                    vd_info!(
                        "mouse buttons 0x{:x} -> 0x{:x} at ({},{})",
                        last_pointer.buttons,
                        buttons,
                        x,
                        y
                    );
                } else {
                    let count =
                        MOUSE_LOG_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    if count < 4 {
                        vd_info!("mouse #{} x={} y={} buttons=0x{:x}", count, x, y, buttons);
                    }
                }
                last_pointer.x = x;
                last_pointer.y = y;
                last_pointer.buttons = buttons;
                last_pointer.initialized = true;
                inject_mouse(comp_chan, x, y, buttons);
            }
        }

        VD_AGENT_MONITORS_CONFIG => {
            // Host pushes the desired monitor layout. We don't dynamically
            // resize the virtio-gpu output from userspace, but the host needs
            // a REPLY (success) to know the agent saw the request — without
            // it, some SPICE clients delay further input.
            vd_debug!("host monitors_config (payload {} bytes)", payload.len());
            send_reply(fd, VD_AGENT_MONITORS_CONFIG, 0);
        }

        _ => {
            // Unknown message type — ignore.
        }
    }
}

// ── Main Entry Point ─────────────────────────────────────────────────────────

/// Try to open the configured port, with documented fallbacks.
///
/// Resolution order:
///   1. `cfg.effective_device_path()` — `/dev/virtio-ports/<port_name>`
///   2. `cfg.alias_device_path()`    — `/dev/vport-spice` etc. (well-known names)
///   3. `/dev/vport0`                 — last-resort fallback for legacy single-port
fn open_port(cfg: &VdAgentConfig) -> (u32, String) {
    let candidates: Vec<String> = {
        let mut v = Vec::new();
        v.push(cfg.effective_device_path());
        if let Some(a) = cfg.alias_device_path() {
            v.push(a);
        }
        v.push(String::from("/dev/vport0"));
        v
    };
    for path in candidates.iter() {
        let fd = fs::open(path.as_str(), 2 /* O_RDWR */);
        if fd != u32::MAX {
            return (fd, path.clone());
        }
        vd_debug!("port {} unavailable", path);
    }
    (u32::MAX, String::new())
}

fn main() {
    let mut args_buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut args_buf);
    if raw.contains("--help") {
        anyos_std::println!(
            "vdagent - SPICE clipboard agent\n\
             \n\
             Usage: vdagent\n\
             \n\
             Configuration is read from confd at services/vdagent/config.\n\
             See docs/spice-vdagent.md for details."
        );
        return;
    }

    let mut lifecycle = ServiceLifecycle::connect("vdagent").ok();
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_starting();
        let _ = lifecycle.set_health("starting");
    }

    // Register schema with confd (idempotent) so that the runtime keys
    // — port_name, log_level, clipboard_poll_ms, etc. — are visible at
    // /services/vdagent/config/* and overridable via confctl. Failure is
    // logged but non-fatal: we proceed with built-in defaults.
    let _registered = config::register_manifest();
    let cfg = config::load();
    set_log_level(cfg.log_level);

    if !cfg.enabled {
        vd_info!("disabled via config (services/vdagent/config/enabled=false) — exiting");
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.set_health("disabled");
            let _ = lifecycle.notify_ready();
        }
        return;
    }

    vd_info!(
        "starting (port_name={}, poll={}ms, idle={}ms)",
        cfg.port_name, cfg.clipboard_poll_ms, cfg.idle_sleep_ms
    );

    // Open the VirtIO serial port (named, alias, or fallback).
    let (fd, opened_path) = open_port(&cfg);
    if fd == u32::MAX {
        vd_error!(
            "no virtio-serial port found (tried /dev/virtio-ports/{}, alias, /dev/vport0)",
            cfg.port_name
        );
        if let Some(lifecycle) = lifecycle.as_mut() {
            let _ = lifecycle.notify_failed("virtio_serial_missing");
        }
        return;
    }
    vd_info!("opened {}", opened_path);

    // Connect to compositor event channel.
    let comp_chan = ipc::evt_chan_create("compositor");
    let reply_chan = ipc::evt_chan_create("vdagent.reply");
    let sub_id = ipc::evt_chan_subscribe(reply_chan, 0);
    let tid = process::getpid();
    ipc::evt_chan_emit(comp_chan, &[CMD_REGISTER_SUB, tid, sub_id, reply_chan, 0]);
    vd_debug!("connected to compositor");

    // Announce our capabilities to the host.
    send_capabilities(fd);
    vd_debug!("announced capabilities");
    if let Some(lifecycle) = lifecycle.as_mut() {
        let _ = lifecycle.notify_ready();
        let _ = lifecycle.set_health("ready");
    }

    // State
    let mut host_has_clipboard = false;
    let mut last_pointer = PointerState {
        x: 0,
        y: 0,
        buttons: 0,
        initialized: false,
    };
    // Pre-load the compositor's existing clipboard so we don't immediately
    // fire a GRAB on the first poll. If we did, spice-gtk would mark the
    // guest as the clipboard owner before any user action, and host->guest
    // sync would be blocked until the agent observed an explicit RELEASE.
    let mut last_clipboard: Vec<u8> = Vec::new();
    {
        let mut buf = [0u8; 4096];
        let len =
            get_compositor_clipboard(comp_chan, reply_chan, sub_id, &mut buf) as usize;
        if len > 0 && len <= buf.len() {
            last_clipboard.extend_from_slice(&buf[..len]);
            vd_debug!("startup baseline clipboard: {} bytes", len);
        }
    }
    let mut last_clipboard_check: u32 = 0;

    // Reassembly buffer for partial VDI chunk reads.
    let mut reassembly_buf: Vec<u8> = Vec::new();

    loop {
        // ── Read from virtio-serial (non-blocking) ───────────────────────
        let mut read_buf = [0u8; 4096];
        let n = fs::read(fd, &mut read_buf);
        if n > 0 && n != u32::MAX {
            let n = n as usize;
            // Wire dump (debug-only): first 32 bytes of every inbound read.
            if matches!(unsafe { CURRENT_LOG_LEVEL }, LogLevel::Debug | LogLevel::Trace) {
                let dump_len = n.min(32);
                let mut hex = String::new();
                for b in &read_buf[..dump_len] {
                    let _ = core::fmt::write(&mut hex, format_args!("{:02x} ", b));
                }
                vd_debug!("wire IN: {}B {}{}", n, hex, if n > dump_len { "..." } else { "" });
            }
            reassembly_buf.extend_from_slice(&read_buf[..n]);

            // Process complete messages from the reassembly buffer.
            loop {
                if reassembly_buf.len() < VDI_CHUNK_HDR_SIZE {
                    break; // Need more data for chunk header.
                }

                let _chunk_port = read_u32_le(&reassembly_buf, 0);
                let chunk_size = read_u32_le(&reassembly_buf, 4) as usize;

                let total_needed = VDI_CHUNK_HDR_SIZE + chunk_size;
                if reassembly_buf.len() < total_needed {
                    break; // Need more data for full chunk.
                }

                // We have a complete chunk. Parse the VDAgent message inside.
                let chunk_data = &reassembly_buf[VDI_CHUNK_HDR_SIZE..total_needed];

                if chunk_size >= VD_AGENT_MSG_HDR_SIZE {
                    let _protocol = read_u32_le(chunk_data, 0);
                    let msg_type = read_u32_le(chunk_data, 4);
                    let _opaque = read_u64_le(chunk_data, 8);
                    let msg_size = read_u32_le(chunk_data, 16) as usize;

                    let payload_start = VD_AGENT_MSG_HDR_SIZE;
                    let payload_end =
                        payload_start + msg_size.min(chunk_size - VD_AGENT_MSG_HDR_SIZE);
                    let payload = &chunk_data[payload_start..payload_end];

                    handle_agent_message(
                        fd,
                        comp_chan,
                        reply_chan,
                        sub_id,
                        msg_type,
                        payload,
                        &mut host_has_clipboard,
                        &mut last_pointer,
                    );
                }

                // Remove processed chunk from reassembly buffer.
                let remaining = reassembly_buf[total_needed..].to_vec();
                reassembly_buf = remaining;
            }
        }

        // ── Poll compositor clipboard for changes → notify host ──────────
        let now = anyos_std::sys::uptime_ms();
        if now.wrapping_sub(last_clipboard_check) >= cfg.clipboard_poll_ms {
            last_clipboard_check = now;

            let mut clip_buf = [0u8; 4096];
            let clip_len = get_compositor_clipboard(comp_chan, reply_chan, sub_id, &mut clip_buf);
            if clip_len > 0 {
                let clip_data = &clip_buf[..clip_len];
                if clip_data != last_clipboard.as_slice() {
                    last_clipboard.clear();
                    last_clipboard.extend_from_slice(clip_data);
                    vd_debug!("local clipboard changed ({} bytes) -> host", clip_data.len());
                    send_clipboard_grab(fd);
                }
            }
        }

        // Sleep briefly to avoid busy-spinning.
        process::sleep(cfg.idle_sleep_ms);
    }
}
