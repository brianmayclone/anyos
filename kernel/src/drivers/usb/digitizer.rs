//! USB HID Digitizer (Drawing Tablet) support.
//!
//! Handles devices that implement HID Usage Page 0x0D (Digitizer) —
//! drawing tablets from Wacom, XP-Pen, Huion, etc.
//!
//! These devices report absolute X/Y coordinates plus pen pressure, tilt,
//! and tool type (pen, eraser, touch). The report descriptor is parsed to
//! discover the field layout since it varies between manufacturers.
//!
//! Events are injected into the compositor's input system as absolute
//! pointer events with pressure metadata.

use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

/// HID Usage Page: Digitizer (0x0D)
const USAGE_PAGE_DIGITIZER: u16 = 0x0D;

/// Digitizer-specific Usage IDs (Usage Page 0x0D)
const USAGE_TIP_SWITCH: u8 = 0x42; // Pen touching surface
const USAGE_BARREL_SWITCH: u8 = 0x44; // Side button (right-click)
const USAGE_ERASER: u8 = 0x45; // Eraser mode
const USAGE_IN_RANGE: u8 = 0x32; // Pen is hovering over tablet
const USAGE_TIP_PRESSURE: u8 = 0x30; // Pressure (0..max)
const USAGE_X_TILT: u8 = 0x3D; // Tilt angle X
const USAGE_Y_TILT: u8 = 0x3E; // Tilt angle Y

/// HID Usage Page: Generic Desktop (0x01) — X/Y coordinates
const USAGE_PAGE_GENERIC_DESKTOP: u16 = 0x01;
const USAGE_X: u8 = 0x30;
const USAGE_Y: u8 = 0x31;

/// Describes where a field lives in the HID report.
#[derive(Debug, Clone, Copy, Default)]
struct ReportField {
    /// Bit offset from start of report.
    bit_offset: u16,
    /// Size in bits.
    bit_size: u8,
    /// Logical minimum.
    logical_min: i32,
    /// Logical maximum.
    logical_max: i32,
    /// True if this field was found in the descriptor.
    present: bool,
}

/// Parsed layout of a digitizer's HID report.
#[derive(Debug, Clone, Default)]
pub struct DigitizerLayout {
    x: ReportField,
    y: ReportField,
    pressure: ReportField,
    tip_switch: ReportField,
    barrel_switch: ReportField,
    eraser: ReportField,
    in_range: ReportField,
    x_tilt: ReportField,
    y_tilt: ReportField,
    /// Total report size in bytes.
    pub report_size_bytes: u16,
}

/// A digitizer event produced from an HID report.
#[derive(Debug, Clone, Copy)]
pub struct DigitizerEvent {
    /// Absolute X coordinate (0..max_x).
    pub x: u32,
    /// Absolute Y coordinate (0..max_y).
    pub y: u32,
    /// Pen pressure (0..max_pressure, 0 = no contact).
    pub pressure: u16,
    /// Pen tip is touching the surface.
    pub tip_down: bool,
    /// Barrel button (side button) pressed.
    pub barrel: bool,
    /// Eraser mode active.
    pub eraser: bool,
    /// Pen is within detection range (hovering or touching).
    pub in_range: bool,
    /// X tilt in degrees (-90..90, 0 if not supported).
    pub x_tilt: i16,
    /// Y tilt in degrees (-90..90, 0 if not supported).
    pub y_tilt: i16,
    /// Maximum X value (for coordinate scaling).
    pub max_x: u32,
    /// Maximum Y value (for coordinate scaling).
    pub max_y: u32,
    /// Maximum pressure value.
    pub max_pressure: u16,
}

/// Registered digitizer device.
struct DigitizerDevice {
    usb_address: u8,
    layout: DigitizerLayout,
}

static DIGITIZERS: Spinlock<Vec<DigitizerDevice>> = Spinlock::new(Vec::new());

/// Event buffer for digitizer events (consumed by compositor).
static EVENT_BUF: Spinlock<Vec<DigitizerEvent>> = Spinlock::new(Vec::new());

/// Parse a HID Report Descriptor to extract digitizer field layout.
/// This is a simplified parser that handles the most common descriptor patterns
/// from Wacom, XP-Pen, and Huion tablets.
pub fn parse_report_descriptor(desc: &[u8]) -> Option<DigitizerLayout> {
    let mut layout = DigitizerLayout::default();
    let mut usage_page: u16 = 0;
    let mut usage: u8 = 0;
    let mut logical_min: i32 = 0;
    let mut logical_max: i32 = 0;
    let mut report_size: u8 = 0; // bits per field
    let mut report_count: u8 = 0;
    let mut bit_offset: u16 = 0;
    let mut is_digitizer_collection = false;

    let mut i = 0;
    while i < desc.len() {
        let prefix = desc[i];
        let bsize = match prefix & 0x03 {
            0 => 0,
            1 => 1,
            2 => 2,
            3 => 4,
            _ => 0,
        };
        let btype = (prefix >> 2) & 0x03; // 0=Main, 1=Global, 2=Local
        let btag = (prefix >> 4) & 0x0F;

        if i + 1 + bsize > desc.len() {
            break;
        }

        let data = match bsize {
            0 => 0u32,
            1 => desc[i + 1] as u32,
            2 => u16::from_le_bytes([desc[i + 1], desc[i + 2]]) as u32,
            4 => u32::from_le_bytes([desc[i + 1], desc[i + 2], desc[i + 3], desc[i + 4]]),
            _ => 0,
        };

        match (btype, btag) {
            // Global items
            (1, 0) => usage_page = data as u16,  // Usage Page
            (1, 1) => logical_min = data as i32, // Logical Minimum
            (1, 2) => logical_max = data as i32, // Logical Maximum
            (1, 7) => report_size = data as u8,  // Report Size (bits)
            (1, 9) => report_count = data as u8, // Report Count

            // Local items
            (2, 0) => usage = data as u8, // Usage

            // Main items
            (0, 8) => {
                // Input
                let is_variable = data & 0x02 != 0; // Bit 1 = Variable (vs Array)
                if is_variable && is_digitizer_collection {
                    // Map each field to our layout
                    for _ in 0..report_count {
                        let field = ReportField {
                            bit_offset,
                            bit_size: report_size,
                            logical_min,
                            logical_max,
                            present: true,
                        };

                        if usage_page == USAGE_PAGE_GENERIC_DESKTOP {
                            match usage {
                                USAGE_X => layout.x = field,
                                USAGE_Y => layout.y = field,
                                _ => {}
                            }
                        } else if usage_page == USAGE_PAGE_DIGITIZER {
                            match usage {
                                USAGE_TIP_SWITCH => layout.tip_switch = field,
                                USAGE_BARREL_SWITCH => layout.barrel_switch = field,
                                USAGE_ERASER => layout.eraser = field,
                                USAGE_IN_RANGE => layout.in_range = field,
                                USAGE_TIP_PRESSURE => layout.pressure = field,
                                USAGE_X_TILT => layout.x_tilt = field,
                                USAGE_Y_TILT => layout.y_tilt = field,
                                _ => {}
                            }
                        }

                        bit_offset += report_size as u16;
                    }
                } else {
                    // Constant/padding or array — just skip bits
                    bit_offset += (report_size as u16) * (report_count as u16);
                }
                usage = 0; // Reset local usage after Input
            }
            (0, 10) => {
                // Collection
                if usage_page == USAGE_PAGE_DIGITIZER {
                    is_digitizer_collection = true;
                }
            }
            (0, 12) => { // End Collection
                 // Don't clear is_digitizer_collection — some descriptors nest
            }
            _ => {}
        }

        i += 1 + bsize;
    }

    layout.report_size_bytes = (bit_offset + 7) / 8;

    // Need at least X and Y to be useful
    if layout.x.present && layout.y.present {
        Some(layout)
    } else {
        None
    }
}

/// Extract a field value from a raw HID report.
fn extract_field(report: &[u8], field: &ReportField) -> i32 {
    if !field.present {
        return 0;
    }

    let byte_offset = (field.bit_offset / 8) as usize;
    let bit_in_byte = field.bit_offset % 8;
    let bits = field.bit_size as u32;

    if byte_offset >= report.len() {
        return 0;
    }

    // Read up to 4 bytes starting at byte_offset
    let mut raw: u32 = 0;
    for b in 0..4usize {
        if byte_offset + b < report.len() {
            raw |= (report[byte_offset + b] as u32) << (b * 8);
        }
    }

    // Shift and mask
    raw >>= bit_in_byte;
    let mask = (1u32 << bits) - 1;
    let value = raw & mask;

    // Sign-extend if logical_min is negative
    if field.logical_min < 0 {
        let sign_bit = 1u32 << (bits - 1);
        if value & sign_bit != 0 {
            return (value | !mask) as i32;
        }
    }

    value as i32
}

/// Parse a raw HID report from a digitizer device.
pub fn parse_report(layout: &DigitizerLayout, report: &[u8]) -> DigitizerEvent {
    DigitizerEvent {
        x: extract_field(report, &layout.x).max(0) as u32,
        y: extract_field(report, &layout.y).max(0) as u32,
        pressure: extract_field(report, &layout.pressure).max(0) as u16,
        tip_down: extract_field(report, &layout.tip_switch) != 0,
        barrel: extract_field(report, &layout.barrel_switch) != 0,
        eraser: extract_field(report, &layout.eraser) != 0,
        in_range: extract_field(report, &layout.in_range) != 0,
        x_tilt: extract_field(report, &layout.x_tilt) as i16,
        y_tilt: extract_field(report, &layout.y_tilt) as i16,
        max_x: layout.x.logical_max.max(1) as u32,
        max_y: layout.y.logical_max.max(1) as u32,
        max_pressure: layout.pressure.logical_max.max(1) as u16,
    }
}

/// Register a digitizer device with its parsed report layout.
pub fn register(usb_address: u8, layout: DigitizerLayout) {
    crate::serial_println!(
        "[USB-Digitizer] registered addr={} (x:0-{}, y:0-{}, pressure:0-{}, report={}B)",
        usb_address,
        layout.x.logical_max,
        layout.y.logical_max,
        layout.pressure.logical_max,
        layout.report_size_bytes,
    );
    DIGITIZERS.lock().push(DigitizerDevice {
        usb_address,
        layout,
    });
}

/// Process an incoming HID report from a digitizer. Produces a DigitizerEvent
/// and queues it for the compositor.
pub fn handle_report(usb_address: u8, report: &[u8]) {
    let devs = DIGITIZERS.lock();
    if let Some(dev) = devs.iter().find(|d| d.usb_address == usb_address) {
        let event = parse_report(&dev.layout, report);

        // Inject as absolute pointer event to compositor
        if event.in_range || event.tip_down {
            let (screen_w, screen_h) = crate::drivers::framebuffer::info()
                .map(|fb| (fb.width as u64, fb.height as u64))
                .unwrap_or((1024, 768));
            let x = ((event.x as u64 * screen_w) / event.max_x.max(1) as u64) as i32;
            let y = ((event.y as u64 * screen_h) / event.max_y.max(1) as u64) as i32;
            let buttons = crate::drivers::input::mouse::MouseButtons {
                left: event.tip_down,
                right: event.barrel,
                middle: false,
            };
            crate::drivers::input::mouse::inject_absolute(x, y, buttons);
        }

        // Queue event for userspace apps that want full tablet data (pressure, tilt)
        let mut buf = EVENT_BUF.lock();
        buf.push(event);
        // Cap event buffer at 64 entries
        let excess = buf.len().saturating_sub(64);
        if excess > 0 {
            buf.drain(..excess);
        }
    }
}

/// Read pending digitizer events (called from userspace via syscall).
/// Returns events and clears the buffer.
pub fn read_events(out: &mut [DigitizerEvent]) -> usize {
    let mut buf = EVENT_BUF.lock();
    let n = buf.len().min(out.len());
    for i in 0..n {
        out[i] = buf[i];
    }
    buf.drain(..n);
    n
}

/// Check if any digitizer devices are registered.
pub fn is_available() -> bool {
    !DIGITIZERS.lock().is_empty()
}
