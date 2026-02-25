//! ico.pak reader — binary search for icons by name.
//!
//! Supports two formats:
//! - **v1** (18B header): SVG path strings, rendered at runtime
//! - **v2** (20B header): Pre-rasterized alpha maps, color applied at runtime
//!
//! Filled icons are stored first, then outline icons, both sorted by name.

/// Result of looking up an icon in a v1 pak (SVG path strings).
pub struct IconEntry<'a> {
    pub path_count: u16,
    /// Raw SVG path d="" strings, multiple paths separated by \0.
    pub data: &'a [u8],
}

/// Result of looking up an icon in a v2 pak (pre-rasterized alpha map).
pub struct IconEntryV2<'a> {
    /// Alpha map data (icon_size × icon_size bytes, u8 per pixel).
    pub alpha: &'a [u8],
    /// Pre-rendered icon size in pixels (square).
    pub icon_size: u16,
}

/// Detect the IPAK version. Returns 0 if invalid.
pub fn version(pak: &[u8]) -> u16 {
    if pak.len() < 6 || &pak[0..4] != b"IPAK" {
        return 0;
    }
    u16_le(&pak[4..6])
}

/// Look up an icon by name in a v2 ico.pak buffer.
///
/// Returns the icon's pre-rasterized alpha map if found.
pub fn lookup_v2<'a>(pak: &'a [u8], name: &[u8], filled: bool) -> Option<IconEntryV2<'a>> {
    if pak.len() < 20 || &pak[0..4] != b"IPAK" || u16_le(&pak[4..6]) != 2 {
        return None;
    }

    let filled_count = u16_le(&pak[6..8]) as usize;
    let outline_count = u16_le(&pak[8..10]) as usize;
    let icon_size = u16_le(&pak[10..12]);
    let names_offset = u32_le(&pak[12..16]) as usize;
    let data_offset = u32_le(&pak[16..20]) as usize;

    let alpha_len = (icon_size as usize) * (icon_size as usize);

    let (start_idx, count) = if filled {
        (0, filled_count)
    } else {
        (filled_count, outline_count)
    };

    let entry_size = 16;
    let index_base = 20;

    let mut lo = 0usize;
    let mut hi = count;

    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let entry_off = index_base + (start_idx + mid) * entry_size;
        if entry_off + entry_size > pak.len() {
            return None;
        }

        let name_off = u32_le(&pak[entry_off..entry_off + 4]) as usize;
        let name_len = u16_le(&pak[entry_off + 4..entry_off + 6]) as usize;

        let abs_name_off = names_offset + name_off;
        if abs_name_off + name_len > pak.len() {
            return None;
        }

        let entry_name = &pak[abs_name_off..abs_name_off + name_len];

        match cmp_bytes(name, entry_name) {
            core::cmp::Ordering::Equal => {
                let d_off = u32_le(&pak[entry_off + 8..entry_off + 12]) as usize;
                let abs_data_off = data_offset + d_off;
                if abs_data_off + alpha_len > pak.len() {
                    return None;
                }

                return Some(IconEntryV2 {
                    alpha: &pak[abs_data_off..abs_data_off + alpha_len],
                    icon_size,
                });
            }
            core::cmp::Ordering::Less => hi = mid,
            core::cmp::Ordering::Greater => lo = mid + 1,
        }
    }

    None
}

/// Look up an icon by name in a v1 ico.pak buffer (SVG path strings).
pub fn lookup<'a>(pak: &'a [u8], name: &[u8], filled: bool) -> Option<IconEntry<'a>> {
    if pak.len() < 18 || &pak[0..4] != b"IPAK" {
        return None;
    }

    let filled_count = u16_le(&pak[6..8]) as usize;
    let outline_count = u16_le(&pak[8..10]) as usize;
    let names_offset = u32_le(&pak[10..14]) as usize;
    let data_offset = u32_le(&pak[14..18]) as usize;

    let (start_idx, count) = if filled {
        (0, filled_count)
    } else {
        (filled_count, outline_count)
    };

    let entry_size = 16;
    let index_base = 18;

    let mut lo = 0usize;
    let mut hi = count;

    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let entry_off = index_base + (start_idx + mid) * entry_size;
        if entry_off + entry_size > pak.len() {
            return None;
        }

        let name_off = u32_le(&pak[entry_off..entry_off + 4]) as usize;
        let name_len = u16_le(&pak[entry_off + 4..entry_off + 6]) as usize;

        let abs_name_off = names_offset + name_off;
        if abs_name_off + name_len > pak.len() {
            return None;
        }

        let entry_name = &pak[abs_name_off..abs_name_off + name_len];

        match cmp_bytes(name, entry_name) {
            core::cmp::Ordering::Equal => {
                let path_count = u16_le(&pak[entry_off + 6..entry_off + 8]);
                let d_off = u32_le(&pak[entry_off + 8..entry_off + 12]) as usize;
                let d_len = u32_le(&pak[entry_off + 12..entry_off + 16]) as usize;

                let abs_data_off = data_offset + d_off;
                if abs_data_off + d_len > pak.len() {
                    return None;
                }

                return Some(IconEntry {
                    path_count,
                    data: &pak[abs_data_off..abs_data_off + d_len],
                });
            }
            core::cmp::Ordering::Less => hi = mid,
            core::cmp::Ordering::Greater => lo = mid + 1,
        }
    }

    None
}

// ── Helpers ─────────────────────────────────────────────────────────

fn u16_le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}

fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn cmp_bytes(a: &[u8], b: &[u8]) -> core::cmp::Ordering {
    let len = a.len().min(b.len());
    for i in 0..len {
        if a[i] < b[i] { return core::cmp::Ordering::Less; }
        if a[i] > b[i] { return core::cmp::Ordering::Greater; }
    }
    a.len().cmp(&b.len())
}
