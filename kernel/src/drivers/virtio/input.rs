// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! VirtIO Input PCI driver — mouse, keyboard (relative + minimal).
//!
//! Implements the virtio-input device class (VirtIO Spec §5.8) for QEMU's
//! `virtio-mouse-pci` and `virtio-keyboard-pci` devices. SPICE-Viewer routes
//! its input over these devices via QEMU's input subsystem; without a virtio
//! input driver, SPICE clipboard works (via virtio-serial) but mouse and
//! keyboard input is silently dropped because the SPICE protocol does not
//! emulate PS/2 events.
//!
//! VirtIO PCI device IDs:
//! - 0x1052 (modern, virtio-1.x) — virtio-input
//! - 0x1040 (transitional, raw class) — same device, transitional layout
//!
//! Two virtqueues:
//! - `eventq` (queue 0, device-writable): host posts virtio_input_event records
//! - `statusq` (queue 1, device-readable): guest sends LED/FF (stubbed)
//!
//! Each event is the 8-byte struct `{ u16 type; u16 code; u32 value }`. We
//! accumulate REL_X / REL_Y / button changes until EV_SYN, then push one
//! mouse event into [`crate::drivers::input::mouse::MOUSE_BUFFER`]. For
//! keyboard devices, we translate Linux keycodes to PS/2 Set-1 scancodes
//! and feed them into [`crate::drivers::input::keyboard::handle_scancode`].

use alloc::boxed::Box;

use crate::drivers::hal::{Driver, DriverError, DriverType};
use crate::drivers::pci::PciDevice;
use crate::drivers::virtio::virtqueue::VirtQueue;
use crate::drivers::virtio::{self, VirtioDevice, VIRTIO_F_VERSION_1};
use crate::memory::physical;
use crate::sync::spinlock::Spinlock;

// ── virtio-input event constants (Linux input-event-codes.h) ────────────────

const EV_SYN: u16 = 0x00;
const EV_KEY: u16 = 0x01;
const EV_REL: u16 = 0x02;
const EV_ABS: u16 = 0x03;

const REL_X: u16 = 0x00;
const REL_Y: u16 = 0x01;
const REL_WHEEL: u16 = 0x08;

const BTN_LEFT: u16 = 0x110;
const BTN_RIGHT: u16 = 0x111;
const BTN_MIDDLE: u16 = 0x112;

// ── Queue indices ───────────────────────────────────────────────────────────

const EVENT_Q: u16 = 0;
const STATUS_Q: u16 = 1;

// ── Buffer pool ─────────────────────────────────────────────────────────────

/// Number of event buffers we keep posted on the eventq. Generous so a burst
/// of mouse motion + key chatter never starves the device.
const EVENT_SLOTS: usize = 64;

/// Size of one virtio_input_event in bytes.
const EVENT_SIZE: usize = 8;

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct VirtioInputEvent {
    type_: u16,
    code: u16,
    value: u32,
}

// ── Device kind ─────────────────────────────────────────────────────────────

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
enum InputKind {
    Mouse,    // relative pointing
    Keyboard, // EV_KEY
    Tablet,   // absolute pointing — limited support
    Unknown,
}

// ── Per-device bus state ────────────────────────────────────────────────────

struct InputBus {
    vdev: VirtioDevice,
    eventq: VirtQueue,
    /// One physically-contiguous backing region split into EVENT_SLOTS
    /// 8-byte buffers. Storing the base avoids per-slot allocations.
    event_buf_phys: u64,
    /// Per-slot in-flight tracking: descriptor head id when the slot is
    /// currently owned by the device. None when free for re-posting.
    slot_in_flight: [Option<u16>; EVENT_SLOTS],
    kind: InputKind,
    /// Mouse accumulators flushed on EV_SYN.
    acc_dx: i32,
    acc_dy: i32,
    acc_dz: i32,
    /// Latest button state (bit 0 = left, 1 = right, 2 = middle). Updated on
    /// EV_KEY events for BTN_LEFT/RIGHT/MIDDLE; flushed on EV_SYN together
    /// with motion to produce a single mouse event per frame.
    cur_buttons: u8,
    prev_buttons: u8,
}

unsafe impl Send for InputBus {}

/// Up to MAX_INPUT_DEVS virtio-input devices may be probed (e.g. one mouse +
/// one keyboard). Each gets its own bus.
const MAX_INPUT_DEVS: usize = 4;
static BUSES: [Spinlock<Option<InputBus>>; MAX_INPUT_DEVS] = [
    Spinlock::new(None),
    Spinlock::new(None),
    Spinlock::new(None),
    Spinlock::new(None),
];

/// ISR addresses indexed parallel to BUSES, used by the IRQ handler. Plain
/// AtomicU64 because the IRQ handler must read this without locking.
static ISR_ADDRS: [core::sync::atomic::AtomicU64; MAX_INPUT_DEVS] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
];

/// Counter that caps keyboard-event diagnostic logging at the first N events
/// per boot to confirm the EV_KEY → PS/2 path works without flooding serial.
static KBD_LOG_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

// ── Eventq buffer management ────────────────────────────────────────────────

impl InputBus {
    fn slot_phys(&self, slot: usize) -> u64 {
        self.event_buf_phys + (slot as u64) * (EVENT_SIZE as u64)
    }

    /// Refill the eventq so every free slot has a descriptor posted.
    fn post_event_buffers(&mut self) {
        let mut posted_any = false;
        for slot in 0..EVENT_SLOTS {
            if self.slot_in_flight[slot].is_some() {
                continue;
            }
            let phys = self.slot_phys(slot);
            // Zero the event slot before re-posting so stale data doesn't
            // confuse the parser if the device returns a zero-length write.
            unsafe {
                core::ptr::write_bytes(phys as *mut u8, 0, EVENT_SIZE);
            }
            let writable = [(phys, EVENT_SIZE as u32)];
            match self.eventq.push(&[], &writable) {
                Some(id) => {
                    self.slot_in_flight[slot] = Some(id);
                    posted_any = true;
                }
                None => break, // queue full
            }
        }
        if posted_any {
            self.vdev.notify_queue(EVENT_Q);
        }
    }

    /// Drain completed events, parse them, and re-post buffers.
    fn poll_events(&mut self) {
        while let Some((id, len)) = self.eventq.poll_used() {
            // Find which slot this descriptor belongs to.
            let mut slot_found = None;
            for slot in 0..EVENT_SLOTS {
                if self.slot_in_flight[slot] == Some(id) {
                    slot_found = Some(slot);
                    break;
                }
            }
            let slot = match slot_found {
                Some(s) => s,
                None => continue,
            };
            self.slot_in_flight[slot] = None;

            if (len as usize) >= EVENT_SIZE {
                let phys = self.slot_phys(slot);
                let ev = unsafe { core::ptr::read_volatile(phys as *const VirtioInputEvent) };
                self.handle_event(ev);
            }
        }
        self.post_event_buffers();
    }

    fn handle_event(&mut self, ev: VirtioInputEvent) {
        match ev.type_ {
            EV_SYN => {
                // Frame boundary — flush accumulated state.
                self.flush_frame();
            }
            EV_REL => match ev.code {
                REL_X => self.acc_dx = self.acc_dx.wrapping_add(ev.value as i32),
                REL_Y => self.acc_dy = self.acc_dy.wrapping_add(ev.value as i32),
                REL_WHEEL => {
                    // Linux REL_WHEEL: positive = up. anyOS dz: +1 = down.
                    self.acc_dz = self.acc_dz.wrapping_sub(ev.value as i32);
                }
                _ => {}
            },
            EV_ABS => {
                // TODO: tablet absolute positioning. Would require reading the
                // device's ABS_INFO (min/max) via VIRTIO_INPUT_CFG_ABS_INFO and
                // scaling to screen dims via vmmouse::get_screen_size_pub().
                // Stubbed for now — relative mode is enough for SPICE.
            }
            EV_KEY => {
                let pressed = ev.value != 0;
                match ev.code {
                    BTN_LEFT => {
                        self.cur_buttons =
                            (self.cur_buttons & !0x01) | if pressed { 0x01 } else { 0 };
                    }
                    BTN_RIGHT => {
                        self.cur_buttons =
                            (self.cur_buttons & !0x02) | if pressed { 0x02 } else { 0 };
                    }
                    BTN_MIDDLE => {
                        self.cur_buttons =
                            (self.cur_buttons & !0x04) | if pressed { 0x04 } else { 0 };
                    }
                    code => {
                        // Treat as keyboard key. Only meaningful when the
                        // device is actually a keyboard; mouse devices won't
                        // emit non-BTN_* keycodes.
                        if self.kind == InputKind::Keyboard
                            || self.kind == InputKind::Unknown
                        {
                            self.deliver_keyboard(code, pressed);
                        } else {
                            crate::serial_println!(
                                "[virtio-input] kbd EV_KEY code={} dropped (kind={:?})",
                                code,
                                self.kind
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn flush_frame(&mut self) {
        use crate::drivers::input::mouse::{
            MouseButtons, MouseEvent, MouseEventType, MOUSE_BUFFER,
        };

        // Keyboard devices don't accumulate mouse state — nothing to flush.
        if self.kind == InputKind::Keyboard {
            return;
        }

        let dx = core::mem::replace(&mut self.acc_dx, 0);
        let dy = core::mem::replace(&mut self.acc_dy, 0);
        let dz = core::mem::replace(&mut self.acc_dz, 0);
        let buttons_changed = self.cur_buttons != self.prev_buttons;
        let press_mask = self.cur_buttons & !self.prev_buttons;

        // Skip empty SYN frames.
        if dx == 0 && dy == 0 && dz == 0 && !buttons_changed {
            return;
        }

        let new_buttons = MouseButtons {
            left: self.cur_buttons & 0x01 != 0,
            right: self.cur_buttons & 0x02 != 0,
            middle: self.cur_buttons & 0x04 != 0,
        };

        // Boot-splash cursor (lag-free); only relative motion makes sense.
        if dx != 0 || dy != 0 {
            crate::drivers::gpu::splash_cursor_move(dx, dy);
        }

        let mut buf = MOUSE_BUFFER.lock();
        if dz != 0 && buf.len() < 256 {
            buf.push_back(MouseEvent {
                dx,
                dy,
                dz,
                buttons: new_buttons,
                event_type: MouseEventType::Scroll,
            });
        }
        if buttons_changed && buf.len() < 256 {
            let event_type = if press_mask != 0 {
                MouseEventType::ButtonDown
            } else {
                MouseEventType::ButtonUp
            };
            buf.push_back(MouseEvent {
                dx,
                dy,
                dz: 0,
                buttons: new_buttons,
                event_type,
            });
        } else if (dx != 0 || dy != 0) && dz == 0 && buf.len() < 256 {
            buf.push_back(MouseEvent {
                dx,
                dy,
                dz: 0,
                buttons: new_buttons,
                event_type: MouseEventType::Move,
            });
        }
        drop(buf);

        self.prev_buttons = self.cur_buttons;
        crate::syscall::handlers::wake_compositor_if_blocked();
    }

    fn deliver_keyboard(&mut self, linux_code: u16, pressed: bool) {
        // Translate Linux keycode → PS/2 Set-1 scancode (with optional E0
        // prefix for extended keys). Unknown keys are dropped.
        let translated = linux_to_ps2(linux_code);
        // Log only the first 16 events (per device) to confirm the path
        // works without flooding serial.
        let count = KBD_LOG_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if count < 16 {
            crate::serial_println!(
                "[virtio-input] kbd EV_KEY linux={} pressed={} → ps2={:?}",
                linux_code,
                pressed,
                translated
            );
        }
        let (ps2_code, is_e0) = match translated {
            Some(pair) => pair,
            None => return,
        };

        if is_e0 {
            crate::drivers::input::keyboard::handle_scancode(0xE0);
        }
        // PS/2 break codes set bit 7.
        let scancode = if pressed { ps2_code } else { ps2_code | 0x80 };
        crate::drivers::input::keyboard::handle_scancode(scancode);
    }
}

// ── Linux keycode → PS/2 Set 1 translation ──────────────────────────────────
//
// Coverage: alphanumerics, function keys, navigation, modifiers, common
// keypad and OS keys. Sufficient for typical text/GUI use. Multi-media and
// browser keys (KEY_PLAY, KEY_BACK, etc.) are intentionally omitted — they
// can be added on demand.
//
// `is_e0 == true` means we should send 0xE0 then the scancode (extended).
fn linux_to_ps2(code: u16) -> Option<(u8, bool)> {
    // Linux codes are defined in include/uapi/linux/input-event-codes.h.
    let mapped: (u8, bool) = match code {
        // KEY_ESC = 1
        1 => (0x01, false),
        // KEY_1..KEY_0 = 2..11
        2 => (0x02, false),
        3 => (0x03, false),
        4 => (0x04, false),
        5 => (0x05, false),
        6 => (0x06, false),
        7 => (0x07, false),
        8 => (0x08, false),
        9 => (0x09, false),
        10 => (0x0A, false),
        11 => (0x0B, false),
        // KEY_MINUS, KEY_EQUAL = 12,13
        12 => (0x0C, false),
        13 => (0x0D, false),
        // KEY_BACKSPACE = 14, KEY_TAB = 15
        14 => (0x0E, false),
        15 => (0x0F, false),
        // KEY_Q..KEY_P = 16..25
        16 => (0x10, false),
        17 => (0x11, false),
        18 => (0x12, false),
        19 => (0x13, false),
        20 => (0x14, false),
        21 => (0x15, false),
        22 => (0x16, false),
        23 => (0x17, false),
        24 => (0x18, false),
        25 => (0x19, false),
        // KEY_LEFTBRACE = 26, KEY_RIGHTBRACE = 27
        26 => (0x1A, false),
        27 => (0x1B, false),
        // KEY_ENTER = 28
        28 => (0x1C, false),
        // KEY_LEFTCTRL = 29
        29 => (0x1D, false),
        // KEY_A..KEY_L = 30..38
        30 => (0x1E, false),
        31 => (0x1F, false),
        32 => (0x20, false),
        33 => (0x21, false),
        34 => (0x22, false),
        35 => (0x23, false),
        36 => (0x24, false),
        37 => (0x25, false),
        38 => (0x26, false),
        // KEY_SEMICOLON = 39, KEY_APOSTROPHE = 40, KEY_GRAVE = 41
        39 => (0x27, false),
        40 => (0x28, false),
        41 => (0x29, false),
        // KEY_LEFTSHIFT = 42, KEY_BACKSLASH = 43
        42 => (0x2A, false),
        43 => (0x2B, false),
        // KEY_Z..KEY_M = 44..50
        44 => (0x2C, false),
        45 => (0x2D, false),
        46 => (0x2E, false),
        47 => (0x2F, false),
        48 => (0x30, false),
        49 => (0x31, false),
        50 => (0x32, false),
        // KEY_COMMA = 51, KEY_DOT = 52, KEY_SLASH = 53
        51 => (0x33, false),
        52 => (0x34, false),
        53 => (0x35, false),
        // KEY_RIGHTSHIFT = 54
        54 => (0x36, false),
        // KEY_KPASTERISK = 55
        55 => (0x37, false),
        // KEY_LEFTALT = 56
        56 => (0x38, false),
        // KEY_SPACE = 57
        57 => (0x39, false),
        // KEY_CAPSLOCK = 58
        58 => (0x3A, false),
        // KEY_F1..F10 = 59..68
        59 => (0x3B, false),
        60 => (0x3C, false),
        61 => (0x3D, false),
        62 => (0x3E, false),
        63 => (0x3F, false),
        64 => (0x40, false),
        65 => (0x41, false),
        66 => (0x42, false),
        67 => (0x43, false),
        68 => (0x44, false),
        // KEY_NUMLOCK = 69, KEY_SCROLLLOCK = 70
        69 => (0x45, false),
        70 => (0x46, false),
        // Keypad 7..9, minus, 4..6, plus, 1..3, 0, dot
        71 => (0x47, false),
        72 => (0x48, false),
        73 => (0x49, false),
        74 => (0x4A, false),
        75 => (0x4B, false),
        76 => (0x4C, false),
        77 => (0x4D, false),
        78 => (0x4E, false),
        79 => (0x4F, false),
        80 => (0x50, false),
        81 => (0x51, false),
        82 => (0x52, false),
        83 => (0x53, false),
        // KEY_F11=87, KEY_F12=88
        87 => (0x57, false),
        88 => (0x58, false),
        // ── Extended (E0) ──
        // KEY_KPENTER = 96
        96 => (0x1C, true),
        // KEY_RIGHTCTRL = 97
        97 => (0x1D, true),
        // KEY_KPSLASH = 98
        98 => (0x35, true),
        // KEY_RIGHTALT = 100  (AltGr)
        100 => (0x38, true),
        // KEY_HOME = 102
        102 => (0x47, true),
        // KEY_UP = 103, KEY_PAGEUP = 104
        103 => (0x48, true),
        104 => (0x49, true),
        // KEY_LEFT = 105, KEY_RIGHT = 106
        105 => (0x4B, true),
        106 => (0x4D, true),
        // KEY_END = 107, KEY_DOWN = 108, KEY_PAGEDOWN = 109
        107 => (0x4F, true),
        108 => (0x50, true),
        109 => (0x51, true),
        // KEY_INSERT = 110, KEY_DELETE = 111
        110 => (0x52, true),
        111 => (0x53, true),
        // KEY_LEFTMETA = 125, KEY_RIGHTMETA = 126 (Super / Win key)
        125 => (0x5B, true),
        126 => (0x5C, true),
        // KEY_MENU = 127
        127 => (0x5D, true),
        _ => return None,
    };
    Some(mapped)
}

// ── IRQ handler ─────────────────────────────────────────────────────────────

fn input_irq_handler(_irq: u8) {
    // Multiple virtio-input devices may share the same line, so we sweep all
    // installed buses and dispatch only those whose ISR shows pending work.
    // No per-fire logging here — chained virtio devices on the same legacy
    // IRQ line cause this to fire on every neighbor's interrupt, and that
    // floods serial fast enough to starve the scheduler.
    for i in 0..MAX_INPUT_DEVS {
        let isr_addr = ISR_ADDRS[i].load(core::sync::atomic::Ordering::Relaxed);
        if isr_addr == 0 {
            continue;
        }
        let isr_status = virtio::mmio_read8(isr_addr);
        if isr_status & 1 == 0 {
            continue;
        }
        if let Some(mut guard) = BUSES[i].try_lock() {
            if let Some(bus) = guard.as_mut() {
                bus.poll_events();
            }
        }
    }
}

// ── Driver shell (registered for /dev visibility) ───────────────────────────

pub struct VirtioInputDriver {
    kind: InputKind,
}

impl Driver for VirtioInputDriver {
    fn name(&self) -> &str {
        match self.kind {
            InputKind::Mouse => "VirtIO Mouse",
            InputKind::Keyboard => "VirtIO Keyboard",
            InputKind::Tablet => "VirtIO Tablet",
            InputKind::Unknown => "VirtIO Input",
        }
    }
    fn driver_type(&self) -> DriverType {
        DriverType::Char
    }
    fn init(&mut self) -> Result<(), DriverError> {
        Ok(())
    }
    fn read(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize, DriverError> {
        // Events are funnelled into the global mouse/keyboard buffers, not
        // read through this device node.
        Err(DriverError::NotSupported)
    }
    fn write(&self, _offset: usize, _buf: &[u8]) -> Result<usize, DriverError> {
        Err(DriverError::NotSupported)
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

// ── Device-class detection via virtio_input config ──────────────────────────
//
// virtio-input device config (spec §5.8.4):
//   u8 select; u8 subsel; u8 size; u8 reserved[5];
//   union { ... } u;   // 128 bytes
// Writing select/subsel and reading `size` + `u` lets us learn the device
// name, ID, and capability bitmaps. We use it to distinguish mouse / keyboard
// / tablet by checking which event types are supported.

const VIRTIO_INPUT_CFG_UNSET: u8 = 0x00;
const VIRTIO_INPUT_CFG_ID_NAME: u8 = 0x01;
const VIRTIO_INPUT_CFG_EV_BITS: u8 = 0x11;

fn read_input_cfg_bitmap(vdev: &VirtioDevice, sel: u8, subsel: u8) -> ([u8; 128], u8) {
    if vdev.device_cfg == 0 {
        return ([0u8; 128], 0);
    }
    unsafe {
        core::ptr::write_volatile((vdev.device_cfg) as *mut u8, sel);
        core::ptr::write_volatile((vdev.device_cfg + 1) as *mut u8, subsel);
        // Brief settling: spec doesn't strictly require it, but be safe.
        let _ = core::ptr::read_volatile((vdev.device_cfg + 2) as *const u8);
        let size = core::ptr::read_volatile((vdev.device_cfg + 2) as *const u8);
        let mut out = [0u8; 128];
        for i in 0..(size as usize).min(128) {
            out[i] = core::ptr::read_volatile((vdev.device_cfg + 8 + i as u64) as *const u8);
        }
        // Reset selector so it doesn't persist for future probes.
        core::ptr::write_volatile((vdev.device_cfg) as *mut u8, VIRTIO_INPUT_CFG_UNSET);
        (out, size)
    }
}

fn classify(vdev: &VirtioDevice) -> InputKind {
    if vdev.device_cfg == 0 {
        return InputKind::Unknown;
    }

    // EV_KEY supported keycodes.
    let (key_bits, key_sz) = read_input_cfg_bitmap(vdev, VIRTIO_INPUT_CFG_EV_BITS, EV_KEY as u8);
    let (rel_bits, rel_sz) = read_input_cfg_bitmap(vdev, VIRTIO_INPUT_CFG_EV_BITS, EV_REL as u8);
    let (abs_bits, abs_sz) = read_input_cfg_bitmap(vdev, VIRTIO_INPUT_CFG_EV_BITS, EV_ABS as u8);

    let has_rel = rel_sz > 0 && rel_bits.iter().any(|b| *b != 0);
    let has_abs = abs_sz > 0 && abs_bits.iter().any(|b| *b != 0);

    // Mouse buttons live in the BTN_LEFT (0x110 = 272) bit. The bitmap is
    // little-endian; bit 272 is byte 34, bit 0.
    let has_btn_left = (key_sz as usize) > 34 && (key_bits[34] & 0x01) != 0;
    let has_alpha_keys = (key_sz as usize) >= 4
        && (key_bits[1] & 0xFE != 0 || key_bits[2] != 0 || key_bits[3] != 0);

    if has_btn_left && has_rel {
        InputKind::Mouse
    } else if has_btn_left && has_abs {
        InputKind::Tablet
    } else if has_alpha_keys {
        InputKind::Keyboard
    } else {
        InputKind::Unknown
    }
}

// ── Probe ───────────────────────────────────────────────────────────────────

pub fn probe(pci: &PciDevice) -> Option<Box<dyn Driver>> {
    crate::serial_println!(
        "[virtio-input] probing PCI {:04x}:{:04x}",
        pci.vendor_id,
        pci.device_id
    );

    // Find a free slot.
    let slot = {
        let mut chosen: Option<usize> = None;
        for i in 0..MAX_INPUT_DEVS {
            if BUSES[i].lock().is_none() {
                chosen = Some(i);
                break;
            }
        }
        match chosen {
            Some(s) => s,
            None => {
                crate::serial_println!(
                    "[virtio-input] no free bus slot (max {} reached)",
                    MAX_INPUT_DEVS
                );
                return None;
            }
        }
    };

    // 1. Find VirtIO PCI capabilities.
    let caps = match virtio::find_capabilities(pci) {
        Some(c) => c,
        None => {
            crate::serial_println!(
                "[virtio-input] no modern PCI capabilities — abandoning probe"
            );
            return None;
        }
    };

    // 2. Map BARs.
    let vdev = VirtioDevice::new(pci, &caps);

    // 3. Save ISR for IRQ dispatch.
    ISR_ADDRS[slot].store(vdev.isr_addr, core::sync::atomic::Ordering::Relaxed);

    // 4. Negotiate features. virtio-input has no required feature beyond
    //    VERSION_1, and even that is OK to skip on transitional devices.
    let negotiated = match vdev.init_device(VIRTIO_F_VERSION_1) {
        Ok(f) => f,
        Err(e) => {
            crate::serial_println!("[virtio-input] feature negotiation failed: {}", e);
            ISR_ADDRS[slot].store(0, core::sync::atomic::Ordering::Relaxed);
            return None;
        }
    };
    let _ = negotiated;

    // 5. Classify device kind via EV_BITS. Must happen AFTER FEATURES_OK
    //    — config access is allowed only once the device left the "init" state.
    let kind = classify(&vdev);
    crate::serial_println!("[virtio-input] device kind: {:?}", kind);

    // 6. Set up the two queues.
    let eventq = match vdev.setup_queue(EVENT_Q) {
        Some(q) => q,
        None => {
            crate::serial_println!("[virtio-input] eventq setup failed");
            ISR_ADDRS[slot].store(0, core::sync::atomic::Ordering::Relaxed);
            return None;
        }
    };
    // statusq is required by the spec but optional in practice; set up best
    // effort. We never push to it (no LED/FF support yet) — the queue exists
    // only so the device sees it as enabled per the initialization contract.
    let _statusq = vdev.setup_queue(STATUS_Q);

    // 7. Allocate one contiguous buffer for all event slots.
    let pages_needed =
        ((EVENT_SLOTS * EVENT_SIZE) + 4095) / 4096;
    let event_buf_phys = match physical::alloc_contiguous(pages_needed.max(1)) {
        Some(p) => p.as_u64(),
        None => {
            crate::serial_println!("[virtio-input] OOM allocating event buffers");
            ISR_ADDRS[slot].store(0, core::sync::atomic::Ordering::Relaxed);
            return None;
        }
    };
    unsafe {
        core::ptr::write_bytes(event_buf_phys as *mut u8, 0, pages_needed.max(1) * 4096);
    }

    // 8. Build the bus.
    let mut bus = InputBus {
        vdev,
        eventq,
        event_buf_phys,
        slot_in_flight: [None; EVENT_SLOTS],
        kind,
        acc_dx: 0,
        acc_dy: 0,
        acc_dz: 0,
        cur_buttons: 0,
        prev_buttons: 0,
    };

    // 9. Pre-post all event buffers BEFORE setting DRIVER_OK so the device
    //    starts handing back input events as soon as it's marked ready.
    bus.post_event_buffers();

    // 10. DRIVER_OK.
    bus.vdev.set_driver_ok();

    // 11. Install the bus.
    {
        let mut guard = BUSES[slot].lock();
        *guard = Some(bus);
    }

    // 12. Wire up the IRQ. Register the handler unconditionally — multiple
    //     devices share the same handler and we sweep all slots inside it.
    let irq = pci.interrupt_line;
    crate::serial_println!("[virtio-input] IRQ = {} (slot {})", irq, slot);
    crate::arch::x86::irq::register_irq(irq, input_irq_handler);
    if crate::arch::x86::apic::is_initialized() {
        crate::arch::x86::ioapic::unmask_irq(irq);
    } else {
        crate::arch::x86::pic::unmask(irq);
    }

    // 13. Return a thin Driver shell so the PCI bus reports a registered
    //     driver. Real input flows through the shared mouse/keyboard buffers.
    Some(Box::new(VirtioInputDriver { kind }))
}
