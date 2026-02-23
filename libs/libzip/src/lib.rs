//! libzip — ZIP archive library for anyOS.
//!
//! Provides reading and writing of ZIP archives with DEFLATE support.
//! Built as a `.so` shared library loaded via `dl_open`/`dl_sym`.
//!
//! # Architecture
//! - Supports Stored (no compression) and DEFLATE methods
//! - Full inflate (decompression) with fixed and dynamic Huffman
//! - DEFLATE compression with LZ77 and fixed Huffman encoding
//! - CRC-32 verification on extraction
//!
//! # Export Convention
//! All public functions are `extern "C"` with `#[no_mangle]` for use via `dl_sym()`.

#![no_std]
#![no_main]

extern crate alloc;

pub mod syscall;
pub mod crc32;
pub mod inflate;
pub mod deflate;
pub mod zip;

use alloc::vec::Vec;
use zip::{ZipReader, ZipWriter};

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

// ── Handle table ────────────────────────────────────────────────────────────

const MAX_HANDLES: usize = 8;

enum ZipHandle {
    Empty,
    Reader(ZipReader),
    Writer(ZipWriter),
}

static mut HANDLES: [Option<ZipHandle>; MAX_HANDLES] = [
    None, None, None, None, None, None, None, None,
];

fn alloc_handle(h: ZipHandle) -> u32 {
    unsafe {
        for i in 0..MAX_HANDLES {
            if HANDLES[i].is_none() {
                HANDLES[i] = Some(h);
                return (i + 1) as u32;
            }
        }
    }
    0
}

fn get_reader(handle: u32) -> Option<&'static ZipReader> {
    let idx = handle as usize;
    if idx == 0 || idx > MAX_HANDLES { return None; }
    unsafe {
        match &HANDLES[idx - 1] {
            Some(ZipHandle::Reader(r)) => Some(r),
            _ => None,
        }
    }
}

fn get_writer(handle: u32) -> Option<&'static mut ZipWriter> {
    let idx = handle as usize;
    if idx == 0 || idx > MAX_HANDLES { return None; }
    unsafe {
        match &mut HANDLES[idx - 1] {
            Some(ZipHandle::Writer(w)) => Some(w),
            _ => None,
        }
    }
}

fn free_handle(handle: u32) {
    let idx = handle as usize;
    if idx > 0 && idx <= MAX_HANDLES {
        unsafe { HANDLES[idx - 1] = None; }
    }
}

// ── C ABI Exports ───────────────────────────────────────────────────────────

/// Open a ZIP archive for reading.
/// Returns handle (>0) on success, 0 on error.
#[no_mangle]
pub extern "C" fn libzip_open(path_ptr: *const u8, path_len: u32) -> u32 {
    let path = unsafe {
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, path_len as usize))
    };

    let fd = syscall::open(path, 0);
    if fd == u32::MAX { return 0; }

    let size = syscall::file_size(fd) as usize;
    let mut data = alloc::vec![0u8; size];
    let mut read = 0usize;
    while read < size {
        let chunk = &mut data[read..];
        let n = syscall::read(fd, chunk);
        if n == 0 || n == u32::MAX { break; }
        read += n as usize;
    }
    syscall::close(fd);

    if read < size {
        data.truncate(read);
    }

    match ZipReader::parse(data) {
        Some(reader) => alloc_handle(ZipHandle::Reader(reader)),
        None => 0,
    }
}

/// Create a new ZIP archive for writing.
/// Returns handle (>0) on success, 0 on error.
#[no_mangle]
pub extern "C" fn libzip_create() -> u32 {
    alloc_handle(ZipHandle::Writer(ZipWriter::new()))
}

/// Close a ZIP handle (reader or writer).
#[no_mangle]
pub extern "C" fn libzip_close(handle: u32) {
    free_handle(handle);
}

/// Get the number of entries in a ZIP archive (reader only).
#[no_mangle]
pub extern "C" fn libzip_entry_count(handle: u32) -> u32 {
    match get_reader(handle) {
        Some(r) => r.entry_count() as u32,
        None => 0,
    }
}

/// Get the name of an entry. Writes to `buf`, returns bytes written.
#[no_mangle]
pub extern "C" fn libzip_entry_name(handle: u32, index: u32, buf: *mut u8, buf_len: u32) -> u32 {
    let reader = match get_reader(handle) {
        Some(r) => r,
        None => return 0,
    };
    let entry = match reader.entries.get(index as usize) {
        Some(e) => e,
        None => return 0,
    };
    let name = entry.name.as_bytes();
    let copy_len = name.len().min(buf_len as usize);
    unsafe {
        core::ptr::copy_nonoverlapping(name.as_ptr(), buf, copy_len);
    }
    copy_len as u32
}

/// Get uncompressed size of an entry.
#[no_mangle]
pub extern "C" fn libzip_entry_size(handle: u32, index: u32) -> u32 {
    match get_reader(handle) {
        Some(r) => r.entries.get(index as usize).map(|e| e.uncompressed_size).unwrap_or(0),
        None => 0,
    }
}

/// Get compressed size of an entry.
#[no_mangle]
pub extern "C" fn libzip_entry_compressed_size(handle: u32, index: u32) -> u32 {
    match get_reader(handle) {
        Some(r) => r.entries.get(index as usize).map(|e| e.compressed_size).unwrap_or(0),
        None => 0,
    }
}

/// Get compression method of an entry (0=stored, 8=deflate).
#[no_mangle]
pub extern "C" fn libzip_entry_method(handle: u32, index: u32) -> u32 {
    match get_reader(handle) {
        Some(r) => r.entries.get(index as usize).map(|e| e.method as u32).unwrap_or(u32::MAX),
        None => u32::MAX,
    }
}

/// Check if entry is a directory (name ends with '/').
#[no_mangle]
pub extern "C" fn libzip_entry_is_dir(handle: u32, index: u32) -> u32 {
    match get_reader(handle) {
        Some(r) => {
            match r.entries.get(index as usize) {
                Some(e) => if e.name.ends_with('/') { 1 } else { 0 },
                None => 0,
            }
        }
        None => 0,
    }
}

/// Extract an entry to a buffer. Returns bytes written, or u32::MAX on error.
#[no_mangle]
pub extern "C" fn libzip_extract(handle: u32, index: u32, buf: *mut u8, buf_len: u32) -> u32 {
    let reader = match get_reader(handle) {
        Some(r) => r,
        None => return u32::MAX,
    };

    let data = match reader.extract(index as usize) {
        Some(d) => d,
        None => return u32::MAX,
    };

    let copy_len = data.len().min(buf_len as usize);
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), buf, copy_len);
    }
    copy_len as u32
}

/// Extract an entry directly to a file. Returns 0 on success, u32::MAX on error.
#[no_mangle]
pub extern "C" fn libzip_extract_to_file(
    handle: u32, index: u32, path_ptr: *const u8, path_len: u32,
) -> u32 {
    let reader = match get_reader(handle) {
        Some(r) => r,
        None => return u32::MAX,
    };

    let data = match reader.extract(index as usize) {
        Some(d) => d,
        None => return u32::MAX,
    };

    let path = unsafe {
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, path_len as usize))
    };

    let fd = syscall::open(path, syscall::O_WRITE | syscall::O_CREATE | syscall::O_TRUNC);
    if fd == u32::MAX { return u32::MAX; }

    let mut written = 0usize;
    while written < data.len() {
        let n = syscall::write(fd, &data[written..]);
        if n == u32::MAX { break; }
        written += n as usize;
    }
    syscall::close(fd);

    if written == data.len() { 0 } else { u32::MAX }
}

/// Add a file to a ZIP writer. `compress`: 0=stored, 1=deflate.
/// Returns 0 on success, u32::MAX on error.
#[no_mangle]
pub extern "C" fn libzip_add_file(
    handle: u32,
    name_ptr: *const u8, name_len: u32,
    data_ptr: *const u8, data_len: u32,
    compress: u32,
) -> u32 {
    let writer = match get_writer(handle) {
        Some(w) => w,
        None => return u32::MAX,
    };

    let name = unsafe {
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let data = unsafe {
        core::slice::from_raw_parts(data_ptr, data_len as usize)
    };

    writer.add(name, data, compress != 0);
    0
}

/// Add a directory entry to a ZIP writer.
/// Returns 0 on success, u32::MAX on error.
#[no_mangle]
pub extern "C" fn libzip_add_dir(
    handle: u32,
    name_ptr: *const u8, name_len: u32,
) -> u32 {
    let writer = match get_writer(handle) {
        Some(w) => w,
        None => return u32::MAX,
    };

    let name = unsafe {
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(name_ptr, name_len as usize))
    };

    writer.add_directory(name);
    0
}

/// Finalize the ZIP writer and write to a file.
/// The handle is consumed (freed) by this call.
/// Returns 0 on success, u32::MAX on error.
#[no_mangle]
pub extern "C" fn libzip_write_to_file(handle: u32, path_ptr: *const u8, path_len: u32) -> u32 {
    let idx = handle as usize;
    if idx == 0 || idx > MAX_HANDLES { return u32::MAX; }

    // Take ownership of the writer
    let writer = unsafe {
        match HANDLES[idx - 1].take() {
            Some(ZipHandle::Writer(w)) => w,
            other => {
                HANDLES[idx - 1] = other;
                return u32::MAX;
            }
        }
    };

    let data = writer.finish();

    let path = unsafe {
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(path_ptr, path_len as usize))
    };

    let fd = syscall::open(path, syscall::O_WRITE | syscall::O_CREATE | syscall::O_TRUNC);
    if fd == u32::MAX { return u32::MAX; }

    let mut written = 0usize;
    while written < data.len() {
        let n = syscall::write(fd, &data[written..]);
        if n == u32::MAX { break; }
        written += n as usize;
    }
    syscall::close(fd);

    if written == data.len() { 0 } else { u32::MAX }
}
