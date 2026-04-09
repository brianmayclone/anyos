//! TLS record layer (RFC 8446 Section 5, RFC 5246 Section 6).

/// TLS record content types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ContentType {
    ChangeCipherSpec = 20,
    Alert = 21,
    Handshake = 22,
    ApplicationData = 23,
}

/// TLS protocol version as sent on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolVersion {
    pub major: u8,
    pub minor: u8,
}

impl ProtocolVersion {
    pub const TLS10: Self = Self { major: 3, minor: 1 };
    pub const TLS11: Self = Self { major: 3, minor: 2 };
    pub const TLS12: Self = Self { major: 3, minor: 3 };
    /// TLS 1.3 uses 0x0303 on the wire for record layer compatibility.
    pub const TLS13_RECORD: Self = Self { major: 3, minor: 3 };
    /// TLS 1.3 uses 0x0301 in the ClientHello record for middlebox compatibility.
    pub const TLS13_COMPAT: Self = Self { major: 3, minor: 1 };
}

/// Maximum TLS record payload size (2^14 = 16384 bytes).
pub const MAX_RECORD_PAYLOAD: usize = 16384;
/// TLS record header size (type + version + length).
pub const RECORD_HEADER_SIZE: usize = 5;

/// A parsed TLS record header.
#[derive(Debug, Clone, Copy)]
pub struct RecordHeader {
    pub content_type: u8,
    pub version: ProtocolVersion,
    pub length: u16,
}

impl RecordHeader {
    /// Parse a 5-byte record header.
    pub fn parse(data: &[u8; RECORD_HEADER_SIZE]) -> Self {
        Self {
            content_type: data[0],
            version: ProtocolVersion {
                major: data[1],
                minor: data[2],
            },
            length: u16::from_be_bytes([data[3], data[4]]),
        }
    }

    /// Serialize to 5 bytes.
    pub fn to_bytes(&self) -> [u8; RECORD_HEADER_SIZE] {
        [
            self.content_type,
            self.version.major,
            self.version.minor,
            (self.length >> 8) as u8,
            (self.length & 0xFF) as u8,
        ]
    }
}
