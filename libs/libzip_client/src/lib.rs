//! libzip_client — Safe Rust wrapper for the libzip shared library.
//!
//! Loads `libzip.so` via `dl_open`/`dl_sym` and provides ergonomic Rust types
//! (`ZipReader`, `ZipWriter`) for archive operations.
//!
//! # Usage
//! ```rust
//! libzip_client::init();
//! let reader = libzip_client::ZipReader::open("/path/to/file.zip").unwrap();
//! for i in 0..reader.entry_count() {
//!     let name = reader.entry_name(i);
//!     reader.extract_to_file(i, &name);
//! }
//! ```

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec;
use dynlink::{dl_open, dl_sym, DlHandle};

// ── Function pointer cache ──────────────────────────────────────────────────

struct LibZip {
    _handle: DlHandle,
    open: extern "C" fn(*const u8, u32) -> u32,
    create: extern "C" fn() -> u32,
    close: extern "C" fn(u32),
    entry_count: extern "C" fn(u32) -> u32,
    entry_name: extern "C" fn(u32, u32, *mut u8, u32) -> u32,
    entry_size: extern "C" fn(u32, u32) -> u32,
    entry_compressed_size: extern "C" fn(u32, u32) -> u32,
    entry_method: extern "C" fn(u32, u32) -> u32,
    entry_is_dir: extern "C" fn(u32, u32) -> u32,
    extract: extern "C" fn(u32, u32, *mut u8, u32) -> u32,
    extract_to_file: extern "C" fn(u32, u32, *const u8, u32) -> u32,
    add_file: extern "C" fn(u32, *const u8, u32, *const u8, u32, u32) -> u32,
    add_dir: extern "C" fn(u32, *const u8, u32) -> u32,
    write_to_file: extern "C" fn(u32, *const u8, u32) -> u32,
}

static mut LIB: Option<LibZip> = None;

fn lib() -> &'static LibZip {
    unsafe { LIB.as_ref().expect("libzip not loaded — call init() first") }
}

unsafe fn resolve<T: Copy>(handle: &DlHandle, name: &str) -> T {
    let ptr = dl_sym(handle, name)
        .unwrap_or_else(|| panic!("libzip: symbol not found: {}", name));
    unsafe { core::mem::transmute_copy::<*const (), T>(&ptr) }
}

// ── Initialization ──────────────────────────────────────────────────────────

/// Load libzip.so and cache all function pointers. Returns true on success.
pub fn init() -> bool {
    let handle = match dl_open("/Libraries/libzip.so") {
        Some(h) => h,
        None => return false,
    };

    unsafe {
        let lib = LibZip {
            open: resolve(&handle, "libzip_open"),
            create: resolve(&handle, "libzip_create"),
            close: resolve(&handle, "libzip_close"),
            entry_count: resolve(&handle, "libzip_entry_count"),
            entry_name: resolve(&handle, "libzip_entry_name"),
            entry_size: resolve(&handle, "libzip_entry_size"),
            entry_compressed_size: resolve(&handle, "libzip_entry_compressed_size"),
            entry_method: resolve(&handle, "libzip_entry_method"),
            entry_is_dir: resolve(&handle, "libzip_entry_is_dir"),
            extract: resolve(&handle, "libzip_extract"),
            extract_to_file: resolve(&handle, "libzip_extract_to_file"),
            add_file: resolve(&handle, "libzip_add_file"),
            add_dir: resolve(&handle, "libzip_add_dir"),
            write_to_file: resolve(&handle, "libzip_write_to_file"),
            _handle: handle,
        };
        LIB = Some(lib);
    }
    true
}

// ── ZipReader ───────────────────────────────────────────────────────────────

/// An open ZIP archive for reading.
pub struct ZipReader {
    handle: u32,
}

impl ZipReader {
    /// Open a ZIP archive for reading.
    pub fn open(path: &str) -> Option<ZipReader> {
        let h = (lib().open)(path.as_ptr(), path.len() as u32);
        if h == 0 { None } else { Some(ZipReader { handle: h }) }
    }

    /// Number of entries in the archive.
    pub fn entry_count(&self) -> u32 {
        (lib().entry_count)(self.handle)
    }

    /// Get entry name by index.
    pub fn entry_name(&self, index: u32) -> String {
        let mut buf = [0u8; 256];
        let n = (lib().entry_name)(self.handle, index, buf.as_mut_ptr(), 256);
        let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
        String::from(s)
    }

    /// Get uncompressed size of an entry.
    pub fn entry_size(&self, index: u32) -> u32 {
        (lib().entry_size)(self.handle, index)
    }

    /// Get compressed size of an entry.
    pub fn entry_compressed_size(&self, index: u32) -> u32 {
        (lib().entry_compressed_size)(self.handle, index)
    }

    /// Get compression method (0=stored, 8=deflate).
    pub fn entry_method(&self, index: u32) -> u32 {
        (lib().entry_method)(self.handle, index)
    }

    /// Check if entry is a directory.
    pub fn entry_is_dir(&self, index: u32) -> bool {
        (lib().entry_is_dir)(self.handle, index) == 1
    }

    /// Extract an entry to a byte vector.
    pub fn extract(&self, index: u32) -> Option<alloc::vec::Vec<u8>> {
        let size = self.entry_size(index);
        if size == 0 {
            return Some(alloc::vec::Vec::new());
        }
        let mut buf = vec![0u8; size as usize];
        let n = (lib().extract)(self.handle, index, buf.as_mut_ptr(), size);
        if n == u32::MAX { None } else { buf.truncate(n as usize); Some(buf) }
    }

    /// Extract an entry directly to a file.
    pub fn extract_to_file(&self, index: u32, path: &str) -> bool {
        (lib().extract_to_file)(self.handle, index, path.as_ptr(), path.len() as u32) == 0
    }
}

impl Drop for ZipReader {
    fn drop(&mut self) {
        if self.handle != 0 {
            (lib().close)(self.handle);
        }
    }
}

// ── ZipWriter ───────────────────────────────────────────────────────────────

/// A ZIP archive being created.
pub struct ZipWriter {
    handle: u32,
}

impl ZipWriter {
    /// Create a new empty ZIP archive.
    pub fn new() -> Option<ZipWriter> {
        let h = (lib().create)();
        if h == 0 { None } else { Some(ZipWriter { handle: h }) }
    }

    /// Add a file with data. `compress` = true uses DEFLATE.
    pub fn add_file(&self, name: &str, data: &[u8], compress: bool) -> bool {
        (lib().add_file)(
            self.handle,
            name.as_ptr(), name.len() as u32,
            data.as_ptr(), data.len() as u32,
            if compress { 1 } else { 0 },
        ) == 0
    }

    /// Add a directory entry (name should end with '/').
    pub fn add_dir(&self, name: &str) -> bool {
        (lib().add_dir)(self.handle, name.as_ptr(), name.len() as u32) == 0
    }

    /// Finalize and write the archive to a file.
    /// Consumes the writer handle.
    pub fn write_to_file(self, path: &str) -> bool {
        let result = (lib().write_to_file)(self.handle, path.as_ptr(), path.len() as u32) == 0;
        core::mem::forget(self); // Handle already freed by write_to_file
        result
    }
}

impl Drop for ZipWriter {
    fn drop(&mut self) {
        if self.handle != 0 {
            (lib().close)(self.handle);
        }
    }
}
