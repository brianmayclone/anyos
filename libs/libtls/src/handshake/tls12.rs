//! TLS 1.2 handshake state machine (RFC 5246).
//!
//! Implements the TLS 1.2 client handshake as a fallback when the server
//! doesn't support TLS 1.3. Only supports ECDHE_RSA cipher suites with
//! AES-GCM to keep the implementation focused and secure.
//!
//! Flow:
//! 1. (ClientHello already sent by tls13 code with both 1.3 and 1.2 suites)
//! 2. ServerHello (selected TLS 1.2 suite)
//! 3. Certificate
//! 4. ServerKeyExchange (ECDHE parameters)
//! 5. ServerHelloDone
//! 6. ClientKeyExchange (our ECDHE public key)
//! 7. ChangeCipherSpec
//! 8. Finished

use alloc::vec::Vec;
use crate::error::TlsError;
use crate::record::{ContentType, ProtocolVersion, RecordHeader, RECORD_HEADER_SIZE};
use crate::cipher_suite::CipherSuite;
use crate::crypto::{sha256, hmac, x25519, gcm, chacha20poly1305};

/// TLS 1.2 handshake result.
pub struct Tls12Handshake {
    pub cipher_suite: CipherSuite,
    pub client_write_key: Vec<u8>,
    pub client_write_iv: Vec<u8>,
    pub server_write_key: Vec<u8>,
    pub server_write_iv: Vec<u8>,
}

/// Continue a TLS 1.2 handshake after the ServerHello indicated TLS 1.2.
///
/// `client_hello_bytes`: The original ClientHello for the transcript.
/// `server_hello_bytes`: The ServerHello already received.
pub fn continue_tls12_handshake(
    fd: u32,
    client_hello_bytes: &[u8],
    server_hello_bytes: &[u8],
    cipher_suite: CipherSuite,
    send_fn: fn(u32, &[u8]) -> i32,
    recv_fn: fn(u32, &mut [u8]) -> i32,
    random_fn: fn(&mut [u8]) -> u32,
) -> Result<Tls12Handshake, TlsError> {
    let mut transcript = Vec::with_capacity(4096);
    transcript.extend_from_slice(client_hello_bytes);
    transcript.extend_from_slice(server_hello_bytes);

    // Extract client_random and server_random from the messages
    let client_random = extract_random(client_hello_bytes);
    let server_random = extract_random(server_hello_bytes);

    // Receive Certificate
    let cert_msg = recv_handshake(fd, recv_fn)?;
    transcript.extend_from_slice(&cert_msg);
    // Trust-all: we don't verify the certificate chain

    // Receive ServerKeyExchange (for ECDHE)
    let ske_msg = recv_handshake(fd, recv_fn)?;
    transcript.extend_from_slice(&ske_msg);

    // Parse ServerKeyExchange to get the server's ECDHE public key
    let server_ecdhe_public = parse_server_key_exchange(&ske_msg)?;

    // Receive ServerHelloDone
    let shd_msg = recv_handshake(fd, recv_fn)?;
    transcript.extend_from_slice(&shd_msg);
    if shd_msg.is_empty() || shd_msg[0] != 14 {
        return Err(TlsError::UnexpectedMessage);
    }

    // Generate our ECDHE key pair (X25519)
    let mut private_key = [0u8; 32];
    random_fn(&mut private_key);
    let public_key = x25519::x25519_base(&private_key);

    // Compute shared secret
    let shared_secret = if server_ecdhe_public.len() == 32 {
        let mut peer = [0u8; 32];
        peer.copy_from_slice(&server_ecdhe_public);
        x25519::x25519(&private_key, &peer)
    } else {
        return Err(TlsError::KeyExchangeFailed);
    };

    // Send ClientKeyExchange
    let mut cke_body = Vec::with_capacity(1 + 32);
    cke_body.push(32); // public key length
    cke_body.extend_from_slice(&public_key);

    let cke_msg = build_handshake_msg(16, &cke_body); // ClientKeyExchange = 16
    transcript.extend_from_slice(&cke_msg);
    send_record(fd, ContentType::Handshake as u8, &cke_msg, send_fn)?;

    // Derive keys using TLS 1.2 PRF (HMAC-SHA256 based)
    let pre_master_secret = &shared_secret;
    let mut seed = Vec::with_capacity(64);
    seed.extend_from_slice(&client_random);
    seed.extend_from_slice(&server_random);

    let mut master_secret = [0u8; 48];
    prf_sha256(pre_master_secret, b"master secret", &seed, &mut master_secret);

    // Key expansion
    let key_len = cipher_suite.key_len();
    let iv_len = 4; // TLS 1.2 GCM uses 4-byte implicit IV
    let key_block_len = 2 * key_len + 2 * iv_len;
    let mut key_block = alloc::vec![0u8; key_block_len];
    let mut kb_seed = Vec::with_capacity(64);
    kb_seed.extend_from_slice(&server_random);
    kb_seed.extend_from_slice(&client_random);
    prf_sha256(&master_secret, b"key expansion", &kb_seed, &mut key_block);

    let mut pos = 0;
    let client_write_key = key_block[pos..pos + key_len].to_vec();
    pos += key_len;
    let server_write_key = key_block[pos..pos + key_len].to_vec();
    pos += key_len;
    let client_write_iv = key_block[pos..pos + iv_len].to_vec();
    pos += iv_len;
    let server_write_iv = key_block[pos..pos + iv_len].to_vec();

    // Send ChangeCipherSpec
    send_record(fd, ContentType::ChangeCipherSpec as u8, &[1], send_fn)?;

    // Compute and send Finished
    let transcript_hash = sha256::sha256(&transcript);
    let mut verify_data = [0u8; 12];
    prf_sha256(&master_secret, b"client finished", &transcript_hash, &mut verify_data);

    let finished_msg = build_handshake_msg(20, &verify_data);
    // Encrypt the Finished message with client key
    // For the first encrypted message, sequence number is 0
    let encrypted = encrypt_tls12_record(
        cipher_suite, &client_write_key, &client_write_iv,
        0, ContentType::Handshake as u8, &finished_msg,
        random_fn,
    );
    send_record_raw(fd, &encrypted, send_fn)?;

    // Receive server ChangeCipherSpec
    let _ccs = recv_record(fd, recv_fn)?;

    // Receive server Finished (encrypted)
    let _server_finished = recv_record(fd, recv_fn)?;
    // Trust-all: accept without verification

    Ok(Tls12Handshake {
        cipher_suite,
        client_write_key,
        client_write_iv,
        server_write_key,
        server_write_iv,
    })
}

// -- TLS 1.2 PRF (RFC 5246 Section 5) --

/// P_SHA256(secret, seed) — HMAC-based PRF expansion.
fn prf_sha256(secret: &[u8], label: &[u8], seed: &[u8], output: &mut [u8]) {
    let mut combined_seed = Vec::with_capacity(label.len() + seed.len());
    combined_seed.extend_from_slice(label);
    combined_seed.extend_from_slice(seed);

    let mut a = hmac::hmac_sha256(secret, &combined_seed); // A(1)
    let mut pos = 0;

    while pos < output.len() {
        let mut input = Vec::with_capacity(32 + combined_seed.len());
        input.extend_from_slice(&a);
        input.extend_from_slice(&combined_seed);
        let p = hmac::hmac_sha256(secret, &input);

        let copy = (output.len() - pos).min(32);
        output[pos..pos + copy].copy_from_slice(&p[..copy]);
        pos += copy;

        a = hmac::hmac_sha256(secret, &a); // A(i+1)
    }
}

// -- Helpers --

fn extract_random(hello_msg: &[u8]) -> [u8; 32] {
    // Handshake type(1) + length(3) + version(2) + random(32)
    let mut random = [0u8; 32];
    if hello_msg.len() >= 38 {
        random.copy_from_slice(&hello_msg[6..38]);
    }
    random
}

fn parse_server_key_exchange(msg: &[u8]) -> Result<Vec<u8>, TlsError> {
    // msg[0] = 12 (ServerKeyExchange type)
    if msg.len() < 4 || msg[0] != 12 {
        return Err(TlsError::UnexpectedMessage);
    }
    let body = &msg[4..];

    // ECParameters: curve_type(1) + named_curve(2)
    if body.len() < 4 {
        return Err(TlsError::KeyExchangeFailed);
    }
    let _curve_type = body[0]; // 3 = named_curve
    let _named_curve = u16::from_be_bytes([body[1], body[2]]);
    let pub_len = body[3] as usize;

    if body.len() < 4 + pub_len {
        return Err(TlsError::KeyExchangeFailed);
    }
    Ok(body[4..4 + pub_len].to_vec())
}

fn build_handshake_msg(msg_type: u8, body: &[u8]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(4 + body.len());
    msg.push(msg_type);
    let len = body.len();
    msg.push((len >> 16) as u8);
    msg.push((len >> 8) as u8);
    msg.push(len as u8);
    msg.extend_from_slice(body);
    msg
}

fn send_record(fd: u32, content_type: u8, data: &[u8], send_fn: fn(u32, &[u8]) -> i32) -> Result<(), TlsError> {
    let header = RecordHeader {
        content_type,
        version: ProtocolVersion::TLS12,
        length: data.len() as u16,
    };
    let hdr = header.to_bytes();
    if send_fn(fd, &hdr) < 0 || send_fn(fd, data) < 0 {
        return Err(TlsError::SendFailed);
    }
    Ok(())
}

fn send_record_raw(fd: u32, data: &[u8], send_fn: fn(u32, &[u8]) -> i32) -> Result<(), TlsError> {
    if send_fn(fd, data) < 0 {
        return Err(TlsError::SendFailed);
    }
    Ok(())
}

fn recv_handshake(fd: u32, recv_fn: fn(u32, &mut [u8]) -> i32) -> Result<Vec<u8>, TlsError> {
    let mut hdr = [0u8; RECORD_HEADER_SIZE];
    recv_exact(fd, &mut hdr, recv_fn)?;
    let header = RecordHeader::parse(&hdr);
    let mut body = alloc::vec![0u8; header.length as usize];
    recv_exact(fd, &mut body, recv_fn)?;
    Ok(body)
}

fn recv_record(fd: u32, recv_fn: fn(u32, &mut [u8]) -> i32) -> Result<Vec<u8>, TlsError> {
    recv_handshake(fd, recv_fn) // Same implementation
}

fn recv_exact(fd: u32, buf: &mut [u8], recv_fn: fn(u32, &mut [u8]) -> i32) -> Result<(), TlsError> {
    let mut filled = 0;
    while filled < buf.len() {
        let n = recv_fn(fd, &mut buf[filled..]);
        if n <= 0 {
            return Err(TlsError::RecvFailed);
        }
        filled += n as usize;
    }
    Ok(())
}

/// Encrypt a TLS 1.2 record (AES-GCM with explicit nonce).
fn encrypt_tls12_record(
    suite: CipherSuite,
    key: &[u8],
    implicit_iv: &[u8],
    seq: u64,
    content_type: u8,
    plaintext: &[u8],
    random_fn: fn(&mut [u8]) -> u32,
) -> Vec<u8> {
    // TLS 1.2 GCM: nonce = implicit_iv (4 bytes) + explicit_nonce (8 bytes)
    let mut explicit_nonce = [0u8; 8];
    explicit_nonce.copy_from_slice(&seq.to_be_bytes());

    let mut nonce = [0u8; 12];
    nonce[..4].copy_from_slice(&implicit_iv[..4]);
    nonce[4..12].copy_from_slice(&explicit_nonce);

    // AAD: seq(8) + type(1) + version(2) + length(2)
    let mut aad = [0u8; 13];
    aad[..8].copy_from_slice(&seq.to_be_bytes());
    aad[8] = content_type;
    aad[9] = 0x03; aad[10] = 0x03; // TLS 1.2
    let pt_len = plaintext.len() as u16;
    aad[11] = (pt_len >> 8) as u8;
    aad[12] = pt_len as u8;

    let mut data = plaintext.to_vec();
    let mut tag = [0u8; 16];

    match suite {
        CipherSuite::EcdheRsaAes128GcmSha256 |
        CipherSuite::EcdheEcdsaAes128GcmSha256 |
        CipherSuite::Aes128GcmSha256 => {
            let mut k = [0u8; 16];
            k.copy_from_slice(key);
            let aes = gcm::AesGcm::new_128(&k);
            aes.encrypt(&nonce, &aad, &mut data, &mut tag);
        }
        CipherSuite::EcdheRsaChacha20Poly1305Sha256 |
        CipherSuite::Chacha20Poly1305Sha256 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            chacha20poly1305::encrypt(&k, &nonce, &aad, &mut data, &mut tag);
        }
        _ => {}
    }

    // Record: header + explicit_nonce + ciphertext + tag
    let record_len = 8 + data.len() + 16;
    let mut record = Vec::with_capacity(5 + record_len);
    record.push(content_type);
    record.push(0x03); record.push(0x03);
    record.push((record_len >> 8) as u8);
    record.push(record_len as u8);
    record.extend_from_slice(&explicit_nonce);
    record.extend_from_slice(&data);
    record.extend_from_slice(&tag);
    record
}
