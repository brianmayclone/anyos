//! TLS connection state machine.

use crate::cipher_suite::CipherSuite;
use crate::crypto::{chacha20poly1305, gcm};
use crate::error::TlsError;
use crate::handshake::{extensions, tls12, tls13, NegotiatedVersion};
use crate::record::{
    ContentType, ProtocolVersion, RecordHeader, MAX_RECORD_PAYLOAD, RECORD_HEADER_SIZE,
};
use crate::{transport_random, transport_recv, transport_send, transport_sleep};
use alloc::vec::Vec;

/// Connection state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnState {
    Handshaking,
    Application,
    Closed,
    Error,
}

/// A TLS connection over a TCP socket.
pub struct TlsConnection {
    fd: u32,
    state: ConnState,
    version: NegotiatedVersion,
    cipher_suite: CipherSuite,
    error: TlsError,
    // Application traffic keys
    client_key: Vec<u8>,
    client_iv: Vec<u8>,
    server_key: Vec<u8>,
    server_iv: Vec<u8>,
    // Sequence numbers
    send_seq: u64,
    recv_seq: u64,
    // Buffered decrypted data from partial record reads
    plaintext_buf: Vec<u8>,
    plaintext_pos: usize,
}

impl TlsConnection {
    /// Perform a TLS handshake over the given TCP socket.
    pub fn connect(fd: u32, host: &str) -> Result<Self, TlsError> {
        // Generate ephemeral X25519 keypair
        let mut private_key = [0u8; 32];
        transport_random(&mut private_key);
        let public_key = crate::crypto::x25519::x25519_base(&private_key);

        // Generate client random
        let mut client_random = [0u8; 32];
        transport_random(&mut client_random);

        // Build ClientHello
        let client_hello = extensions::build_client_hello(&client_random, host, &public_key);

        // Start transcript
        let mut transcript = Vec::with_capacity(4096);
        transcript.extend_from_slice(&client_hello);

        // Send ClientHello record
        send_plaintext_record(fd, ContentType::Handshake as u8, &client_hello)?;

        // Receive ServerHello
        let server_hello = recv_plaintext_record(fd)?;
        if server_hello.is_empty() || server_hello[0] != 0x02 {
            return Err(TlsError::UnexpectedMessage);
        }
        transcript.extend_from_slice(&server_hello);

        // Determine negotiated version
        let version = detect_version(&server_hello);

        match version {
            NegotiatedVersion::Tls13 => {
                // Continue TLS 1.3 handshake from the ServerHello we already received.
                let hs = do_tls13_continuation(
                    fd,
                    &private_key,
                    &public_key,
                    &client_random,
                    &client_hello,
                    &server_hello,
                    host,
                )?;

                Ok(Self {
                    fd,
                    state: ConnState::Application,
                    version: NegotiatedVersion::Tls13,
                    cipher_suite: hs.cipher_suite,
                    error: TlsError::None,
                    client_key: hs.client_app_key,
                    client_iv: hs.client_app_iv,
                    server_key: hs.server_app_key,
                    server_iv: hs.server_app_iv,
                    send_seq: 0,
                    recv_seq: 0,
                    plaintext_buf: Vec::new(),
                    plaintext_pos: 0,
                })
            }
            NegotiatedVersion::Tls12 => {
                // Parse cipher suite from ServerHello
                let cipher_suite = parse_cipher_suite(&server_hello)?;

                let hs = tls12::continue_tls12_handshake(
                    fd,
                    &client_hello,
                    &server_hello,
                    cipher_suite,
                    transport_send,
                    transport_recv,
                    transport_random,
                )?;

                Ok(Self {
                    fd,
                    state: ConnState::Application,
                    version: NegotiatedVersion::Tls12,
                    cipher_suite: hs.cipher_suite,
                    error: TlsError::None,
                    client_key: hs.client_write_key,
                    client_iv: hs.client_write_iv,
                    server_key: hs.server_write_key,
                    server_iv: hs.server_write_iv,
                    send_seq: 0,
                    recv_seq: 0,
                    plaintext_buf: Vec::new(),
                    plaintext_pos: 0,
                })
            }
        }
    }

    /// Send application data over the TLS connection.
    pub fn send(&mut self, data: &[u8]) -> i32 {
        if self.state != ConnState::Application {
            return -1;
        }

        // Encrypt and send as TLS record(s)
        let mut offset = 0;
        while offset < data.len() {
            let chunk_len = (data.len() - offset).min(16384); // Max record payload
            let chunk = &data[offset..offset + chunk_len];

            let record = match self.version {
                NegotiatedVersion::Tls13 => tls13::encrypt_record(
                    self.cipher_suite,
                    &self.client_key,
                    &self.client_iv,
                    self.send_seq,
                    chunk,
                    ContentType::ApplicationData as u8,
                ),
                NegotiatedVersion::Tls12 => encrypt_tls12_app_record(
                    self.cipher_suite,
                    &self.client_key,
                    &self.client_iv,
                    self.send_seq,
                    chunk,
                ),
            };

            if send_all(self.fd, &record).is_err() {
                self.state = ConnState::Error;
                self.error = TlsError::SendFailed;
                return -1;
            }
            self.send_seq += 1;
            offset += chunk_len;
        }

        data.len() as i32
    }

    /// Receive application data from the TLS connection.
    pub fn recv(&mut self, buf: &mut [u8]) -> i32 {
        if self.state != ConnState::Application {
            return -1;
        }

        // Return buffered plaintext first
        if self.plaintext_pos < self.plaintext_buf.len() {
            let available = self.plaintext_buf.len() - self.plaintext_pos;
            let copy_len = available.min(buf.len());
            buf[..copy_len].copy_from_slice(
                &self.plaintext_buf[self.plaintext_pos..self.plaintext_pos + copy_len],
            );
            self.plaintext_pos += copy_len;
            if self.plaintext_pos >= self.plaintext_buf.len() {
                self.plaintext_buf.clear();
                self.plaintext_pos = 0;
            }
            return copy_len as i32;
        }

        // Read and decrypt TLS records. Loop to skip post-handshake messages
        // (NewSessionTicket, KeyUpdate) which return empty plaintext.
        let plaintext = loop {
            let mut header_buf = [0u8; RECORD_HEADER_SIZE];
            if recv_exact_raw(self.fd, &mut header_buf).is_err() {
                self.error = TlsError::RecvFailed;
                return -1;
            }
            let header = RecordHeader::parse(&header_buf);
            if header.length as usize > MAX_RECORD_PAYLOAD + 256 {
                self.error = TlsError::RecordOverflow;
                return -1;
            }

            // TLS 1.3 middlebox compat: skip ChangeCipherSpec records
            if header.content_type == ContentType::ChangeCipherSpec as u8 {
                let mut discard = alloc::vec![0u8; header.length as usize];
                let _ = recv_exact_raw(self.fd, &mut discard);
                continue;
            }

            let mut record_body = alloc::vec![0u8; header.length as usize];
            if recv_exact_raw(self.fd, &mut record_body).is_err() {
                self.error = TlsError::RecvFailed;
                return -1;
            }

            let pt = match self.version {
                NegotiatedVersion::Tls13 => {
                    match decrypt_tls13_app_record(
                        self.cipher_suite,
                        &self.server_key,
                        &self.server_iv,
                        self.recv_seq,
                        &record_body,
                    ) {
                        Ok(pt) => pt,
                        Err(_) => return -1,
                    }
                }
                NegotiatedVersion::Tls12 => {
                    match decrypt_tls12_record(
                        self.cipher_suite,
                        &self.server_key,
                        &self.server_iv,
                        self.recv_seq,
                        header.content_type,
                        &record_body,
                    ) {
                        Ok(pt) => pt,
                        Err(_) => return -1,
                    }
                }
            };
            self.recv_seq += 1;

            if !pt.is_empty() {
                break pt;
            }
            // Empty plaintext = post-handshake message or padding, read next record
        };

        // Copy what we can to buf, buffer the rest
        let copy_len = plaintext.len().min(buf.len());
        buf[..copy_len].copy_from_slice(&plaintext[..copy_len]);

        if plaintext.len() > copy_len {
            self.plaintext_buf = plaintext[copy_len..].to_vec();
            self.plaintext_pos = 0;
        }

        copy_len as i32
    }

    /// Close the TLS connection.
    pub fn close(&mut self) {
        if self.state == ConnState::Application {
            // Send close_notify alert
            let alert = [1, 0]; // warning(1), close_notify(0)
            match self.version {
                NegotiatedVersion::Tls13 => {
                    let record = tls13::encrypt_record(
                        self.cipher_suite,
                        &self.client_key,
                        &self.client_iv,
                        self.send_seq,
                        &alert,
                        ContentType::Alert as u8,
                    );
                    let _ = transport_send(self.fd, &record);
                }
                NegotiatedVersion::Tls12 => {
                    // Send unencrypted close_notify for simplicity
                    let header = RecordHeader {
                        content_type: ContentType::Alert as u8,
                        version: ProtocolVersion::TLS12,
                        length: 2,
                    };
                    let hdr = header.to_bytes();
                    let _ = transport_send(self.fd, &hdr);
                    let _ = transport_send(self.fd, &alert);
                }
            }
        }
        self.state = ConnState::Closed;
    }

    pub fn last_error(&self) -> TlsError {
        self.error
    }
}

// -- Internal helpers --

fn detect_version(server_hello: &[u8]) -> NegotiatedVersion {
    // Check if ServerHello has supported_versions extension indicating TLS 1.3
    if server_hello.len() < 4 {
        return NegotiatedVersion::Tls12;
    }
    let body = &server_hello[4..];
    if body.len() < 38 {
        return NegotiatedVersion::Tls12;
    }

    // Skip to extensions
    let mut pos = 34; // version(2) + random(32)
    if pos >= body.len() {
        return NegotiatedVersion::Tls12;
    }
    let sid_len = body[pos] as usize;
    pos += 1 + sid_len + 2 + 1; // session_id + cipher_suite + compression

    if pos + 2 > body.len() {
        return NegotiatedVersion::Tls12;
    }
    let ext_len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
    pos += 2;

    let ext_data = if pos + ext_len <= body.len() {
        &body[pos..pos + ext_len]
    } else {
        return NegotiatedVersion::Tls12;
    };

    let parsed = extensions::parse_server_hello_extensions(ext_data);
    if parsed.negotiated_version == Some(0x0304) {
        NegotiatedVersion::Tls13
    } else {
        NegotiatedVersion::Tls12
    }
}

fn parse_cipher_suite(server_hello: &[u8]) -> Result<CipherSuite, TlsError> {
    if server_hello.len() < 4 {
        return Err(TlsError::UnexpectedMessage);
    }
    let body = &server_hello[4..];
    if body.len() < 38 {
        return Err(TlsError::UnexpectedMessage);
    }
    let mut pos = 34;
    let sid_len = body[pos] as usize;
    pos += 1 + sid_len;
    if pos + 2 > body.len() {
        return Err(TlsError::UnexpectedMessage);
    }
    let suite_id = u16::from_be_bytes([body[pos], body[pos + 1]]);
    CipherSuite::from_u16(suite_id).ok_or(TlsError::NoCipherSuite)
}

/// Send ALL bytes, retrying on partial sends. TCP may accept fewer bytes
/// than requested when the send buffer is under pressure (many concurrent
/// connections). Without this, the server would receive a truncated TLS
/// record and either hang or send a fatal alert.
fn send_all(fd: u32, data: &[u8]) -> Result<(), TlsError> {
    let mut offset = 0;
    while offset < data.len() {
        let n = transport_send(fd, &data[offset..]);
        if n < 0 {
            return Err(TlsError::SendFailed);
        }
        if n == 0 {
            // TCP buffer full, brief sleep and retry
            transport_sleep(1);
            continue;
        }
        offset += n as usize;
    }
    Ok(())
}

fn send_plaintext_record(fd: u32, content_type: u8, data: &[u8]) -> Result<(), TlsError> {
    let header = RecordHeader {
        content_type,
        version: ProtocolVersion::TLS13_COMPAT,
        length: data.len() as u16,
    };
    let hdr = header.to_bytes();
    send_all(fd, &hdr)?;
    send_all(fd, data)?;
    Ok(())
}

fn recv_plaintext_record(fd: u32) -> Result<Vec<u8>, TlsError> {
    let mut hdr = [0u8; RECORD_HEADER_SIZE];
    recv_exact_raw(fd, &mut hdr)?;
    let header = RecordHeader::parse(&hdr);
    let mut body = alloc::vec![0u8; header.length as usize];
    recv_exact_raw(fd, &mut body)?;
    Ok(body)
}

fn recv_exact_raw(fd: u32, buf: &mut [u8]) -> Result<(), TlsError> {
    const MAX_EMPTY_READS: u32 = 10;
    let mut filled = 0;
    let mut retries = 0;
    while filled < buf.len() {
        let n = transport_recv(fd, &mut buf[filled..]);
        if n < 0 {
            return Err(TlsError::RecvFailed);
        }
        if n == 0 {
            retries += 1;
            if retries > MAX_EMPTY_READS {
                return Err(TlsError::RecvFailed);
            }
            transport_sleep(100);
            continue;
        }
        retries = 0;
        filled += n as usize;
    }
    Ok(())
}

/// Continue TLS 1.3 handshake after we've already exchanged ClientHello/ServerHello.
fn do_tls13_continuation(
    fd: u32,
    private_key: &[u8; 32],
    _public_key: &[u8; 32],
    _client_random: &[u8; 32],
    client_hello: &[u8],
    server_hello: &[u8],
    _host: &str,
) -> Result<tls13::Tls13Handshake, TlsError> {
    // Parse ServerHello for cipher suite and key share
    let (cipher_suite, (group, server_key_data)) = parse_server_hello_full(server_hello)?;

    // Compute shared secret
    if group != crate::cipher_suite::NamedGroup::X25519 as u16 || server_key_data.len() != 32 {
        return Err(TlsError::KeyExchangeFailed);
    }
    let mut peer_key = [0u8; 32];
    peer_key.copy_from_slice(&server_key_data);
    let shared_secret = crate::crypto::x25519::x25519(private_key, &peer_key);

    // Build transcript from the messages we already have
    let mut transcript = Vec::with_capacity(4096);
    transcript.extend_from_slice(client_hello);
    transcript.extend_from_slice(server_hello);

    let hash_len = cipher_suite.hash_len();
    let transcript_hash = compute_hash(&transcript, cipher_suite);

    // Key schedule
    let zero_key = alloc::vec![0u8; hash_len];
    let empty_hash = compute_hash(&[], cipher_suite);

    let early_secret = hkdf_extract(cipher_suite, &zero_key, &zero_key);
    let derived1 = derive_secret(cipher_suite, &early_secret, b"derived", &empty_hash);
    let handshake_secret = hkdf_extract(cipher_suite, &derived1, &shared_secret);

    let s_hs_secret = derive_secret(
        cipher_suite,
        &handshake_secret,
        b"s hs traffic",
        &transcript_hash,
    );
    let c_hs_secret = derive_secret(
        cipher_suite,
        &handshake_secret,
        b"c hs traffic",
        &transcript_hash,
    );

    let s_hs_key = hkdf_expand_label(
        cipher_suite,
        &s_hs_secret,
        b"key",
        &[],
        cipher_suite.key_len(),
    );
    let s_hs_iv = hkdf_expand_label(
        cipher_suite,
        &s_hs_secret,
        b"iv",
        &[],
        cipher_suite.iv_len(),
    );

    // Receive encrypted server handshake messages
    let mut server_seq: u64 = 0;
    let mut server_finished_received = false;

    while !server_finished_received {
        let mut hdr = [0u8; RECORD_HEADER_SIZE];
        recv_exact_raw(fd, &mut hdr)?;
        let header = RecordHeader::parse(&hdr);
        if header.length as usize > MAX_RECORD_PAYLOAD + 256 {
            return Err(TlsError::RecordOverflow);
        }

        let mut record_body = alloc::vec![0u8; header.length as usize];
        recv_exact_raw(fd, &mut record_body)?;

        // Check for unencrypted ChangeCipherSpec (middlebox compatibility)
        if header.content_type == ContentType::ChangeCipherSpec as u8 {
            continue; // Ignore CCS in TLS 1.3
        }

        let plaintext =
            decrypt_tls13_record_raw(cipher_suite, &s_hs_key, &s_hs_iv, server_seq, &record_body)?;
        server_seq += 1;

        if plaintext.is_empty() {
            continue;
        }

        let real_ct = plaintext[plaintext.len() - 1];
        let msg_data = &plaintext[..plaintext.len() - 1];

        if real_ct == ContentType::Handshake as u8 {
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

                transcript.extend_from_slice(&msg_data[pos..msg_end]);

                if msg_type == 20 {
                    // Finished
                    server_finished_received = true;
                }
                pos = msg_end;
            }
        } else if real_ct == ContentType::Alert as u8 {
            return Err(TlsError::AlertReceived);
        }
    }

    // Send client Finished
    let transcript_hash_finished = compute_hash(&transcript, cipher_suite);
    let c_hs_key = hkdf_expand_label(
        cipher_suite,
        &c_hs_secret,
        b"key",
        &[],
        cipher_suite.key_len(),
    );
    let c_hs_iv = hkdf_expand_label(
        cipher_suite,
        &c_hs_secret,
        b"iv",
        &[],
        cipher_suite.iv_len(),
    );

    let finished_key = hkdf_expand_label(cipher_suite, &c_hs_secret, b"finished", &[], hash_len);
    let verify_data = compute_hmac(cipher_suite, &finished_key, &transcript_hash_finished);

    let mut finished_msg = Vec::with_capacity(4 + verify_data.len());
    finished_msg.push(20);
    let vlen = verify_data.len();
    finished_msg.push((vlen >> 16) as u8);
    finished_msg.push((vlen >> 8) as u8);
    finished_msg.push(vlen as u8);
    finished_msg.extend_from_slice(&verify_data);

    let encrypted = tls13::encrypt_record(
        cipher_suite,
        &c_hs_key,
        &c_hs_iv,
        0,
        &finished_msg,
        ContentType::Handshake as u8,
    );
    send_all(fd, &encrypted)?;

    // Derive application traffic secrets BEFORE adding client Finished to transcript.
    // Per RFC 8446 Section 7.1: app traffic secrets use Transcript-Hash(CH..SF),
    // which is ClientHello through Server Finished — NOT including Client Finished.
    let transcript_hash_for_app = compute_hash(&transcript, cipher_suite);

    transcript.extend_from_slice(&finished_msg);

    let derived2 = derive_secret(cipher_suite, &handshake_secret, b"derived", &empty_hash);
    let master_secret = hkdf_extract(cipher_suite, &derived2, &zero_key);

    let c_app = derive_secret(
        cipher_suite,
        &master_secret,
        b"c ap traffic",
        &transcript_hash_for_app,
    );
    let s_app = derive_secret(
        cipher_suite,
        &master_secret,
        b"s ap traffic",
        &transcript_hash_for_app,
    );

    Ok(tls13::Tls13Handshake {
        cipher_suite,
        client_app_key: hkdf_expand_label(
            cipher_suite,
            &c_app,
            b"key",
            &[],
            cipher_suite.key_len(),
        ),
        client_app_iv: hkdf_expand_label(cipher_suite, &c_app, b"iv", &[], cipher_suite.iv_len()),
        server_app_key: hkdf_expand_label(
            cipher_suite,
            &s_app,
            b"key",
            &[],
            cipher_suite.key_len(),
        ),
        server_app_iv: hkdf_expand_label(cipher_suite, &s_app, b"iv", &[], cipher_suite.iv_len()),
    })
}

fn parse_server_hello_full(server_hello: &[u8]) -> Result<(CipherSuite, (u16, Vec<u8>)), TlsError> {
    if server_hello.len() < 4 {
        return Err(TlsError::UnexpectedMessage);
    }
    let body = &server_hello[4..];
    if body.len() < 38 {
        return Err(TlsError::UnexpectedMessage);
    }

    let mut pos = 34;
    let sid_len = body[pos] as usize;
    pos += 1 + sid_len;

    if pos + 2 > body.len() {
        return Err(TlsError::UnexpectedMessage);
    }
    let suite_id = u16::from_be_bytes([body[pos], body[pos + 1]]);
    let cipher_suite = CipherSuite::from_u16(suite_id).ok_or(TlsError::NoCipherSuite)?;
    pos += 2 + 1; // cipher + compression

    let mut key_share = None;
    if pos + 2 <= body.len() {
        let ext_len = u16::from_be_bytes([body[pos], body[pos + 1]]) as usize;
        pos += 2;
        if pos + ext_len <= body.len() {
            let parsed = extensions::parse_server_hello_extensions(&body[pos..pos + ext_len]);
            key_share = parsed.key_share;
        }
    }

    let key_share = key_share.ok_or(TlsError::KeyExchangeFailed)?;
    Ok((cipher_suite, key_share))
}

// -- Crypto helpers (reused from tls13 module pattern) --

fn compute_hash(data: &[u8], suite: CipherSuite) -> Vec<u8> {
    match suite {
        CipherSuite::Aes256GcmSha384 => crate::crypto::sha384::sha384(data).to_vec(),
        _ => crate::crypto::sha256::sha256(data).to_vec(),
    }
}

fn compute_hmac(suite: CipherSuite, key: &[u8], data: &[u8]) -> Vec<u8> {
    match suite {
        CipherSuite::Aes256GcmSha384 => crate::crypto::hmac::hmac_sha384(key, data).to_vec(),
        _ => crate::crypto::hmac::hmac_sha256(key, data).to_vec(),
    }
}

fn hkdf_extract(suite: CipherSuite, salt: &[u8], ikm: &[u8]) -> Vec<u8> {
    match suite {
        CipherSuite::Aes256GcmSha384 => {
            crate::crypto::hkdf::hkdf_extract_sha384(salt, ikm).to_vec()
        }
        _ => crate::crypto::hkdf::hkdf_extract_sha256(salt, ikm).to_vec(),
    }
}

fn derive_secret(suite: CipherSuite, secret: &[u8], label: &[u8], hash: &[u8]) -> Vec<u8> {
    let mut out = alloc::vec![0u8; suite.hash_len()];
    match suite {
        CipherSuite::Aes256GcmSha384 => {
            crate::crypto::hkdf::tls13_hkdf_expand_label_sha384(secret, label, hash, &mut out)
        }
        _ => crate::crypto::hkdf::tls13_hkdf_expand_label_sha256(secret, label, hash, &mut out),
    }
    out
}

fn hkdf_expand_label(
    suite: CipherSuite,
    secret: &[u8],
    label: &[u8],
    context: &[u8],
    length: usize,
) -> Vec<u8> {
    let mut out = alloc::vec![0u8; length];
    match suite {
        CipherSuite::Aes256GcmSha384 => {
            crate::crypto::hkdf::tls13_hkdf_expand_label_sha384(secret, label, context, &mut out)
        }
        _ => crate::crypto::hkdf::tls13_hkdf_expand_label_sha256(secret, label, context, &mut out),
    }
    out
}

fn build_nonce(iv: &[u8], seq: u64) -> [u8; 12] {
    let mut nonce = [0u8; 12];
    let len = iv.len().min(12);
    nonce[..len].copy_from_slice(&iv[..len]);
    let seq_bytes = seq.to_be_bytes();
    for i in 0..8 {
        nonce[4 + i] ^= seq_bytes[i];
    }
    nonce
}

/// Decrypt a TLS 1.3 record and return the plaintext WITH the content type
/// byte still at the end. Used by both handshake and application data paths.
fn decrypt_tls13_record_raw(
    suite: CipherSuite,
    key: &[u8],
    iv: &[u8],
    seq: u64,
    ciphertext: &[u8],
) -> Result<Vec<u8>, TlsError> {
    if ciphertext.len() < 17 {
        // Need at least 1 byte plaintext + 16 byte tag
        return Err(TlsError::DecryptionFailed);
    }
    let nonce = build_nonce(iv, seq);
    let tag_start = ciphertext.len() - 16;
    let mut data = ciphertext[..tag_start].to_vec();
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&ciphertext[tag_start..]);

    let record_len = ciphertext.len();
    let aad = [23, 0x03, 0x03, (record_len >> 8) as u8, record_len as u8];

    let ok = aead_decrypt(suite, key, &nonce, &aad, &mut data, &tag)?;
    if !ok {
        return Err(TlsError::DecryptionFailed);
    }

    // Strip trailing zeros (TLS 1.3 padding) but keep the content type byte
    while data.len() > 1 && data[data.len() - 1] == 0 && data[data.len() - 2] == 0 {
        data.pop();
    }
    Ok(data)
}

/// Decrypt a TLS 1.3 application-data record.
/// Returns only ApplicationData content. Post-handshake messages
/// (NewSessionTicket etc.) return empty Vec to signal "read next record".
fn decrypt_tls13_app_record(
    suite: CipherSuite,
    key: &[u8],
    iv: &[u8],
    seq: u64,
    ciphertext: &[u8],
) -> Result<Vec<u8>, TlsError> {
    let mut data = decrypt_tls13_record_raw(suite, key, iv, seq, ciphertext)?;
    if data.is_empty() {
        return Ok(Vec::new());
    }

    let content_type = data[data.len() - 1];
    data.pop(); // Remove content type byte

    if content_type == ContentType::ApplicationData as u8 {
        Ok(data)
    } else if content_type == ContentType::Alert as u8 {
        if data.len() >= 2 && data[0] == 1 && data[1] == 0 {
            return Ok(Vec::new()); // close_notify
        }
        Err(TlsError::AlertReceived)
    } else {
        // Post-handshake messages (NewSessionTicket, KeyUpdate, etc.)
        Ok(Vec::new())
    }
}

fn decrypt_tls12_record(
    suite: CipherSuite,
    key: &[u8],
    iv: &[u8],
    seq: u64,
    content_type: u8,
    record_body: &[u8],
) -> Result<Vec<u8>, TlsError> {
    // TLS 1.2 GCM: explicit_nonce(8) + ciphertext + tag(16)
    if record_body.len() < 8 + 16 {
        return Err(TlsError::DecryptionFailed);
    }
    let explicit_nonce = &record_body[..8];
    let ct_and_tag = &record_body[8..];
    let tag_start = ct_and_tag.len() - 16;
    let mut data = ct_and_tag[..tag_start].to_vec();
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&ct_and_tag[tag_start..]);

    let mut nonce = [0u8; 12];
    nonce[..4].copy_from_slice(&iv[..iv.len().min(4)]);
    nonce[4..12].copy_from_slice(explicit_nonce);

    let pt_len = data.len() as u16;
    let mut aad = [0u8; 13];
    aad[..8].copy_from_slice(&seq.to_be_bytes());
    aad[8] = content_type;
    aad[9] = 0x03;
    aad[10] = 0x03;
    aad[11] = (pt_len >> 8) as u8;
    aad[12] = pt_len as u8;

    let ok = aead_decrypt(suite, key, &nonce, &aad, &mut data, &tag)?;
    if !ok {
        return Err(TlsError::DecryptionFailed);
    }
    Ok(data)
}

fn encrypt_tls12_app_record(
    suite: CipherSuite,
    key: &[u8],
    iv: &[u8],
    seq: u64,
    plaintext: &[u8],
) -> Vec<u8> {
    let explicit_nonce = seq.to_be_bytes();
    let mut nonce = [0u8; 12];
    nonce[..4].copy_from_slice(&iv[..iv.len().min(4)]);
    nonce[4..12].copy_from_slice(&explicit_nonce);

    let pt_len = plaintext.len() as u16;
    let mut aad = [0u8; 13];
    aad[..8].copy_from_slice(&seq.to_be_bytes());
    aad[8] = ContentType::ApplicationData as u8;
    aad[9] = 0x03;
    aad[10] = 0x03;
    aad[11] = (pt_len >> 8) as u8;
    aad[12] = pt_len as u8;

    let mut data = plaintext.to_vec();
    let mut tag = [0u8; 16];
    aead_encrypt(suite, key, &nonce, &aad, &mut data, &mut tag);

    let record_len = 8 + data.len() + 16;
    let mut record = Vec::with_capacity(5 + record_len);
    record.push(ContentType::ApplicationData as u8);
    record.push(0x03);
    record.push(0x03);
    record.push((record_len >> 8) as u8);
    record.push(record_len as u8);
    record.extend_from_slice(&explicit_nonce);
    record.extend_from_slice(&data);
    record.extend_from_slice(&tag);
    record
}

fn aead_decrypt(
    suite: CipherSuite,
    key: &[u8],
    nonce: &[u8; 12],
    aad: &[u8],
    data: &mut [u8],
    tag: &[u8; 16],
) -> Result<bool, TlsError> {
    Ok(match suite {
        CipherSuite::Chacha20Poly1305Sha256 | CipherSuite::EcdheRsaChacha20Poly1305Sha256 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            chacha20poly1305::decrypt(&k, nonce, aad, data, tag)
        }
        CipherSuite::Aes128GcmSha256
        | CipherSuite::EcdheRsaAes128GcmSha256
        | CipherSuite::EcdheEcdsaAes128GcmSha256 => {
            let mut k = [0u8; 16];
            k.copy_from_slice(key);
            gcm::AesGcm::new_128(&k).decrypt(nonce, aad, data, tag)
        }
        CipherSuite::Aes256GcmSha384 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            gcm::AesGcm::new_256(&k).decrypt(nonce, aad, data, tag)
        }
        _ => return Err(TlsError::NoCipherSuite),
    })
}

fn aead_encrypt(
    suite: CipherSuite,
    key: &[u8],
    nonce: &[u8; 12],
    aad: &[u8],
    data: &mut [u8],
    tag: &mut [u8; 16],
) {
    match suite {
        CipherSuite::Chacha20Poly1305Sha256 | CipherSuite::EcdheRsaChacha20Poly1305Sha256 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            chacha20poly1305::encrypt(&k, nonce, aad, data, tag);
        }
        CipherSuite::Aes128GcmSha256
        | CipherSuite::EcdheRsaAes128GcmSha256
        | CipherSuite::EcdheEcdsaAes128GcmSha256 => {
            let mut k = [0u8; 16];
            k.copy_from_slice(key);
            gcm::AesGcm::new_128(&k).encrypt(nonce, aad, data, tag);
        }
        CipherSuite::Aes256GcmSha384 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(key);
            gcm::AesGcm::new_256(&k).encrypt(nonce, aad, data, tag);
        }
        _ => {}
    }
}
