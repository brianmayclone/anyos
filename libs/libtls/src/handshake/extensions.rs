//! TLS handshake extensions (RFC 8446 Section 4.2).
//!
//! Builds ClientHello extensions and parses ServerHello extensions.

use crate::cipher_suite::{CipherSuite, NamedGroup, SignatureScheme};
use alloc::vec::Vec;

// Extension types
const EXT_SERVER_NAME: u16 = 0;
const EXT_SUPPORTED_GROUPS: u16 = 10;
const EXT_SIGNATURE_ALGORITHMS: u16 = 13;
const EXT_SUPPORTED_VERSIONS: u16 = 43;
const EXT_KEY_SHARE: u16 = 51;

/// Build the extensions block for a TLS 1.3 ClientHello.
///
/// Includes: SNI, supported_versions, supported_groups, signature_algorithms, key_share.
pub fn build_client_hello_extensions(host: &str, x25519_public: &[u8; 32]) -> Vec<u8> {
    let mut extensions = Vec::new();

    // SNI (Server Name Indication)
    append_sni_extension(&mut extensions, host);

    // Supported Groups
    append_supported_groups(&mut extensions);

    // Signature Algorithms
    append_signature_algorithms(&mut extensions);

    // Supported Versions (we support TLS 1.3 and 1.2)
    append_supported_versions(&mut extensions);

    // Key Share (X25519 public key)
    append_key_share(&mut extensions, x25519_public);

    extensions
}

/// SNI extension: server_name type=0.
fn append_sni_extension(buf: &mut Vec<u8>, host: &str) {
    let host_bytes = host.as_bytes();
    let entry_len = 1 + 2 + host_bytes.len(); // type(1) + length(2) + name
    let list_len = entry_len;
    let ext_len = 2 + list_len; // server_name_list length(2) + list

    push_u16(buf, EXT_SERVER_NAME);
    push_u16(buf, ext_len as u16);
    push_u16(buf, list_len as u16); // ServerNameList length
    buf.push(0x00); // HostName type
    push_u16(buf, host_bytes.len() as u16);
    buf.extend_from_slice(host_bytes);
}

/// supported_groups: X25519 + secp256r1.
fn append_supported_groups(buf: &mut Vec<u8>) {
    let groups = [NamedGroup::X25519 as u16, NamedGroup::Secp256r1 as u16];
    let list_len = groups.len() * 2;

    push_u16(buf, EXT_SUPPORTED_GROUPS);
    push_u16(buf, (2 + list_len) as u16); // extension data length
    push_u16(buf, list_len as u16); // named_group_list length
    for &g in &groups {
        push_u16(buf, g);
    }
}

/// signature_algorithms: our supported schemes.
fn append_signature_algorithms(buf: &mut Vec<u8>) {
    let schemes = SignatureScheme::SUPPORTED;
    let list_len = schemes.len() * 2;

    push_u16(buf, EXT_SIGNATURE_ALGORITHMS);
    push_u16(buf, (2 + list_len) as u16);
    push_u16(buf, list_len as u16);
    for scheme in schemes {
        push_u16(buf, *scheme as u16);
    }
}

/// supported_versions: TLS 1.3 (0x0304) and TLS 1.2 (0x0303).
fn append_supported_versions(buf: &mut Vec<u8>) {
    push_u16(buf, EXT_SUPPORTED_VERSIONS);
    push_u16(buf, 5); // extension data length: 1 (list len) + 2*2 (versions)
    buf.push(4); // list length in bytes
    push_u16(buf, 0x0304); // TLS 1.3
    push_u16(buf, 0x0303); // TLS 1.2
}

/// key_share: X25519 public key.
fn append_key_share(buf: &mut Vec<u8>, x25519_public: &[u8; 32]) {
    let entry_len = 2 + 2 + 32; // group(2) + key_len(2) + key(32)

    push_u16(buf, EXT_KEY_SHARE);
    push_u16(buf, (2 + entry_len) as u16); // extension data length
    push_u16(buf, entry_len as u16); // client_shares length
    push_u16(buf, NamedGroup::X25519 as u16);
    push_u16(buf, 32); // key_exchange length
    buf.extend_from_slice(x25519_public);
}

// -- ServerHello extension parsing --

/// Parsed ServerHello extensions.
#[derive(Debug)]
pub struct ServerHelloExtensions {
    /// Negotiated TLS version from supported_versions extension.
    pub negotiated_version: Option<u16>,
    /// Server's key share (group + public key).
    pub key_share: Option<(u16, Vec<u8>)>,
}

/// Parse extensions from a ServerHello message.
pub fn parse_server_hello_extensions(data: &[u8]) -> ServerHelloExtensions {
    let mut result = ServerHelloExtensions {
        negotiated_version: None,
        key_share: None,
    };

    let mut pos = 0;
    while pos + 4 <= data.len() {
        let ext_type = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let ext_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        if pos + ext_len > data.len() {
            break;
        }
        let ext_data = &data[pos..pos + ext_len];

        match ext_type {
            EXT_SUPPORTED_VERSIONS => {
                if ext_data.len() >= 2 {
                    result.negotiated_version =
                        Some(u16::from_be_bytes([ext_data[0], ext_data[1]]));
                }
            }
            EXT_KEY_SHARE => {
                if ext_data.len() >= 4 {
                    let group = u16::from_be_bytes([ext_data[0], ext_data[1]]);
                    let key_len = u16::from_be_bytes([ext_data[2], ext_data[3]]) as usize;
                    if ext_data.len() >= 4 + key_len {
                        result.key_share = Some((group, ext_data[4..4 + key_len].to_vec()));
                    }
                }
            }
            _ => {} // Ignore unknown extensions
        }

        pos += ext_len;
    }

    result
}

fn push_u16(buf: &mut Vec<u8>, val: u16) {
    buf.push((val >> 8) as u8);
    buf.push(val as u8);
}

/// Build a full ClientHello message (handshake type 0x01 + body).
pub fn build_client_hello(random: &[u8; 32], host: &str, x25519_public: &[u8; 32]) -> Vec<u8> {
    let mut body = Vec::with_capacity(512);

    // Client Version: TLS 1.2 (0x0303) for compatibility (real version in extensions)
    push_u16(&mut body, 0x0303);

    // Random (32 bytes)
    body.extend_from_slice(random);

    // Session ID (TLS 1.3 uses empty or echoed session ID for middlebox compat)
    // Using 32-byte random session ID for middlebox compatibility
    body.push(32); // session ID length
    body.extend_from_slice(random); // reuse random as session ID

    // Cipher Suites
    let all_suites: Vec<u16> = CipherSuite::TLS13_SUITES
        .iter()
        .chain(CipherSuite::TLS12_SUITES.iter())
        .map(|s| *s as u16)
        .collect();
    push_u16(&mut body, (all_suites.len() * 2) as u16);
    for suite in &all_suites {
        push_u16(&mut body, *suite);
    }

    // Compression Methods: null only
    body.push(1); // length
    body.push(0); // null compression

    // Extensions
    let extensions = build_client_hello_extensions(host, x25519_public);
    push_u16(&mut body, extensions.len() as u16);
    body.extend_from_slice(&extensions);

    // Wrap in handshake message: type(1) + length(3) + body
    let mut msg = Vec::with_capacity(4 + body.len());
    msg.push(0x01); // ClientHello
    let len = body.len();
    msg.push((len >> 16) as u8);
    msg.push((len >> 8) as u8);
    msg.push(len as u8);
    msg.extend_from_slice(&body);
    msg
}
