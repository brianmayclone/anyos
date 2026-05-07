//! TLS cipher suite definitions and AEAD abstractions.

/// TLS 1.3 cipher suites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum CipherSuite {
    /// TLS_AES_128_GCM_SHA256
    Aes128GcmSha256 = 0x1301,
    /// TLS_AES_256_GCM_SHA384
    Aes256GcmSha384 = 0x1302,
    /// TLS_CHACHA20_POLY1305_SHA256
    Chacha20Poly1305Sha256 = 0x1303,
    /// TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256 (TLS 1.2)
    EcdheRsaAes128GcmSha256 = 0xC02F,
    /// TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256 (TLS 1.2)
    EcdheEcdsaAes128GcmSha256 = 0xC02B,
    /// TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256 (TLS 1.2)
    EcdheRsaChacha20Poly1305Sha256 = 0xCCA8,
}

impl CipherSuite {
    /// Ordered list of TLS 1.3 cipher suites we support (preferred first).
    pub const TLS13_SUITES: &'static [CipherSuite] = &[
        CipherSuite::Chacha20Poly1305Sha256,
        CipherSuite::Aes128GcmSha256,
        CipherSuite::Aes256GcmSha384,
    ];

    /// Ordered list of TLS 1.2 cipher suites we support (preferred first).
    pub const TLS12_SUITES: &'static [CipherSuite] = &[
        CipherSuite::EcdheRsaChacha20Poly1305Sha256,
        CipherSuite::EcdheRsaAes128GcmSha256,
        CipherSuite::EcdheEcdsaAes128GcmSha256,
    ];

    /// AEAD key size in bytes.
    pub fn key_len(&self) -> usize {
        match self {
            CipherSuite::Aes128GcmSha256
            | CipherSuite::EcdheRsaAes128GcmSha256
            | CipherSuite::EcdheEcdsaAes128GcmSha256 => 16,
            CipherSuite::Aes256GcmSha384 => 32,
            CipherSuite::Chacha20Poly1305Sha256 | CipherSuite::EcdheRsaChacha20Poly1305Sha256 => 32,
        }
    }

    /// AEAD nonce/IV size in bytes.
    pub fn iv_len(&self) -> usize {
        12 // All our cipher suites use 12-byte nonces
    }

    /// AEAD authentication tag size in bytes.
    pub fn tag_len(&self) -> usize {
        16 // All our cipher suites use 16-byte tags
    }

    /// Hash digest size used by this cipher suite's key schedule.
    pub fn hash_len(&self) -> usize {
        match self {
            CipherSuite::Aes256GcmSha384 => 48, // SHA-384
            _ => 32,                            // SHA-256
        }
    }

    /// Wire encoding as 2 bytes.
    pub fn to_bytes(&self) -> [u8; 2] {
        (*self as u16).to_be_bytes()
    }

    /// Parse from wire encoding.
    pub fn from_u16(v: u16) -> Option<CipherSuite> {
        match v {
            0x1301 => Some(CipherSuite::Aes128GcmSha256),
            0x1302 => Some(CipherSuite::Aes256GcmSha384),
            0x1303 => Some(CipherSuite::Chacha20Poly1305Sha256),
            0xC02F => Some(CipherSuite::EcdheRsaAes128GcmSha256),
            0xC02B => Some(CipherSuite::EcdheEcdsaAes128GcmSha256),
            0xCCA8 => Some(CipherSuite::EcdheRsaChacha20Poly1305Sha256),
            _ => None,
        }
    }
}

/// Named groups for key exchange (RFC 8446 Section 4.2.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum NamedGroup {
    /// x25519 (preferred)
    X25519 = 0x001D,
    /// secp256r1 / P-256 (fallback)
    Secp256r1 = 0x0017,
}

/// Signature algorithms (RFC 8446 Section 4.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum SignatureScheme {
    RsaPssRsaeSha256 = 0x0804,
    RsaPssRsaeSha384 = 0x0805,
    RsaPkcs1Sha256 = 0x0401,
    RsaPkcs1Sha384 = 0x0501,
    EcdsaSecp256r1Sha256 = 0x0403,
    EcdsaSecp384r1Sha384 = 0x0503,
}

impl SignatureScheme {
    /// Signature algorithms we support, in preference order.
    pub const SUPPORTED: &'static [SignatureScheme] = &[
        SignatureScheme::EcdsaSecp256r1Sha256,
        SignatureScheme::RsaPssRsaeSha256,
        SignatureScheme::RsaPkcs1Sha256,
        SignatureScheme::RsaPssRsaeSha384,
        SignatureScheme::RsaPkcs1Sha384,
        SignatureScheme::EcdsaSecp384r1Sha384,
    ];

    pub fn to_bytes(&self) -> [u8; 2] {
        (*self as u16).to_be_bytes()
    }

    pub fn from_u16(v: u16) -> Option<SignatureScheme> {
        match v {
            0x0804 => Some(SignatureScheme::RsaPssRsaeSha256),
            0x0805 => Some(SignatureScheme::RsaPssRsaeSha384),
            0x0401 => Some(SignatureScheme::RsaPkcs1Sha256),
            0x0501 => Some(SignatureScheme::RsaPkcs1Sha384),
            0x0403 => Some(SignatureScheme::EcdsaSecp256r1Sha256),
            0x0503 => Some(SignatureScheme::EcdsaSecp384r1Sha384),
            _ => None,
        }
    }
}
