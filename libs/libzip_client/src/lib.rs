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

#[cfg(feature = "host")]
mod host {
    use super::*;

    pub struct ZipReader;
    impl ZipReader {
        pub fn open(_path: &str) -> Option<ZipReader> {
            None
        }
        pub fn entry_count(&self) -> u32 {
            0
        }
        pub fn entry_name(&self, _index: u32) -> String {
            String::new()
        }
        pub fn entry_size(&self, _index: u32) -> u32 {
            0
        }
        pub fn entry_compressed_size(&self, _index: u32) -> u32 {
            0
        }
        pub fn entry_method(&self, _index: u32) -> u32 {
            0
        }
        pub fn entry_is_dir(&self, _index: u32) -> bool {
            false
        }
        pub fn extract(&self, _index: u32) -> Option<alloc::vec::Vec<u8>> {
            None
        }
        pub fn extract_to_file(&self, _index: u32, _path: &str) -> bool {
            false
        }
    }

    pub struct ZipWriter;
    impl ZipWriter {
        pub fn new() -> Option<ZipWriter> {
            None
        }
        pub fn add_file(&self, _name: &str, _data: &[u8], _compress: bool) -> bool {
            false
        }
        pub fn add_dir(&self, _name: &str) -> bool {
            false
        }
        pub fn write_to_file(self, _path: &str) -> bool {
            false
        }
    }

    pub fn gzip_compress_file(_in_path: &str, _out_path: &str) -> bool {
        false
    }
    pub fn gzip_decompress_file(_in_path: &str, _out_path: &str) -> bool {
        false
    }

    pub struct TarReader;
    impl TarReader {
        pub fn open(_path: &str) -> Option<TarReader> {
            None
        }
        pub fn entry_count(&self) -> u32 {
            0
        }
        pub fn entry_name(&self, _index: u32) -> String {
            String::new()
        }
        pub fn entry_size(&self, _index: u32) -> u32 {
            0
        }
        pub fn entry_is_dir(&self, _index: u32) -> bool {
            false
        }
        pub fn extract(&self, _index: u32) -> Option<alloc::vec::Vec<u8>> {
            None
        }
        pub fn extract_to_file(&self, _index: u32, _path: &str) -> bool {
            false
        }
    }

    pub struct TarWriter;
    impl TarWriter {
        pub fn new() -> Option<TarWriter> {
            None
        }
        pub fn add_file(&self, _name: &str, _data: &[u8]) -> bool {
            false
        }
        pub fn add_dir(&self, _name: &str) -> bool {
            false
        }
        pub fn write_to_file(self, _path: &str, _compress: bool) -> bool {
            false
        }
    }
}

#[cfg(feature = "host")]
pub use host::*;

#[cfg(not(feature = "host"))]
mod imp {
    use super::*;

    dynlink::dll_exports! {
        lib_path: "/Libraries/libzip.so",
        lib_struct: LibZip,
        symbols: {
            // Zip functions
            libzip_open(path: *const u8, len: u32) -> u32,
            libzip_create() -> u32,
            libzip_close(handle: u32) -> (),
            libzip_entry_count(handle: u32) -> u32,
            libzip_entry_name(handle: u32, index: u32, buf: *mut u8, buf_len: u32) -> u32,
            libzip_entry_size(handle: u32, index: u32) -> u32,
            libzip_entry_compressed_size(handle: u32, index: u32) -> u32,
            libzip_entry_method(handle: u32, index: u32) -> u32,
            libzip_entry_is_dir(handle: u32, index: u32) -> u32,
            libzip_extract(handle: u32, index: u32, buf: *mut u8, buf_len: u32) -> u32,
            libzip_extract_to_file(handle: u32, index: u32, path: *const u8, path_len: u32) -> u32,
            libzip_add_file(handle: u32, name: *const u8, name_len: u32, data: *const u8, data_len: u32, compress: u32) -> u32,
            libzip_add_dir(handle: u32, name: *const u8, name_len: u32) -> u32,
            libzip_write_to_file(handle: u32, path: *const u8, path_len: u32) -> u32,
            // Gzip functions
            libzip_gzip_compress_file(in_path: *const u8, in_len: u32, out_path: *const u8, out_len: u32) -> u32,
            libzip_gzip_decompress_file(in_path: *const u8, in_len: u32, out_path: *const u8, out_len: u32) -> u32,
            // Tar functions
            libzip_tar_open(path: *const u8, len: u32) -> u32,
            libzip_tar_create() -> u32,
            libzip_tar_close(handle: u32) -> (),
            libzip_tar_entry_count(handle: u32) -> u32,
            libzip_tar_entry_name(handle: u32, index: u32, buf: *mut u8, buf_len: u32) -> u32,
            libzip_tar_entry_size(handle: u32, index: u32) -> u32,
            libzip_tar_entry_is_dir(handle: u32, index: u32) -> u32,
            libzip_tar_extract(handle: u32, index: u32, buf: *mut u8, buf_len: u32) -> u32,
            libzip_tar_extract_to_file(handle: u32, index: u32, path: *const u8, path_len: u32) -> u32,
            libzip_tar_add_file(handle: u32, name: *const u8, name_len: u32, data: *const u8, data_len: u32) -> u32,
            libzip_tar_add_dir(handle: u32, name: *const u8, name_len: u32) -> u32,
            libzip_tar_write_to_file(handle: u32, path: *const u8, path_len: u32, compress: u32) -> u32,
        }
    }

    // ── ZipReader ───────────────────────────────────────────────────────────────

    /// An open ZIP archive for reading.
    pub struct ZipReader {
        handle: u32,
    }

    impl ZipReader {
        /// Open a ZIP archive for reading.
        pub fn open(path: &str) -> Option<ZipReader> {
            let h = (lib().libzip_open)(path.as_ptr(), path.len() as u32);
            if h == 0 {
                None
            } else {
                Some(ZipReader { handle: h })
            }
        }

        /// Number of entries in the archive.
        pub fn entry_count(&self) -> u32 {
            (lib().libzip_entry_count)(self.handle)
        }

        /// Get entry name by index.
        pub fn entry_name(&self, index: u32) -> String {
            let mut buf = [0u8; 256];
            let n = (lib().libzip_entry_name)(self.handle, index, buf.as_mut_ptr(), 256);
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
            String::from(s)
        }

        /// Get uncompressed size of an entry.
        pub fn entry_size(&self, index: u32) -> u32 {
            (lib().libzip_entry_size)(self.handle, index)
        }

        /// Get compressed size of an entry.
        pub fn entry_compressed_size(&self, index: u32) -> u32 {
            (lib().libzip_entry_compressed_size)(self.handle, index)
        }

        /// Get compression method (0=stored, 8=deflate).
        pub fn entry_method(&self, index: u32) -> u32 {
            (lib().libzip_entry_method)(self.handle, index)
        }

        /// Check if entry is a directory.
        pub fn entry_is_dir(&self, index: u32) -> bool {
            (lib().libzip_entry_is_dir)(self.handle, index) == 1
        }

        /// Extract an entry to a byte vector.
        pub fn extract(&self, index: u32) -> Option<alloc::vec::Vec<u8>> {
            let size = self.entry_size(index);
            if size == 0 {
                return Some(alloc::vec::Vec::new());
            }
            let mut buf = vec![0u8; size as usize];
            let n = (lib().libzip_extract)(self.handle, index, buf.as_mut_ptr(), size);
            if n == u32::MAX {
                None
            } else {
                buf.truncate(n as usize);
                Some(buf)
            }
        }

        /// Extract an entry directly to a file.
        pub fn extract_to_file(&self, index: u32, path: &str) -> bool {
            (lib().libzip_extract_to_file)(self.handle, index, path.as_ptr(), path.len() as u32)
                == 0
        }
    }

    impl Drop for ZipReader {
        fn drop(&mut self) {
            if self.handle != 0 {
                (lib().libzip_close)(self.handle);
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
            let h = (lib().libzip_create)();
            if h == 0 {
                None
            } else {
                Some(ZipWriter { handle: h })
            }
        }

        /// Add a file with data. `compress` = true uses DEFLATE.
        pub fn add_file(&self, name: &str, data: &[u8], compress: bool) -> bool {
            (lib().libzip_add_file)(
                self.handle,
                name.as_ptr(),
                name.len() as u32,
                data.as_ptr(),
                data.len() as u32,
                if compress { 1 } else { 0 },
            ) == 0
        }

        /// Add a directory entry (name should end with '/').
        pub fn add_dir(&self, name: &str) -> bool {
            (lib().libzip_add_dir)(self.handle, name.as_ptr(), name.len() as u32) == 0
        }

        /// Finalize and write the archive to a file.
        /// Consumes the writer handle.
        pub fn write_to_file(self, path: &str) -> bool {
            let result =
                (lib().libzip_write_to_file)(self.handle, path.as_ptr(), path.len() as u32) == 0;
            core::mem::forget(self); // Handle already freed by write_to_file
            result
        }
    }

    impl Drop for ZipWriter {
        fn drop(&mut self) {
            if self.handle != 0 {
                (lib().libzip_close)(self.handle);
            }
        }
    }

    // ── Gzip ────────────────────────────────────────────────────────────────────

    /// Compress a file with gzip. Returns true on success.
    pub fn gzip_compress_file(in_path: &str, out_path: &str) -> bool {
        (lib().libzip_gzip_compress_file)(
            in_path.as_ptr(),
            in_path.len() as u32,
            out_path.as_ptr(),
            out_path.len() as u32,
        ) == 0
    }

    /// Decompress a gzip file. Returns true on success.
    pub fn gzip_decompress_file(in_path: &str, out_path: &str) -> bool {
        (lib().libzip_gzip_decompress_file)(
            in_path.as_ptr(),
            in_path.len() as u32,
            out_path.as_ptr(),
            out_path.len() as u32,
        ) == 0
    }

    // ── TarReader ───────────────────────────────────────────────────────────────

    /// An open tar archive for reading.
    pub struct TarReader {
        handle: u32,
    }

    impl TarReader {
        /// Open a tar (or .tar.gz) archive for reading.
        pub fn open(path: &str) -> Option<TarReader> {
            let h = (lib().libzip_tar_open)(path.as_ptr(), path.len() as u32);
            if h == 0 {
                None
            } else {
                Some(TarReader { handle: h })
            }
        }

        /// Number of entries in the archive.
        pub fn entry_count(&self) -> u32 {
            (lib().libzip_tar_entry_count)(self.handle)
        }

        /// Get entry name by index.
        pub fn entry_name(&self, index: u32) -> String {
            let mut buf = [0u8; 512];
            let n = (lib().libzip_tar_entry_name)(self.handle, index, buf.as_mut_ptr(), 512);
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
            String::from(s)
        }

        /// Get size of an entry.
        pub fn entry_size(&self, index: u32) -> u32 {
            (lib().libzip_tar_entry_size)(self.handle, index)
        }

        /// Check if entry is a directory.
        pub fn entry_is_dir(&self, index: u32) -> bool {
            (lib().libzip_tar_entry_is_dir)(self.handle, index) == 1
        }

        /// Extract an entry to a byte vector.
        pub fn extract(&self, index: u32) -> Option<alloc::vec::Vec<u8>> {
            let size = self.entry_size(index);
            if size == 0 {
                return Some(alloc::vec::Vec::new());
            }
            let mut buf = vec![0u8; size as usize];
            let n = (lib().libzip_tar_extract)(self.handle, index, buf.as_mut_ptr(), size);
            if n == u32::MAX {
                None
            } else {
                buf.truncate(n as usize);
                Some(buf)
            }
        }

        /// Extract an entry directly to a file.
        pub fn extract_to_file(&self, index: u32, path: &str) -> bool {
            (lib().libzip_tar_extract_to_file)(self.handle, index, path.as_ptr(), path.len() as u32)
                == 0
        }
    }

    impl Drop for TarReader {
        fn drop(&mut self) {
            if self.handle != 0 {
                (lib().libzip_tar_close)(self.handle);
            }
        }
    }

    // ── TarWriter ───────────────────────────────────────────────────────────────

    /// A tar archive being created.
    pub struct TarWriter {
        handle: u32,
    }

    impl TarWriter {
        /// Create a new empty tar archive.
        pub fn new() -> Option<TarWriter> {
            let h = (lib().libzip_tar_create)();
            if h == 0 {
                None
            } else {
                Some(TarWriter { handle: h })
            }
        }

        /// Add a file with data.
        pub fn add_file(&self, name: &str, data: &[u8]) -> bool {
            (lib().libzip_tar_add_file)(
                self.handle,
                name.as_ptr(),
                name.len() as u32,
                data.as_ptr(),
                data.len() as u32,
            ) == 0
        }

        /// Add a directory entry.
        pub fn add_dir(&self, name: &str) -> bool {
            (lib().libzip_tar_add_dir)(self.handle, name.as_ptr(), name.len() as u32) == 0
        }

        /// Finalize and write the archive to a file.
        /// If `compress` is true, output is gzip-compressed (.tar.gz).
        /// Consumes the writer handle.
        pub fn write_to_file(self, path: &str, compress: bool) -> bool {
            let result = (lib().libzip_tar_write_to_file)(
                self.handle,
                path.as_ptr(),
                path.len() as u32,
                if compress { 1 } else { 0 },
            ) == 0;
            core::mem::forget(self);
            result
        }
    }

    impl Drop for TarWriter {
        fn drop(&mut self) {
            if self.handle != 0 {
                (lib().libzip_tar_close)(self.handle);
            }
        }
    }
}

#[cfg(not(feature = "host"))]
pub use imp::*;
