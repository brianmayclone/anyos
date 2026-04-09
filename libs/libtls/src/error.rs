//! TLS error types.

/// TLS error codes. Numeric values are used as negative return codes in the public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum TlsError {
    /// No error.
    None = 0,
    /// No free connection slots.
    NoSlotsAvailable = 1,
    /// TCP send failed.
    SendFailed = 2,
    /// TCP receive failed or timed out.
    RecvFailed = 3,
    /// TLS handshake failed (generic).
    HandshakeFailed = 4,
    /// Server sent an unexpected message.
    UnexpectedMessage = 5,
    /// Record layer decode error.
    RecordOverflow = 6,
    /// Server sent a fatal alert.
    AlertReceived = 7,
    /// No common cipher suite with server.
    NoCipherSuite = 8,
    /// Certificate parsing error.
    CertificateError = 9,
    /// Server signature verification failed.
    SignatureVerifyFailed = 10,
    /// Connection already closed.
    ConnectionClosed = 11,
    /// Internal error (bug).
    InternalError = 12,
    /// Decryption failed (bad MAC or padding).
    DecryptionFailed = 13,
    /// Unsupported TLS version from server.
    UnsupportedVersion = 14,
    /// Key exchange failed.
    KeyExchangeFailed = 15,
}
