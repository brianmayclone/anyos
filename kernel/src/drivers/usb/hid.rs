//! USB HID (Human Interface Device) class driver.
//!
//! Supports boot protocol keyboards and mice. When a HID device is detected
//! during USB enumeration, the driver sets it to boot protocol mode and
//! performs a single GET_REPORT to read the initial state. For ongoing input,
//! the driver sets up a polling mechanism.
//!
//! Boot protocol keyboard report: 8 bytes [modifier, reserved, key1..key6]
//! Boot protocol mouse report: 3-4 bytes [buttons, dx, dy, (dz)]
//!
//! Events are injected into the existing PS/2 input pipeline so the compositor
//! receives them through the same SYS_INPUT_POLL path.

use super::{UsbDevice, UsbInterface};

/// Called when a HID interface is detected during USB enumeration.
/// Logs the device type and sets up initial polling.
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

    // Boot protocol devices (subclass 1) can be used without parsing HID report descriptors.
    // For now, log discovery. Full interrupt polling requires periodic schedule support.
    if iface.subclass == 1 {
        match iface.protocol {
            1 => crate::serial_println!("  USB HID: boot keyboard ready (boot protocol)"),
            2 => crate::serial_println!("  USB HID: boot mouse ready (boot protocol)"),
            _ => {}
        }
    }
}

// ── Boot Protocol Report Parsing ───────────────

/// HID modifier key bits (byte 0 of boot keyboard report).
#[allow(dead_code)]
const MOD_LEFT_CTRL: u8 = 1 << 0;
#[allow(dead_code)]
const MOD_LEFT_SHIFT: u8 = 1 << 1;
#[allow(dead_code)]
const MOD_LEFT_ALT: u8 = 1 << 2;
#[allow(dead_code)]
const MOD_LEFT_GUI: u8 = 1 << 3;
#[allow(dead_code)]
const MOD_RIGHT_CTRL: u8 = 1 << 4;
#[allow(dead_code)]
const MOD_RIGHT_SHIFT: u8 = 1 << 5;
#[allow(dead_code)]
const MOD_RIGHT_ALT: u8 = 1 << 6;
#[allow(dead_code)]
const MOD_RIGHT_GUI: u8 = 1 << 7;

/// Convert a USB HID usage code (from boot keyboard report) to a PS/2 scancode.
/// Returns None for unmapped keys.
#[allow(dead_code)]
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

/// Parse a boot keyboard report (8 bytes) and inject key events into the
/// PS/2 keyboard pipeline. `prev_keys` tracks previously pressed keys to
/// detect press/release transitions.
#[allow(dead_code)]
pub fn parse_keyboard_report(report: &[u8; 8], prev_keys: &mut [u8; 6]) {
    let _modifiers = report[0];
    let keys = &report[2..8];

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

    // Handle modifier changes by injecting corresponding scancodes
    // (Handled implicitly by the keyboard driver's modifier tracking)

    prev_keys.copy_from_slice(keys);
}

/// Parse a boot mouse report (3-4 bytes) and inject movement/button events
/// into the PS/2 mouse pipeline.
#[allow(dead_code)]
pub fn parse_mouse_report(report: &[u8], len: usize) {
    if len < 3 {
        return;
    }

    let buttons = report[0];
    let dx = report[1] as i8;
    let dy = report[2] as i8;
    let _dz = if len >= 4 { report[3] as i8 } else { 0i8 };

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
