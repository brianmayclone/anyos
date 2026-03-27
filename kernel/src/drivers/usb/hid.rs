//! USB HID (Human Interface Device) class driver.
//!
//! Supports boot protocol keyboards (protocol=1), mice (protocol=2), and
//! tablets/absolute pointers (protocol=0, e.g. QEMU usb-tablet).
//!
//! Data is read via **interrupt IN transfers** on the device's interrupt endpoint,
//! matching how real USB HID hardware and QEMU deliver input data.
//! (GET_REPORT control transfers are not used — QEMU does not update them.)
//!
//! Boot protocol keyboard report: 8 bytes [modifier, reserved, key1..key6]
//! Boot protocol mouse report: 3-4 bytes [buttons, dx, dy, (dz)]
//! Tablet report (QEMU): 6 bytes [buttons, x_lo, x_hi, y_lo, y_hi, scroll]

use super::{ControllerType, SetupPacket, UsbDevice, UsbInterface, UsbSpeed};
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

// ── HID Device Registry ─────────────────────────

/// HID device type — determines report format.
#[derive(Debug, Clone, Copy, PartialEq)]
enum HidType {
    Keyboard,   // boot protocol=1, 8-byte reports
    Mouse,      // boot protocol=2, 3-4 byte relative reports
    Tablet,     // protocol=0 (QEMU usb-tablet), 6-byte absolute reports
}

struct HidDevice {
    address: u8,
    controller: ControllerType,
    speed: UsbSpeed,
    hid_type: HidType,
    int_endpoint: u8,     // interrupt IN endpoint address (e.g. 0x81)
    int_max_packet: u16,  // max packet size for interrupt endpoint
    data_toggle: u8,      // DATA0/DATA1 toggle for interrupt transfers
    prev_keys: [u8; 6],   // keyboard only — tracks pressed keys for delta detection
}

static HID_DEVICES: Spinlock<Vec<HidDevice>> = Spinlock::new(Vec::new());

/// Called when a HID interface is detected during USB enumeration.
/// Accepts boot protocol keyboards/mice (subclass=1) and tablets (subclass=0).
pub fn probe(dev: &UsbDevice, iface: &UsbInterface) {
    // Find interrupt IN endpoint — required for all HID devices
    let int_ep = match iface.endpoints.iter().find(|ep| {
        (ep.attributes & 0x03) == 3 && (ep.address & 0x80) != 0
    }) {
        Some(ep) => ep,
        None => {
            crate::serial_verbose_println!("  USB HID: no interrupt IN endpoint, skipping");
            return;
        }
    };

    // Determine device type
    let hid_type = match (iface.subclass, iface.protocol) {
        (1, 1) => HidType::Keyboard,
        (1, 2) => HidType::Mouse,
        (_, 0) => HidType::Tablet,   // usb-tablet: subclass=0, protocol=0
        _ => {
            crate::serial_verbose_println!("  USB HID: unknown sub={} proto={}, skipping",
                iface.subclass, iface.protocol);
            return;
        }
    };

    let type_name = match hid_type {
        HidType::Keyboard => "Keyboard",
        HidType::Mouse => "Mouse",
        HidType::Tablet => "Tablet",
    };

    // Set boot protocol for boot-interface devices (keyboard/mouse)
    if iface.subclass == 1 {
        let set_protocol = SetupPacket {
            bm_request_type: 0x21,
            b_request: 0x0B,  // SET_PROTOCOL
            w_value: 0,       // 0 = Boot Protocol
            w_index: iface.number as u16,
            w_length: 0,
        };
        let _ = super::hid_control_transfer(
            dev.address, dev.controller, dev.speed, dev.max_packet_size,
            &set_protocol, false, 0,
        );
    }

    // Set idle rate to 0 (report only on change) — reduces USB traffic
    let set_idle = SetupPacket {
        bm_request_type: 0x21,
        b_request: 0x0A,  // SET_IDLE
        w_value: 0,        // duration=0 (indefinite), report_id=0
        w_index: iface.number as u16,
        w_length: 0,
    };
    let _ = super::hid_control_transfer(
        dev.address, dev.controller, dev.speed, dev.max_packet_size,
        &set_idle, false, 0,
    );

    let hid_dev = HidDevice {
        address: dev.address,
        controller: dev.controller,
        speed: dev.speed,
        hid_type,
        int_endpoint: int_ep.address,
        int_max_packet: int_ep.max_packet_size,
        data_toggle: 0,
        prev_keys: [0u8; 6],
    };

    HID_DEVICES.lock().push(hid_dev);
    crate::serial_println!(
        "[USB-HID] registered {} (addr={}, ep={:#04x}, {:?})",
        type_name, dev.address, int_ep.address, dev.controller,
    );
}

/// Per-device debug counters (first N events logged).
struct PollDebug {
    ok_count: u32,
    nak_count: u32,
    err_count: u32,
    logged: bool,
}

static POLL_DEBUG: Spinlock<[PollDebug; 4]> = Spinlock::new([
    PollDebug { ok_count: 0, nak_count: 0, err_count: 0, logged: false },
    PollDebug { ok_count: 0, nak_count: 0, err_count: 0, logged: false },
    PollDebug { ok_count: 0, nak_count: 0, err_count: 0, logged: false },
    PollDebug { ok_count: 0, nak_count: 0, err_count: 0, logged: false },
]);

/// Poll all registered HID devices via interrupt IN transfers.
/// Called from the USB poll thread at ~10ms intervals.
pub fn poll_all() {
    let mut devs = HID_DEVICES.lock();
    for (idx, dev) in devs.iter_mut().enumerate() {
        let mut buf = [0u8; 16];
        let max = (dev.int_max_packet as usize).min(buf.len());

        match super::interrupt_in_transfer(
            dev.address, dev.controller, dev.speed,
            dev.int_endpoint, dev.int_max_packet,
            &mut dev.data_toggle, &mut buf[..max],
        ) {
            Ok(n) if n >= 3 => {
                // Debug: log first successful data for each device
                if idx < 4 {
                    let mut dbg = POLL_DEBUG.lock();
                    dbg[idx].ok_count += 1;
                    if dbg[idx].ok_count <= 2 {
                        crate::serial_println!("[USB-HID] dev {} ({:?}) data: n={} {:02x?}",
                            dev.address, dev.hid_type, n, &buf[..n.min(8)]);
                    }
                }
                match dev.hid_type {
                    HidType::Keyboard if n >= 8 => {
                        let mut report = [0u8; 8];
                        report.copy_from_slice(&buf[..8]);
                        parse_keyboard_report(&report, &mut dev.prev_keys);
                    }
                    HidType::Mouse => {
                        parse_mouse_report(&buf[..n], n);
                    }
                    HidType::Tablet if n >= 5 => {
                        parse_tablet_report(&buf[..n], n);
                    }
                    _ => {}
                }
            }
            Ok(_) => {
                // NAK or short — log summary after 500 polls (~5s)
                if idx < 4 {
                    let mut dbg = POLL_DEBUG.lock();
                    dbg[idx].nak_count += 1;
                    if dbg[idx].nak_count == 500 && !dbg[idx].logged {
                        dbg[idx].logged = true;
                        crate::serial_println!("[USB-HID] dev {} ({:?}) 500 NAKs, {} ok, {} err",
                            dev.address, dev.hid_type, dbg[idx].ok_count, dbg[idx].err_count);
                    }
                }
            }
            Err(_e) => {
                if idx < 4 {
                    let mut dbg = POLL_DEBUG.lock();
                    dbg[idx].err_count += 1;
                    if dbg[idx].err_count <= 3 {
                        crate::serial_println!("[USB-HID] dev {} ({:?}) int xfer err: {}",
                            dev.address, dev.hid_type, _e);
                    }
                }
            }
        }
    }
}

// ── Boot Protocol Report Parsing ───────────────

/// HID modifier key bits (byte 0 of boot keyboard report).
const MOD_LEFT_CTRL: u8 = 1 << 0;
const MOD_LEFT_SHIFT: u8 = 1 << 1;
const MOD_LEFT_ALT: u8 = 1 << 2;
#[allow(dead_code)]
const MOD_LEFT_GUI: u8 = 1 << 3;
const MOD_RIGHT_CTRL: u8 = 1 << 4;
const MOD_RIGHT_SHIFT: u8 = 1 << 5;
const MOD_RIGHT_ALT: u8 = 1 << 6;
#[allow(dead_code)]
const MOD_RIGHT_GUI: u8 = 1 << 7;

/// Modifier bit → PS/2 scancode mapping (make code; release = | 0x80).
/// Third element: if true, send 0xE0 prefix before the scancode so the keyboard
/// driver distinguishes Right Alt (AltGr) / Right Ctrl from their left counterparts.
const MOD_SCANCODES: [(u8, u8, bool); 6] = [
    (MOD_LEFT_CTRL,   0x1D, false),
    (MOD_LEFT_SHIFT,  0x2A, false),
    (MOD_LEFT_ALT,    0x38, false),
    (MOD_RIGHT_CTRL,  0x1D, true),   // E0 prefix — Right Ctrl
    (MOD_RIGHT_SHIFT, 0x36, false),
    (MOD_RIGHT_ALT,   0x38, true),   // E0 prefix — Right Alt (AltGr)
];

/// Convert a USB HID usage code to a PS/2 scancode.
/// Returns `(scancode, needs_e0)`. `needs_e0 = true` means the caller must
/// send 0xE0 prefix before the scancode (navigation cluster, keypad specials).
fn hid_usage_to_scancode(usage: u8) -> Option<(u8, bool)> {
    // USB HID Usage Table → PS/2 Scancode Set 1
    // (scancode, needs_e0_prefix)
    match usage {
        0x00 | 0x01 => None, // No key / Error rollover

        // ── Letters (A-Z) ────────────────────────────
        0x04 => Some((0x1E, false)),  // A
        0x05 => Some((0x30, false)),  // B
        0x06 => Some((0x2E, false)),  // C
        0x07 => Some((0x20, false)),  // D
        0x08 => Some((0x12, false)),  // E
        0x09 => Some((0x21, false)),  // F
        0x0A => Some((0x22, false)),  // G
        0x0B => Some((0x23, false)),  // H
        0x0C => Some((0x17, false)),  // I
        0x0D => Some((0x24, false)),  // J
        0x0E => Some((0x25, false)),  // K
        0x0F => Some((0x26, false)),  // L
        0x10 => Some((0x32, false)),  // M
        0x11 => Some((0x31, false)),  // N
        0x12 => Some((0x18, false)),  // O
        0x13 => Some((0x19, false)),  // P
        0x14 => Some((0x10, false)),  // Q
        0x15 => Some((0x13, false)),  // R
        0x16 => Some((0x1F, false)),  // S
        0x17 => Some((0x14, false)),  // T
        0x18 => Some((0x16, false)),  // U
        0x19 => Some((0x2F, false)),  // V
        0x1A => Some((0x11, false)),  // W
        0x1B => Some((0x2D, false)),  // X
        0x1C => Some((0x15, false)),  // Y
        0x1D => Some((0x2C, false)),  // Z

        // ── Numbers (1-0) ────────────────────────────
        0x1E => Some((0x02, false)),  // 1
        0x1F => Some((0x03, false)),  // 2
        0x20 => Some((0x04, false)),  // 3
        0x21 => Some((0x05, false)),  // 4
        0x22 => Some((0x06, false)),  // 5
        0x23 => Some((0x07, false)),  // 6
        0x24 => Some((0x08, false)),  // 7
        0x25 => Some((0x09, false)),  // 8
        0x26 => Some((0x0A, false)),  // 9
        0x27 => Some((0x0B, false)),  // 0

        // ── Control keys ─────────────────────────────
        0x28 => Some((0x1C, false)),  // Enter
        0x29 => Some((0x01, false)),  // Escape
        0x2A => Some((0x0E, false)),  // Backspace
        0x2B => Some((0x0F, false)),  // Tab
        0x2C => Some((0x39, false)),  // Space

        // ── Symbols ──────────────────────────────────
        0x2D => Some((0x0C, false)),  // - _
        0x2E => Some((0x0D, false)),  // = +
        0x2F => Some((0x1A, false)),  // [ {
        0x30 => Some((0x1B, false)),  // ] }
        0x31 => Some((0x2B, false)),  // \ |
        0x32 => Some((0x2B, false)),  // Non-US # ~ (ISO, same PS/2 scancode as \)
        0x33 => Some((0x27, false)),  // ; :
        0x34 => Some((0x28, false)),  // ' "
        0x35 => Some((0x29, false)),  // ` ~
        0x36 => Some((0x33, false)),  // , <
        0x37 => Some((0x34, false)),  // . >
        0x38 => Some((0x35, false)),  // / ?

        // ── Lock keys ────────────────────────────────
        0x39 => Some((0x3A, false)),  // Caps Lock
        0x47 => Some((0x46, false)),  // Scroll Lock
        0x53 => Some((0x45, false)),  // Num Lock

        // ── Function keys (F1-F12) ───────────────────
        0x3A => Some((0x3B, false)),  // F1
        0x3B => Some((0x3C, false)),  // F2
        0x3C => Some((0x3D, false)),  // F3
        0x3D => Some((0x3E, false)),  // F4
        0x3E => Some((0x3F, false)),  // F5
        0x3F => Some((0x40, false)),  // F6
        0x40 => Some((0x41, false)),  // F7
        0x41 => Some((0x42, false)),  // F8
        0x42 => Some((0x43, false)),  // F9
        0x43 => Some((0x44, false)),  // F10
        0x44 => Some((0x57, false)),  // F11
        0x45 => Some((0x58, false)),  // F12

        // ── Print Screen / Pause ─────────────────────
        0x46 => Some((0x37, true)),   // Print Screen (E0 37)
        0x48 => Some((0x45, true)),   // Pause/Break  (simplified as E0 45)

        // ── Navigation cluster (E0-prefixed) ─────────
        0x49 => Some((0x52, true)),   // Insert    (E0 52)
        0x4A => Some((0x47, true)),   // Home      (E0 47)
        0x4B => Some((0x49, true)),   // Page Up   (E0 49)
        0x4C => Some((0x53, true)),   // Delete    (E0 53)
        0x4D => Some((0x4F, true)),   // End       (E0 4F)
        0x4E => Some((0x51, true)),   // Page Down (E0 51)

        // ── Arrow keys (E0-prefixed) ─────────────────
        0x4F => Some((0x4D, true)),   // Right Arrow (E0 4D)
        0x50 => Some((0x4B, true)),   // Left Arrow  (E0 4B)
        0x51 => Some((0x50, true)),   // Down Arrow  (E0 50)
        0x52 => Some((0x48, true)),   // Up Arrow    (E0 48)

        // ── Keypad ───────────────────────────────────
        0x54 => Some((0x35, true)),   // Keypad /     (E0 35)
        0x55 => Some((0x37, false)),  // Keypad *     (37, no E0)
        0x56 => Some((0x4A, false)),  // Keypad -
        0x57 => Some((0x4E, false)),  // Keypad +
        0x58 => Some((0x1C, true)),   // Keypad Enter (E0 1C)
        0x59 => Some((0x4F, false)),  // Keypad 1 / End
        0x5A => Some((0x50, false)),  // Keypad 2 / Down
        0x5B => Some((0x51, false)),  // Keypad 3 / PgDn
        0x5C => Some((0x4B, false)),  // Keypad 4 / Left
        0x5D => Some((0x4C, false)),  // Keypad 5
        0x5E => Some((0x4D, false)),  // Keypad 6 / Right
        0x5F => Some((0x47, false)),  // Keypad 7 / Home
        0x60 => Some((0x48, false)),  // Keypad 8 / Up
        0x61 => Some((0x49, false)),  // Keypad 9 / PgUp
        0x62 => Some((0x52, false)),  // Keypad 0 / Ins
        0x63 => Some((0x53, false)),  // Keypad . / Del

        // ── ISO / International ──────────────────────
        0x64 => Some((0x56, false)),  // Non-US \ | (ISO key, between LShift and Z)
        0x65 => Some((0x5D, true)),   // Application / Menu (E0 5D)

        _ => None,
    }
}

/// Track previous modifier state for press/release detection.
static PREV_MODIFIERS: Spinlock<u8> = Spinlock::new(0);

/// Parse a boot keyboard report (8 bytes) and inject key events into the
/// PS/2 keyboard pipeline. `prev_keys` tracks previously pressed keys to
/// detect press/release transitions.
fn parse_keyboard_report(report: &[u8; 8], prev_keys: &mut [u8; 6]) {
    let modifiers = report[0];
    let keys = &report[2..8];

    // Handle modifier key changes
    let mut prev_mods = PREV_MODIFIERS.lock();
    let changed_mods = modifiers ^ *prev_mods;
    for &(bit, scancode, needs_e0) in &MOD_SCANCODES {
        if changed_mods & bit != 0 {
            if needs_e0 {
                crate::drivers::input::keyboard::handle_scancode(0xE0);
            }
            if modifiers & bit != 0 {
                crate::drivers::input::keyboard::handle_scancode(scancode);
            } else {
                crate::drivers::input::keyboard::handle_scancode(scancode | 0x80);
            }
        }
    }
    *prev_mods = modifiers;

    // Detect released keys (in prev but not in current)
    for &prev in prev_keys.iter() {
        if prev != 0 && !keys.contains(&prev) {
            if let Some((scancode, needs_e0)) = hid_usage_to_scancode(prev) {
                if needs_e0 {
                    crate::drivers::input::keyboard::handle_scancode(0xE0);
                }
                crate::drivers::input::keyboard::handle_scancode(scancode | 0x80);
            }
        }
    }

    // Detect pressed keys (in current but not in prev)
    for &key in keys.iter() {
        if key != 0 && !prev_keys.contains(&key) {
            if let Some((scancode, needs_e0)) = hid_usage_to_scancode(key) {
                if needs_e0 {
                    crate::drivers::input::keyboard::handle_scancode(0xE0);
                }
                crate::drivers::input::keyboard::handle_scancode(scancode);
            }
        }
    }

    prev_keys.copy_from_slice(keys);
}

/// Track previous USB mouse button state for press/release detection.
static USB_MOUSE_BUTTONS: crate::sync::spinlock::Spinlock<u8> = crate::sync::spinlock::Spinlock::new(0);

/// Parse a boot mouse report (3-4 bytes) and inject movement/button events
/// directly into the mouse event buffer, bypassing the PS/2 state machine.
///
/// Previous approach of injecting PS/2 bytes via `handle_byte()` broke when the
/// PS/2 mouse init detected IntelliMouse (4-byte packets): USB reports inject
/// 3 bytes, the state machine expects 4, causing packet misalignment.
fn parse_mouse_report(report: &[u8], len: usize) {
    use crate::drivers::input::mouse::{MouseEvent, MouseEventType, MouseButtons, MOUSE_BUFFER};

    if len < 3 {
        return;
    }

    let buttons_raw = report[0];
    let dx = report[1] as i8 as i32;
    // USB HID Y is inverted relative to screen coordinates
    let dy = -(report[2] as i8 as i32);
    let dz = if len >= 4 { -(report[3] as i8 as i32) } else { 0 };

    let new_buttons = MouseButtons {
        left:   buttons_raw & 0x01 != 0,
        right:  buttons_raw & 0x02 != 0,
        middle: buttons_raw & 0x04 != 0,
    };

    // Detect button state changes
    let mut prev = USB_MOUSE_BUTTONS.lock();
    let old = *prev;
    *prev = buttons_raw & 0x07;
    drop(prev);

    let buttons_changed = (buttons_raw & 0x07) != old;

    // Skip completely empty reports (no movement, no button change, no scroll)
    if dx == 0 && dy == 0 && dz == 0 && !buttons_changed {
        return;
    }

    // Boot splash: update HW cursor directly (lag-free)
    crate::drivers::gpu::splash_cursor_move(dx, dy);

    // Determine event type and push directly into the mouse buffer
    if dz != 0 {
        let mut buf = MOUSE_BUFFER.lock();
        if buf.len() < 256 {
            buf.push_back(MouseEvent {
                dx, dy, dz, buttons: new_buttons,
                event_type: MouseEventType::Scroll,
            });
        }
    }

    if buttons_changed {
        let is_down = (buttons_raw & 0x07) & !(old) != 0;
        let mut buf = MOUSE_BUFFER.lock();
        if buf.len() < 256 {
            buf.push_back(MouseEvent {
                dx, dy, dz: 0, buttons: new_buttons,
                event_type: if is_down { MouseEventType::ButtonDown } else { MouseEventType::ButtonUp },
            });
        }
    } else if dx != 0 || dy != 0 {
        let mut buf = MOUSE_BUFFER.lock();
        if buf.len() < 256 {
            buf.push_back(MouseEvent {
                dx, dy, dz: 0, buttons: new_buttons,
                event_type: MouseEventType::Move,
            });
        }
    }

    crate::syscall::handlers::wake_compositor_if_blocked();
}

/// Track previous tablet button state.
static USB_TABLET_BUTTONS: Spinlock<u8> = Spinlock::new(0);

/// Parse a QEMU usb-tablet report (absolute positioning).
/// Format: [buttons, x_lo, x_hi, y_lo, y_hi, (scroll)]
/// X/Y range: 0..32767 (0x7FFF), scaled to screen pixels.
fn parse_tablet_report(report: &[u8], len: usize) {
    use crate::drivers::input::mouse::{MouseEvent, MouseEventType, MouseButtons, MOUSE_BUFFER};

    if len < 5 { return; }

    let buttons_raw = report[0];
    let raw_x = u16::from_le_bytes([report[1], report[2]]);
    let raw_y = u16::from_le_bytes([report[3], report[4]]);
    let dz = if len >= 6 { -(report[5] as i8 as i32) } else { 0 };

    let new_buttons = MouseButtons {
        left:   buttons_raw & 0x01 != 0,
        right:  buttons_raw & 0x02 != 0,
        middle: buttons_raw & 0x04 != 0,
    };

    // Scale from 0..32767 to screen pixel coordinates
    let (screen_w, screen_h) = crate::drivers::input::vmmouse::get_screen_size_pub();
    let x = ((raw_x as u64 * screen_w as u64) / 32767) as i32;
    let y = ((raw_y as u64 * screen_h as u64) / 32767) as i32;

    // Track button changes
    let mut prev = USB_TABLET_BUTTONS.lock();
    let old = *prev;
    *prev = buttons_raw & 0x07;
    drop(prev);
    let buttons_changed = (buttons_raw & 0x07) != old;

    // Scroll events
    if dz != 0 {
        let mut buf = MOUSE_BUFFER.lock();
        if buf.len() < 256 {
            buf.push_back(MouseEvent {
                dx: x, dy: y, dz, buttons: new_buttons,
                event_type: MouseEventType::Scroll,
            });
        }
    }

    // Inject absolute position + button events
    crate::drivers::input::mouse::inject_absolute(x, y, new_buttons);

    crate::syscall::handlers::wake_compositor_if_blocked();
}
