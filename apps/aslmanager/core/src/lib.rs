//! Pure-logic helpers for `aslmanager`.
//!
//! Everything in this crate is allocator-only (no syscalls, no I/O) so it
//! works in both `no_std` (anyOS user program build) and `std` (host
//! `cargo test`) environments. The aslmanager binary re-uses these helpers
//! via the `aslmanager_core` dependency.
//!
//! ADR references:
//! - ADR-0010 (Enterprise-Quality-Bar): test coverage is mandatory for
//!   user-input boundaries. Whitelists and safety checks live here.
//! - ADR-0011 (Image-Trust): `artifact_size_ok` and `raw_disk_header_ok`
//!   implement the stage-1 verification described in the ADR.

#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;

// ============================================================================
//  URL whitelisting
// ============================================================================

pub const DEBIAN_URL_PREFIX: &str = "https://deb.debian.org/debian/";
pub const DEBIAN_CLOUD_URL_PREFIX: &str = "https://cloud.debian.org/images/cloud/";
pub const DEBIAN_CLOUD_HTTP_URL_PREFIX: &str = "http://cloud.debian.org/images/cloud/";

/// True if `url` points to a known-good Debian distribution endpoint.
///
/// Rejects URLs containing `/../` (path traversal) or NUL bytes (truncation
/// attacks against C-string consumers downstream).
pub fn is_allowed_debian_url(url: &str) -> bool {
    (url.starts_with(DEBIAN_URL_PREFIX)
        || url.starts_with(DEBIAN_CLOUD_URL_PREFIX)
        || url.starts_with(DEBIAN_CLOUD_HTTP_URL_PREFIX))
        && !url.contains('\0')
        && !url.contains("/../")
}

// ============================================================================
//  Path safety
// ============================================================================

/// True if `path` is a safe absolute directory path:
/// - starts with `/`
/// - has at least one path component
/// - no NUL byte
/// - no `/../` segment (defense against traversal)
/// - no trailing `/..`
/// - no trailing `/`
pub fn is_safe_absolute_dir(path: &str) -> bool {
    path.starts_with('/')
        && path.len() > 1
        && !path.contains('\0')
        && !path.contains("/../")
        && !path.ends_with("/..")
        && !path.ends_with('/')
}

/// True if `path` is inside `images_dir` (with a literal `/` boundary)
/// and contains no path-traversal tokens.
///
/// Pure function — no I/O. The caller is responsible for ensuring
/// `images_dir` itself is safe.
pub fn is_safe_artifact_path(path: &str, images_dir: &str) -> bool {
    let in_images = path.len() > images_dir.len()
        && path.starts_with(images_dir)
        && path.as_bytes().get(images_dir.len()) == Some(&b'/');
    in_images
        && !path.contains('\0')
        && !path.contains("/../")
        && !path.ends_with("/..")
}

// ============================================================================
//  Config parsing
// ============================================================================

/// Split a `key = value` line into `(key, value)` with trimming. Returns
/// `None` if no `=` is present or either side is empty after trimming.
pub fn split_key_value(line: &str) -> Option<(&str, &str)> {
    let pos = line.find('=')?;
    let key = line[..pos].trim();
    let value = line[pos + 1..].trim();
    if key.is_empty() || value.is_empty() {
        None
    } else {
        Some((key, value))
    }
}

/// Join two path components with exactly one `/` between them.
///
/// `String` is part of `alloc`, so this works in both `std` and `no_std`
/// environments that have an allocator. We use `std` here since this crate
/// is the test-friendly side.
pub fn join_path(parent: &str, child: &str) -> String {
    if parent.ends_with('/') {
        format!("{}{}", parent, child)
    } else {
        format!("{}/{}", parent, child)
    }
}

// ============================================================================
//  Artifact validation (ADR-0011 stage 1)
// ============================================================================

/// Sentinel value indicating that a 32-bit stat could not represent the
/// real file size (i.e. the file is >= 4 GiB). When the kernel exposes a
/// 64-bit stat this can be replaced with a real boundary check.
pub const STAT_SIZE_OVERFLOW_SENTINEL: u32 = u32::MAX;

/// True if a 32-bit stat size is large enough for the artifact and is not
/// at the overflow sentinel.
///
/// Pure function — no I/O.
pub fn artifact_size_ok(stat_size: u32, min_size_bytes: u64) -> bool {
    if stat_size == STAT_SIZE_OVERFLOW_SENTINEL {
        return false;
    }
    (stat_size as u64) >= min_size_bytes
}

/// True if the prefix bytes look like a bootable raw disk image:
/// at least 512 bytes were read and the MBR boot signature 0x55 0xAA is
/// present at offset 510.
///
/// Pure function — no I/O.
pub fn raw_disk_header_ok(header: &[u8], read: usize) -> bool {
    read >= 512 && header.len() >= 512 && header[510] == 0x55 && header[511] == 0xaa
}

// ============================================================================
//  HTTP fallback policy
// ============================================================================

/// True if a failed HTTPS download warrants a HTTP-fallback retry.
///
/// HTTP-status >= 400 means the server answered — no transport problem —
/// and we should not silently downgrade. Otherwise: connection-class
/// errors (DNS, TCP, send, no-response, TLS) and a fully empty status
/// (no answer at all) trigger fallback.
pub fn should_try_http_fallback(status: u32, error: u32) -> bool {
    if status >= 400 {
        return false;
    }
    matches!(error, 2 | 3 | 4 | 5 | 7) || status == 0
}

/// Translate an `https://cloud.debian.org/images/cloud/...` URL into the
/// equivalent `http://cloud.debian.org/images/cloud/...` URL for fallback.
/// Returns `None` if the input is not a Debian Cloud HTTPS URL.
pub fn official_http_fallback(url: &str) -> Option<String> {
    if !url.starts_with(DEBIAN_CLOUD_URL_PREFIX) {
        return None;
    }
    let suffix = &url[DEBIAN_CLOUD_URL_PREFIX.len()..];
    Some(format!("{}{}", DEBIAN_CLOUD_HTTP_URL_PREFIX, suffix))
}

// ============================================================================
//  Image hashing helpers (ADR-0011 stage 2)
// ============================================================================
//
// SHA-512 itself lives in `libtls::crypto::sha512` (already used by the
// TLS stack — no duplicate implementation). This module just re-exports
// the type aliases the installer needs and provides hex/encoding helpers.

pub use libtls::crypto::sha512::Sha512;

/// Hex-encode a byte slice as lowercase ASCII. No allocation control —
/// uses `String`. Suitable for log output and hash comparison.
pub fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

/// Constant-time comparison of two equal-length byte slices. Returns
/// `false` if lengths differ. Used for hash comparison so that timing
/// observation cannot reveal how many leading bytes matched.
pub fn const_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for i in 0..a.len() {
        diff |= a[i] ^ b[i];
    }
    diff == 0
}

/// Decode a 128-character lowercase hex string into a 64-byte SHA-512
/// digest. Returns `None` if the input has the wrong length or contains
/// non-hex characters.
pub fn parse_sha512_hex(text: &str) -> Option<[u8; 64]> {
    let bytes = text.as_bytes();
    if bytes.len() != 128 {
        return None;
    }
    let mut out = [0u8; 64];
    for i in 0..64 {
        let hi = hex_digit(bytes[i * 2])?;
        let lo = hex_digit(bytes[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

fn hex_digit(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ============================================================================
//  Numeric parsing
// ============================================================================

/// Parse a non-negative ASCII integer with saturating arithmetic. Non-digit
/// characters cause an immediate `0`. Used by metric-display paths where a
/// best-effort number is acceptable.
pub fn parse_u64(text: &str) -> u64 {
    let mut n = 0u64;
    for b in text.bytes() {
        if !b.is_ascii_digit() {
            return 0;
        }
        n = n.saturating_mul(10).saturating_add((b - b'0') as u64);
    }
    n
}

// ============================================================================
//  Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- URL whitelist ------------------------------------------------------

    #[test]
    fn allows_official_debian_https() {
        assert!(is_allowed_debian_url(
            "https://deb.debian.org/debian/pool/main/x.deb"
        ));
        assert!(is_allowed_debian_url(
            "https://cloud.debian.org/images/cloud/trixie/latest/x.raw"
        ));
    }

    #[test]
    fn allows_debian_cloud_http_fallback() {
        assert!(is_allowed_debian_url(
            "http://cloud.debian.org/images/cloud/trixie/latest/x.raw"
        ));
    }

    #[test]
    fn rejects_non_debian_hosts() {
        assert!(!is_allowed_debian_url("https://example.com/debian/x.deb"));
        assert!(!is_allowed_debian_url("https://attacker.tld/x.raw"));
    }

    #[test]
    fn rejects_path_traversal_in_url() {
        assert!(!is_allowed_debian_url(
            "https://deb.debian.org/debian/../etc/passwd"
        ));
    }

    #[test]
    fn rejects_nul_byte_in_url() {
        let evil = "https://deb.debian.org/debian/x\0.deb";
        assert!(!is_allowed_debian_url(evil));
    }

    #[test]
    fn rejects_http_not_in_cloud() {
        assert!(!is_allowed_debian_url("http://deb.debian.org/debian/x.deb"));
    }

    // -- Path safety --------------------------------------------------------

    #[test]
    fn safe_absolute_dir_accepts_normal_paths() {
        assert!(is_safe_absolute_dir("/System/var/asl"));
        assert!(is_safe_absolute_dir("/a"));
    }

    #[test]
    fn safe_absolute_dir_rejects_relative_or_traversal() {
        assert!(!is_safe_absolute_dir("System/var"));
        assert!(!is_safe_absolute_dir("/foo/../bar"));
        assert!(!is_safe_absolute_dir("/foo/.."));
        assert!(!is_safe_absolute_dir("/foo/"));
        assert!(!is_safe_absolute_dir(""));
        assert!(!is_safe_absolute_dir("/"));
    }

    #[test]
    fn safe_absolute_dir_rejects_nul() {
        assert!(!is_safe_absolute_dir("/foo\0bar"));
    }

    #[test]
    fn artifact_path_must_be_inside_images_dir() {
        let dir = "/System/var/asl/distros/debian/images";
        assert!(is_safe_artifact_path(
            "/System/var/asl/distros/debian/images/base.img",
            dir
        ));
        // sibling dir
        assert!(!is_safe_artifact_path(
            "/System/var/asl/distros/debian/state.img",
            dir
        ));
        // would match prefix without slash boundary
        assert!(!is_safe_artifact_path(
            "/System/var/asl/distros/debian/images-evil/base.img",
            dir
        ));
        // exactly the dir itself
        assert!(!is_safe_artifact_path(dir, dir));
    }

    #[test]
    fn artifact_path_rejects_traversal() {
        let dir = "/System/var/asl/distros/debian/images";
        assert!(!is_safe_artifact_path(
            "/System/var/asl/distros/debian/images/../../etc/passwd",
            dir
        ));
    }

    // -- Config parsing -----------------------------------------------------

    #[test]
    fn split_key_value_basic() {
        assert_eq!(
            split_key_value("asl_root = /System/var/asl"),
            Some(("asl_root", "/System/var/asl"))
        );
    }

    #[test]
    fn split_key_value_handles_no_spaces() {
        assert_eq!(
            split_key_value("debian_raw_url=https://deb.debian.org/debian/x"),
            Some(("debian_raw_url", "https://deb.debian.org/debian/x"))
        );
    }

    #[test]
    fn split_key_value_rejects_empty_sides() {
        assert!(split_key_value("=value").is_none());
        assert!(split_key_value("key=").is_none());
        assert!(split_key_value("nokey").is_none());
    }

    #[test]
    fn join_path_inserts_single_slash() {
        assert_eq!(join_path("/a", "b"), "/a/b");
        assert_eq!(join_path("/a/", "b"), "/a/b"); // already has slash
    }

    // -- Artifact size ------------------------------------------------------

    #[test]
    fn artifact_size_accepts_above_minimum() {
        assert!(artifact_size_ok(1_000_000_000, 500_000_000));
    }

    #[test]
    fn artifact_size_rejects_below_minimum() {
        assert!(!artifact_size_ok(100_000_000, 500_000_000));
    }

    #[test]
    fn artifact_size_rejects_overflow_sentinel() {
        // u32::MAX means stat could not represent the real size — must
        // not be silently accepted as "very large file is fine".
        assert!(!artifact_size_ok(STAT_SIZE_OVERFLOW_SENTINEL, 500_000_000));
    }

    #[test]
    fn artifact_size_handles_min_above_u32_range() {
        // min_size > 4 GiB — once we hit this regime we genuinely cannot
        // represent the size in 32 bits, so any stat_size value must
        // refuse, including u32::MAX (sentinel).
        assert!(!artifact_size_ok(u32::MAX, 5 * 1024 * 1024 * 1024));
        assert!(!artifact_size_ok(0, 5 * 1024 * 1024 * 1024));
    }

    // -- Raw-disk MBR header -----------------------------------------------

    #[test]
    fn raw_disk_header_accepts_valid_mbr() {
        let mut header = [0u8; 520];
        header[510] = 0x55;
        header[511] = 0xaa;
        assert!(raw_disk_header_ok(&header, 520));
    }

    #[test]
    fn raw_disk_header_rejects_short_read() {
        let mut header = [0u8; 520];
        header[510] = 0x55;
        header[511] = 0xaa;
        assert!(!raw_disk_header_ok(&header, 256));
    }

    #[test]
    fn raw_disk_header_rejects_wrong_magic() {
        let mut header = [0u8; 520];
        header[510] = 0xab;
        header[511] = 0xcd;
        assert!(!raw_disk_header_ok(&header, 520));
    }

    #[test]
    fn raw_disk_header_rejects_too_small_buffer() {
        let header = [0u8; 100];
        assert!(!raw_disk_header_ok(&header, 100));
    }

    // -- HTTP fallback policy ----------------------------------------------

    #[test]
    fn fallback_triggers_on_connection_failures() {
        // 0 = none, 2 = DNS, 3 = TCP, 4 = send, 5 = timeout, 7 = TLS
        assert!(should_try_http_fallback(0, 2));
        assert!(should_try_http_fallback(0, 3));
        assert!(should_try_http_fallback(0, 4));
        assert!(should_try_http_fallback(0, 5));
        assert!(should_try_http_fallback(0, 7));
    }

    #[test]
    fn fallback_skipped_when_server_answered_with_error() {
        // 4xx/5xx means the server answered. Don't downgrade.
        assert!(!should_try_http_fallback(404, 0));
        assert!(!should_try_http_fallback(503, 0));
    }

    #[test]
    fn fallback_skipped_when_server_redirects_too_much() {
        // error=6 is "too many redirects". The server is reachable but
        // misbehaving; switching to HTTP won't fix that. Status comes
        // through as 200 typically, so we test the explicit 200 path.
        assert!(!should_try_http_fallback(200, 6));
    }

    #[test]
    fn fallback_triggered_when_status_is_zero_regardless_of_error() {
        // Documents the deliberately-defensive behaviour of the existing
        // policy: status==0 means "no answer at all", and we always try
        // the fallback URL. Even if `error=1` (invalid URL) was reported,
        // the fallback URL might be valid. This is the established
        // invariant — pinning it so a future refactor must consciously
        // change it.
        assert!(should_try_http_fallback(0, 1));
        assert!(should_try_http_fallback(0, 6));
    }

    #[test]
    fn fallback_url_translates_https_to_http() {
        let https = "https://cloud.debian.org/images/cloud/trixie/latest/x.raw";
        assert_eq!(
            official_http_fallback(https).as_deref(),
            Some("http://cloud.debian.org/images/cloud/trixie/latest/x.raw")
        );
    }

    #[test]
    fn fallback_url_refuses_non_cloud_hosts() {
        assert!(official_http_fallback("https://deb.debian.org/debian/x").is_none());
        assert!(official_http_fallback("https://attacker.tld/x").is_none());
    }

    // -- parse_u64 ---------------------------------------------------------

    #[test]
    fn parse_u64_basic() {
        assert_eq!(parse_u64("0"), 0);
        assert_eq!(parse_u64("12345"), 12345);
    }

    #[test]
    fn parse_u64_returns_zero_on_non_digit() {
        assert_eq!(parse_u64(""), 0);
        assert_eq!(parse_u64("abc"), 0);
        assert_eq!(parse_u64("12a3"), 0);
    }

    #[test]
    fn parse_u64_saturates_on_overflow() {
        let huge = "99999999999999999999999999999999"; // > u64::MAX
        assert_eq!(parse_u64(huge), u64::MAX);
    }

    // -- SHA-512 / hex helpers (ADR-0011 stage 2) --------------------------

    #[test]
    fn hex_encode_basic() {
        assert_eq!(hex_encode(&[]), "");
        assert_eq!(hex_encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(hex_encode(&[0x00, 0x01, 0xff]), "0001ff");
    }

    #[test]
    fn parse_sha512_hex_roundtrip() {
        // 64 bytes = 128 hex chars
        let bytes: [u8; 64] = core::array::from_fn(|i| i as u8);
        let hex = hex_encode(&bytes);
        assert_eq!(hex.len(), 128);
        let back = parse_sha512_hex(&hex).expect("parse round-trip");
        assert_eq!(back, bytes);
    }

    #[test]
    fn parse_sha512_hex_rejects_wrong_length() {
        assert!(parse_sha512_hex("").is_none());
        assert!(parse_sha512_hex("abcdef").is_none());
        let too_long = "a".repeat(129);
        assert!(parse_sha512_hex(&too_long).is_none());
    }

    #[test]
    fn parse_sha512_hex_rejects_non_hex_chars() {
        let mut bad = String::from("g");
        bad.push_str(&"a".repeat(127));
        assert!(parse_sha512_hex(&bad).is_none());
    }

    #[test]
    fn parse_sha512_hex_accepts_uppercase() {
        let lower = "a".repeat(128);
        let upper = "A".repeat(128);
        let lo = parse_sha512_hex(&lower).expect("lower");
        let up = parse_sha512_hex(&upper).expect("upper");
        assert_eq!(lo, up);
    }

    #[test]
    fn const_time_eq_basic() {
        assert!(const_time_eq(b"", b""));
        assert!(const_time_eq(b"abc", b"abc"));
        assert!(!const_time_eq(b"abc", b"abd"));
        assert!(!const_time_eq(b"abc", b"ab"));
        assert!(!const_time_eq(b"ab", b"abc"));
    }

    #[test]
    fn sha512_empty_input() {
        // FIPS 180-4 test vector for SHA-512("") (NIST):
        // cf83e1357eefb8bd f1542850d66d8007 d620e4050b5715dc 83f4a921d36ce9ce
        // 47d0d13c5d85f2b0 ff8318d2877eec2f 63b931bd47417a81 a538327af927da3e
        let mut ctx = Sha512::new();
        ctx.update(b"");
        let digest = ctx.finalize();
        let expected = parse_sha512_hex(
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e",
        )
        .expect("expected hex parses");
        assert_eq!(digest, expected);
    }

    #[test]
    fn sha512_abc() {
        // FIPS 180-4 test vector for SHA-512("abc"):
        let mut ctx = Sha512::new();
        ctx.update(b"abc");
        let digest = ctx.finalize();
        let expected = parse_sha512_hex(
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f",
        )
        .expect("expected hex parses");
        assert_eq!(digest, expected);
    }

    #[test]
    fn sha512_streaming_matches_one_shot() {
        let data = b"the quick brown fox jumps over the lazy dog";
        let one_shot = {
            let mut ctx = Sha512::new();
            ctx.update(data);
            ctx.finalize()
        };
        let streamed = {
            let mut ctx = Sha512::new();
            for chunk in data.chunks(7) {
                ctx.update(chunk);
            }
            ctx.finalize()
        };
        assert_eq!(one_shot, streamed);
    }

    #[test]
    fn sha512_handles_block_boundary() {
        // SHA-512 block size is 128 bytes. Verify exactly-one-block input
        // and one-byte-over-boundary input both produce stable digests.
        let block = vec![0x42u8; 128];
        let block_plus_one = vec![0x42u8; 129];
        let mut ctx = Sha512::new();
        ctx.update(&block);
        let d1 = ctx.finalize();
        let mut ctx = Sha512::new();
        ctx.update(&block_plus_one);
        let d2 = ctx.finalize();
        assert_ne!(d1, d2);
    }
}
