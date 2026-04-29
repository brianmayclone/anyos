//! ASN.1/DER parser for X.509 certificate parsing.
//!
//! Provides a Tag-Length-Value walker that can navigate through DER-encoded
//! structures to extract certificate fields needed for TLS.

/// ASN.1 tag classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TagClass {
    Universal,
    Application,
    ContextSpecific,
    Private,
}

/// A parsed ASN.1 TLV (Tag-Length-Value).
#[derive(Debug, Clone)]
pub struct Tlv<'a> {
    pub tag: u8,
    pub class: TagClass,
    pub constructed: bool,
    pub value: &'a [u8],
    /// Total bytes consumed (tag + length + value).
    pub total_len: usize,
}

/// Parse one TLV from the start of `data`.
pub fn parse_tlv(data: &[u8]) -> Option<Tlv<'_>> {
    if data.is_empty() {
        return None;
    }

    let tag_byte = data[0];
    let class = match tag_byte >> 6 {
        0 => TagClass::Universal,
        1 => TagClass::Application,
        2 => TagClass::ContextSpecific,
        3 => TagClass::Private,
        _ => unreachable!(),
    };
    let constructed = (tag_byte & 0x20) != 0;
    let tag = tag_byte & 0x1F;

    let mut pos = 1;

    // Handle multi-byte tags (tag number >= 31)
    if tag == 0x1F {
        // Skip long-form tag bytes (we just need the raw tag byte for our use)
        while pos < data.len() && (data[pos] & 0x80) != 0 {
            pos += 1;
        }
        if pos < data.len() {
            pos += 1;
        }
    }

    if pos >= data.len() {
        return None;
    }

    // Parse length
    let (length, len_bytes) = parse_der_length(&data[pos..])?;
    pos += len_bytes;

    if pos + length > data.len() {
        return None;
    }

    let value = &data[pos..pos + length];
    let total_len = pos + length;

    Some(Tlv {
        tag: tag_byte,
        class,
        constructed,
        value,
        total_len,
    })
}

/// Parse DER length encoding. Returns (length, bytes_consumed).
fn parse_der_length(data: &[u8]) -> Option<(usize, usize)> {
    if data.is_empty() {
        return None;
    }

    let first = data[0];
    if first < 0x80 {
        // Short form: length is the byte itself
        Some((first as usize, 1))
    } else if first == 0x80 {
        // Indefinite length (not valid in DER)
        None
    } else {
        // Long form: first byte tells how many bytes follow
        let num_bytes = (first & 0x7F) as usize;
        if num_bytes > 4 || num_bytes + 1 > data.len() {
            return None;
        }
        let mut length = 0usize;
        for i in 0..num_bytes {
            length = (length << 8) | (data[1 + i] as usize);
        }
        Some((length, 1 + num_bytes))
    }
}

/// Iterate over TLVs within a constructed value.
pub struct TlvIterator<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> TlvIterator<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
}

impl<'a> Iterator for TlvIterator<'a> {
    type Item = Tlv<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.data.len() {
            return None;
        }
        let tlv = parse_tlv(&self.data[self.pos..])?;
        self.pos += tlv.total_len;
        Some(tlv)
    }
}

/// Parse a DER INTEGER as unsigned big-endian bytes.
/// Strips the leading zero byte that DER uses for positive integers.
pub fn parse_der_integer(value: &[u8]) -> &[u8] {
    if value.len() > 1 && value[0] == 0x00 {
        &value[1..]
    } else {
        value
    }
}

/// Parse an OID from DER-encoded value bytes.
/// Returns the OID as a list of u32 components.
pub fn parse_oid(value: &[u8]) -> alloc::vec::Vec<u32> {
    let mut oid = alloc::vec::Vec::new();
    if value.is_empty() {
        return oid;
    }

    // First byte encodes two components: first*40 + second
    oid.push((value[0] / 40) as u32);
    oid.push((value[0] % 40) as u32);

    let mut i = 1;
    while i < value.len() {
        let mut component = 0u32;
        loop {
            if i >= value.len() {
                break;
            }
            let b = value[i];
            i += 1;
            component = (component << 7) | ((b & 0x7F) as u32);
            if b & 0x80 == 0 {
                break;
            }
        }
        oid.push(component);
    }
    oid
}

// -- ASN.1 tag constants --

pub const TAG_INTEGER: u8 = 0x02;
pub const TAG_BIT_STRING: u8 = 0x03;
pub const TAG_OCTET_STRING: u8 = 0x04;
pub const TAG_NULL: u8 = 0x05;
pub const TAG_OID: u8 = 0x06;
pub const TAG_SEQUENCE: u8 = 0x30;
pub const TAG_SET: u8 = 0x31;

// Context-specific tags (common in X.509)
pub const TAG_CONTEXT_0: u8 = 0xA0; // [0] EXPLICIT
pub const TAG_CONTEXT_1: u8 = 0xA1;
pub const TAG_CONTEXT_3: u8 = 0xA3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_integer() {
        let data = [0x02, 0x01, 0x05]; // INTEGER 5
        let tlv = parse_tlv(&data).unwrap();
        assert_eq!(tlv.tag, TAG_INTEGER);
        assert_eq!(tlv.value, &[0x05]);
        assert_eq!(tlv.total_len, 3);
    }

    #[test]
    fn test_parse_sequence() {
        // SEQUENCE { INTEGER 1, INTEGER 2 }
        let data = [0x30, 0x06, 0x02, 0x01, 0x01, 0x02, 0x01, 0x02];
        let tlv = parse_tlv(&data).unwrap();
        assert_eq!(tlv.tag, TAG_SEQUENCE);
        assert!(tlv.constructed);
        assert_eq!(tlv.value.len(), 6);

        // Iterate children
        let children: alloc::vec::Vec<_> = TlvIterator::new(tlv.value).collect();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].value, &[0x01]);
        assert_eq!(children[1].value, &[0x02]);
    }

    #[test]
    fn test_parse_long_length() {
        // Tag + long length (2 bytes for 256)
        let mut data = alloc::vec![0x04, 0x82, 0x01, 0x00]; // OCTET STRING, length 256
        data.extend_from_slice(&[0xAA; 256]);
        let tlv = parse_tlv(&data).unwrap();
        assert_eq!(tlv.value.len(), 256);
    }

    #[test]
    fn test_parse_oid() {
        // OID 1.2.840.113549.1.1.11 (sha256WithRSAEncryption)
        let oid_bytes = [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x01, 0x0B];
        let oid = parse_oid(&oid_bytes);
        assert_eq!(oid, alloc::vec![1, 2, 840, 113549, 1, 1, 11]);
    }
}
