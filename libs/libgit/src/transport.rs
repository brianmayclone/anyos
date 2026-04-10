//! Git smart HTTP transport protocol.
//!
//! Implements the git smart HTTP protocol for clone, fetch, and push:
//! - Reference discovery via GET /info/refs
//! - Upload-pack (fetch) via POST /git-upload-pack
//! - Receive-pack (push) via POST /git-receive-pack

use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use crate::pack::{OBJ_COMMIT, OBJ_TREE, OBJ_BLOB, OBJ_TAG, OBJ_REF_DELTA, OBJ_OFS_DELTA};
use crate::oid::Oid;
use crate::remote::GitUrl;
use crate::repo::{Result, Error};

/// A reference advertised by the remote.
#[derive(Debug, Clone)]
pub struct RemoteRef {
    pub oid: Oid,
    pub name: String,
}

/// Capabilities advertised by the remote.
#[derive(Debug, Clone)]
pub struct Capabilities {
    pub multi_ack: bool,
    pub thin_pack: bool,
    pub side_band: bool,
    pub side_band_64k: bool,
    pub ofs_delta: bool,
    pub shallow: bool,
    pub no_progress: bool,
    pub include_tag: bool,
    pub allow_tip_sha1_in_want: bool,
    pub allow_reachable_sha1_in_want: bool,
    pub no_done: bool,
    pub symref_head: Option<String>,
}

impl Capabilities {
    fn new() -> Self {
        Capabilities {
            multi_ack: false,
            thin_pack: false,
            side_band: false,
            side_band_64k: false,
            ofs_delta: false,
            shallow: false,
            no_progress: false,
            include_tag: false,
            allow_tip_sha1_in_want: false,
            allow_reachable_sha1_in_want: false,
            no_done: false,
            symref_head: None,
        }
    }

    fn parse(caps_str: &str) -> Self {
        let mut caps = Capabilities::new();
        for cap in caps_str.split(' ') {
            match cap {
                "multi_ack" => caps.multi_ack = true,
                "thin-pack" => caps.thin_pack = true,
                "side-band" => caps.side_band = true,
                "side-band-64k" => caps.side_band_64k = true,
                "ofs-delta" => caps.ofs_delta = true,
                "shallow" => caps.shallow = true,
                "no-progress" => caps.no_progress = true,
                "include-tag" => caps.include_tag = true,
                "allow-tip-sha1-in-want" => caps.allow_tip_sha1_in_want = true,
                "allow-reachable-sha1-in-want" => caps.allow_reachable_sha1_in_want = true,
                "no-done" => caps.no_done = true,
                s if s.starts_with("symref=HEAD:") => {
                    caps.symref_head = Some(String::from(&s[12..]));
                }
                _ => {}
            }
        }
        caps
    }
}

/// Discover references from a remote via smart HTTP.
pub fn discover_refs(url: &GitUrl, service: &str) -> Result<(Vec<RemoteRef>, Capabilities)> {
    let info_url = url.info_refs_url(service);

    libhttp_client::init();
    let response = libhttp_client::get(&info_url)
        .ok_or(Error::Other(format!("HTTP GET failed: {}", info_url)))?;

    let text = core::str::from_utf8(&response)
        .map_err(|_| Error::Other(String::from("invalid UTF-8 in response")))?;

    parse_ref_discovery(text, service)
}

/// Parse the reference discovery response.
fn parse_ref_discovery(text: &str, _service: &str) -> Result<(Vec<RemoteRef>, Capabilities)> {
    let mut refs = Vec::new();
    let mut caps = Capabilities::new();
    let mut first_line = true;

    for line in text.lines() {
        // Skip service announcement and flush lines
        if line.starts_with('#') || line.len() < 4 {
            continue;
        }

        // Parse pkt-line: first 4 chars are hex length
        let content = if line.len() >= 4 {
            let len_hex = &line[..4];
            if let Ok(len) = usize::from_str_radix(len_hex, 16) {
                if len == 0 {
                    continue; // flush
                }
                &line[4..]
            } else {
                line
            }
        } else {
            line
        };

        // Skip empty or service lines
        if content.is_empty() || content.starts_with("# ") {
            continue;
        }

        // First ref line may contain capabilities after \0
        let (ref_part, caps_part) = if let Some(null_pos) = content.find('\0') {
            (&content[..null_pos], Some(&content[null_pos + 1..]))
        } else {
            (content, None)
        };

        // Parse capabilities from first line
        if first_line {
            if let Some(caps_str) = caps_part {
                caps = Capabilities::parse(caps_str.trim());
            }
            first_line = false;
        }

        // Parse "sha1 refname"
        let ref_part = ref_part.trim();
        if ref_part.len() >= 41 {
            let hex = &ref_part[..40];
            let name = ref_part[41..].trim();
            if let Some(oid) = Oid::from_hex(hex) {
                refs.push(RemoteRef {
                    oid,
                    name: String::from(name),
                });
            }
        }
    }

    Ok((refs, caps))
}

/// Build an upload-pack request body (want/have negotiation).
pub fn build_upload_pack_request(
    wants: &[Oid],
    haves: &[Oid],
    caps: &Capabilities,
) -> Vec<u8> {
    let mut body = Vec::new();

    // Minimal capabilities for initial clone (no haves)
    let cap_str = String::from("ofs-delta agent=agit/1.0");

    // Want lines
    for (i, oid) in wants.iter().enumerate() {
        let line = if i == 0 {
            format!("want {} {}\n", oid.to_hex(), cap_str)
        } else {
            format!("want {}\n", oid.to_hex())
        };
        write_pkt_line(&mut body, &line);
    }

    // Flush after wants
    write_flush(&mut body);

    // Have lines
    for oid in haves {
        let line = format!("have {}\n", oid.to_hex());
        write_pkt_line(&mut body, &line);
    }

    // Done
    write_pkt_line(&mut body, "done\n");

    body
}

/// Build a receive-pack request body (push).
pub fn build_receive_pack_request(
    updates: &[(Oid, Oid, String)], // (old_oid, new_oid, refname)
    pack_data: &[u8],
) -> Vec<u8> {
    let mut body = Vec::new();

    // Reference update lines
    let cap_str = "report-status side-band-64k";
    for (i, (old, new, refname)) in updates.iter().enumerate() {
        let line = if i == 0 {
            format!("{} {} {}\0{}\n", old.to_hex(), new.to_hex(), refname, cap_str)
        } else {
            format!("{} {} {}\n", old.to_hex(), new.to_hex(), refname)
        };
        write_pkt_line(&mut body, &line);
    }

    // Flush after ref updates
    write_flush(&mut body);

    // Pack data follows
    body.extend_from_slice(pack_data);

    body
}

/// Perform git-upload-pack (fetch objects from remote).
/// Fetch pack data and stream it directly into the repository.
/// Returns the number of objects written. Uses constant memory.
pub fn fetch_pack_streamed(
    url: &GitUrl,
    wants: &[Oid],
    haves: &[Oid],
    repo: &crate::repo::Repository,
) -> Result<u32> {
    // Step 1: Discover refs and capabilities
    let (_, caps) = discover_refs(url, "git-upload-pack")?;

    // Step 2: Build request body
    let request_body = build_upload_pack_request(wants, haves, &caps);

    // Step 3: Open streaming connection
    let service_path = format!("{}/git-upload-pack", url.path.trim_end_matches('/'));
    let extra_headers = "Accept: application/x-git-upload-pack-result\r\n";

    if crate::pack::verbose() {
        anyos_std::println!("[fetch] POST https://{}{} ({} bytes)", url.host, service_path, request_body.len());
    }

    let mut stream = crate::stream::HttpStream::post(
        &url.host,
        &service_path,
        &request_body,
        "application/x-git-upload-pack-request",
        extra_headers,
    ).map_err(|e| Error::Other(e))?;

    // Step 4: Skip pkt-line NAK/ACK before PACK data
    // Read until we find "PACK" magic
    let mut prefix = [0u8; 4];
    let mut skipped = 0;
    let mut debug_buf = Vec::new();
    loop {
        if !stream.read_exact(&mut prefix) {
            if crate::pack::verbose() {
                anyos_std::println!("[fetch] EOF before PACK. Got {} bytes: {:?}",
                    debug_buf.len(),
                    core::str::from_utf8(&debug_buf).unwrap_or("(binary)"));
            }
            return Err(Error::Other(String::from("EOF before PACK header")));
        }
        if skipped == 0 {
            debug_buf.extend_from_slice(&prefix);
        }
        if &prefix == b"PACK" {
            break;
        }
        // Not PACK yet — skip one byte and try again (slide window)
        skipped += 1;
        if skipped <= 256 {
            debug_buf.push(prefix[0]);
        }
        prefix[0] = prefix[1];
        prefix[1] = prefix[2];
        prefix[2] = prefix[3];
        let mut one = [0u8; 1];
        if !stream.read_exact(&mut one) {
            if crate::pack::verbose() {
                anyos_std::println!("[fetch] EOF scanning for PACK. Got: {:?}",
                    core::str::from_utf8(&debug_buf).unwrap_or("(binary)"));
            }
            return Err(Error::Other(String::from("EOF before PACK header")));
        }
        prefix[3] = one[0];
        if skipped <= 256 {
            debug_buf.push(one[0]);
        }

        if skipped > 1024 {
            anyos_std::print!("First 64 bytes (hex): ");
            for (i, b) in debug_buf.iter().take(64).enumerate() {
                anyos_std::print!("{:02x} ", b);
                if (i + 1) % 16 == 0 { anyos_std::println!(); }
            }
            anyos_std::println!();
            // Also try as text
            if let Ok(text) = core::str::from_utf8(&debug_buf[..debug_buf.len().min(128)]) {
                anyos_std::println!("As text: {}", text);
            }
            return Err(Error::Other(String::from("PACK header not found in first 1KB")));
        }
    }

    // Step 5: Read pack version + count (8 bytes)
    let mut pack_hdr = [0u8; 8];
    if !stream.read_exact(&mut pack_hdr) {
        return Err(Error::Other(String::from("truncated pack header")));
    }

    // "PACK" already consumed; pack_hdr has version(4) + count(4)
    // But parse_pack_streamed expects to read from AFTER the 12-byte header.
    // We need to re-inject the version+count into the stream... or just
    // call parse_pack_streamed which reads the 12-byte header itself.
    // Let's skip re-injection: we already have version+count.

    let version = crate::pack::read_u32_be(&pack_hdr[0..4]);
    let num_objects = crate::pack::read_u32_be(&pack_hdr[4..8]);

    if crate::pack::verbose() {
        anyos_std::println!("[fetch] PACK v{} with {} objects (streaming)", version, num_objects);
    }

    // Step 6: Stream-parse objects directly into the repository
    let count = stream_parse_objects(&mut stream, repo, num_objects)?;

    if crate::pack::verbose() {
        anyos_std::println!("[fetch] {} objects written, {} bytes received", count, stream.total_read);
    }

    // Stream is closed on drop
    Ok(count)
}

/// Parse pack objects from a stream and write to repository.
fn stream_parse_objects(
    stream: &mut crate::stream::HttpStream,
    repo: &crate::repo::Repository,
    num_objects: u32,
) -> Result<u32> {
    use crate::object::{Object, ObjectType};

    let mut resolved: Vec<(Oid, Vec<u8>, u8)> = Vec::new();
    let mut count = 0u32;

    for i in 0..num_objects {
        let (obj_type_raw, _size) = crate::pack::read_entry_header_stream(stream)
            .map_err(|e| Error::Other(e))?;

        if i % 200 == 0 || i == num_objects - 1 {
            anyos_std::print!("\rReceiving objects: {}% ({}/{})", (i + 1) * 100 / num_objects, i + 1, num_objects);
        }

        match obj_type_raw {
            OBJ_COMMIT | OBJ_TREE | OBJ_BLOB | OBJ_TAG => {
                let inflated = crate::pack::inflate_from_stream(stream)
                    .map_err(|e| Error::Other(e))?;

                let obj_type = match obj_type_raw {
                    OBJ_COMMIT => ObjectType::Commit,
                    OBJ_TREE => ObjectType::Tree,
                    OBJ_BLOB => ObjectType::Blob,
                    _ => ObjectType::Tag,
                };

                let oid = Oid::from_bytes(crate::sha1::hash_object(obj_type.as_str(), &inflated));
                let obj = Object { obj_type, data: inflated.clone() };
                let _ = repo.write_object(&obj);
                resolved.push((oid, inflated, obj_type_raw));
                count += 1;
            }
            OBJ_REF_DELTA => {
                let mut base_sha = [0u8; 20];
                if !stream.read_exact(&mut base_sha) {
                    break;
                }
                let base_oid = Oid::from_bytes(base_sha);

                let delta_data = crate::pack::inflate_from_stream(stream)
                    .map_err(|e| Error::Other(e))?;

                let base = resolved.iter().find(|(o, _, _)| *o == base_oid)
                    .map(|(_, d, t)| (d.clone(), *t))
                    .or_else(|| {
                        repo.read_object(&base_oid).ok().map(|o| {
                            let t = match o.obj_type {
                                ObjectType::Commit => OBJ_COMMIT,
                                ObjectType::Tree => OBJ_TREE,
                                ObjectType::Blob => OBJ_BLOB,
                                ObjectType::Tag => OBJ_TAG,
                            };
                            (o.data, t)
                        })
                    });

                if let Some((base_data, base_type)) = base {
                    let result = crate::pack::apply_delta(&base_data, &delta_data);
                    let obj_type = crate::pack::pack_type_to_object_type(base_type);
                    let oid = Oid::from_bytes(crate::sha1::hash_object(obj_type.as_str(), &result));
                    let obj = Object { obj_type, data: result.clone() };
                    let _ = repo.write_object(&obj);
                    resolved.push((oid, result, base_type));
                    count += 1;
                }
            }
            OBJ_OFS_DELTA => {
                let _offset = crate::pack::read_ofs_offset_stream(stream)
                    .map_err(|e| Error::Other(e))?;
                let delta_data = crate::pack::inflate_from_stream(stream)
                    .map_err(|e| Error::Other(e))?;

                if let Some((_, base_data, base_type)) = resolved.last() {
                    let result = crate::pack::apply_delta(base_data, &delta_data);
                    let obj_type = crate::pack::pack_type_to_object_type(*base_type);
                    let oid = Oid::from_bytes(crate::sha1::hash_object(obj_type.as_str(), &result));
                    let obj = Object { obj_type, data: result.clone() };
                    let _ = repo.write_object(&obj);
                    resolved.push((oid, result, *base_type));
                    count += 1;
                }
            }
            _ => {}
        }

        // Cap delta cache
        if resolved.len() > 8192 {
            resolved.drain(..resolved.len() - 4096);
        }
    }

    anyos_std::println!("\rReceiving objects: 100% ({}/{}), done.", num_objects, num_objects);
    Ok(count)
}

/// Perform git-receive-pack (push objects to remote).
pub fn push_pack(
    url: &GitUrl,
    updates: &[(Oid, Oid, String)],
    pack_data: &[u8],
) -> Result<String> {
    let request_body = build_receive_pack_request(updates, pack_data);
    let service_url = url.service_url("git-receive-pack");
    let content_type = "application/x-git-receive-pack-request";

    libhttp_client::init();
    let response = libhttp_client::post(&service_url, &request_body, content_type)
        .ok_or(Error::Other(format!("POST failed: {}", service_url)))?;

    let text = core::str::from_utf8(&response).unwrap_or("(binary)");
    Ok(String::from(text))
}

// ── pkt-line encoding ───────────────────────────────────────────────────────

fn write_pkt_line(buf: &mut Vec<u8>, line: &str) {
    let len = line.len() + 4; // 4 hex digits for length
    let hex = format!("{:04x}", len);
    buf.extend_from_slice(hex.as_bytes());
    buf.extend_from_slice(line.as_bytes());
}

fn write_flush(buf: &mut Vec<u8>) {
    buf.extend_from_slice(b"0000");
}

/// Find where "PACK" header starts in response data.
fn find_pack_start(data: &[u8]) -> usize {
    for i in 0..data.len().saturating_sub(4) {
        if &data[i..i + 4] == b"PACK" {
            return i;
        }
    }
    0
}

/// Parse a pkt-line response to extract lines.
pub fn parse_pkt_lines(data: &[u8]) -> Vec<Vec<u8>> {
    let mut lines = Vec::new();
    let mut pos = 0;

    while pos + 4 <= data.len() {
        let hex = core::str::from_utf8(&data[pos..pos + 4]).unwrap_or("0000");
        let len = usize::from_str_radix(hex, 16).unwrap_or(0);
        pos += 4;

        if len == 0 {
            continue; // flush
        }
        if len == 1 {
            continue; // delimiter
        }

        let payload_len = len.saturating_sub(4);
        let end = core::cmp::min(pos + payload_len, data.len());
        lines.push(data[pos..end].to_vec());
        pos = end;
    }

    lines
}
