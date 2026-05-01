//! libhttp_client — Safe Rust wrapper for the libhttp shared library.
//!
//! Loads `libhttp.so` via `dl_open`/`dl_sym` and provides ergonomic Rust functions
//! for HTTP/HTTPS operations.
//!
//! # Usage
//! ```rust
//! libhttp_client::init();
//! let data = libhttp_client::get("https://example.com/file.txt").unwrap();
//! libhttp_client::download("https://example.com/pkg.tar.gz", "/tmp/pkg.tar.gz");
//! ```

#![no_std]

extern crate alloc;
#[cfg(feature = "host")]
extern crate std;

#[cfg(feature = "host")]
use alloc::format;
#[cfg(feature = "host")]
use alloc::string::String;
#[cfg(not(feature = "host"))]
use alloc::vec;
use alloc::vec::Vec;

/// Progress callback type: `(received_bytes, total_bytes, userdata)`.
/// `total_bytes` is 0 if the server did not provide Content-Length.
pub type ProgressCallback = extern "C" fn(u32, u32, u64);

#[cfg(feature = "host")]
mod host {
    use super::*;
    use std::io::Write;
    use std::process::{Command, Stdio};

    pub fn init() -> bool {
        true
    }

    pub fn get(url: &str) -> Option<Vec<u8>> {
        curl_request(url, &[], None)
    }

    pub fn get_into(url: &str, buf: &mut [u8]) -> Option<usize> {
        let data = get(url)?;
        if data.len() > buf.len() {
            return None;
        }
        buf[..data.len()].copy_from_slice(&data);
        Some(data.len())
    }

    pub fn download(url: &str, path: &str) -> bool {
        Command::new("curl")
            .arg("-fsSL")
            .arg("--proto")
            .arg("=http,https")
            .arg("--max-time")
            .arg("300")
            .arg("--user-agent")
            .arg("git/anyos")
            .arg("-o")
            .arg(path)
            .arg(url)
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    pub fn download_progress(
        url: &str,
        path: &str,
        _callback: ProgressCallback,
        _userdata: u64,
    ) -> bool {
        download(url, path)
    }

    pub fn drain_progress(
        url: &str,
        callback: ProgressCallback,
        userdata: u64,
    ) -> Option<u32> {
        let data = get(url)?;
        callback(data.len() as u32, data.len() as u32, userdata);
        Some(data.len() as u32)
    }

    pub fn post(url: &str, body: &[u8], content_type: &str) -> Option<Vec<u8>> {
        let header = format!("Content-Type: {}", content_type);
        curl_request(url, &[header], Some(body))
    }

    pub fn post_with_headers(
        url: &str,
        body: &[u8],
        content_type: &str,
        extra_headers: &str,
    ) -> Option<Vec<u8>> {
        let mut headers = Vec::new();
        headers.push(format!("Content-Type: {}", content_type));
        for header in extra_headers.lines() {
            let trimmed = header.trim();
            if !trimmed.is_empty() {
                headers.push(String::from(trimmed));
            }
        }
        curl_request(url, &headers, Some(body))
    }

    pub fn last_status() -> u32 {
        0
    }

    pub fn last_error() -> u32 {
        0
    }

    fn curl_request(url: &str, headers: &[String], body: Option<&[u8]>) -> Option<Vec<u8>> {
        let mut cmd = Command::new("curl");
        cmd.arg("-fsSL")
            .arg("--proto")
            .arg("=http,https")
            .arg("--max-time")
            .arg("300")
            .arg("--user-agent")
            .arg("git/anyos");

        for header in headers {
            cmd.arg("-H").arg(header);
        }

        if body.is_some() {
            cmd.arg("--data-binary").arg("@-");
        }

        cmd.arg(url).stdout(Stdio::piped()).stderr(Stdio::piped());

        if body.is_some() {
            cmd.stdin(Stdio::piped());
        }

        let mut child = cmd.spawn().ok()?;
        if let Some(body) = body {
            let mut stdin = child.stdin.take()?;
            if stdin.write_all(body).is_err() {
                return None;
            }
        }

        let output = child.wait_with_output().ok()?;
        if output.status.success() {
            Some(output.stdout)
        } else {
            None
        }
    }
}

#[cfg(feature = "host")]
pub use host::*;

#[cfg(not(feature = "host"))]
mod imp {
    use super::*;

    dynlink::dll_exports! {
        lib_path: "/Libraries/libhttp.so",
        lib_struct: LibHttp,
        init_call: "libhttp_init",
        symbols: {
            libhttp_get(url: *const u8, url_len: u32, buf: *mut u8, buf_len: u32) -> u32,
            libhttp_download(url: *const u8, url_len: u32, path: *const u8, path_len: u32) -> u32,
            libhttp_drain_progress(url: *const u8, url_len: u32,
                cb: Option<ProgressCallback>, userdata: u64) -> u32,
            libhttp_download_progress(url: *const u8, url_len: u32, path: *const u8, path_len: u32,
                cb: Option<ProgressCallback>, userdata: u64) -> u32,
            libhttp_post(url: *const u8, url_len: u32, body: *const u8, body_len: u32,
                ct: *const u8, ct_len: u32, buf: *mut u8, buf_len: u32) -> u32,
            libhttp_post_with_headers(url: *const u8, url_len: u32, body: *const u8, body_len: u32,
                ct: *const u8, ct_len: u32, headers: *const u8, headers_len: u32,
                buf: *mut u8, buf_len: u32) -> u32,
            libhttp_last_status() -> u32,
            libhttp_last_error() -> u32,
        }
    }

    // ── Public API ──────────────────────────────────────────────────────────────

    /// Perform an HTTP(S) GET request and return the response body.
    ///
    /// Allocates a 4 MiB receive buffer. For larger responses, use `get_into()`
    /// with a custom buffer, or use `download()` to write directly to a file.
    ///
    /// Returns `Some(body)` on success, `None` on error.
    /// Use `last_error()` to get the error code on failure.
    pub fn get(url: &str) -> Option<Vec<u8>> {
        let buf_size: usize = 4 * 1024 * 1024; // 4 MiB
        let mut buf = vec![0u8; buf_size];
        let n = (lib().libhttp_get)(
            url.as_ptr(),
            url.len() as u32,
            buf.as_mut_ptr(),
            buf.len() as u32,
        );

        if n == u32::MAX {
            return None;
        }

        buf.truncate(n as usize);
        Some(buf)
    }

    /// Perform an HTTP(S) GET request with a caller-provided buffer.
    ///
    /// Returns the number of bytes written to `buf`, or `None` on error.
    /// More memory-efficient than `get()` when you know the max response size.
    pub fn get_into(url: &str, buf: &mut [u8]) -> Option<usize> {
        let n = (lib().libhttp_get)(
            url.as_ptr(),
            url.len() as u32,
            buf.as_mut_ptr(),
            buf.len() as u32,
        );
        if n == u32::MAX {
            None
        } else {
            Some(n as usize)
        }
    }

    /// Download a URL directly to a file path.
    ///
    /// Returns true on success, false on error.
    /// More memory-efficient than `get()` for large files.
    pub fn download(url: &str, path: &str) -> bool {
        let result = (lib().libhttp_download)(
            url.as_ptr(),
            url.len() as u32,
            path.as_ptr(),
            path.len() as u32,
        );
        result == 0
    }

    /// Download a URL to a file with progress reporting.
    ///
    /// The `callback` is called after each received chunk with
    /// `(received_bytes, total_bytes, userdata)`.
    /// Returns true on success, false on error.
    pub fn download_progress(
        url: &str,
        path: &str,
        callback: ProgressCallback,
        userdata: u64,
    ) -> bool {
        let result = (lib().libhttp_download_progress)(
            url.as_ptr(),
            url.len() as u32,
            path.as_ptr(),
            path.len() as u32,
            Some(callback),
            userdata,
        );
        result == 0
    }

    /// Perform an HTTP(S) GET and discard the response body with progress reporting.
    ///
    /// The `callback` is called after each received chunk with
    /// `(received_bytes, total_bytes, userdata)`.
    /// Returns the received byte count on success, or `None` on error.
    pub fn drain_progress(
        url: &str,
        callback: ProgressCallback,
        userdata: u64,
    ) -> Option<u32> {
        let result = (lib().libhttp_drain_progress)(
            url.as_ptr(),
            url.len() as u32,
            Some(callback),
            userdata,
        );
        if result == u32::MAX { None } else { Some(result) }
    }

    /// Perform an HTTP(S) POST request.
    ///
    /// Returns `Some(response_body)` on success, `None` on error.
    pub fn post(url: &str, body: &[u8], content_type: &str) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 256 * 1024];
        let n = (lib().libhttp_post)(
            url.as_ptr(),
            url.len() as u32,
            body.as_ptr(),
            body.len() as u32,
            content_type.as_ptr(),
            content_type.len() as u32,
            buf.as_mut_ptr(),
            buf.len() as u32,
        );
        if n == u32::MAX {
            return None;
        }
        buf.truncate(n as usize);
        Some(buf)
    }

    /// Perform an HTTP(S) POST request with custom headers.
    ///
    /// `extra_headers` must be pre-formatted with CRLF line endings, e.g.:
    /// `"Authorization: Bearer sk-...\r\nX-Custom: value\r\n"`
    ///
    /// Returns `Some(response_body)` on success, `None` on error.
    pub fn post_with_headers(
        url: &str,
        body: &[u8],
        content_type: &str,
        extra_headers: &str,
    ) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 256 * 1024];
        let n = (lib().libhttp_post_with_headers)(
            url.as_ptr(),
            url.len() as u32,
            body.as_ptr(),
            body.len() as u32,
            content_type.as_ptr(),
            content_type.len() as u32,
            extra_headers.as_ptr(),
            extra_headers.len() as u32,
            buf.as_mut_ptr(),
            buf.len() as u32,
        );
        if n == u32::MAX {
            return None;
        }
        buf.truncate(n as usize);
        Some(buf)
    }

    /// Returns the HTTP status code of the last request (e.g. 200, 404, 0 if no request).
    pub fn last_status() -> u32 {
        (lib().libhttp_last_status)()
    }

    /// Returns the error code of the last request.
    ///
    /// Error codes:
    /// - 0: No error
    /// - 1: Invalid URL
    /// - 2: DNS resolution failure
    /// - 3: TCP connection failure
    /// - 4: Send failure
    /// - 5: No response / timeout
    /// - 6: Too many redirects
    /// - 7: TLS handshake failed
    /// - 8: Output buffer too small
    /// - 9: File write error
    pub fn last_error() -> u32 {
        (lib().libhttp_last_error)()
    }
}

#[cfg(not(feature = "host"))]
pub use imp::*;
