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

use alloc::vec;
use alloc::vec::Vec;
use dynlink::{dl_open, dl_sym, DlHandle};

// ── Function pointer cache ──────────────────────────────────────────────────

/// Progress callback type: `(received_bytes, total_bytes, userdata)`.
/// `total_bytes` is 0 if the server did not provide Content-Length.
pub type ProgressCallback = extern "C" fn(u32, u32, u64);

struct LibHttp {
    _handle: DlHandle,
    init_fn: extern "C" fn() -> u32,
    get: extern "C" fn(*const u8, u32, *mut u8, u32) -> u32,
    download: extern "C" fn(*const u8, u32, *const u8, u32) -> u32,
    download_progress: extern "C" fn(*const u8, u32, *const u8, u32,
        Option<ProgressCallback>, u64) -> u32,
    post: extern "C" fn(*const u8, u32, *const u8, u32, *const u8, u32, *mut u8, u32) -> u32,
    last_status: extern "C" fn() -> u32,
    last_error: extern "C" fn() -> u32,
}

static mut LIB: Option<LibHttp> = None;

fn lib() -> &'static LibHttp {
    unsafe { LIB.as_ref().expect("libhttp not loaded — call init() first") }
}

unsafe fn resolve<T: Copy>(handle: &DlHandle, name: &str) -> T {
    let ptr = dl_sym(handle, name)
        .unwrap_or_else(|| panic!("libhttp: symbol not found: {}", name));
    unsafe { core::mem::transmute_copy::<*const (), T>(&ptr) }
}

// ── Initialization ──────────────────────────────────────────────────────────

/// Load libhttp.so and initialize the library. Returns true on success.
pub fn init() -> bool {
    let handle = match dl_open("/Libraries/libhttp.so") {
        Some(h) => h,
        None => return false,
    };

    unsafe {
        let lib = LibHttp {
            init_fn: resolve(&handle, "libhttp_init"),
            get: resolve(&handle, "libhttp_get"),
            download: resolve(&handle, "libhttp_download"),
            download_progress: resolve(&handle, "libhttp_download_progress"),
            post: resolve(&handle, "libhttp_post"),
            last_status: resolve(&handle, "libhttp_last_status"),
            last_error: resolve(&handle, "libhttp_last_error"),
            _handle: handle,
        };
        LIB = Some(lib);
    }

    // Call libhttp_init() to initialize the library
    let result = (lib().init_fn)();
    result == 1
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
    let n = (lib().get)(
        url.as_ptr(), url.len() as u32,
        buf.as_mut_ptr(), buf.len() as u32,
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
    let n = (lib().get)(
        url.as_ptr(), url.len() as u32,
        buf.as_mut_ptr(), buf.len() as u32,
    );
    if n == u32::MAX { None } else { Some(n as usize) }
}

/// Download a URL directly to a file path.
///
/// Returns true on success, false on error.
/// More memory-efficient than `get()` for large files.
pub fn download(url: &str, path: &str) -> bool {
    let result = (lib().download)(
        url.as_ptr(), url.len() as u32,
        path.as_ptr(), path.len() as u32,
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
    let result = (lib().download_progress)(
        url.as_ptr(), url.len() as u32,
        path.as_ptr(), path.len() as u32,
        Some(callback), userdata,
    );
    result == 0
}

/// Perform an HTTP(S) POST request.
///
/// Returns `Some(response_body)` on success, `None` on error.
pub fn post(url: &str, body: &[u8], content_type: &str) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 256 * 1024];
    let n = (lib().post)(
        url.as_ptr(), url.len() as u32,
        body.as_ptr(), body.len() as u32,
        content_type.as_ptr(), content_type.len() as u32,
        buf.as_mut_ptr(), buf.len() as u32,
    );
    if n == u32::MAX { return None; }
    buf.truncate(n as usize);
    Some(buf)
}

/// Returns the HTTP status code of the last request (e.g. 200, 404, 0 if no request).
pub fn last_status() -> u32 {
    (lib().last_status)()
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
    (lib().last_error)()
}
