//! Native localStorage / sessionStorage host objects.
//!
//! `make_storage(origin, persistent)` creates a storage object backed by an
//! in-memory JS object.  When `persistent = true` the data is additionally
//! written to `/tmp/surf_ls_<sanitized-origin>.dat` on every mutation and
//! loaded from that file at construction time.
//!
//! File format (one entry per line):
//! ```text
//! key\tvalue\n
//! ```
//! Backslash, tab and newline inside keys/values are escaped as `\\`, `\t`,
//! `\n` respectively.

use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::RefCell;

use libjs::JsValue;
use libjs::Vm;
use libjs::value::JsObject;
use libjs::vm::native_fn;

use super::arg_string;

// ═══════════════════════════════════════════════════════════
// Public constructor
// ═══════════════════════════════════════════════════════════

/// Create a storage object (localStorage or sessionStorage).
///
/// * `origin` — the page origin string (e.g. `"https://example.com"`).
///   Used to derive the persistence file path when `persistent = true`.
/// * `persistent` — when `true` the storage is loaded from and saved to disk;
///   when `false` (sessionStorage) data lives only in memory.
pub fn make_storage(origin: &str, persistent: bool) -> JsValue {
    let mut obj = JsObject::new();
    obj.set(String::from("__data"), JsValue::Object(Rc::new(RefCell::new(JsObject::new()))));

    // For persistent (localStorage), derive the file path and pre-load data.
    if persistent && !origin.is_empty() {
        let path = storage_path(origin);
        if let Ok(contents) = anyos_std::fs::read_to_string(&path) {
            load_entries_into(&obj, &contents);
        }
        obj.set(String::from("__path"), JsValue::String(path));
    }

    obj.set(String::from("getItem"), native_fn("getItem", storage_get_item));
    obj.set(String::from("setItem"), native_fn("setItem", storage_set_item));
    obj.set(String::from("removeItem"), native_fn("removeItem", storage_remove_item));
    obj.set(String::from("clear"), native_fn("clear", storage_clear));
    obj.set(String::from("key"), native_fn("key", storage_key));

    JsValue::Object(Rc::new(RefCell::new(obj)))
}

// ═══════════════════════════════════════════════════════════
// File path helpers
// ═══════════════════════════════════════════════════════════

/// Derive a safe filesystem path from `origin`.
///
/// Characters outside `[A-Za-z0-9\-.]` are replaced with `_`.
fn storage_path(origin: &str) -> String {
    let mut path = String::from("/tmp/surf_ls_");
    for c in origin.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' => path.push(c),
            _ => path.push('_'),
        }
    }
    path.push_str(".dat");
    path
}

// ═══════════════════════════════════════════════════════════
// Serialization helpers
// ═══════════════════════════════════════════════════════════

/// Escape `s` for the storage file format (backslash, tab, newline → `\\`, `\t`, `\n`).
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// Unescape a storage file token (reverse of `escape`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('\\') => out.push('\\'),
                Some('t')  => out.push('\t'),
                Some('n')  => out.push('\n'),
                Some('r')  => out.push('\r'),
                Some(other)=> { out.push('\\'); out.push(other); }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Populate `obj.__data` from the contents of a storage file.
fn load_entries_into(obj: &JsObject, contents: &str) {
    let data = match obj.properties.get("__data") {
        Some(p) => p.value.clone(),
        None => return,
    };
    for line in contents.lines() {
        if let Some(tab_pos) = line.find('\t') {
            let key = unescape(&line[..tab_pos]);
            let val = unescape(&line[tab_pos + 1..]);
            data.set_property(key, JsValue::String(val));
        }
    }
}

/// Serialize all entries from a data object into the file format.
fn serialize_data(data: &JsValue) -> String {
    let mut out = String::new();
    if let JsValue::Object(obj) = data {
        for (k, prop) in &obj.borrow().properties {
            let val_str = prop.value.to_js_string();
            out.push_str(&escape(k));
            out.push('\t');
            out.push_str(&escape(&val_str));
            out.push('\n');
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════
// Persistence trigger
// ═══════════════════════════════════════════════════════════

/// Write current storage contents to disk if this is a persistent storage.
fn persist(vm: &Vm) {
    let path = get_path(vm);
    if path.is_empty() { return; }
    if let Some(data) = get_data(vm) {
        let contents = serialize_data(&data);
        let _ = anyos_std::fs::write_bytes(&path, contents.as_bytes());
    }
}

// ═══════════════════════════════════════════════════════════
// Internal accessors
// ═══════════════════════════════════════════════════════════

fn get_data(vm: &Vm) -> Option<JsValue> {
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        if let Some(p) = o.properties.get("__data") {
            return Some(p.value.clone());
        }
    }
    None
}

fn get_path(vm: &Vm) -> String {
    if let JsValue::Object(obj) = &vm.current_this {
        let o = obj.borrow();
        if let Some(p) = o.properties.get("__path") {
            if let JsValue::String(s) = &p.value { return s.clone(); }
        }
    }
    String::new()
}

// ═══════════════════════════════════════════════════════════
// Native method implementations
// ═══════════════════════════════════════════════════════════

fn storage_get_item(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = arg_string(args, 0);
    if let Some(data) = get_data(vm) {
        let val = data.get_property(&key);
        if matches!(val, JsValue::Undefined) { return JsValue::Null; }
        return val;
    }
    JsValue::Null
}

fn storage_set_item(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = arg_string(args, 0);
    let val = arg_string(args, 1);
    if let Some(data) = get_data(vm) {
        data.set_property(key, JsValue::String(val));
    }
    persist(vm);
    JsValue::Undefined
}

fn storage_remove_item(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let key = arg_string(args, 0);
    if let Some(data) = get_data(vm) {
        if let JsValue::Object(obj) = &data {
            obj.borrow_mut().properties.remove(&key);
        }
    }
    persist(vm);
    JsValue::Undefined
}

fn storage_clear(vm: &mut Vm, _args: &[JsValue]) -> JsValue {
    if let JsValue::Object(obj) = &vm.current_this {
        obj.borrow_mut().set(String::from("__data"), JsValue::Object(Rc::new(RefCell::new(JsObject::new()))));
    }
    persist(vm);
    JsValue::Undefined
}

fn storage_key(vm: &mut Vm, args: &[JsValue]) -> JsValue {
    let idx = args.first().map(|v| v.to_number() as usize).unwrap_or(0);
    if let Some(data) = get_data(vm) {
        if let JsValue::Object(obj) = &data {
            let o = obj.borrow();
            let keys: Vec<&String> = o.properties.keys().collect();
            if let Some(k) = keys.get(idx) {
                return JsValue::String((*k).clone());
            }
        }
    }
    JsValue::Null
}
