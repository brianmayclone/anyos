//! libini_client — Safe Rust wrapper for the libini shared library.
//!
//! Loads `/Libraries/libini.so` via `dl_open`/`dl_sym` and provides
//! ergonomic Rust types for INI file parsing.
//!
//! # Usage
//! ```rust
//! libini_client::init();
//! let ini = libini_client::IniDoc::parse("[section]\nkey = value\n").unwrap();
//! let val = ini.get("section", "key");  // Some("value")
//! let num = ini.get_u32("section", "count", 0);
//! ```

#![no_std]

extern crate alloc;

use alloc::string::String;

dynlink::dll_exports! {
    lib_path: "/Libraries/libini.so",
    lib_struct: LibIni,
    symbols: {
        libini_parse(text_ptr: *const u8, text_len: u32) -> u32,
        libini_close(handle: u32) -> (),
        libini_get(
            handle: u32,
            section_ptr: *const u8, section_len: u32,
            key_ptr: *const u8, key_len: u32,
            buf_ptr: *mut u8, buf_len: u32
        ) -> u32,
        libini_get_u32(
            handle: u32,
            section_ptr: *const u8, section_len: u32,
            key_ptr: *const u8, key_len: u32,
            default_val: u32
        ) -> u32,
        libini_get_i32(
            handle: u32,
            section_ptr: *const u8, section_len: u32,
            key_ptr: *const u8, key_len: u32,
            default_val: i32
        ) -> i32,
        libini_get_bool(
            handle: u32,
            section_ptr: *const u8, section_len: u32,
            key_ptr: *const u8, key_len: u32,
            default_val: u32
        ) -> u32,
        libini_get_hex(
            handle: u32,
            section_ptr: *const u8, section_len: u32,
            key_ptr: *const u8, key_len: u32,
            default_val: u32
        ) -> u32,
        libini_has_section(
            handle: u32,
            section_ptr: *const u8, section_len: u32
        ) -> u32,
        libini_section_count(handle: u32) -> u32,
        libini_section_name(
            handle: u32, index: u32,
            buf_ptr: *mut u8, buf_len: u32
        ) -> u32,
        libini_entry_count(
            handle: u32,
            section_ptr: *const u8, section_len: u32
        ) -> u32,
        libini_entry_key(
            handle: u32,
            section_ptr: *const u8, section_len: u32,
            index: u32,
            buf_ptr: *mut u8, buf_len: u32
        ) -> u32,
        libini_entry_value(
            handle: u32,
            section_ptr: *const u8, section_len: u32,
            index: u32,
            buf_ptr: *mut u8, buf_len: u32
        ) -> u32,
    }
}

// ── IniDoc ─────────────────────────────────────────────────────────────────

/// A parsed INI document handle.
pub struct IniDoc {
    handle: u32,
}

impl IniDoc {
    /// Parse INI text. Returns `None` if parsing fails or library not loaded.
    pub fn parse(text: &str) -> Option<IniDoc> {
        let h = (lib().libini_parse)(text.as_ptr(), text.len() as u32);
        if h == 0 { None } else { Some(IniDoc { handle: h }) }
    }

    /// Load and parse an INI file from disk.
    pub fn load(path: &str) -> Option<IniDoc> {
        let text = anyos_std::fs::read_to_string(path).ok()?;
        Self::parse(&text)
    }

    /// Look up a string value. Returns `None` if not found.
    pub fn get(&self, section: &str, key: &str) -> Option<String> {
        let mut buf = [0u8; 512];
        let len = (lib().libini_get)(
            self.handle,
            section.as_ptr(), section.len() as u32,
            key.as_ptr(), key.len() as u32,
            buf.as_mut_ptr(), buf.len() as u32,
        );
        if len == 0 { return None; }
        let s = core::str::from_utf8(&buf[..len as usize]).ok()?;
        Some(String::from(s))
    }

    /// Look up a u32 value, returning `default` if not found.
    pub fn get_u32(&self, section: &str, key: &str, default: u32) -> u32 {
        (lib().libini_get_u32)(
            self.handle,
            section.as_ptr(), section.len() as u32,
            key.as_ptr(), key.len() as u32,
            default,
        )
    }

    /// Look up an i32 value.
    pub fn get_i32(&self, section: &str, key: &str, default: i32) -> i32 {
        (lib().libini_get_i32)(
            self.handle,
            section.as_ptr(), section.len() as u32,
            key.as_ptr(), key.len() as u32,
            default,
        )
    }

    /// Look up a boolean value. Recognises yes/true/1/on and no/false/0/off.
    pub fn get_bool(&self, section: &str, key: &str, default: bool) -> bool {
        let result = (lib().libini_get_bool)(
            self.handle,
            section.as_ptr(), section.len() as u32,
            key.as_ptr(), key.len() as u32,
            if default { 1 } else { 0 },
        );
        result != 0
    }

    /// Look up a hex color value (e.g. `0xFF00AA55`).
    pub fn get_hex(&self, section: &str, key: &str, default: u32) -> u32 {
        (lib().libini_get_hex)(
            self.handle,
            section.as_ptr(), section.len() as u32,
            key.as_ptr(), key.len() as u32,
            default,
        )
    }

    /// Check if a section exists.
    pub fn has_section(&self, section: &str) -> bool {
        (lib().libini_has_section)(
            self.handle,
            section.as_ptr(), section.len() as u32,
        ) != 0
    }

    /// Number of sections in the document.
    pub fn section_count(&self) -> u32 {
        (lib().libini_section_count)(self.handle)
    }

    /// Get section name by index.
    pub fn section_name(&self, index: u32) -> Option<String> {
        let mut buf = [0u8; 256];
        let len = (lib().libini_section_name)(
            self.handle, index,
            buf.as_mut_ptr(), buf.len() as u32,
        );
        if len == 0 { return None; }
        let s = core::str::from_utf8(&buf[..len as usize]).ok()?;
        Some(String::from(s))
    }

    /// Number of key=value entries in a section.
    pub fn entry_count(&self, section: &str) -> u32 {
        (lib().libini_entry_count)(
            self.handle,
            section.as_ptr(), section.len() as u32,
        )
    }

    /// Get key name at index within a section.
    pub fn entry_key(&self, section: &str, index: u32) -> Option<String> {
        let mut buf = [0u8; 256];
        let len = (lib().libini_entry_key)(
            self.handle,
            section.as_ptr(), section.len() as u32,
            index,
            buf.as_mut_ptr(), buf.len() as u32,
        );
        if len == 0 { return None; }
        let s = core::str::from_utf8(&buf[..len as usize]).ok()?;
        Some(String::from(s))
    }

    /// Get value at index within a section.
    pub fn entry_value(&self, section: &str, index: u32) -> Option<String> {
        let mut buf = [0u8; 512];
        let len = (lib().libini_entry_value)(
            self.handle,
            section.as_ptr(), section.len() as u32,
            index,
            buf.as_mut_ptr(), buf.len() as u32,
        );
        if len == 0 { return None; }
        let s = core::str::from_utf8(&buf[..len as usize]).ok()?;
        Some(String::from(s))
    }
}

impl Drop for IniDoc {
    fn drop(&mut self) {
        (lib().libini_close)(self.handle);
    }
}
