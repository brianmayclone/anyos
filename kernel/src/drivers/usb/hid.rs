//! USB HID (Human Interface Device) class driver.
//!
//! Supports boot protocol keyboards and mice. When a HID device is detected
//! during USB enumeration, the driver sets it to boot protocol mode and
//! registers it for periodic GET_REPORT polling.
//!
//! Boot protocol keyboard report: 8 bytes [modifier, reserved, key1..key6]
//! Boot protocol mouse report: 3-4 bytes [buttons, dx, dy, (dz)]
//!
//! Events are injected into the existing PS/2 input pipeline so the compositor
//! receives them through the same SYS_INPUT_POLL path.

use super::{ControllerType, SetupPacket, UsbDevice, UsbInterface, UsbSpeed};
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

// ── HID Device Registry ─────────────────────────

struct HidDevice {
    address: u8,
    controller: ControllerType,
    speed: UsbSpeed,
    max_packet: u16,
    protocol: u8,       // 1=keyboard, 2=mouse
    interface_num: u8,
    prev_keys: [u8; 6], // keyboard only — tracks which keys were pressed last poll
}

static HID_DEVICES: Spinlock<Vec<HidDevice>> = Spinlock::new(Vec::new());

/// Called when a HID interface is detected during USB enumeration.
/// Sets boot protocol mode and registers the device for polling.
pub fn probe(dev: &UsbDevice, iface: &UsbInterface) {
    let device_type = match iface.protocol {
        1 => "Keyboard",
        2 => "Mouse",
        _ => "Unknown HID",
    };

    let subclass_desc = match iface.subclass {
        0 => "No Subclass",
        1 => "Boot Interface",
        _ => "Unknown",
    };

    crate::serial_println!(
        "  USB HID: {} detected (subclass={} [{}], protocol={}, addr={})",
        device_type, iface.subclass, subclass_desc, iface.protocol, dev.address,
    );

    // Find interrupt IN endpoint for this interface
    let int_ep = iface.endpoints.iter().find(|ep| {
        (ep.attributes & 0x03) == 3   // Interrupt transfer type
            && (ep.address & 0x80) != 0 // IN direction
    });

    if let Some(ep) = int_ep {
        crate::serial_println!(
            "  USB HID: interrupt IN endpoint {:#04x}, max_packet={}, interval={}ms",
            ep.address, ep.max_packet_size, ep.interval,
        );
    } else {
        crate::serial_println!("  USB HID: no interrupt IN endpoint found");
    }

    // Only handle boot protocol devices (subclass 1)
    if iface.subclass != 1 {
        crate::serial_println!("  USB HID: skipping non-boot device");
        return;
    }

    // Send SET_PROTOCOL(0 = boot protocol)
    let set_protocol = SetupPacket {
        bm_request_type: 0x21, // Host-to-device, Class, Interface
        b_request: 0x0B,       // SET_PROTOCOL
        w_value: 0,            // 0 = Boot Protocol
        w_index: iface.number as u16,
        w_length: 0,
    };

    match super::hid_control_transfer(
        dev.address, dev.controller, dev.speed, dev.max_packet_size,
        &set_protocol, false, 0,
    ) {
        Ok(_) => crate::serial_println!("  USB HID: SET_PROTOCOL(boot) OK for addr={}", dev.address),
        Err(e) => {
            crate::serial_println!("  USB HID: SET_PROTOCOL failed: {} — continuing anyway", e);
            // Some devices work in boot protocol by default, so don't bail
        }
    }

    // Register for polling
    let hid_dev = HidDevice {
        address: dev.address,
        controller: dev.controller,
        speed: dev.speed,
        max_packet: dev.max_packet_size,
        protocol: iface.protocol,
        interface_num: iface.number,
        prev_keys: [0u8; 6],
    };

    HID_DEVICES.lock().push(hid_dev);
    crate::serial_println!(
        "  USB HID: registered {} (addr={}) for polling",
        device_type, dev.address,
    );
}

/// Poll all registered HID devices via GET_REPORT control transfer.
/// Called from the USB poll thread at ~10ms intervals.
pub fn poll_all() {
    let mut devs = HID_DEVICES.lock();
    for dev in devs.iter_mut() {
        let report_len: u16 = if dev.protocol == 1 { 8 } else { 4 };

        let get_report = SetupPacket {
            bm_request_type: 0xA1, // Device-to-host, Class, Interface
            b_request: 0x01,       // GET_REPORT
            w_value: 0x0100,       // Report type = Input (1), Report ID = 0
            w_index: dev.interface_num as u16,
            w_length: report_len,
        };

        match super::hid_control_transfer(
            dev.address, dev.controller, dev.speed, dev.max_packet,
            &get_report, true, report_len,
        ) {
            Ok(data) => {
                if data.len() >= 3 {
                    match dev.protocol {
                        1 if data.len() >= 8 => {
                            let mut report = [0u8; 8];
                            report.copy_from_slice(&data[..8]);
                            parse_keyboard_report(&report, &mut dev.prev_keys);
                        }
                        2 => {
                            parse_mouse_report(&data, data.len());
                        }
                        _ => {}
                    }
                }
            }
            Err(_) => {
                // Device may have been unplugged — silently skip
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
const MOD_SCANCODES: [(u8, u8); 6] = [
    (MOD_LEFT_CTRL,   0x1D),
    (MOD_LEFT_SHIFT,  0x2A),
    (MOD_LEFT_ALT,    0x38),
    (MOD_RIGHT_CTRL,  0x1D), // simplified — PS/2 right ctrl is E0 1D
    (MOD_RIGHT_SHIFT, 0x36),
    (MOD_RIGHT_ALT,   0x38), // simplified — PS/2 right alt is E0 38
];

/// Convert a USB HID usage code (from boot keyboard report) to a PS/2 scancode.
/// Returns None for unmapped keys.
fn hid_usage_to_scancode(usage: u8) -> Option<u8> {
    // USB HID Usage Table → PS/2 Scancode Set 1
    match usage {
        0x00 => None,        // No key
        0x01 => None,        // Error rollover
        0x04 => Some(0x1E),  // A
        0x05 => Some(0x30),  // B
        0x06 => Some(0x2E),  // C
        0x07 => Some(0x20),  // D
        0x08 => Some(0x12),  // E
        0x09 => Some(0x21),  // F
        0x0A => Some(0x22),  // G
        0x0B => Some(0x23),  // H
        0x0C => Some(0x17),  // I
        0x0D => Some(0x24),  // J
        0x0E => Some(0x25),  // K
        0x0F => Some(0x26),  // L
        0x10 => Some(0x32),  // M
        0x11 => Some(0x31),  // N
        0x12 => Some(0x18),  // O
        0x13 => Some(0x19),  // P
        0x14 => Some(0x10),  // Q
        0x15 => Some(0x13),  // R
        0x16 => Some(0x1F),  // S
        0x17 => Some(0x14),  // T
        0x18 => Some(0x16),  // U
        0x19 => Some(0x2F),  // V
        0x1A => Some(0x11),  // W
        0x1B => Some(0x2D),  // X
        0x1C => Some(0x15),  // Y
        0x1D => Some(0x2C),  // Z
        0x1E => Some(0x02),  // 1
        0x1F => Some(0x03),  // 2
        0x20 => Some(0x04),  // 3
        0x21 => Some(0x05),  // 4
        0x22 => Some(0x06),  // 5
        0x23 => Some(0x07),  // 6
        0x24 => Some(0x08),  // 7
        0x25 => Some(0x09),  // 8
        0x26 => Some(0x0A),  // 9
        0x27 => Some(0x0B),  // 0
        0x28 => Some(0x1C),  // Enter
        0x29 => Some(0x01),  // Escape
        0x2A => Some(0x0E),  // Backspace
        0x2B => Some(0x0F),  // Tab
        0x2C => Some(0x39),  // Space
        0x2D => Some(0x0C),  // -
        0x2E => Some(0x0D),  // =
        0x2F => Some(0x1A),  // [
        0x30 => Some(0x1B),  // ]
        0x31 => Some(0x2B),  // backslash
        0x33 => Some(0x27),  // ;
        0x34 => Some(0x28),  // '
        0x35 => Some(0x29),  // `
        0x36 => Some(0x33),  // ,
        0x37 => Some(0x34),  // .
        0x38 => Some(0x35),  // /
        0x39 => Some(0x3A),  // Caps Lock
        0x3A => Some(0x3B),  // F1
        0x3B => Some(0x3C),  // F2
        0x3C => Some(0x3D),  // F3
        0x3D => Some(0x3E),  // F4
        0x3E => Some(0x3F),  // F5
        0x3F => Some(0x40),  // F6
        0x40 => Some(0x41),  // F7
        0x41 => Some(0x42),  // F8
        0x42 => Some(0x43),  // F9
        0x43 => Some(0x44),  // F10
        0x44 => Some(0x57),  // F11
        0x45 => Some(0x58),  // F12
        0x4F => Some(0x4D),  // Right Arrow
        0x50 => Some(0x4B),  // Left Arrow
        0x51 => Some(0x50),  // Down Arrow
        0x52 => Some(0x48),  // Up Arrow
        0x53 => Some(0x45),  // Num Lock
        0x62 => Some(0x52),  // Keypad 0
        0x63 => Some(0x53),  // Keypad .
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
    for &(bit, scancode) in &MOD_SCANCODES {
        if changed_mods & bit != 0 {
            if modifiers & bit != 0 {
                // Modifier pressed
                crate::drivers::input::keyboard::handle_scancode(scancode);
            } else {
                // Modifier released
                crate::drivers::input::keyboard::handle_scancode(scancode | 0x80);
            }
        }
    }
    *prev_mods = modifiers;

    // Detect released keys (in prev but not in current)
    for &prev in prev_keys.iter() {
        if prev != 0 && !keys.contains(&prev) {
            if let Some(scancode) = hid_usage_to_scancode(prev) {
                // PS/2 release = scancode | 0x80
                crate::drivers::input::keyboard::handle_scancode(scancode | 0x80);
            }
        }
    }

    // Detect pressed keys (in current but not in prev)
    for &key in keys.iter() {
        if key != 0 && !prev_keys.contains(&key) {
            if let Some(scancode) = hid_usage_to_scancode(key) {
                crate::drivers::input::keyboard::handle_scancode(scancode);
            }
        }
    }

    prev_keys.copy_from_slice(keys);
}

/// Parse a boot mouse report (3-4 bytes) and inject movement/button events
/// into the PS/2 mouse pipeline.
fn parse_mouse_report(report: &[u8], len: usize) {
    if len < 3 {
        return;
    }

    let buttons = report[0];
    let dx = report[1] as i8;
    let dy = report[2] as i8;

    // Skip empty reports (no movement, no button changes)
    if dx == 0 && dy == 0 && buttons == 0 {
        return;
    }

    // Reconstruct PS/2 mouse byte 0: buttons + sign bits + overflow
    let mut byte0: u8 = 0x08; // bit 3 always set in PS/2
    if buttons & 0x01 != 0 { byte0 |= 0x01; } // left button
    if buttons & 0x02 != 0 { byte0 |= 0x02; } // right button
    if buttons & 0x04 != 0 { byte0 |= 0x04; } // middle button
    if dx < 0 { byte0 |= 0x10; } // X sign bit
    if dy < 0 { byte0 |= 0x20; } // Y sign bit

    // Inject as PS/2 mouse bytes
    crate::drivers::input::mouse::handle_byte(byte0);
    crate::drivers::input::mouse::handle_byte(dx as u8);
    // PS/2 Y axis is inverted relative to USB HID
    crate::drivers::input::mouse::handle_byte((-(dy as i16)) as u8);
}
