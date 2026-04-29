//! Cryptographic primitives for TLS.
//!
//! All implementations are pure Rust, `no_std` compatible, and designed
//! for correctness first. SHA-256 and SHA-1 are adapted from the anyOS
//! kernel crypto module.

pub mod aes;
pub mod bignum;
pub mod chacha20;
pub mod chacha20poly1305;
pub mod gcm;
pub mod hkdf;
pub mod hmac;
pub mod p256;
pub mod poly1305;
pub mod rsa;
pub mod sha1;
pub mod sha256;
pub mod sha384;
pub mod sha512;
pub mod x25519;
