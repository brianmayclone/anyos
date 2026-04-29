//! Git smart HTTP transport protocol.
//!
//! Implements the git smart HTTP protocol for clone, fetch, and push:
//! - Reference discovery via GET /info/refs
//! - Upload-pack (fetch) via POST /git-upload-pack
//! - Receive-pack (push) via POST /git-receive-pack

use crate::oid::Oid;
use crate::pack::{OBJ_BLOB, OBJ_COMMIT, OBJ_OFS_DELTA, OBJ_REF_DELTA, OBJ_TAG, OBJ_TREE};
use crate::remote::GitUrl;
use crate::repo::{Error, Result};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

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
    let response = libhttp_client::get(&info_url).ok_or_else(|| {
        let status = libhttp_client::last_status();
        let err = libhttp_client::last_error();
        Error::Other(format!(
            "HTTP GET failed: {} (status {}, error {})",
            info_url, status, err
        ))
    })?;

    parse_ref_discovery(&response, service)
}

/// Parse the reference discovery response.
fn parse_ref_discovery(response: &[u8], service: &str) -> Result<(Vec<RemoteRef>, Capabilities)> {
    if response.len() >= 4 && pkt_len(&response[..4]).is_some() {
        parse_pkt_ref_discovery(response, service)
    } else {
        let text = core::str::from_utf8(response)
            .map_err(|_| Error::Other(String::from("invalid UTF-8 in response")))?;
        parse_line_ref_discovery(text)
    }
}

fn parse_pkt_ref_discovery(
    response: &[u8],
    service: &str,
) -> Result<(Vec<RemoteRef>, Capabilities)> {
    let mut refs = Vec::new();
    let mut caps = Capabilities::new();
    let mut first_ref = true;
    let mut pos = 0;

    while pos < response.len() {
        if response.len() - pos < 4 {
            let rest = trim_ascii(&response[pos..]);
            if rest.is_empty() {
                break;
            }
            return Err(Error::Other(String::from("truncated pkt-line length")));
        }

        let len = pkt_len(&response[pos..pos + 4])
            .ok_or_else(|| Error::Other(String::from("invalid pkt-line length")))?;
        pos += 4;

        if len == 0 {
            continue;
        }
        if len < 4 {
            return Err(Error::Other(String::from("invalid pkt-line length")));
        }

        let payload_len = len - 4;
        if response.len() - pos < payload_len {
            return Err(Error::Other(String::from("truncated pkt-line payload")));
        }

        let mut content = &response[pos..pos + payload_len];
        pos += payload_len;

        if content.ends_with(b"\n") {
            content = &content[..content.len() - 1];
        }
        if content.ends_with(b"\r") {
            content = &content[..content.len() - 1];
        }

        parse_ref_advertisement(content, service, &mut first_ref, &mut refs, &mut caps)?;
    }

    Ok((refs, caps))
}

fn parse_line_ref_discovery(text: &str) -> Result<(Vec<RemoteRef>, Capabilities)> {
    let mut refs = Vec::new();
    let mut caps = Capabilities::new();
    let mut first_ref = true;

    for line in text.lines() {
        parse_ref_advertisement(line.as_bytes(), "", &mut first_ref, &mut refs, &mut caps)?;
    }

    Ok((refs, caps))
}

fn parse_ref_advertisement(
    content: &[u8],
    service: &str,
    first_ref: &mut bool,
    refs: &mut Vec<RemoteRef>,
    caps: &mut Capabilities,
) -> Result<()> {
    let content = trim_ascii(content);
    if content.is_empty() || content.starts_with(b"# ") {
        return Ok(());
    }
    if !service.is_empty() && content == format!("# service={}", service).as_bytes() {
        return Ok(());
    }

    let null_pos = content.iter().position(|b| *b == 0);
    let (ref_part, caps_part) = match null_pos {
        Some(pos) => (&content[..pos], Some(&content[pos + 1..])),
        None => (content, None),
    };

    if *first_ref {
        if let Some(caps_bytes) = caps_part {
            let caps_str = core::str::from_utf8(trim_ascii(caps_bytes))
                .map_err(|_| Error::Other(String::from("invalid UTF-8 in capabilities")))?;
            *caps = Capabilities::parse(caps_str);
        }
        *first_ref = false;
    }

    let ref_part = trim_ascii(ref_part);
    if ref_part.len() < 41 || !is_ascii_whitespace(ref_part[40]) {
        return Ok(());
    }

    let hex = core::str::from_utf8(&ref_part[..40])
        .map_err(|_| Error::Other(String::from("invalid UTF-8 in object id")))?;
    let name = core::str::from_utf8(trim_ascii(&ref_part[41..]))
        .map_err(|_| Error::Other(String::from("invalid UTF-8 in ref name")))?;

    if let Some(oid) = Oid::from_hex(hex) {
        refs.push(RemoteRef {
            oid,
            name: String::from(name),
        });
    }

    Ok(())
}

fn pkt_len(hex: &[u8]) -> Option<usize> {
    if hex.len() != 4 {
        return None;
    }
    let mut value = 0usize;
    for b in hex {
        value <<= 4;
        value |= match *b {
            b'0'..=b'9' => (*b - b'0') as usize,
            b'a'..=b'f' => (*b - b'a' + 10) as usize,
            b'A'..=b'F' => (*b - b'A' + 10) as usize,
            _ => return None,
        };
    }
    Some(value)
}

fn trim_ascii(mut bytes: &[u8]) -> &[u8] {
    while let Some((first, rest)) = bytes.split_first() {
        if !is_ascii_whitespace(*first) {
            break;
        }
        bytes = rest;
    }
    while let Some((last, rest)) = bytes.split_last() {
        if !is_ascii_whitespace(*last) {
            break;
        }
        bytes = rest;
    }
    bytes
}

fn is_ascii_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n')
}

/// Build an upload-pack request body (want/have negotiation).
pub fn build_upload_pack_request(wants: &[Oid], haves: &[Oid], _caps: &Capabilities) -> Vec<u8> {
    let mut body = Vec::new();

    // Minimal capabilities for initial clone (no haves)
    let cap_str = String::from("ofs-delta agent=git/anyos");

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
            format!(
                "{} {} {}\0{}\n",
                old.to_hex(),
                new.to_hex(),
                refname,
                cap_str
            )
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

    fetch_pack_streamed_with_caps(url, wants, haves, repo, &caps)
}

/// Fetch pack data using capabilities already obtained from reference discovery.
pub fn fetch_pack_streamed_with_caps(
    url: &GitUrl,
    wants: &[Oid],
    haves: &[Oid],
    repo: &crate::repo::Repository,
    caps: &Capabilities,
) -> Result<u32> {
    // Step 2: Build request body
    let request_body = build_upload_pack_request(wants, haves, caps);

    // Step 3: Open streaming connection
    let service_path = format!("{}/git-upload-pack", url.path.trim_end_matches('/'));
    let extra_headers = "Accept: application/x-git-upload-pack-result\r\n";

    if crate::pack::verbose() {
        anyos_std::println!(
            "[fetch] POST https://{}{} ({} bytes)",
            url.host,
            service_path,
            request_body.len()
        );
    }

    let mut stream = crate::stream::HttpStream::post(
        &url.host,
        &service_path,
        &request_body,
        "application/x-git-upload-pack-request",
        extra_headers,
    )
    .map_err(|e| Error::Other(e))?;

    // Step 4: Skip pkt-line NAK/ACK before PACK data
    // Read until we find "PACK" magic
    let mut prefix = [0u8; 4];
    let mut skipped = 0;
    let mut debug_buf = Vec::new();
    if !stream.read_exact(&mut prefix) {
        return Err(Error::Other(String::from("EOF before PACK header")));
    }
    debug_buf.extend_from_slice(&prefix);
    loop {
        if &prefix == b"PACK" {
            break;
        }
        // Not PACK yet: advance one byte and keep the 4-byte rolling window.
        skipped += 1;
        prefix[0] = prefix[1];
        prefix[1] = prefix[2];
        prefix[2] = prefix[3];
        let mut one = [0u8; 1];
        if !stream.read_exact(&mut one) {
            if crate::pack::verbose() {
                anyos_std::println!(
                    "[fetch] EOF scanning for PACK. Got: {:?}",
                    core::str::from_utf8(&debug_buf).unwrap_or("(binary)")
                );
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
                if (i + 1) % 16 == 0 {
                    anyos_std::println!();
                }
            }
            anyos_std::println!();
            // Also try as text
            if let Ok(text) = core::str::from_utf8(&debug_buf[..debug_buf.len().min(128)]) {
                anyos_std::println!("As text: {}", text);
            }
            return Err(Error::Other(String::from(
                "PACK header not found in first 1KB",
            )));
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
        anyos_std::println!(
            "[fetch] PACK v{} with {} objects (streaming)",
            version,
            num_objects
        );
    }

    // Step 6: Stream-parse objects directly into the repository
    let pack_base_pos = stream.decoded_pos().saturating_sub(12);
    let count = stream_parse_objects(&mut stream, repo, num_objects, pack_base_pos)?;

    if crate::pack::verbose() {
        anyos_std::println!(
            "[fetch] {} objects written, {} bytes received",
            count,
            stream.total_read
        );
    }

    // Stream is closed on drop
    Ok(count)
}

/// Parse pack objects from a stream and write to repository.
fn stream_parse_objects(
    stream: &mut crate::stream::HttpStream,
    repo: &crate::repo::Repository,
    num_objects: u32,
    pack_base_pos: usize,
) -> Result<u32> {
    use crate::object::{Object, ObjectType};

    let mut resolved: Vec<(usize, Oid, Vec<u8>, u8)> = Vec::new();
    let mut offset_index: Vec<(usize, Oid, u8)> = Vec::new();
    let mut resolved_bytes = 0usize;
    let mut count = 0u32;

    for i in 0..num_objects {
        let entry_start = stream.decoded_pos().saturating_sub(pack_base_pos);
        let (obj_type_raw, size) =
            crate::pack::read_entry_header_stream(stream).map_err(|e| Error::Other(e))?;

        if i % 200 == 0 || i == num_objects - 1 {
            anyos_std::print!(
                "\rReceiving objects: {}% ({}/{})",
                (i + 1) * 100 / num_objects,
                i + 1,
                num_objects
            );
        }

        match obj_type_raw {
            OBJ_COMMIT | OBJ_TREE | OBJ_BLOB | OBJ_TAG => {
                let inflated =
                    crate::pack::inflate_from_stream(stream, size).map_err(|e| Error::Other(e))?;

                let obj_type = match obj_type_raw {
                    OBJ_COMMIT => ObjectType::Commit,
                    OBJ_TREE => ObjectType::Tree,
                    OBJ_BLOB => ObjectType::Blob,
                    _ => ObjectType::Tag,
                };

                let oid = Oid::from_bytes(crate::sha1::hash_object(obj_type.as_str(), &inflated));
                let obj = Object {
                    obj_type,
                    data: inflated.clone(),
                };
                let _ = repo.write_object(&obj);
                push_delta_cache(
                    &mut resolved,
                    &mut offset_index,
                    &mut resolved_bytes,
                    entry_start,
                    oid,
                    inflated,
                    obj_type_raw,
                );
                count += 1;
            }
            OBJ_REF_DELTA => {
                let mut base_sha = [0u8; 20];
                if !stream.read_exact(&mut base_sha) {
                    break;
                }
                let base_oid = Oid::from_bytes(base_sha);

                let delta_data =
                    crate::pack::inflate_from_stream(stream, size).map_err(|e| Error::Other(e))?;

                let base = resolved
                    .iter()
                    .find(|(_, o, _, _)| *o == base_oid)
                    .map(|(_, _, d, t)| (d.clone(), *t))
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
                    let obj = Object {
                        obj_type,
                        data: result.clone(),
                    };
                    let _ = repo.write_object(&obj);
                    push_delta_cache(
                        &mut resolved,
                        &mut offset_index,
                        &mut resolved_bytes,
                        entry_start,
                        oid,
                        result,
                        base_type,
                    );
                    count += 1;
                }
            }
            OBJ_OFS_DELTA => {
                let offset =
                    crate::pack::read_ofs_offset_stream(stream).map_err(|e| Error::Other(e))?;
                let delta_data =
                    crate::pack::inflate_from_stream(stream, size).map_err(|e| Error::Other(e))?;

                let base_abs = entry_start.saturating_sub(offset);
                let base = resolved
                    .iter()
                    .find(|(off, _, _, _)| *off == base_abs)
                    .map(|(_, _, data, obj_type)| (data.clone(), *obj_type))
                    .or_else(|| {
                        offset_index
                            .iter()
                            .find(|(off, _, _)| *off == base_abs)
                            .and_then(|(_, oid, obj_type)| {
                                repo.read_object(oid).ok().map(|obj| (obj.data, *obj_type))
                            })
                    });

                if let Some((base_data, base_type)) = base {
                    let result = crate::pack::apply_delta(&base_data, &delta_data);
                    let obj_type = crate::pack::pack_type_to_object_type(base_type);
                    let oid = Oid::from_bytes(crate::sha1::hash_object(obj_type.as_str(), &result));
                    let obj = Object {
                        obj_type,
                        data: result.clone(),
                    };
                    let _ = repo.write_object(&obj);
                    push_delta_cache(
                        &mut resolved,
                        &mut offset_index,
                        &mut resolved_bytes,
                        entry_start,
                        oid,
                        result,
                        base_type,
                    );
                    count += 1;
                }
            }
            _ => {}
        }
    }

    anyos_std::println!(
        "\rReceiving objects: 100% ({}/{}), done.",
        num_objects,
        num_objects
    );
    Ok(count)
}

const DELTA_CACHE_MAX_OBJECTS: usize = 128;
const DELTA_CACHE_MAX_BYTES: usize = 4 * 1024 * 1024;

fn push_delta_cache(
    cache: &mut Vec<(usize, Oid, Vec<u8>, u8)>,
    offset_index: &mut Vec<(usize, Oid, u8)>,
    cache_bytes: &mut usize,
    pack_offset: usize,
    oid: Oid,
    data: Vec<u8>,
    obj_type: u8,
) {
    offset_index.push((pack_offset, oid, obj_type));

    if data.len() > DELTA_CACHE_MAX_BYTES / 2 {
        return;
    }

    *cache_bytes += data.len();
    cache.push((pack_offset, oid, data, obj_type));

    while cache.len() > DELTA_CACHE_MAX_OBJECTS || *cache_bytes > DELTA_CACHE_MAX_BYTES {
        if cache.is_empty() {
            *cache_bytes = 0;
            break;
        }
        let (_, _, data, _) = cache.remove(0);
        *cache_bytes = cache_bytes.saturating_sub(data.len());
    }
}

/// Perform git-receive-pack (push objects to remote).
pub fn push_pack(url: &GitUrl, updates: &[(Oid, Oid, String)], pack_data: &[u8]) -> Result<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    use alloc::string::ToString;

    fn pkt(payload: &[u8]) -> Vec<u8> {
        let mut out = format!("{:04x}", payload.len() + 4).into_bytes();
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn parses_ref_discovery_pkt_lines_without_line_slicing() {
        let mut response = Vec::new();
        response.extend_from_slice(&pkt(b"# service=git-upload-pack\n"));
        response.extend_from_slice(b"0000");
        response.extend_from_slice(&pkt(
            b"1111111111111111111111111111111111111111 HEAD\0multi_ack side-band-64k ofs-delta symref=HEAD:refs/heads/main\n",
        ));
        response.extend_from_slice(&pkt(
            b"2222222222222222222222222222222222222222 refs/heads/main\n",
        ));
        response.extend_from_slice(b"0000");

        let (refs, caps) = parse_ref_discovery(&response, "git-upload-pack").unwrap();

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "HEAD");
        assert_eq!(refs[1].name, "refs/heads/main");
        assert!(caps.multi_ack);
        assert!(caps.side_band_64k);
        assert!(caps.ofs_delta);
        assert_eq!(caps.symref_head, Some("refs/heads/main".to_string()));
    }

    #[test]
    fn rejects_truncated_ref_discovery_pkt_lines() {
        let response = b"003f1111111111111111111111111111111111111111 HEAD\n";
        let err = parse_ref_discovery(response, "git-upload-pack").unwrap_err();

        match err {
            Error::Other(message) => assert!(message.contains("truncated pkt-line payload")),
            _ => panic!("unexpected error: {:?}", err),
        }
    }
}
