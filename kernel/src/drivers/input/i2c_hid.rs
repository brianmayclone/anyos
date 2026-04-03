//! I2C-HID Touchpad/Touchscreen driver.
//!
//! Modern laptops connect their trackpad and touchscreen via I2C (not PS/2).
//! The I2C-HID protocol (Microsoft HID-over-I2C spec) wraps standard HID
//! reports in an I2C transport layer. Common controllers:
//! - Synaptics (most ThinkPads, HP)
//! - ELAN (ASUS, Acer, many Chromebooks)
//! - Atmel mXT (some Dell, Surface)
//! - Goodix (touchscreens)
//!
//! The device is discovered via ACPI (DSDT contains the I2C address, IRQ,
//! and HID descriptor address). This driver handles:
//! 1. ACPI-based device discovery (I2C slave address + GPIO IRQ)
//! 2. I2C-HID protocol (descriptor fetch, report reading)
//! 3. HID report parsing (absolute X/Y, multitouch contacts, buttons)
//! 4. Event injection into the compositor input system

use alloc::vec;
use alloc::vec::Vec;
use crate::sync::spinlock::Spinlock;

// ── I2C-HID Protocol Constants ──────────────────────────────────────────────

/// I2C-HID register addresses (from HID descriptor)
const I2C_HID_DESC_REGISTER: u16 = 0x0001; // Default, overridden by ACPI
const I2C_HID_REPORT_DESC_REG: u16 = 0x0002;
const I2C_HID_INPUT_REG: u16 = 0x0003;
const I2C_HID_OUTPUT_REG: u16 = 0x0004;
const I2C_HID_COMMAND_REG: u16 = 0x0005;
const I2C_HID_DATA_REG: u16 = 0x0006;

/// I2C-HID commands (written to command register)
const CMD_RESET: u8 = 0x01;
const CMD_GET_REPORT: u8 = 0x02;
const CMD_SET_REPORT: u8 = 0x03;
const CMD_SET_POWER: u8 = 0x08;

/// Power states
const POWER_ON: u8 = 0x00;
const POWER_SLEEP: u8 = 0x01;

// ── I2C-HID Descriptor (returned by device at desc register) ────────────────

/// The HID descriptor returned by the I2C device.
/// 30 bytes, defines register addresses and report sizes.
#[repr(C, packed)]
#[derive(Debug, Clone, Copy, Default)]
pub struct I2cHidDescriptor {
    pub desc_length: u16,        // Total descriptor length (should be 30)
    pub bcd_version: u16,        // I2C-HID version (0x0100 = 1.0)
    pub report_desc_length: u16, // Length of HID report descriptor
    pub report_desc_register: u16,
    pub input_register: u16,
    pub max_input_length: u16,
    pub output_register: u16,
    pub max_output_length: u16,
    pub command_register: u16,
    pub data_register: u16,
    pub vendor_id: u16,
    pub product_id: u16,
    pub version_id: u16,
    pub reserved: u32,
}

// ── Touchpad Event ──────────────────────────────────────────────────────────

/// A parsed touchpad/touchscreen event.
#[derive(Debug, Clone, Copy, Default)]
pub struct TouchEvent {
    /// Absolute X coordinate (0..max_x).
    pub x: u16,
    /// Absolute Y coordinate (0..max_y).
    pub y: u16,
    /// Finger/contact is touching the surface.
    pub touching: bool,
    /// Physical button pressed (left click area).
    pub button: bool,
    /// Contact ID (for multitouch, 0-9).
    pub contact_id: u8,
    /// Pressure (0-255, 0 if not supported).
    pub pressure: u8,
}

/// Parsed touchpad layout from HID report descriptor.
#[derive(Debug, Clone, Default)]
pub struct TouchpadLayout {
    pub x_bit_offset: u16,
    pub x_bit_size: u8,
    pub x_max: u16,
    pub y_bit_offset: u16,
    pub y_bit_size: u8,
    pub y_max: u16,
    pub tip_bit_offset: u16,
    pub button_bit_offset: u16,
    pub pressure_bit_offset: u16,
    pub pressure_bit_size: u8,
    pub contact_id_bit_offset: u16,
    pub contact_id_bit_size: u8,
    pub contact_count_bit_offset: u16,
    pub report_size_bytes: u16,
    pub max_contacts: u8,
}

// ── Device State ────────────────────────────────────────────────────────────

struct I2cHidDevice {
    /// I2C slave address (7-bit, typically 0x15 or 0x2C).
    i2c_addr: u8,
    /// GPIO IRQ number for the touchpad.
    irq: u8,
    /// I2C-HID descriptor fetched from device.
    hid_desc: I2cHidDescriptor,
    /// Parsed touchpad layout.
    layout: TouchpadLayout,
    /// Screen dimensions for coordinate scaling.
    screen_w: u32,
    screen_h: u32,
    /// Last known finger position (for relative motion fallback).
    last_x: i32,
    last_y: i32,
    last_touching: bool,
}

static DEVICES: Spinlock<Vec<I2cHidDevice>> = Spinlock::new(Vec::new());

// ── I2C Transport ───────────────────────────────────────────────────────────

/// Read bytes from an I2C-HID device register.
/// Uses the kernel's SMBus/I2C driver for the actual bus transaction.
fn i2c_read_register(i2c_addr: u8, register: u16, buf: &mut [u8]) -> bool {
    // Write the 2-byte register address, then read the response
    let reg_bytes = register.to_le_bytes();
    crate::drivers::i2c::write_then_read(i2c_addr, &reg_bytes, buf)
}

/// Write bytes to an I2C-HID device.
fn i2c_write(i2c_addr: u8, data: &[u8]) -> bool {
    crate::drivers::i2c::write(i2c_addr, data)
}

/// Send an I2C-HID command.
fn send_command(dev: &I2cHidDevice, opcode: u8, report_id: u8) {
    let cmd_reg = dev.hid_desc.command_register;
    // Command format: [cmd_reg_lo, cmd_reg_hi, opcode | (report_id_lo << 4), report_id_hi]
    let cmd_lo = (opcode as u16) | ((report_id as u16 & 0x0F) << 8);
    let data = [
        (cmd_reg & 0xFF) as u8,
        (cmd_reg >> 8) as u8,
        (cmd_lo & 0xFF) as u8,
        (cmd_lo >> 8) as u8,
    ];
    i2c_write(dev.i2c_addr, &data);
}

// ── Device Initialization ───────────────────────────────────────────────────

/// Initialize an I2C-HID device at the given address.
/// Called during ACPI device enumeration.
pub fn probe_device(i2c_addr: u8, desc_register: u16, irq: u8) -> bool {
    crate::serial_println!("  I2C-HID: probing addr={:#04x} desc_reg={:#06x} irq={}",
        i2c_addr, desc_register, irq);

    // Read HID descriptor
    let mut desc_buf = [0u8; 30];
    if !i2c_read_register(i2c_addr, desc_register, &mut desc_buf) {
        crate::serial_println!("  I2C-HID: failed to read descriptor");
        return false;
    }

    let hid_desc: I2cHidDescriptor = unsafe { core::ptr::read_unaligned(desc_buf.as_ptr() as *const _) };

    // Copy fields out of packed struct to avoid unaligned references
    let desc_len = { hid_desc.desc_length };
    let bcd_ver = { hid_desc.bcd_version };
    let vid = { hid_desc.vendor_id };
    let pid = { hid_desc.product_id };
    let ver = { hid_desc.version_id };
    let rd_length = { hid_desc.report_desc_length };

    if desc_len < 30 || bcd_ver == 0 {
        crate::serial_println!("  I2C-HID: invalid descriptor (len={}, ver={:#06x})",
            desc_len, bcd_ver);
        return false;
    }

    crate::serial_println!("  I2C-HID: VID={:#06x} PID={:#06x} ver={:#06x} report_desc_len={}",
        vid, pid, ver, rd_length);

    // Reset device
    send_command(&I2cHidDevice {
        i2c_addr, irq, hid_desc, layout: TouchpadLayout::default(),
        screen_w: 1920, screen_h: 1080, last_x: 0, last_y: 0, last_touching: false,
    }, CMD_RESET, 0);

    // Small delay for reset
    for _ in 0..100_000 { core::hint::spin_loop(); }

    // Power on
    send_command(&I2cHidDevice {
        i2c_addr, irq, hid_desc, layout: TouchpadLayout::default(),
        screen_w: 1920, screen_h: 1080, last_x: 0, last_y: 0, last_touching: false,
    }, CMD_SET_POWER, POWER_ON);

    // Read HID report descriptor
    let rd_len = rd_length as usize;
    if rd_len == 0 || rd_len > 4096 {
        crate::serial_println!("  I2C-HID: invalid report desc length {}", rd_len);
        return false;
    }

    let mut report_desc = vec![0u8; rd_len];
    let rd_register = { hid_desc.report_desc_register };
    if !i2c_read_register(i2c_addr, rd_register, &mut report_desc) {
        crate::serial_println!("  I2C-HID: failed to read report descriptor");
        return false;
    }

    // Parse report descriptor for touchpad fields
    let layout = parse_touchpad_descriptor(&report_desc);

    let (screen_w, screen_h) = crate::drivers::framebuffer::info()
        .map(|fb| (fb.width, fb.height))
        .unwrap_or((1920, 1080));

    let dev = I2cHidDevice {
        i2c_addr, irq, hid_desc, layout,
        screen_w, screen_h,
        last_x: 0, last_y: 0, last_touching: false,
    };

    crate::serial_println!("[OK] I2C-HID Touchpad: addr={:#04x} ({}x{}, max_contacts={})",
        i2c_addr, dev.layout.x_max, dev.layout.y_max, dev.layout.max_contacts);

    DEVICES.lock().push(dev);
    true
}

// ── HID Report Descriptor Parsing ───────────────────────────────────────────

fn parse_touchpad_descriptor(desc: &[u8]) -> TouchpadLayout {
    let mut layout = TouchpadLayout::default();
    let mut usage_page: u16 = 0;
    let mut usage: u8 = 0;
    let mut logical_max: u16 = 0;
    let mut report_size: u8 = 0;
    let mut report_count: u8 = 0;
    let mut bit_offset: u16 = 0;
    let mut in_digitizer = false;
    let mut contact_count_found = false;

    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        let bsize = match prefix & 0x03 { 0 => 0, 1 => 1, 2 => 2, 3 => 4, _ => 0 };
        let btype = (prefix >> 2) & 0x03;
        let btag = (prefix >> 4) & 0x0F;
        if i + 1 + bsize > desc.len() { break; }

        let data = match bsize {
            1 => desc[i + 1] as u32,
            2 => u16::from_le_bytes([desc[i + 1], desc[i + 2]]) as u32,
            4 => u32::from_le_bytes([desc[i + 1], desc[i + 2], desc[i + 3], desc[i + 4]]),
            _ => 0,
        };

        match (btype, btag) {
            (1, 0) => usage_page = data as u16,
            (1, 1) => {} // logical min
            (1, 2) => logical_max = data as u16,
            (1, 7) => report_size = data as u8,
            (1, 9) => report_count = data as u8,
            (2, 0) => usage = data as u8,
            (0, 10) => { // Collection
                if usage_page == 0x0D { in_digitizer = true; }
            }
            (0, 8) => { // Input
                if in_digitizer && data & 0x02 != 0 { // Variable
                    // Map fields based on usage page + usage
                    if usage_page == 0x01 { // Generic Desktop
                        match usage {
                            0x30 => { // X
                                layout.x_bit_offset = bit_offset;
                                layout.x_bit_size = report_size;
                                layout.x_max = logical_max;
                            }
                            0x31 => { // Y
                                layout.y_bit_offset = bit_offset;
                                layout.y_bit_size = report_size;
                                layout.y_max = logical_max;
                            }
                            _ => {}
                        }
                    } else if usage_page == 0x0D { // Digitizer
                        match usage {
                            0x42 => layout.tip_bit_offset = bit_offset,          // Tip Switch
                            0x47 => layout.button_bit_offset = bit_offset,       // Confidence / Button
                            0x30 => { // Tip Pressure
                                layout.pressure_bit_offset = bit_offset;
                                layout.pressure_bit_size = report_size;
                            }
                            0x51 => { // Contact Identifier
                                layout.contact_id_bit_offset = bit_offset;
                                layout.contact_id_bit_size = report_size;
                            }
                            0x54 => { // Contact Count
                                layout.contact_count_bit_offset = bit_offset;
                                contact_count_found = true;
                            }
                            0x55 => { // Contact Count Maximum
                                layout.max_contacts = data as u8;
                            }
                            _ => {}
                        }
                    }
                }
                bit_offset += (report_size as u16) * (report_count as u16);
                usage = 0;
            }
            _ => {}
        }
        i += 1 + bsize;
    }

    layout.report_size_bytes = (bit_offset + 7) / 8;
    if layout.max_contacts == 0 { layout.max_contacts = 1; }
    layout
}

// ── Input Report Processing ─────────────────────────────────────────────────

/// Extract a field value from a raw HID report.
fn extract_field(report: &[u8], bit_offset: u16, bit_size: u8) -> u32 {
    if bit_size == 0 { return 0; }
    let byte_off = (bit_offset / 8) as usize;
    let bit_in_byte = bit_offset % 8;
    if byte_off >= report.len() { return 0; }

    let mut raw: u32 = 0;
    for b in 0..4usize {
        if byte_off + b < report.len() {
            raw |= (report[byte_off + b] as u32) << (b * 8);
        }
    }
    (raw >> bit_in_byte) & ((1u32 << bit_size) - 1)
}

/// Process an incoming I2C-HID input report.
pub fn handle_input_report(i2c_addr: u8, report: &[u8]) {
    let mut devs = DEVICES.lock();
    let dev = match devs.iter_mut().find(|d| d.i2c_addr == i2c_addr) {
        Some(d) => d,
        None => return,
    };

    if report.len() < 2 { return; }

    // First 2 bytes = report length (I2C-HID framing)
    let report_len = u16::from_le_bytes([report[0], report[1]]) as usize;
    if report_len < 3 || report_len > report.len() { return; }
    let data = &report[2..report_len]; // Skip length prefix

    let layout = &dev.layout;
    let touching = extract_field(data, layout.tip_bit_offset, 1) != 0;
    let x_raw = extract_field(data, layout.x_bit_offset, layout.x_bit_size);
    let y_raw = extract_field(data, layout.y_bit_offset, layout.y_bit_size);

    // Scale to screen coordinates
    let x_max = layout.x_max.max(1) as u64;
    let y_max = layout.y_max.max(1) as u64;
    let screen_x = ((x_raw as u64 * dev.screen_w as u64) / x_max) as i32;
    let screen_y = ((y_raw as u64 * dev.screen_h as u64) / y_max) as i32;

    let button = extract_field(data, layout.button_bit_offset, 1) != 0;

    if touching {
        if dev.last_touching {
            // Finger dragging — compute delta and move cursor position
            let dx = screen_x - dev.last_x;
            let dy = screen_y - dev.last_y;
            if dx != 0 || dy != 0 {
                // Inject as absolute position update (cursor tracks finger delta)
                let buttons = crate::drivers::input::mouse::MouseButtons {
                    left: button, right: false, middle: false,
                };
                crate::drivers::input::mouse::inject_absolute(screen_x, screen_y, buttons);
            }
        }
        dev.last_x = screen_x;
        dev.last_y = screen_y;
    }

    // Tap detection: finger down → up quickly = left click
    if button && !dev.last_touching {
        let buttons = crate::drivers::input::mouse::MouseButtons {
            left: true, right: false, middle: false,
        };
        crate::drivers::input::mouse::inject_absolute(dev.last_x, dev.last_y, buttons);
    } else if !touching && dev.last_touching {
        // Release
        let buttons = crate::drivers::input::mouse::MouseButtons {
            left: false, right: false, middle: false,
        };
        crate::drivers::input::mouse::inject_absolute(dev.last_x, dev.last_y, buttons);
    }

    dev.last_touching = touching;
}

/// Check if any I2C-HID touchpad devices are available.
pub fn is_available() -> bool {
    !DEVICES.lock().is_empty()
}

/// Get the number of registered I2C-HID devices.
pub fn device_count() -> usize {
    DEVICES.lock().len()
}
