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
    use std::fs;
    use std::io::Write;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Mutex, OnceLock};

    static LAST_STATUS: AtomicU32 = AtomicU32::new(0);
    static LAST_HEADERS: OnceLock<Mutex<String>> = OnceLock::new();

    fn headers_store() -> &'static Mutex<String> {
        LAST_HEADERS.get_or_init(|| Mutex::new(String::new()))
    }

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

    pub fn download_progress_resume(
        url: &str,
        path: &str,
        callback: ProgressCallback,
        userdata: u64,
        _resume_from: u32,
    ) -> bool {
        download_progress(url, path, callback, userdata)
    }

    pub fn drain_progress(url: &str, callback: ProgressCallback, userdata: u64) -> Option<u32> {
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

    pub fn request_with_headers(
        url: &str,
        method: &str,
        body: &[u8],
        content_type: &str,
        extra_headers: &str,
    ) -> Option<Vec<u8>> {
        let mut headers = Vec::new();
        if !content_type.is_empty() && !has_header(extra_headers, "content-type") {
            headers.push(format!("Content-Type: {}", content_type));
        }
        for header in extra_headers.lines() {
            let trimmed = header.trim();
            if !trimmed.is_empty() {
                headers.push(String::from(trimmed));
            }
        }
        let body_arg = if body.is_empty()
            && (method.eq_ignore_ascii_case("GET") || method.eq_ignore_ascii_case("HEAD"))
        {
            None
        } else {
            Some(body)
        };
        curl_request_with_method(url, method, &headers, body_arg)
    }

    pub fn last_status() -> u32 {
        LAST_STATUS.load(Ordering::Relaxed)
    }

    pub fn last_headers() -> String {
        headers_store()
            .lock()
            .map(|headers| headers.clone())
            .unwrap_or_default()
    }

    pub fn last_error() -> u32 {
        0
    }

    fn curl_request(url: &str, headers: &[String], body: Option<&[u8]>) -> Option<Vec<u8>> {
        let method = if body.is_some() { "POST" } else { "GET" };
        curl_request_with_method(url, method, headers, body)
    }

    fn curl_request_with_method(
        url: &str,
        method: &str,
        headers: &[String],
        body: Option<&[u8]>,
    ) -> Option<Vec<u8>> {
        let unique = format!(
            "anyos-http-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_nanos()
        );
        let header_path = std::env::temp_dir().join(format!("{}.headers", unique));
        let body_path = std::env::temp_dir().join(format!("{}.body", unique));

        let mut cmd = Command::new("curl");
        cmd.arg("-sSL")
            .arg("--proto")
            .arg("=http,https")
            .arg("--max-time")
            .arg("300")
            .arg("--user-agent")
            .arg("node/anyos")
            .arg("-X")
            .arg(method)
            .arg("-D")
            .arg(&header_path)
            .arg("-w")
            .arg("%{http_code}")
            .arg("-o")
            .arg(&body_path);

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
        let headers = fs::read_to_string(&header_path).unwrap_or_default();
        let body = fs::read(&body_path).unwrap_or_default();
        let _ = fs::remove_file(&header_path);
        let _ = fs::remove_file(&body_path);
        if let Ok(mut stored) = headers_store().lock() {
            *stored = headers;
        }
        LAST_STATUS.store(
            parse_curl_status(&output.stdout).unwrap_or(0),
            Ordering::Relaxed,
        );
        if output.status.success() {
            Some(body)
        } else {
            None
        }
    }

    fn parse_curl_status(stderr: &[u8]) -> Option<u32> {
        let text = core::str::from_utf8(stderr).ok()?;
        text.trim()
            .rsplit(|ch: char| !ch.is_ascii_digit())
            .find(|part| !part.is_empty())
            .and_then(|part| part.parse::<u32>().ok())
    }

    fn has_header(headers: &str, name: &str) -> bool {
        headers.lines().any(|line| {
            line.find(':')
                .map(|idx| line[..idx].trim().eq_ignore_ascii_case(name))
                .unwrap_or(false)
        })
    }
}

#[cfg(feature = "host")]
pub use host::*;

#[cfg(not(feature = "host"))]
mod imp {
    use alloc::string::String;

    use super::*;

    const LIBHTTP_PATH: &str = "/Libraries/libhttp.so";

    type GetFn = extern "C" fn(*const u8, u32, *mut u8, u32) -> u32;
    type DownloadFn = extern "C" fn(*const u8, u32, *const u8, u32) -> u32;
    type DrainProgressFn = extern "C" fn(*const u8, u32, Option<ProgressCallback>, u64) -> u32;
    type DownloadProgressFn =
        extern "C" fn(*const u8, u32, *const u8, u32, Option<ProgressCallback>, u64) -> u32;
    type DownloadProgressResumeFn =
        extern "C" fn(*const u8, u32, *const u8, u32, Option<ProgressCallback>, u64, u32) -> u32;
    type PostFn =
        extern "C" fn(*const u8, u32, *const u8, u32, *const u8, u32, *mut u8, u32) -> u32;
    type PostHeadersFn = extern "C" fn(
        *const u8,
        u32,
        *const u8,
        u32,
        *const u8,
        u32,
        *const u8,
        u32,
        *mut u8,
        u32,
    ) -> u32;
    type RequestHeadersFn = extern "C" fn(
        *const u8,
        u32,
        *const u8,
        u32,
        *const u8,
        u32,
        *const u8,
        u32,
        *const u8,
        u32,
        *mut u8,
        u32,
    ) -> u32;
    type LastStatusFn = extern "C" fn() -> u32;
    type LastHeadersFn = extern "C" fn(*mut u8, u32) -> u32;
    type LastErrorFn = extern "C" fn() -> u32;

    struct LibHttp {
        _handle: dynlink::DlHandle,
        libhttp_get: GetFn,
        libhttp_download: DownloadFn,
        libhttp_drain_progress: Option<DrainProgressFn>,
        libhttp_download_progress: Option<DownloadProgressFn>,
        libhttp_download_progress_resume: Option<DownloadProgressResumeFn>,
        libhttp_post: PostFn,
        libhttp_post_with_headers: Option<PostHeadersFn>,
        libhttp_request_with_headers: Option<RequestHeadersFn>,
        libhttp_last_status: LastStatusFn,
        libhttp_last_headers: Option<LastHeadersFn>,
        libhttp_last_error: LastErrorFn,
    }

    static mut LIB: Option<LibHttp> = None;

    #[allow(static_mut_refs)]
    fn lib() -> &'static LibHttp {
        unsafe { LIB.as_ref().expect("LibHttp not loaded") }
    }

    fn load_get(handle: &dynlink::DlHandle) -> Option<GetFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_get")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), GetFn>(&ptr) })
    }

    fn load_download(handle: &dynlink::DlHandle) -> Option<DownloadFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_download")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), DownloadFn>(&ptr) })
    }

    fn load_drain_progress(handle: &dynlink::DlHandle) -> Option<DrainProgressFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_drain_progress")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), DrainProgressFn>(&ptr) })
    }

    fn load_download_progress(handle: &dynlink::DlHandle) -> Option<DownloadProgressFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_download_progress")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), DownloadProgressFn>(&ptr) })
    }

    fn load_download_progress_resume(
        handle: &dynlink::DlHandle,
    ) -> Option<DownloadProgressResumeFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_download_progress_resume")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), DownloadProgressResumeFn>(&ptr) })
    }

    fn load_post(handle: &dynlink::DlHandle) -> Option<PostFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_post")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), PostFn>(&ptr) })
    }

    fn load_post_headers(handle: &dynlink::DlHandle) -> Option<PostHeadersFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_post_with_headers")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), PostHeadersFn>(&ptr) })
    }

    fn load_request_headers(handle: &dynlink::DlHandle) -> Option<RequestHeadersFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_request_with_headers")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), RequestHeadersFn>(&ptr) })
    }

    fn load_last_status(handle: &dynlink::DlHandle) -> Option<LastStatusFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_last_status")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), LastStatusFn>(&ptr) })
    }

    fn load_last_headers(handle: &dynlink::DlHandle) -> Option<LastHeadersFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_last_headers")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), LastHeadersFn>(&ptr) })
    }

    fn load_last_error(handle: &dynlink::DlHandle) -> Option<LastErrorFn> {
        let ptr = dynlink::dl_sym(handle, "libhttp_last_error")?;
        Some(unsafe { core::mem::transmute_copy::<*const (), LastErrorFn>(&ptr) })
    }

    pub fn init() -> bool {
        let handle = match dynlink::dl_open(LIBHTTP_PATH) {
            Some(handle) => handle,
            None => {
                dynlink::log_open_failed(LIBHTTP_PATH);
                return false;
            }
        };

        let Some(libhttp_get) = load_get(&handle) else {
            dynlink::log_missing_symbol(LIBHTTP_PATH, "libhttp_get");
            return false;
        };
        let Some(libhttp_download) = load_download(&handle) else {
            dynlink::log_missing_symbol(LIBHTTP_PATH, "libhttp_download");
            return false;
        };
        let Some(libhttp_post) = load_post(&handle) else {
            dynlink::log_missing_symbol(LIBHTTP_PATH, "libhttp_post");
            return false;
        };
        let Some(libhttp_last_status) = load_last_status(&handle) else {
            dynlink::log_missing_symbol(LIBHTTP_PATH, "libhttp_last_status");
            return false;
        };
        let Some(libhttp_last_error) = load_last_error(&handle) else {
            dynlink::log_missing_symbol(LIBHTTP_PATH, "libhttp_last_error");
            return false;
        };

        let libhttp_drain_progress = load_drain_progress(&handle);
        let libhttp_download_progress = load_download_progress(&handle);
        let libhttp_download_progress_resume = load_download_progress_resume(&handle);
        let libhttp_post_with_headers = load_post_headers(&handle);
        let libhttp_request_with_headers = load_request_headers(&handle);
        let libhttp_last_headers = load_last_headers(&handle);

        let lib = LibHttp {
            _handle: handle,
            libhttp_get,
            libhttp_download,
            libhttp_drain_progress,
            libhttp_download_progress,
            libhttp_download_progress_resume,
            libhttp_post,
            libhttp_post_with_headers,
            libhttp_request_with_headers,
            libhttp_last_status,
            libhttp_last_headers,
            libhttp_last_error,
        };

        if let Some(init_ptr) = dynlink::dl_sym(&lib._handle, "libhttp_init") {
            let init_fn: extern "C" fn() -> u32 = unsafe {
                core::mem::transmute_copy::<*const (), extern "C" fn() -> u32>(&init_ptr)
            };
            let _ = init_fn();
        }

        unsafe {
            LIB = Some(lib);
        }
        true
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
        let result = if let Some(download_progress) = lib().libhttp_download_progress {
            download_progress(
                url.as_ptr(),
                url.len() as u32,
                path.as_ptr(),
                path.len() as u32,
                Some(callback),
                userdata,
            )
        } else {
            callback(0, 0, userdata);
            let result = (lib().libhttp_download)(
                url.as_ptr(),
                url.len() as u32,
                path.as_ptr(),
                path.len() as u32,
            );
            if result == 0 {
                callback(1, 1, userdata);
            }
            result
        };
        result == 0
    }

    /// Download a URL with progress reporting, resuming at `resume_from` bytes if supported.
    ///
    /// If the loaded libhttp lacks resume support, this falls back to a full
    /// download so older images keep working.
    pub fn download_progress_resume(
        url: &str,
        path: &str,
        callback: ProgressCallback,
        userdata: u64,
        resume_from: u32,
    ) -> bool {
        let result = if let Some(download_progress_resume) = lib().libhttp_download_progress_resume
        {
            download_progress_resume(
                url.as_ptr(),
                url.len() as u32,
                path.as_ptr(),
                path.len() as u32,
                Some(callback),
                userdata,
                resume_from,
            )
        } else {
            return download_progress(url, path, callback, userdata);
        };
        result == 0
    }

    /// Perform an HTTP(S) GET and discard the response body with progress reporting.
    ///
    /// The `callback` is called after each received chunk with
    /// `(received_bytes, total_bytes, userdata)`.
    /// Returns the received byte count on success, or `None` on error.
    pub fn drain_progress(url: &str, callback: ProgressCallback, userdata: u64) -> Option<u32> {
        let result = if let Some(drain_progress) = lib().libhttp_drain_progress {
            drain_progress(url.as_ptr(), url.len() as u32, Some(callback), userdata)
        } else {
            let body = get(url)?;
            let received = body.len() as u32;
            callback(received, received, userdata);
            received
        };
        if result == u32::MAX {
            None
        } else {
            Some(result)
        }
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
        let Some(post_with_headers) = lib().libhttp_post_with_headers else {
            return post(url, body, content_type);
        };
        let mut buf = vec![0u8; 256 * 1024];
        let n = post_with_headers(
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

    pub fn request_with_headers(
        url: &str,
        method: &str,
        body: &[u8],
        content_type: &str,
        extra_headers: &str,
    ) -> Option<Vec<u8>> {
        let Some(request_with_headers) = lib().libhttp_request_with_headers else {
            if method.eq_ignore_ascii_case("GET") && body.is_empty() {
                return get(url);
            }
            if method.eq_ignore_ascii_case("POST") {
                return post(url, body, content_type);
            }
            return None;
        };
        let mut buf = vec![0u8; 4 * 1024 * 1024];
        let n = request_with_headers(
            url.as_ptr(),
            url.len() as u32,
            method.as_ptr(),
            method.len() as u32,
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

    pub fn last_headers() -> String {
        let Some(last_headers) = lib().libhttp_last_headers else {
            return String::new();
        };
        let mut buf = vec![0u8; 64 * 1024];
        let n = last_headers(buf.as_mut_ptr(), buf.len() as u32);
        let used = (n as usize).min(buf.len());
        buf.truncate(used);
        String::from_utf8(buf).unwrap_or_default()
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
