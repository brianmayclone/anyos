//! NTFS attribute type definitions and header parsing.
//!
//! MFT records contain a chain of attributes. Each attribute has a common
//! header followed by either resident (inline) data or non-resident data
//! described by data runs.

/// Well-known NTFS attribute types.
pub(super) mod types {
    pub const STANDARD_INFORMATION: u32 = 0x10;
    pub const ATTRIBUTE_LIST: u32 = 0x20;
    pub const FILE_NAME: u32 = 0x30;
    pub const DATA: u32 = 0x80;
    pub const INDEX_ROOT: u32 = 0x90;
    pub const INDEX_ALLOCATION: u32 = 0xA0;
    pub const BITMAP: u32 = 0xB0;
    pub const END_MARKER: u32 = 0xFFFFFFFF;
}

/// Common attribute header fields.
#[derive(Debug, Clone)]
pub(super) struct AttrHeader {
    pub attr_type: u32,
    pub length: u32,
    pub non_resident: bool,
    pub name_length: u8,
    pub name_offset: u16,
    pub flags: u16,
    pub attr_id: u16,
}

/// Resident attribute: data is inline in the MFT record.
#[derive(Debug, Clone)]
pub(super) struct ResidentData {
    pub data_offset: u16,
    pub data_length: u32,
}

/// Non-resident attribute: data is described by data runs on disk.
#[derive(Debug, Clone)]
pub(super) struct NonResidentData {
    pub lowest_vcn: u64,
    pub highest_vcn: u64,
    pub data_runs_offset: u16,
    pub allocated_size: u64,
    pub real_size: u64,
    pub initialized_size: u64,
}

/// Parsed attribute with header + resident/non-resident specifics.
#[derive(Debug, Clone)]
pub(super) struct NtfsAttr {
    pub header: AttrHeader,
    pub resident: Option<ResidentData>,
    pub non_resident: Option<NonResidentData>,
    /// Attribute name (UTF-16LE, usually empty for unnamed $DATA).
    pub name: Option<alloc::string::String>,
    /// Offset of this attribute within the MFT record buffer.
    pub record_offset: usize,
}

impl NtfsAttr {
    /// Check if this is the unnamed default attribute (e.g. unnamed $DATA).
    pub fn is_unnamed(&self) -> bool {
        self.header.name_length == 0
    }
}

/// Parse all attributes from an MFT record buffer (after fixup applied).
///
/// `attr_offset` is the offset of the first attribute (from MFT header field at 0x14).
pub(super) fn parse_attributes(record: &[u8], attr_offset: u16) -> alloc::vec::Vec<NtfsAttr> {
    let mut attrs = alloc::vec::Vec::new();
    let mut offset = attr_offset as usize;

    loop {
        if offset + 4 > record.len() {
            break;
        }

        let attr_type = u32::from_le_bytes([
            record[offset], record[offset + 1], record[offset + 2], record[offset + 3],
        ]);

        if attr_type == types::END_MARKER || attr_type == 0 {
            break;
        }

        if offset + 16 > record.len() {
            break;
        }

        let length = u32::from_le_bytes([
            record[offset + 4], record[offset + 5], record[offset + 6], record[offset + 7],
        ]);

        if length < 16 || length as usize > record.len() - offset {
            break;
        }

        let non_resident = record[offset + 8] != 0;
        let name_length = record[offset + 9];
        let name_offset = u16::from_le_bytes([record[offset + 10], record[offset + 11]]);
        let flags = u16::from_le_bytes([record[offset + 12], record[offset + 13]]);
        let attr_id = u16::from_le_bytes([record[offset + 14], record[offset + 15]]);

        let header = AttrHeader {
            attr_type,
            length,
            non_resident,
            name_length,
            name_offset,
            flags,
            attr_id,
        };

        // Reject compressed/encrypted attributes
        if flags & 0x0001 != 0 || flags & 0x4000 != 0 {
            offset += length as usize;
            continue;
        }

        let (resident, non_res) = if non_resident {
            if offset + 64 > record.len() {
                break;
            }
            let nr = NonResidentData {
                lowest_vcn: u64::from_le_bytes([
                    record[offset + 16], record[offset + 17],
                    record[offset + 18], record[offset + 19],
                    record[offset + 20], record[offset + 21],
                    record[offset + 22], record[offset + 23],
                ]),
                highest_vcn: u64::from_le_bytes([
                    record[offset + 24], record[offset + 25],
                    record[offset + 26], record[offset + 27],
                    record[offset + 28], record[offset + 29],
                    record[offset + 30], record[offset + 31],
                ]),
                data_runs_offset: u16::from_le_bytes([record[offset + 32], record[offset + 33]]),
                allocated_size: u64::from_le_bytes([
                    record[offset + 40], record[offset + 41],
                    record[offset + 42], record[offset + 43],
                    record[offset + 44], record[offset + 45],
                    record[offset + 46], record[offset + 47],
                ]),
                real_size: u64::from_le_bytes([
                    record[offset + 48], record[offset + 49],
                    record[offset + 50], record[offset + 51],
                    record[offset + 52], record[offset + 53],
                    record[offset + 54], record[offset + 55],
                ]),
                initialized_size: u64::from_le_bytes([
                    record[offset + 56], record[offset + 57],
                    record[offset + 58], record[offset + 59],
                    record[offset + 60], record[offset + 61],
                    record[offset + 62], record[offset + 63],
                ]),
            };
            (None, Some(nr))
        } else {
            if offset + 24 > record.len() {
                break;
            }
            let res = ResidentData {
                data_length: u32::from_le_bytes([
                    record[offset + 16], record[offset + 17],
                    record[offset + 18], record[offset + 19],
                ]),
                data_offset: u16::from_le_bytes([record[offset + 20], record[offset + 21]]),
            };
            (Some(res), None)
        };

        // Parse attribute name (UTF-16LE)
        let name = if name_length > 0 {
            let name_start = offset + name_offset as usize;
            let name_end = name_start + name_length as usize * 2;
            if name_end <= record.len() {
                let mut s = alloc::string::String::new();
                for i in 0..name_length as usize {
                    let c = u16::from_le_bytes([
                        record[name_start + i * 2],
                        record[name_start + i * 2 + 1],
                    ]);
                    if c < 128 {
                        s.push(c as u8 as char);
                    } else {
                        s.push('?');
                    }
                }
                Some(s)
            } else {
                None
            }
        } else {
            None
        };

        attrs.push(NtfsAttr {
            header,
            resident,
            non_resident: non_res,
            name,
            record_offset: offset,
        });

        offset += length as usize;
    }

    attrs
}

/// Get resident data bytes from an attribute within the record buffer.
pub(super) fn get_resident_data<'a>(record: &'a [u8], attr: &NtfsAttr) -> Option<&'a [u8]> {
    let res = attr.resident.as_ref()?;
    let start = attr.record_offset + res.data_offset as usize;
    let end = start + res.data_length as usize;
    if end <= record.len() {
        Some(&record[start..end])
    } else {
        None
    }
}
