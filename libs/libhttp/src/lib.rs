//! libhttp — HTTP/HTTPS client library for anyOS.
//!
//! Provides HTTP/1.1 GET, POST, and file download over HTTP and HTTPS (BearSSL TLS).
//! Built as a `.so` shared library loaded via `dl_open`/`dl_sym`.
//!
//! # Features
//! - HTTP and HTTPS (TLS via BearSSL with trust-all validator)
//! - Automatic redirect following (301, 302, 303, 307, 308)
//! - Chunked transfer-encoding support
//! - gzip/deflate content-encoding decompression
//! - Direct file download for memory efficiency
//!
//! # Export Convention
//! All public functions are `extern "C"` with `#[no_mangle]` for use via `dl_sym()`.

#![no_std]
#![no_main]

extern crate alloc;

pub mod syscall;
pub mod tls;
pub mod url;
pub mod http;
pub mod deflate;

// ── Allocator ───────────────────────────────────────────────────────────────

mod allocator {
    use core::alloc::{GlobalAlloc, Layout};
    use core::ptr;
    use libheap::{FreeBlock, block_size, free_list_alloc, free_list_dealloc};

    struct DllFreeListAlloc;

    static mut FREE_LIST: *mut FreeBlock = ptr::null_mut();

    unsafe impl GlobalAlloc for DllFreeListAlloc {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let size = block_size(layout);
            let ptr = unsafe { free_list_alloc(&mut FREE_LIST, size) };
            if !ptr.is_null() { return ptr; }
            let brk = crate::syscall::sbrk(0);
            if brk == u64::MAX { return ptr::null_mut(); }
            let align = layout.align().max(16) as u64;
            let aligned = (brk + align - 1) & !(align - 1);
            let needed = (aligned - brk + size as u64) as u32;
            let result = crate::syscall::sbrk(needed);
            if result == u64::MAX { return ptr::null_mut(); }
            aligned as *mut u8
        }

        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            unsafe { free_list_dealloc(&mut FREE_LIST, ptr, block_size(layout)); }
        }
    }

    #[global_allocator]
    static ALLOCATOR: DllFreeListAlloc = DllFreeListAlloc;
}

// ── Panic handler ───────────────────────────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::exit(1);
}

// ── C ABI Exports ───────────────────────────────────────────────────────────

/// Initialize the library. Must be called once before other functions.
/// Returns 1 on success, 0 on failure.
#[no_mangle]
pub extern "C" fn libhttp_init() -> u32 {
    // No runtime initialization needed currently.
    // TLS state is lazily initialized on first HTTPS request.
    1
}

/// Perform HTTP(S) GET request. Response body written to `out_buf`.
///
/// Returns: bytes written to `out_buf` on success,
///          `u32::MAX` on error (check `libhttp_last_error()` for code).
///
/// Follows redirects automatically (up to 10 hops).
#[no_mangle]
pub extern "C" fn libhttp_get(
    url_ptr: *const u8, url_len: u32,
    out_buf: *mut u8, out_buf_len: u32,
) -> u32 {
    let url_str = unsafe {
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(url_ptr, url_len as usize))
    };

    match http::get(url_str) {
        Some(body) => {
            let copy_len = body.len().min(out_buf_len as usize);
            if copy_len > 0 {
                unsafe {
                    core::ptr::copy_nonoverlapping(body.as_ptr(), out_buf, copy_len);
                }
            }
            if body.len() > out_buf_len as usize {
                // Buffer too small, but we copied what we could
                http::set_error(http::ERR_BUFFER_TOO_SMALL);
            }
            copy_len as u32
        }
        None => u32::MAX,
    }
}

/// Download a URL directly to a file path.
///
/// Returns: 0 on success, `u32::MAX` on error.
#[no_mangle]
pub extern "C" fn libhttp_download(
    url_ptr: *const u8, url_len: u32,
    path_ptr: *const u8, path_len: u32,
) -> u32 {
    let url_str = unsafe {
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(url_ptr, url_len as usize))
    };
    let path = unsafe {
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, path_len as usize))
    };

    if http::download(url_str, path) { 0 } else { u32::MAX }
}

/// Perform HTTP(S) POST request with body.
///
/// Returns: bytes written to `out_buf`, or `u32::MAX` on error.
#[no_mangle]
pub extern "C" fn libhttp_post(
    url_ptr: *const u8, url_len: u32,
    body_ptr: *const u8, body_len: u32,
    content_type_ptr: *const u8, content_type_len: u32,
    out_buf: *mut u8, out_buf_len: u32,
) -> u32 {
    let url_str = unsafe {
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(url_ptr, url_len as usize))
    };
    let body = unsafe {
        core::slice::from_raw_parts(body_ptr, body_len as usize)
    };
    let content_type = unsafe {
        core::str::from_utf8_unchecked(
            core::slice::from_raw_parts(content_type_ptr, content_type_len as usize)
        )
    };

    match http::post(url_str, body, content_type) {
        Some(resp_body) => {
            let copy_len = resp_body.len().min(out_buf_len as usize);
            if copy_len > 0 {
                unsafe {
                    core::ptr::copy_nonoverlapping(resp_body.as_ptr(), out_buf, copy_len);
                }
            }
            copy_len as u32
        }
        None => u32::MAX,
    }
}

/// Returns the HTTP status code of the last request (e.g. 200, 404).
#[no_mangle]
pub extern "C" fn libhttp_last_status() -> u32 {
    http::last_status()
}

/// Returns the last error code (0 = no error).
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
#[no_mangle]
pub extern "C" fn libhttp_last_error() -> u32 {
    http::last_error()
}
