//! TLS 1.3 handshake state machine (RFC 8446).
//!
//! Implements the full TLS 1.3 client handshake:
//! 1. Send ClientHello (with X25519 key share)
//! 2. Receive ServerHello (get server's key share)
//! 3. Derive handshake keys (HKDF)
//! 4. Receive EncryptedExtensions, Certificate, CertificateVerify, Finished
//! 5. Send client Finished
//! 6. Derive application traffic keys

use alloc::vec::Vec;
use crate::error::TlsError;
use crate::record::{self, ContentType, ProtocolVersion, RecordHeader, RECORD_HEADER_SIZE, MAX_RECORD_PAYLOAD};
use crate::cipher_suite::{CipherSuite, NamedGroup};
use crate::crypto::{sha256, sha384, hmac, hkdf, x25519, gcm, chacha20poly1305};
use crate::x509;
use super::extensions;

/// TLS 1.3 handshake result containing the negotiated state.
pub struct Tls13Handshake {
    /// Negotiated cipher suite.
    pub cipher_suite: CipherSuite,
    /// Client application traffic key.
    pub client_app_key: Vec<u8>,
    /// Client application traffic IV.
    pub client_app_iv: Vec<u8>,
    /// Server application traffic key.
    pub server_app_key: Vec<u8>,
    /// Server application traffic IV.
    pub server_app_iv: Vec<u8>,
}

/// Perform the TLS 1.3 client handshake.
pub fn do_handshake(
    fd: u32,
    host: &str,
    send_fn: fn(u32, &[u8]) -> i32,
    recv_fn: fn(u32, &mut [u8]) -> i32,
    random_fn: fn(&mut [u8]) -> u32,
) -> Result<Tls13Handshake, TlsError> {
    // 1. Generate ephemeral X25519 key pair
    let mut private_key = [0u8; 32];
    random_fn(&mut private_key);
    let public_key = x25519::x25519_base(&private_key);

    // 2. Generate client random
    let mut client_random = [0u8; 32];
    random_fn(&mut client_random);

    // 3. Build and send ClientHello
    let client_hello = extensions::build_client_hello(&client_random, host, &public_key);

    // Transcript hash starts with ClientHello
    let mut transcript = Vec::with_capacity(4096);
    transcript.extend_from_slice(&client_hello);

    // Wrap in TLS record and send
    send_record(fd, ContentType::Handshake as u8, &client_hello, send_fn)?;

    // 4. Receive ServerHello
    let server_hello_raw = recv_handshake_msg(fd, recv_fn)?;
    if server_hello_raw.is_empty() || server_hello_raw[0] != 0x02 {
        return Err(TlsError::UnexpectedMessage);
    }
    transcript.extend_from_slice(&server_hello_raw);

    // Parse ServerHello
    let (cipher_suite, server_key_share) = parse_server_hello(&server_hello_raw)?;

    // 5. Compute shared secret via X25519
    let shared_secret = match server_key_share {
        (group, ref key_data) if group == NamedGroup::X25519 as u16 => {
            if key_data.len() != 32 {
                return Err(TlsError::KeyExchangeFailed);
            }
            let mut peer_key = [0u8; 32];
            peer_key.copy_from_slice(key_data);
            x25519::x25519(&private_key, &peer_key)
        }
        _ => return Err(TlsError::KeyExchangeFailed),
    };

    // 6. Derive handshake keys using HKDF
    let hash_len = cipher_suite.hash_len();
    let transcript_hash = compute_hash(&transcript, cipher_suite);

    // Early Secret = HKDF-Extract(salt=0, IKM=0)
    let zero_key = alloc::vec![0u8; hash_len];
    let early_secret = hkdf_extract(cipher_suite, &zero_key, &zero_key);

    // Derive-Secret(., "derived", "")
    let empty_hash = compute_hash(&[], cipher_suite);
    let derived_secret = derive_secret(cipher_suite, &early_secret, b"derived", &empty_hash);

    // Handshake Secret = HKDF-Extract(salt=derived, IKM=shared_secret)
    let handshake_secret = hkdf_extract(cipher_suite, &derived_secret, &shared_secret);

    // client_handshake_traffic_secret
    let c_hs_secret = derive_secret(cipher_suite, &handshake_secret, b"c hs traffic", &transcript_hash);
    // server_handshake_traffic_secret
    let s_hs_secret = derive_secret(cipher_suite, &handshake_secret, b"s hs traffic", &transcript_hash);

    // Derive handshake keys
    let s_hs_key = hkdf_expand_label(cipher_suite, &s_hs_secret, b"key", &[], cipher_suite.key_len());
    let s_hs_iv = hkdf_expand_label(cipher_suite, &s_hs_secret, b"iv", &[], cipher_suite.iv_len());

    // 7. Receive encrypted handshake messages
    let mut server_seq: u64 = 0;

    // Read all server handshake messages (EncryptedExtensions, Certificate, CertVerify, Finished)
    let mut server_finished_received = false;
    while !server_finished_received {
        let record = recv_raw_record(fd, recv_fn)?;
        if record.is_empty() {
            return Err(TlsError::RecvFailed);
        }

        // Decrypt the record (it's ApplicationData type wrapping handshake)
        let plaintext = decrypt_record(cipher_suite, &s_hs_key, &s_hs_iv, server_seq, &record)?;
        server_seq += 1;

        // The last byte of plaintext is the real content type
        if plaintext.is_empty() {
            return Err(TlsError::DecryptionFailed);
        }
        let real_content_type = plaintext[plaintext.len() - 1];
        let msg_data = &plaintext[..plaintext.len() - 1];

        if real_content_type == ContentType::Handshake as u8 {
            // Process handshake messages within this record
            let mut pos = 0;
            while pos < msg_data.len() {
                if pos + 4 > msg_data.len() {
                    break;
                }
                let msg_type = msg_data[pos];
                let msg_len = ((msg_data[pos + 1] as usize) << 16)
                    | ((msg_data[pos + 2] as usize) << 8)
                    | (msg_data[pos + 3] as usize);
                let msg_end = pos + 4 + msg_len;
                if msg_end > msg_data.len() {
                    break;
                }

                // Add to transcript (for all except Finished)
                transcript.extend_from_slice(&msg_data[pos..msg_end]);

                match msg_type {
                    8 => {} // EncryptedExtensions — we trust-all, skip
                    11 => {} // Certificate — trust-all, skip validation
                    15 => {} // CertificateVerify — trust-all, skip
                    20 => {
                        // Finished — verify server's finished
                        server_finished_received = true;
                        // Remove Finished from transcript (it was added above but
                        // the finished verify_data is computed over transcript BEFORE Finished)
                        let finish_len = msg_end - pos;
                        let tlen = transcript.len();
                        transcript.truncate(tlen - finish_len);

                        // TODO: Verify Finished verify_data
                        // For trust-all mode, we accept it

                        // Re-add to transcript for client Finished computation
                        transcript.extend_from_slice(&msg_data[pos..msg_end]);
                    }
                    _ => {} // Unknown — skip
                }
                pos = msg_end;
            }
        } else if real_content_type == ContentType::Alert as u8 {
            return Err(TlsError::AlertReceived);
        }
    }

    // 8. Send client Finished
    let transcript_hash_for_finished = compute_hash(&transcript, cipher_suite);
    let c_hs_key = hkdf_expand_label(cipher_suite, &c_hs_secret, b"key", &[], cipher_suite.key_len());
    let c_hs_iv = hkdf_expand_label(cipher_suite, &c_hs_secret, b"iv", &[], cipher_suite.iv_len());

    let finished_key = hkdf_expand_label(cipher_suite, &c_hs_secret, b"finished", &[], hash_len);
    let verify_data = compute_hmac(cipher_suite, &finished_key, &transcript_hash_for_finished);

    // Build Finished handshake message
    let mut finished_msg = Vec::with_capacity(4 + verify_data.len());
    finished_msg.push(20); // Finished type
    let vlen = verify_data.len();
    finished_msg.push((vlen >> 16) as u8);
    finished_msg.push((vlen >> 8) as u8);
    finished_msg.push(vlen as u8);
    finished_msg.extend_from_slice(&verify_data);

    // Encrypt and send
    let mut client_seq: u64 = 0;
    let encrypted = encrypt_record(
        cipher_suite, &c_hs_key, &c_hs_iv, client_seq,
        &finished_msg, ContentType::Handshake as u8,
    );
    send_record_raw(fd, &encrypted, send_fn)?;
    client_seq += 1;

    // Add client Finished to transcript
    transcript.extend_from_slice(&finished_msg);

    // 9. Derive application traffic secrets
    let derived_secret2 = derive_secret(cipher_suite, &handshake_secret, b"derived", &empty_hash);
    let master_secret = hkdf_extract(cipher_suite, &derived_secret2, &zero_key);

    let transcript_hash_final = compute_hash(&transcript, cipher_suite);
    let c_app_secret = derive_secret(cipher_suite, &master_secret, b"c ap traffic", &transcript_hash_final);
    let s_app_secret = derive_secret(cipher_suite, &master_secret, b"s ap traffic", &transcript_hash_final);

    let client_app_key = hkdf_expand_label(cipher_suite, &c_app_secret, b"key", &[], cipher_suite.key_len());
    let client_app_iv = hkdf_expand_label(cipher_suite, &c_app_secret, b"iv", &[], cipher_suite.iv_len());
    let server_app_key = hkdf_expand_label(cipher_suite, &s_app_secret, b"key", &[], cipher_suite.key_len());
    let server_app_iv = hkdf_expand_label(cipher_suite, &s_app_secret, b"iv", &[], cipher_suite.iv_len());

    Ok(Tls13Handshake {
        cipher_suite,
        client_app_key,
        client_app_iv,
        server_app_key,
        server_app_iv,
    })
}

// -- Helper functions --

fn parse_server_hello(data: &[u8]) -> Result<(CipherSuite, (u16, Vec<u8>)), TlsError> {
    // data[0] = 0x02 (ServerHello type)
    // data[1..4] = length
    if data.len() < 4 {
        return Err(TlsError::UnexpectedMessage);
    }
    let body = &data[4..];
    if body.len() < 38 {
        return Err(TlsError::UnexpectedMessage);
    }

    // server_version (2) + random (32) = 34
    let _server_version = u16::from_be_bytes([body[0], body[1]]);
    let _server_random = &body[2..34];

    let mut pos = 34;

    // session_id
    if pos >= body.len() {
        return Err(TlsError::UnexpectedMessage);
    }
    let session_id_len = body[pos] as usize;
    pos += 1 + session_id_len;

    // cipher_suite (2 bytes)
    if pos + 2 > body.len() {
        return Err(TlsError::UnexpectedMessage);
    }
    let suite_id = u16::from_be_bytes([body[pos], body[pos + 1]]);
    let cipher_suite = CipherSuite::from_u16(suite_id)
        .ok_or(TlsError::NoCipherSuite)?;
    pos += 2;

    // compression_method (1 byte, must be 0)
    pos += 1;

    // extensions
    let mut key_share = None;
    if pos + 2 <= body.len() {
        let ext_len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
        pos += 2;
        if pos + ext_len <= body.len() {
            let ext_data = &body[pos..pos + ext_len];
            let parsed = extensions::parse_server_hello_extensions(ext_data);
            key_share = parsed.key_share;
        }
    }

    let key_share = key_share.ok_or(TlsError::KeyExchangeFailed)?;
    Ok((cipher_suite, key_share))
}

fn send_record(fd: u32, content_type: u8, data: &[u8], send_fn: fn(u32, &[u8]) -> i32) -> Result<(), TlsError> {
    let header = RecordHeader {
        content_type,
        version: ProtocolVersion::TLS13_COMPAT,
        length: data.len() as u16,
    };
    let header_bytes = header.to_bytes();
    if send_fn(fd, &header_bytes) < 0 {
        return Err(TlsError::SendFailed);
    }
    if send_fn(fd, data) < 0 {
        return Err(TlsError::SendFailed);
    }
    Ok(())
}

fn send_record_raw(fd: u32, data: &[u8], send_fn: fn(u32, &[u8]) -> i32) -> Result<(), TlsError> {
    // data already includes the 5-byte record header
    if send_fn(fd, data) < 0 {
        return Err(TlsError::SendFailed);
    }
    Ok(())
}

fn recv_handshake_msg(fd: u32, recv_fn: fn(u32, &mut [u8]) -> i32) -> Result<Vec<u8>, TlsError> {
    let mut header_buf = [0u8; RECORD_HEADER_SIZE];
    recv_exact(fd, &mut header_buf, recv_fn)?;

    let header = RecordHeader::parse(&header_buf);
    if header.length as usize > MAX_RECORD_PAYLOAD + 256 {
        return Err(TlsError::RecordOverflow);
    }

    let mut body = alloc::vec![0u8; header.length as usize];
    recv_exact(fd, &mut body, recv_fn)?;
    Ok(body)
}

fn recv_raw_record(fd: u32, recv_fn: fn(u32, &mut [u8]) -> i32) -> Result<Vec<u8>, TlsError> {
    let mut header_buf = [0u8; RECORD_HEADER_SIZE];
    recv_exact(fd, &mut header_buf, recv_fn)?;

    let header = RecordHeader::parse(&header_buf);
    let mut body = alloc::vec![0u8; header.length as usize];
    recv_exact(fd, &mut body, recv_fn)?;
    Ok(body)
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

// -- Crypto helpers --

fn compute_hash(data: &[u8], suite: CipherSuite) -> Vec<u8> {
    match suite {
        CipherSuite::Aes256GcmSha384 => sha384::sha384(data).to_vec(),
        _ => sha256::sha256(data).to_vec(),
    }
}

fn compute_hmac(suite: CipherSuite, key: &[u8], data: &[u8]) -> Vec<u8> {
    match suite {
        CipherSuite::Aes256GcmSha384 => hmac::hmac_sha384(key, data).to_vec(),
        _ => hmac::hmac_sha256(key, data).to_vec(),
    }
}

fn hkdf_extract(suite: CipherSuite, salt: &[u8], ikm: &[u8]) -> Vec<u8> {
    match suite {
        CipherSuite::Aes256GcmSha384 => hkdf::hkdf_extract_sha384(salt, ikm).to_vec(),
        _ => hkdf::hkdf_extract_sha256(salt, ikm).to_vec(),
    }
}

fn derive_secret(suite: CipherSuite, secret: &[u8], label: &[u8], hash: &[u8]) -> Vec<u8> {
    let hash_len = suite.hash_len();
    let mut out = alloc::vec![0u8; hash_len];
    match suite {
        CipherSuite::Aes256GcmSha384 => {
            hkdf::tls13_hkdf_expand_label_sha384(secret, label, hash, &mut out);
        }
        _ => {
            hkdf::tls13_hkdf_expand_label_sha256(secret, label, hash, &mut out);
        }
    }
    out
}

fn hkdf_expand_label(suite: CipherSuite, secret: &[u8], label: &[u8], context: &[u8], length: usize) -> Vec<u8> {
    let mut out = alloc::vec![0u8; length];
    match suite {
        CipherSuite::Aes256GcmSha384 => {
            hkdf::tls13_hkdf_expand_label_sha384(secret, label, context, &mut out);
        }
        _ => {
            hkdf::tls13_hkdf_expand_label_sha256(secret, label, context, &mut out);
        }
    }
    out
}

/// Build nonce for AEAD by XORing IV with sequence number.
fn build_nonce(iv: &[u8], seq: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&iv[..12]);
    let seq_bytes = seq.to_be_bytes();
    for i in 0..8 {
        nonce[4 + i] ^= seq_bytes[i];
    }
    nonce
}

fn decrypt_record(
    suite: CipherSuite,
    key: &[u8],
    iv: &[u8],
    seq: u64,
    ciphertext: &[u8],
) -> Result<Vec<u8>, TlsError> {
    if ciphertext.len() < 16 {
        return Err(TlsError::DecryptionFailed);
    }

    let nonce = build_nonce(iv, seq);
    let tag_start = ciphertext.len() - 16;
    let mut data = ciphertext[..tag_start].to_vec();
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&ciphertext[tag_start..]);

    // AAD for TLS 1.3: the 5-byte record header
    // ApplicationData(23) + TLS12(0x0303) + length
    let record_len = ciphertext.len();
    let aad = [
        ContentType::ApplicationData as u8,
        0x03, 0x03,
        (record_len >> 8) as u8, record_len as u8,
    ];

    let ok = match suite {
        CipherSuite::Chacha20Poly1305Sha256 |
        CipherSuite::EcdheRsaChacha20Poly1305Sha256 => {
            if key.len() != 32 {
                return Err(TlsError::InternalError);
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            let mut n = [0u8; 12];
            n.copy_from_slice(&nonce);
            chacha20poly1305::decrypt(&k, &n, &aad, &mut data, &tag)
        }
        CipherSuite::Aes128GcmSha256 |
        CipherSuite::EcdheRsaAes128GcmSha256 |
        CipherSuite::EcdheEcdsaAes128GcmSha256 => {
            if key.len() != 16 {
                return Err(TlsError::InternalError);
            }
            let mut k = [0u8; 16];
            k.copy_from_slice(key);
            let aes = gcm::AesGcm::new_128(&k);
            let mut n = [0u8; 12];
            n.copy_from_slice(&nonce);
            aes.decrypt(&n, &aad, &mut data, &tag)
        }
        CipherSuite::Aes256GcmSha384 => {
            if key.len() != 32 {
                return Err(TlsError::InternalError);
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            let aes = gcm::AesGcm::new_256(&k);
            let mut n = [0u8; 12];
            n.copy_from_slice(&nonce);
            aes.decrypt(&n, &aad, &mut data, &tag)
        }
        _ => return Err(TlsError::NoCipherSuite),
    };

    if !ok {
        return Err(TlsError::DecryptionFailed);
    }
    Ok(data)
}

/// Encrypt a handshake/application data record for TLS 1.3.
/// Returns the full record including the 5-byte header.
pub fn encrypt_record(
    suite: CipherSuite,
    key: &[u8],
    iv: &[u8],
    seq: u64,
    plaintext: &[u8],
    content_type: u8,
) -> Vec<u8> {
    let nonce = build_nonce(iv, seq);

    // Inner plaintext: data + content type byte
    let mut inner = Vec::with_capacity(plaintext.len() + 1);
    inner.extend_from_slice(plaintext);
    inner.push(content_type);

    // AAD: record header with encrypted length
    let encrypted_len = inner.len() + 16; // + tag
    let aad = [
        ContentType::ApplicationData as u8,
        0x03, 0x03,
        (encrypted_len >> 8) as u8, encrypted_len as u8,
    ];

    let mut tag = [0u8; 16];

    match suite {
        CipherSuite::Chacha20Poly1305Sha256 |
        CipherSuite::EcdheRsaChacha20Poly1305Sha256 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            let mut n = [0u8; 12];
            n.copy_from_slice(&nonce);
            chacha20poly1305::encrypt(&k, &n, &aad, &mut inner, &mut tag);
        }
        CipherSuite::Aes128GcmSha256 |
        CipherSuite::EcdheRsaAes128GcmSha256 |
        CipherSuite::EcdheEcdsaAes128GcmSha256 => {
            let mut k = [0u8; 16];
            k.copy_from_slice(key);
            let aes = gcm::AesGcm::new_128(&k);
            let mut n = [0u8; 12];
            n.copy_from_slice(&nonce);
            aes.encrypt(&n, &aad, &mut inner, &mut tag);
        }
        CipherSuite::Aes256GcmSha384 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            let aes = gcm::AesGcm::new_256(&k);
            let mut n = [0u8; 12];
            n.copy_from_slice(&nonce);
            aes.encrypt(&n, &aad, &mut inner, &mut tag);
        }
        _ => {}
    }

    // Build full record: header + ciphertext + tag
    let mut record = Vec::with_capacity(5 + inner.len() + 16);
    record.extend_from_slice(&aad); // header is the AAD
    record.extend_from_slice(&inner);
    record.extend_from_slice(&tag);
    record
}
