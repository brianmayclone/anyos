//! EDID (Extended Display Identification Data) parser.
//!
//! Parses the 128-byte EDID base block (VESA E-EDID Standard 1.4) to extract
//! monitor properties: manufacturer, model, serial, physical size, gamma,
//! chromaticity coordinates, color depth, and supported display modes.

/// Maximum number of display modes extracted from a single EDID.
pub const MAX_MODES: usize = 32;

/// EDID header signature: `00 FF FF FF FF FF FF 00`.
const EDID_HEADER: [u8; 8] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];

// ──────────────────────────────────────────────
// Structs
// ──────────────────────────────────────────────

/// A supported display resolution with refresh rate.
#[derive(Clone, Copy, Default)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    /// Refresh rate × 100 (e.g. 6000 = 60.00 Hz).
    pub refresh_hz_100: u32,
    pub is_preferred: bool,
    pub is_interlaced: bool,
}

/// CIE 1931 chromaticity coordinates for the monitor's color primaries and white point.
/// Each value is a 10-bit fixed-point number (0..1023 maps to 0.0..~1.0).
#[derive(Clone, Copy, Default)]
pub struct ChromaticityCoords {
    pub red_x: u16,
    pub red_y: u16,
    pub green_x: u16,
    pub green_y: u16,
    pub blue_x: u16,
    pub blue_y: u16,
    pub white_x: u16,
    pub white_y: u16,
}

/// A standard timing entry (2 bytes in EDID bytes 38-53).
#[derive(Clone, Copy, Default)]
pub struct StandardTiming {
    pub width: u16,
    pub height: u16,
    /// Vertical refresh rate in Hz.
    pub refresh: u8,
    pub valid: bool,
}

/// A detailed timing descriptor (18 bytes, EDID bytes 54-125, 4 entries).
#[derive(Clone, Copy, Default)]
pub struct DetailedTiming {
    /// Pixel clock in kHz (bytes 0-1 × 10).
    pub pixel_clock_khz: u32,
    pub h_active: u16,
    pub h_blank: u16,
    pub v_active: u16,
    pub v_blank: u16,
    pub h_sync_offset: u16,
    pub h_sync_width: u16,
    pub v_sync_offset: u8,
    pub v_sync_width: u8,
    pub width_mm: u16,
    pub height_mm: u16,
    pub interlaced: bool,
    pub valid: bool,
}

/// Parsed EDID base block (128 bytes). All fields are fixed-size, no allocations.
#[derive(Clone)]
pub struct ParsedEdid {
    /// `true` if header and checksum are valid.
    pub valid: bool,
    /// 3-letter PNP manufacturer ID + NUL terminator.
    pub manufacturer: [u8; 4],
    /// EDID product code (little-endian u16).
    pub product_code: u16,
    /// 32-bit serial number from EDID header.
    pub serial_number: u32,
    /// Week of manufacture (1-54, or 0 if unspecified).
    pub manufacture_week: u8,
    /// Year of manufacture (byte 17 + 1990).
    pub manufacture_year: u16,
    /// EDID structure version (typically 1).
    pub edid_version: u8,
    /// EDID structure revision (typically 3 or 4).
    pub edid_revision: u8,
    /// `true` if the display input is digital (DVI/HDMI/DP).
    pub is_digital: bool,
    /// Bits per primary color (6, 8, 10, 12, 16), or 0 for analog/undefined.
    pub color_depth: u8,
    /// Horizontal screen size in cm (0 if undefined).
    pub width_cm: u8,
    /// Vertical screen size in cm (0 if undefined).
    pub height_cm: u8,
    /// Display gamma × 100 (e.g. 220 = 2.20). 0 if defined in extension block.
    pub gamma: u16,
    /// CIE chromaticity coordinates for color primaries and white point.
    pub chromaticity: ChromaticityCoords,
    /// Monitor model name from descriptor tag 0xFC (NUL-terminated).
    pub model_name: [u8; 14],
    /// Monitor serial string from descriptor tag 0xFF (NUL-terminated).
    pub serial_string: [u8; 14],
    /// Preferred (native) resolution width from the first detailed timing.
    pub preferred_width: u32,
    /// Preferred (native) resolution height from the first detailed timing.
    pub preferred_height: u32,
    /// Preferred refresh rate × 100.
    pub preferred_refresh_100: u32,
    /// Preferred timing is interlaced.
    pub preferred_interlaced: bool,
    /// Established timings bitmap (bytes 35-37).
    pub established_modes: u32,
    /// 8 standard timing entries.
    pub standard_timings: [StandardTiming; 8],
    /// 4 detailed timing descriptors.
    pub detailed_timings: [DetailedTiming; 4],
    /// Number of 128-byte extension blocks following the base block.
    pub extension_count: u8,
}

impl Default for ParsedEdid {
    fn default() -> Self {
        Self {
            valid: false,
            manufacturer: [0; 4],
            product_code: 0,
            serial_number: 0,
            manufacture_week: 0,
            manufacture_year: 0,
            edid_version: 0,
            edid_revision: 0,
            is_digital: false,
            color_depth: 0,
            width_cm: 0,
            height_cm: 0,
            gamma: 0,
            chromaticity: ChromaticityCoords::default(),
            model_name: [0; 14],
            serial_string: [0; 14],
            preferred_width: 0,
            preferred_height: 0,
            preferred_refresh_100: 0,
            preferred_interlaced: false,
            established_modes: 0,
            standard_timings: [StandardTiming::default(); 8],
            detailed_timings: [DetailedTiming::default(); 4],
            extension_count: 0,
        }
    }
}

// ──────────────────────────────────────────────
// Checksum + header validation
// ──────────────────────────────────────────────

/// Validate EDID checksum: the sum of all 128 bytes must be 0 (mod 256).
pub fn validate_checksum(raw: &[u8; 128]) -> bool {
    let sum: u8 = raw.iter().fold(0u8, |acc, &b| acc.wrapping_add(b));
    sum == 0
}

/// Check the 8-byte EDID header signature.
fn validate_header(raw: &[u8; 128]) -> bool {
    raw[0..8] == EDID_HEADER
}

// ──────────────────────────────────────────────
// Field decoders
// ──────────────────────────────────────────────

/// Decode the 3-letter PNP manufacturer ID from compressed 5-bit ASCII (bytes 8-9).
fn decode_manufacturer(b8: u8, b9: u8) -> [u8; 4] {
    let c1 = ((b8 >> 2) & 0x1F) + b'A' - 1;
    let c2 = (((b8 & 0x03) << 3) | ((b9 >> 5) & 0x07)) + b'A' - 1;
    let c3 = (b9 & 0x1F) + b'A' - 1;
    [c1, c2, c3, 0]
}

/// Parse chromaticity coordinates from EDID bytes 25-34.
///
/// Bytes 25-26 contain the 2 least-significant bits of each coordinate,
/// bytes 27-34 contain the 8 most-significant bits.
fn parse_chromaticity(raw: &[u8]) -> ChromaticityCoords {
    // Byte 25: Red-x[1:0], Red-y[1:0], Green-x[1:0], Green-y[1:0]
    // Byte 26: Blue-x[1:0], Blue-y[1:0], White-x[1:0], White-y[1:0]
    let rx_lo = (raw[25] >> 6) & 0x03;
    let ry_lo = (raw[25] >> 4) & 0x03;
    let gx_lo = (raw[25] >> 2) & 0x03;
    let gy_lo = raw[25] & 0x03;
    let bx_lo = (raw[26] >> 6) & 0x03;
    let by_lo = (raw[26] >> 4) & 0x03;
    let wx_lo = (raw[26] >> 2) & 0x03;
    let wy_lo = raw[26] & 0x03;

    ChromaticityCoords {
        red_x:   ((raw[27] as u16) << 2) | rx_lo as u16,
        red_y:   ((raw[28] as u16) << 2) | ry_lo as u16,
        green_x: ((raw[29] as u16) << 2) | gx_lo as u16,
        green_y: ((raw[30] as u16) << 2) | gy_lo as u16,
        blue_x:  ((raw[31] as u16) << 2) | bx_lo as u16,
        blue_y:  ((raw[32] as u16) << 2) | by_lo as u16,
        white_x: ((raw[33] as u16) << 2) | wx_lo as u16,
        white_y: ((raw[34] as u16) << 2) | wy_lo as u16,
    }
}

/// Parse color depth from byte 20 (digital input only, EDID 1.4+).
fn parse_color_depth(byte20: u8) -> u8 {
    if byte20 & 0x80 == 0 {
        return 0; // analog input — no color depth info
    }
    match (byte20 >> 4) & 0x07 {
        1 => 6,
        2 => 8,
        3 => 10,
        4 => 12,
        5 => 16,
        _ => 0, // undefined
    }
}

/// Parse a single 18-byte detailed timing descriptor.
fn parse_detailed_timing(desc: &[u8]) -> DetailedTiming {
    let pixel_clock_10khz = (desc[0] as u32) | ((desc[1] as u32) << 8);
    if pixel_clock_10khz == 0 {
        // This is a monitor descriptor, not a timing.
        return DetailedTiming::default();
    }

    let h_active = ((desc[4] as u16 & 0xF0) << 4) | desc[2] as u16;
    let h_blank  = ((desc[4] as u16 & 0x0F) << 8) | desc[3] as u16;
    let v_active = ((desc[7] as u16 & 0xF0) << 4) | desc[5] as u16;
    let v_blank  = ((desc[7] as u16 & 0x0F) << 8) | desc[6] as u16;

    let h_sync_offset = ((desc[11] as u16 & 0xC0) << 2) | desc[8] as u16;
    let h_sync_width  = ((desc[11] as u16 & 0x30) << 4) | desc[9] as u16;
    let v_sync_offset = ((desc[11] & 0x0C) << 2) | ((desc[10] >> 4) & 0x0F);
    let v_sync_width  = ((desc[11] & 0x03) << 4) | (desc[10] & 0x0F);

    let width_mm  = ((desc[14] as u16 & 0xF0) << 4) | desc[12] as u16;
    let height_mm = ((desc[14] as u16 & 0x0F) << 8) | desc[13] as u16;

    let interlaced = desc[17] & 0x80 != 0;

    DetailedTiming {
        pixel_clock_khz: pixel_clock_10khz * 10,
        h_active,
        h_blank,
        v_active,
        v_blank,
        h_sync_offset,
        h_sync_width,
        v_sync_offset,
        v_sync_width,
        width_mm,
        height_mm,
        interlaced,
        valid: true,
    }
}

/// Calculate refresh rate (Hz × 100) from a detailed timing descriptor.
fn calc_refresh_100(dt: &DetailedTiming) -> u32 {
    let h_total = dt.h_active as u32 + dt.h_blank as u32;
    let v_total = dt.v_active as u32 + dt.v_blank as u32;
    if h_total == 0 || v_total == 0 {
        return 0;
    }
    // pixel_clock_khz * 1000 = pixels/sec
    // refresh = pixels_per_sec / (h_total * v_total)
    // refresh_100 = pixels_per_sec * 100 / (h_total * v_total)
    let pixels_per_sec = dt.pixel_clock_khz as u64 * 1000;
    let total_pixels = h_total as u64 * v_total as u64;
    ((pixels_per_sec * 100) / total_pixels) as u32
}

/// Parse a standard timing entry (2 bytes). Returns `None` if the entry is unused.
fn parse_standard_timing(b0: u8, b1: u8) -> Option<StandardTiming> {
    // Unused entries are 0x01 0x01 or 0x00 0x00.
    if (b0 == 0x01 && b1 == 0x01) || (b0 == 0x00 && b1 == 0x00) {
        return None;
    }
    let width = ((b0 as u16) + 31) * 8;
    let aspect = (b1 >> 6) & 0x03;
    let height = match aspect {
        0 => width * 10 / 16, // 16:10
        1 => width * 3 / 4,   // 4:3
        2 => width * 4 / 5,   // 5:4
        3 => width * 9 / 16,  // 16:9
        _ => width * 3 / 4,
    };
    let refresh = (b1 & 0x3F) + 60;
    Some(StandardTiming {
        width,
        height,
        refresh,
        valid: true,
    })
}

/// Extract a monitor descriptor string (model name or serial) from an 18-byte descriptor.
/// The string occupies bytes 5-17 (13 chars), padded with 0x0A (LF) and trailing spaces.
fn extract_descriptor_string(desc: &[u8]) -> [u8; 14] {
    let mut out = [0u8; 14];
    let mut len = 0;
    for i in 5..18 {
        let c = desc[i];
        if c == 0x0A || c == 0x00 {
            break;
        }
        if len < 13 {
            out[len] = c;
            len += 1;
        }
    }
    // Trim trailing spaces.
    while len > 0 && out[len - 1] == b' ' {
        len -= 1;
        out[len] = 0;
    }
    out
}

// ──────────────────────────────────────────────
// Established timings bitmap
// ──────────────────────────────────────────────

/// Well-known resolutions for the established timings bitmap (bytes 35-37).
/// Each entry: (bit_index, width, height, refresh).
static ESTABLISHED_TIMINGS: [(u8, u32, u32, u32); 17] = [
    // Byte 35 (Established Timings I)
    (0,  720,  400, 70),  // bit 7
    (1,  720,  400, 88),  // bit 6
    (2,  640,  480, 60),  // bit 5
    (3,  640,  480, 67),  // bit 4
    (4,  640,  480, 72),  // bit 3
    (5,  640,  480, 75),  // bit 2
    (6,  800,  600, 56),  // bit 1
    (7,  800,  600, 60),  // bit 0
    // Byte 36 (Established Timings II)
    (8,  800,  600, 72),  // bit 7
    (9,  800,  600, 75),  // bit 6
    (10, 832,  624, 75),  // bit 5
    (11, 1024, 768, 87),  // bit 4 (interlaced)
    (12, 1024, 768, 60),  // bit 3
    (13, 1024, 768, 70),  // bit 2
    (14, 1024, 768, 75),  // bit 1
    (15, 1280, 1024, 75), // bit 0
    // Byte 37 (Manufacturer's Timings)
    (16, 1152, 870, 75),  // bit 7
];

// ──────────────────────────────────────────────
// Main parse function
// ──────────────────────────────────────────────

/// Parse a 128-byte EDID base block.
///
/// Returns a [`ParsedEdid`] with `valid = true` if header and checksum pass.
/// On checksum/header failure, fields are still populated where possible
/// but `valid` is set to `false`.
pub fn parse_edid_base(raw: &[u8; 128]) -> ParsedEdid {
    let mut edid = ParsedEdid::default();

    let header_ok = validate_header(raw);
    let checksum_ok = validate_checksum(raw);
    edid.valid = header_ok && checksum_ok;

    // Even if invalid, try to extract as much as possible.

    // Manufacturer (bytes 8-9)
    edid.manufacturer = decode_manufacturer(raw[8], raw[9]);

    // Product code (bytes 10-11, little-endian)
    edid.product_code = (raw[10] as u16) | ((raw[11] as u16) << 8);

    // Serial number (bytes 12-15, little-endian)
    edid.serial_number = (raw[12] as u32)
        | ((raw[13] as u32) << 8)
        | ((raw[14] as u32) << 16)
        | ((raw[15] as u32) << 24);

    // Manufacture date (bytes 16-17)
    edid.manufacture_week = raw[16];
    edid.manufacture_year = raw[17] as u16 + 1990;

    // Version (bytes 18-19)
    edid.edid_version = raw[18];
    edid.edid_revision = raw[19];

    // Video input definition (byte 20)
    edid.is_digital = raw[20] & 0x80 != 0;
    edid.color_depth = parse_color_depth(raw[20]);

    // Physical size (bytes 21-22)
    edid.width_cm = raw[21];
    edid.height_cm = raw[22];

    // Gamma (byte 23): stored as (gamma × 100) - 100, so 0xFF means defined in extension
    if raw[23] != 0xFF {
        edid.gamma = raw[23] as u16 + 100;
    }

    // Chromaticity (bytes 25-34)
    edid.chromaticity = parse_chromaticity(raw);

    // Established timings (bytes 35-37)
    edid.established_modes = ((raw[35] as u32) << 16)
        | ((raw[36] as u32) << 8)
        | (raw[37] as u32);

    // Standard timings (bytes 38-53, 8 × 2 bytes)
    for i in 0..8 {
        let offset = 38 + i * 2;
        if let Some(st) = parse_standard_timing(raw[offset], raw[offset + 1]) {
            edid.standard_timings[i] = st;
        }
    }

    // Detailed timing descriptors (bytes 54-125, 4 × 18 bytes)
    let mut preferred_set = false;
    for i in 0..4 {
        let offset = 54 + i * 18;
        let desc = &raw[offset..offset + 18];

        // Check if this is a timing descriptor (pixel clock != 0) or monitor descriptor.
        let pixel_clock = (desc[0] as u16) | ((desc[1] as u16) << 8);
        if pixel_clock != 0 {
            let dt = parse_detailed_timing(desc);
            edid.detailed_timings[i] = dt;

            // First valid detailed timing is the preferred/native mode.
            if dt.valid && !preferred_set {
                edid.preferred_width = dt.h_active as u32;
                edid.preferred_height = dt.v_active as u32;
                edid.preferred_refresh_100 = calc_refresh_100(&dt);
                edid.preferred_interlaced = dt.interlaced;
                preferred_set = true;
            }
        } else {
            // Monitor descriptor: check tag byte (byte 3).
            match desc[3] {
                0xFC => edid.model_name = extract_descriptor_string(desc),
                0xFF => edid.serial_string = extract_descriptor_string(desc),
                // 0xFD = monitor range limits, 0xFE = ASCII string — ignored for now
                _ => {}
            }
        }
    }

    // Extension count (byte 126)
    edid.extension_count = raw[126];

    edid
}

// ──────────────────────────────────────────────
// Mode extraction
// ──────────────────────────────────────────────

/// Extract all supported display modes from a parsed EDID.
///
/// Returns an array of up to [`MAX_MODES`] entries and the actual count.
/// The preferred mode (from the first detailed timing) comes first.
pub fn extract_modes(edid: &ParsedEdid) -> ([DisplayMode; MAX_MODES], usize) {
    let mut modes = [DisplayMode::default(); MAX_MODES];
    let mut count = 0;

    // 1. Detailed timings (preferred first)
    for dt in &edid.detailed_timings {
        if !dt.valid || count >= MAX_MODES {
            continue;
        }
        let is_first = count == 0;
        modes[count] = DisplayMode {
            width: dt.h_active as u32,
            height: dt.v_active as u32,
            refresh_hz_100: calc_refresh_100(dt),
            is_preferred: is_first,
            is_interlaced: dt.interlaced,
        };
        count += 1;
    }

    // 2. Standard timings
    for st in &edid.standard_timings {
        if !st.valid || count >= MAX_MODES {
            continue;
        }
        // Skip duplicates of detailed timings.
        let w = st.width as u32;
        let h = st.height as u32;
        if modes[..count].iter().any(|m| m.width == w && m.height == h) {
            continue;
        }
        modes[count] = DisplayMode {
            width: w,
            height: h,
            refresh_hz_100: st.refresh as u32 * 100,
            is_preferred: false,
            is_interlaced: false,
        };
        count += 1;
    }

    // 3. Established timings
    let bitmap = edid.established_modes;
    for &(bit, w, h, refresh) in &ESTABLISHED_TIMINGS {
        if count >= MAX_MODES {
            break;
        }
        // Bits are stored MSB-first: bit 0 in our table = bit 23 of the u32.
        let mask = 1u32 << (23 - bit as u32);
        if bitmap & mask == 0 {
            continue;
        }
        // Skip duplicates.
        if modes[..count].iter().any(|m| m.width == w && m.height == h) {
            continue;
        }
        modes[count] = DisplayMode {
            width: w,
            height: h,
            refresh_hz_100: refresh * 100,
            is_preferred: false,
            is_interlaced: bit == 11, // 1024x768@87i
        };
        count += 1;
    }

    (modes, count)
}
