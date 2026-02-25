# anyOS Archive Library (libzip) API Reference

The **libzip** shared library provides reading and writing of ZIP, TAR, and GZIP archives. It includes DEFLATE compression/decompression, CRC-32 verification, and transparent `.tar.gz` handling.

**Format:** ELF64 shared object (.so), loaded on demand via `dl_open("/Libraries/libzip.so")`
**Exports:** 28 (14 ZIP + 2 GZIP + 12 TAR)
**Client crate:** `libzip_client` (uses `dynlink::dl_open` / `dl_sym`)

The library uses a **handle-based API** with an internal table of up to **8 concurrent archive handles**. Handles are integer IDs (>0) returned by open/create calls. The client wrapper types (`ZipReader`, `ZipWriter`, `TarReader`, `TarWriter`) manage handles automatically via `Drop`.

---

## Getting Started

### Dependencies

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
dynlink = { path = "../../libs/dynlink" }
libzip_client = { path = "../../libs/libzip_client" }
```

### Example: Extract a ZIP Archive

```rust
use libzip_client as zip;

zip::init();

let reader = zip::ZipReader::open("/path/to/archive.zip").unwrap();
for i in 0..reader.entry_count() {
    let name = reader.entry_name(i);
    if reader.entry_is_dir(i) {
        anyos_std::fs::mkdir(&name);
    } else {
        reader.extract_to_file(i, &name);
    }
}
// reader is automatically closed on drop
```

### Example: Create a ZIP Archive

```rust
use libzip_client as zip;

zip::init();

let writer = zip::ZipWriter::new().unwrap();
writer.add_file("hello.txt", b"Hello, World!", true);  // true = DEFLATE
writer.add_dir("subdir/");
writer.write_to_file("/path/to/output.zip");  // consumes writer
```

---

## Initialization

### `init() -> bool`

Load `libzip.so` and cache all 28 function pointers. Must be called once before any other operations. Returns `true` on success, `false` if the library cannot be loaded.

---

## ZIP Functions

### ZipReader

A read-only handle to an opened ZIP archive. Implements `Drop` to automatically close the handle.

#### `ZipReader::open(path: &str) -> Option<ZipReader>`

Open a ZIP archive from a filesystem path. Reads the entire file into memory and parses the central directory.

| Parameter | Type | Description |
|-----------|------|-------------|
| path | `&str` | Filesystem path to `.zip` file |
| **Returns** | `Option<ZipReader>` | Reader handle, or `None` on error |

#### `entry_count(&self) -> u32`

Returns the number of entries (files and directories) in the archive.

#### `entry_name(&self, index: u32) -> String`

Get the name of an entry by zero-based index. Names may include path components (e.g. `"dir/file.txt"`). Directory entries end with `'/'`.

#### `entry_size(&self, index: u32) -> u32`

Get the uncompressed size of an entry in bytes.

#### `entry_compressed_size(&self, index: u32) -> u32`

Get the compressed size of an entry in bytes.

#### `entry_method(&self, index: u32) -> u32`

Get the compression method of an entry.

| Value | Method |
|-------|--------|
| 0 | Stored (no compression) |
| 8 | DEFLATE |

#### `entry_is_dir(&self, index: u32) -> bool`

Returns `true` if the entry is a directory (name ends with `'/'`).

#### `extract(&self, index: u32) -> Option<Vec<u8>>`

Extract an entry to a byte vector. Decompresses DEFLATE entries and verifies the CRC-32 checksum. Returns `None` on decompression or CRC error. Returns an empty `Vec` for zero-size entries.

#### `extract_to_file(&self, index: u32, path: &str) -> bool`

Extract an entry directly to a file on disk. Returns `true` on success. The file is created with `O_WRITE | O_CREATE | O_TRUNC`.

---

### ZipWriter

A handle for building a new ZIP archive in memory. Implements `Drop` to close the handle if not consumed by `write_to_file`.

#### `ZipWriter::new() -> Option<ZipWriter>`

Create a new empty ZIP archive. Returns `None` if the handle table is full (8 concurrent handles).

#### `add_file(&self, name: &str, data: &[u8], compress: bool) -> bool`

Add a file entry with data.

| Parameter | Type | Description |
|-----------|------|-------------|
| name | `&str` | Entry name (e.g. `"dir/file.txt"`) |
| data | `&[u8]` | File content bytes |
| compress | `bool` | `true` = DEFLATE, `false` = Stored |

When `compress` is `true`, the library uses DEFLATE but falls back to Stored if the compressed output is not smaller than the original data.

#### `add_dir(&self, name: &str) -> bool`

Add a directory entry. The name should end with `'/'`.

#### `write_to_file(self, path: &str) -> bool`

Finalize the archive and write it to a file. **Consumes the writer** -- the handle is freed after this call. Returns `true` on success.

The output is a valid PKZIP archive with local file headers, central directory, and end-of-central-directory record (APPNOTE 6.3.x compatible, version 2.0).

---

## GZIP Functions

Free functions for file-level gzip compression and decompression. These operate on filesystem paths -- the library reads and writes files internally.

### `gzip_compress_file(in_path: &str, out_path: &str) -> bool`

Compress a file with gzip (RFC 1952). Reads `in_path`, writes compressed output to `out_path`. Returns `true` on success.

### `gzip_decompress_file(in_path: &str, out_path: &str) -> bool`

Decompress a gzip file. Reads `in_path`, verifies the CRC-32 and ISIZE trailer, writes decompressed output to `out_path`. Returns `true` on success.

---

## TAR Functions

### TarReader

A read-only handle to an opened tar archive. Implements `Drop` to automatically close the handle. Transparently handles `.tar.gz` files -- if the input starts with gzip magic bytes (`0x1F 0x8B`), it is decompressed before parsing.

#### `TarReader::open(path: &str) -> Option<TarReader>`

Open a tar or `.tar.gz` archive from a filesystem path.

| Parameter | Type | Description |
|-----------|------|-------------|
| path | `&str` | Filesystem path to `.tar` or `.tar.gz` file |
| **Returns** | `Option<TarReader>` | Reader handle, or `None` on error |

#### `entry_count(&self) -> u32`

Returns the number of entries in the archive.

#### `entry_name(&self, index: u32) -> String`

Get the name of an entry by zero-based index. Supports long names via the ustar prefix+name fields (up to 255 characters). Name buffer is 512 bytes.

#### `entry_size(&self, index: u32) -> u32`

Get the size of an entry in bytes.

#### `entry_is_dir(&self, index: u32) -> bool`

Returns `true` if the entry is a directory (typeflag `'5'` or name ends with `'/'`).

#### `extract(&self, index: u32) -> Option<Vec<u8>>`

Extract an entry to a byte vector. Returns an empty `Vec` for directories and zero-size entries.

#### `extract_to_file(&self, index: u32, path: &str) -> bool`

Extract an entry directly to a file on disk. Returns `true` on success.

---

### TarWriter

A handle for building a new tar archive in memory. Implements `Drop` to close the handle if not consumed by `write_to_file`.

#### `TarWriter::new() -> Option<TarWriter>`

Create a new empty tar archive.

#### `add_file(&self, name: &str, data: &[u8]) -> bool`

Add a regular file entry. The header is written in ustar format with mode `0644`.

#### `add_dir(&self, name: &str) -> bool`

Add a directory entry with mode `0755`. A trailing `'/'` is appended automatically if not present.

#### `write_to_file(self, path: &str, compress: bool) -> bool`

Finalize the archive and write it to a file. **Consumes the writer.**

| Parameter | Type | Description |
|-----------|------|-------------|
| path | `&str` | Output file path |
| compress | `bool` | `true` = gzip-compress the output (`.tar.gz`), `false` = plain tar |

Two 512-byte zero blocks are appended as the end-of-archive marker before writing.

---

## C ABI Exports

All 28 exported functions use `extern "C"` with `#[no_mangle]`. Strings are passed as `(ptr, len)` pairs. Return value conventions: handles return `>0` on success and `0` on error; operations return `0` on success and `u32::MAX` on error.

### ZIP Exports (14)

| Symbol | Signature | Description |
|--------|-----------|-------------|
| `libzip_open` | `(path_ptr, path_len) -> handle` | Open ZIP for reading |
| `libzip_create` | `() -> handle` | Create new ZIP writer |
| `libzip_close` | `(handle)` | Close any ZIP handle |
| `libzip_entry_count` | `(handle) -> u32` | Entry count |
| `libzip_entry_name` | `(handle, index, buf, buf_len) -> bytes_written` | Entry name |
| `libzip_entry_size` | `(handle, index) -> u32` | Uncompressed size |
| `libzip_entry_compressed_size` | `(handle, index) -> u32` | Compressed size |
| `libzip_entry_method` | `(handle, index) -> u32` | Compression method (0/8) |
| `libzip_entry_is_dir` | `(handle, index) -> u32` | 1 if directory, 0 otherwise |
| `libzip_extract` | `(handle, index, buf, buf_len) -> bytes_written` | Extract to buffer |
| `libzip_extract_to_file` | `(handle, index, path_ptr, path_len) -> status` | Extract to file |
| `libzip_add_file` | `(handle, name_ptr, name_len, data_ptr, data_len, compress) -> status` | Add file |
| `libzip_add_dir` | `(handle, name_ptr, name_len) -> status` | Add directory |
| `libzip_write_to_file` | `(handle, path_ptr, path_len) -> status` | Finalize and write (consumes handle) |

### GZIP Exports (2)

| Symbol | Signature | Description |
|--------|-----------|-------------|
| `libzip_gzip_compress_file` | `(in_ptr, in_len, out_ptr, out_len) -> status` | Compress file |
| `libzip_gzip_decompress_file` | `(in_ptr, in_len, out_ptr, out_len) -> status` | Decompress file |

### TAR Exports (12)

| Symbol | Signature | Description |
|--------|-----------|-------------|
| `libzip_tar_open` | `(path_ptr, path_len) -> handle` | Open tar/tar.gz for reading |
| `libzip_tar_create` | `() -> handle` | Create new tar writer |
| `libzip_tar_close` | `(handle)` | Close tar handle |
| `libzip_tar_entry_count` | `(handle) -> u32` | Entry count |
| `libzip_tar_entry_name` | `(handle, index, buf, buf_len) -> bytes_written` | Entry name |
| `libzip_tar_entry_size` | `(handle, index) -> u32` | Entry size |
| `libzip_tar_entry_is_dir` | `(handle, index) -> u32` | 1 if directory, 0 otherwise |
| `libzip_tar_extract` | `(handle, index, buf, buf_len) -> bytes_written` | Extract to buffer |
| `libzip_tar_extract_to_file` | `(handle, index, path_ptr, path_len) -> status` | Extract to file |
| `libzip_tar_add_file` | `(handle, name_ptr, name_len, data_ptr, data_len) -> status` | Add file |
| `libzip_tar_add_dir` | `(handle, name_ptr, name_len) -> status` | Add directory |
| `libzip_tar_write_to_file` | `(handle, path_ptr, path_len, compress) -> status` | Finalize and write (consumes handle) |

---

## Format Support

### ZIP

PKZIP APPNOTE 6.3.x compatible.

| Feature | Supported |
|---------|-----------|
| Stored (method 0, no compression) | Yes |
| DEFLATE (method 8) | Yes (fixed + dynamic Huffman) |
| CRC-32 verification on extract | Yes |
| Central directory parsing | Yes |
| Local file headers | Yes |
| ZIP version 2.0 | Yes |
| ZIP64 extensions | No |
| Encryption | No |
| Multi-disk archives | No |

**DEFLATE compression** uses LZ77 with fixed Huffman encoding. On extraction, both fixed and dynamic Huffman codes are supported via the inflate module.

**Smart compression fallback:** When adding a file with `compress=true`, the library compares compressed vs. uncompressed size and stores uncompressed if DEFLATE does not reduce size.

### TAR

POSIX ustar format.

| Feature | Supported |
|---------|-----------|
| ustar format headers | Yes |
| Long names (prefix+name, up to 255 chars) | Yes |
| Regular files (typeflag `'0'`) | Yes |
| Directories (typeflag `'5'`) | Yes |
| Checksum verification | Yes |
| Transparent .tar.gz decompression | Yes |
| .tar.gz creation (gzip-compressed output) | Yes |
| GNU binary size extension (high-bit encoding) | Yes (read) |
| Symlinks, hardlinks, device nodes | No |
| PAX extended headers | No |

**Header format:** 512-byte blocks, octal ASCII fields, ustar magic (`ustar\0`), version `00`. File data is padded to 512-byte boundaries. Archives end with two consecutive zero blocks.

### GZIP

RFC 1952 compliant.

| Feature | Supported |
|---------|-----------|
| DEFLATE compression (method 8) | Yes |
| CRC-32 verification | Yes |
| ISIZE verification (original size mod 2^32) | Yes |
| FEXTRA (extra field) | Yes (skipped on decompress) |
| FNAME (original filename) | Yes (skipped on decompress) |
| FCOMMENT (comment) | Yes (skipped on decompress) |
| FHCRC (header CRC) | Yes (skipped on decompress) |
| Multi-member gzip streams | No |

---

## Architecture

- **libzip** (`libs/libzip/`) -- the shared library, built as a `staticlib` and linked by `anyld` into an ELF64 `.so`. Contains modules for ZIP (`zip.rs`), TAR (`tar.rs`), GZIP (`gzip.rs`), DEFLATE compression (`deflate.rs`), inflate decompression (`inflate.rs`), and CRC-32 (`crc32.rs`). Exports 28 `#[no_mangle] pub extern "C"` symbols.
- **libzip_client** (`libs/libzip_client/`) -- client wrapper that resolves symbols via `dynlink::dl_open("/Libraries/libzip.so")` + `dl_sym()`. Caches function pointers in a static `LibZip` struct and provides safe Rust types (`ZipReader`, `ZipWriter`, `TarReader`, `TarWriter`) with automatic handle cleanup via `Drop`.

ZIP and TAR share a common handle table (8 slots total across all archive types). Handles are 1-indexed integers; `0` indicates an error.
