# anyOS INI Parser Library (libini) API Reference

The **libini** shared library provides parsing of INI/conf-style configuration files. It supports sections, key=value pairs, comments, quoted values, and typed value parsing (bool, int, hex).

**Format:** ELF64 shared object (.so)
**Exports:** 13
**Client crate:** `libini_client` (uses `dynlink::dl_open("/Libraries/libini.so")`)

---

## Getting Started

### Dependencies

```toml
[dependencies]
anyos_std = { path = "../../libs/stdlib" }
libini_client = { path = "../../libs/libini_client" }
```

### Example

```rust
use libini_client as ini;

ini::init();

// Load from file
let doc = ini::IniDoc::load("/System/etc/myapp.conf").unwrap();

// Or parse from string
let doc = ini::IniDoc::parse("[server]\nport = 8080\ntls = yes\n").unwrap();

// Look up values
let port = doc.get_u32("server", "port", 80);        // 8080
let tls = doc.get_bool("server", "tls", false);       // true
let name = doc.get("server", "name");                  // Option<String>
let color = doc.get_hex("theme", "accent", 0xFF007AFF); // hex u32
```

---

## Supported Syntax

```ini
# comment (lines starting with # or ;)
; also a comment

global_key = value           # key=value before any section → section ""

[section name]
key = value
number = 42
enabled = yes
color = 0xFF00AA55
quoted = "hello world"       # quotes are stripped
single = 'also works'
```

- **Sections:** `[name]` — names are trimmed, lookup is case-insensitive
- **Keys:** `key=value` or `key = value` — both sides trimmed
- **Comments:** lines starting with `#` or `;`
- **Empty lines:** skipped
- **Quoted values:** `"..."` and `'...'` — outer quotes stripped
- **Windows line endings:** `\r\n` handled correctly
- **Case-insensitive:** Section and key lookups are case-insensitive

---

## IniDoc API (libini_client)

### `IniDoc::parse(text: &str) -> Option<IniDoc>`

Parse INI text from a string. Returns `None` if the library is not loaded.

### `IniDoc::load(path: &str) -> Option<IniDoc>`

Read a file from disk and parse it as INI. Returns `None` if the file can't be read.

### `get(section, key) -> Option<String>`

Look up a string value. Section `""` = global (pre-section) entries.

### `get_u32(section, key, default) -> u32`

Look up and parse as unsigned 32-bit integer.

### `get_i32(section, key, default) -> i32`

Look up and parse as signed 32-bit integer.

### `get_bool(section, key, default) -> bool`

Look up and parse as boolean. Recognizes:
- **True:** `yes`, `true`, `1`, `on` (case-insensitive)
- **False:** `no`, `false`, `0`, `off` (case-insensitive)

### `get_hex(section, key, default) -> u32`

Look up and parse as hex u32. Accepts `0xAARRGGBB` or `AARRGGBB`.

### `has_section(section) -> bool`

Check if a section exists.

### `section_count() -> u32`

Number of sections in the document.

### `section_name(index) -> Option<String>`

Get section name by index (0-based, order of appearance).

### `entry_count(section) -> u32`

Number of key=value entries in a section.

### `entry_key(section, index) -> Option<String>`

Get key name at index within a section.

### `entry_value(section, index) -> Option<String>`

Get value at index within a section.

---

## FFI Exports (libini.so)

| Symbol | Signature |
|--------|-----------|
| `libini_parse` | `(text_ptr: *const u8, text_len: u32) -> u32` |
| `libini_close` | `(handle: u32)` |
| `libini_get` | `(handle, section_ptr, section_len, key_ptr, key_len, buf_ptr, buf_len) -> u32` |
| `libini_get_u32` | `(handle, section_ptr, section_len, key_ptr, key_len, default) -> u32` |
| `libini_get_i32` | `(handle, section_ptr, section_len, key_ptr, key_len, default) -> i32` |
| `libini_get_bool` | `(handle, section_ptr, section_len, key_ptr, key_len, default) -> u32` |
| `libini_get_hex` | `(handle, section_ptr, section_len, key_ptr, key_len, default) -> u32` |
| `libini_has_section` | `(handle, section_ptr, section_len) -> u32` |
| `libini_section_count` | `(handle) -> u32` |
| `libini_section_name` | `(handle, index, buf_ptr, buf_len) -> u32` |
| `libini_entry_count` | `(handle, section_ptr, section_len) -> u32` |
| `libini_entry_key` | `(handle, section_ptr, section_len, index, buf_ptr, buf_len) -> u32` |
| `libini_entry_value` | `(handle, section_ptr, section_len, index, buf_ptr, buf_len) -> u32` |

Handle-based API: `libini_parse` returns a handle (1-16), `libini_close` frees it. Up to 16 documents open simultaneously.

## Architecture

```
libs/libini/
├── src/
│   ├── lib.rs        — FFI exports, handle table, allocator
│   ├── parse.rs      — Zero-allocation INI parser (operates on &str)
│   └── syscall.rs    — Minimal syscall wrappers
├── exports.def       — Symbol export list for anyld
└── Cargo.toml

libs/libini_client/
├── src/
│   └── lib.rs        — IniDoc type, dynlink wrapper, safe Rust API
└── Cargo.toml
```

The parser in `parse.rs` works directly on borrowed `&str` slices without allocation. The FFI layer in `lib.rs` owns the text (copied into the handle's heap) so it outlives the caller's stack.
