//! TLS handshake protocol.
//!
//! Handles ClientHello/ServerHello exchange, key derivation, and
//! cipher negotiation for both TLS 1.3 and TLS 1.2.

pub mod extensions;
pub mod tls13;
pub mod tls12;

/// Handshake message types (RFC 8446 Section 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HandshakeType {
    ClientHello = 1,
    ServerHello = 2,
    NewSessionTicket = 4,
    EndOfEarlyData = 5,
    EncryptedExtensions = 8,
    Certificate = 11,
    CertificateRequest = 13,
    CertificateVerify = 15,
    Finished = 20,
    KeyUpdate = 24,
    // TLS 1.2
    ServerKeyExchange = 12,
    ServerHelloDone = 14,
    ClientKeyExchange = 16,
}

/// Negotiated TLS version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegotiatedVersion {
    Tls13,
    Tls12,
}
