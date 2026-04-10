// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Unicode Collation Algorithm — primary-weight comparison.
//!
//! Implements UTS #10 *Level 1* collation:
//!
//!   1. Map each input codepoint to its primary weight(s) using either
//!      - the runtime-loaded full BMP table from
//!        `/System/share/collation/uca.bin`, or
//!      - the embedded fallback table for Latin-1 + Latin Extended-A
//!        (`uca_latin_table::LATIN_PRIMARY_WEIGHTS`).
//!   2. Drop ignorable elements (primary == 0).
//!   3. Compare the resulting weight sequences lexicographically.
//!
//! Codepoints not present in either table fall back to using the codepoint
//! value itself as a weight, so unknown characters sort consistently
//! (although not necessarily in true UCA order).
//!
//! Secondary/tertiary weights (case, diacritic priorities) are not
//! considered — that is sufficient to make `localeCompare` produce
//! human-expected ordering for European text such as
//! `["Apfel", "Äpfel", "Banane"]`.

use alloc::vec::Vec;
use core::cell::UnsafeCell;

use super::uca_latin_table::LATIN_PRIMARY_WEIGHTS;

const UCA_FILE_PATH: &str = "/System/share/collation/uca.bin";
const MAGIC: &[u8; 4] = b"UCA1";

#[cfg(not(feature = "host"))]
fn read_file(path: &str) -> Option<Vec<u8>> {
    anyos_std::fs::read_to_vec(path).ok()
}

#[cfg(feature = "host")]
fn read_file(path: &str) -> Option<Vec<u8>> {
    // On the host build the kernel rootfs is not mounted, so allow overriding
    // the data file via the LIBJS_UCA_FILE environment variable. This is the
    // mechanism used by surf-host and the test262 runner.
    if let Ok(p) = std::env::var("LIBJS_UCA_FILE") {
        if let Some(v) = anyos_std::fs::read_to_vec(&p) {
            return Some(v);
        }
    }
    anyos_std::fs::read_to_vec(path)
}

/// One row of the loaded full-BMP table: codepoint + up to two primaries.
#[derive(Clone, Copy)]
struct Entry {
    cp: u32,
    p1: u16,
    p2: u16,
}

/// Loader state. `None` = not yet attempted, `Some(vec)` = loaded (possibly
/// empty if the file was missing or malformed). The vector is sorted by
/// codepoint so we can binary-search.
struct UcaTable {
    entries: Vec<Entry>,
    loaded: bool,
}

// Single-threaded cache (libjs is currently single-threaded). UnsafeCell is
// used directly because libjs is `no_std` and there is no global Mutex
// available; all VMs run on one thread per process.
struct TableCell(UnsafeCell<UcaTable>);
unsafe impl Sync for TableCell {}

static TABLE: TableCell = TableCell(UnsafeCell::new(UcaTable {
    entries: Vec::new(),
    loaded: false,
}));

/// Returns a reference to the cached table, performing one-shot file load
/// on first access.
fn get_table() -> &'static UcaTable {
    // SAFETY: single-threaded access; the table is initialised exactly once
    // and only ever extended (never freed). After the initial load the
    // entries vector is treated as immutable.
    let t = unsafe { &mut *TABLE.0.get() };
    if !t.loaded {
        t.loaded = true;
        if let Some(parsed) = try_load_file() {
            t.entries = parsed;
        }
    }
    unsafe { &*TABLE.0.get() }
}

fn try_load_file() -> Option<Vec<Entry>> {
    // anyos_std::fs::read_to_vec returns Result on the kernel target and
    // Option in the host build — normalise via a tiny helper.
    let bytes = read_file(UCA_FILE_PATH)?;
    if bytes.len() < 16 || &bytes[0..4] != MAGIC {
        return None;
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != 1 {
        return None;
    }
    let count = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let body = &bytes[16..];
    if body.len() < count * 8 {
        return None;
    }
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let off = i * 8;
        let cp = u32::from_le_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]);
        let p1 = u16::from_le_bytes([body[off + 4], body[off + 5]]);
        let p2 = u16::from_le_bytes([body[off + 6], body[off + 7]]);
        entries.push(Entry { cp, p1, p2 });
    }
    Some(entries)
}

/// Look up the primary weight pair for a codepoint. The returned tuple is
/// `(primary1, primary2)` — `primary1 == 0` means ignorable; `primary2 != 0`
/// indicates a two-element expansion.
fn lookup_primary(cp: u32) -> (u16, u16) {
    // Fast path: embedded Latin range, always available.
    if cp < 0x180 {
        let row = LATIN_PRIMARY_WEIGHTS[cp as usize];
        return (row[0], row[1]);
    }
    // Runtime-loaded full BMP table.
    let table = get_table();
    if !table.entries.is_empty() {
        if let Ok(idx) = table.entries.binary_search_by_key(&cp, |e| e.cp) {
            let e = table.entries[idx];
            return (e.p1, e.p2);
        }
    }
    // Last-resort fallback: use the codepoint itself, biased into the
    // u16 range. This keeps unknown characters sorting consistently and
    // strictly after every entry from the real table (which never exceed
    // ~0x73C2 in current UCA).
    let synth = if cp <= 0xFFFF {
        0x8000_u32.saturating_add(cp).min(0xFFFF) as u16
    } else {
        0xFFFF
    };
    (synth, 0)
}

/// Compare two strings using UCA Level-1 (primary-weight) comparison.
/// Returns `-1`, `0`, or `1` like `String::cmp`.
pub fn compare_primary(a: &str, b: &str) -> core::cmp::Ordering {
    let mut ai = a.chars();
    let mut bi = b.chars();
    let mut a_extra: Option<u16> = None;
    let mut b_extra: Option<u16> = None;
    loop {
        let aw = if let Some(p) = a_extra.take() {
            Some(p)
        } else {
            next_weight(&mut ai, &mut a_extra)
        };
        let bw = if let Some(p) = b_extra.take() {
            Some(p)
        } else {
            next_weight(&mut bi, &mut b_extra)
        };
        match (aw, bw) {
            (None, None) => return core::cmp::Ordering::Equal,
            (None, Some(_)) => return core::cmp::Ordering::Less,
            (Some(_), None) => return core::cmp::Ordering::Greater,
            (Some(x), Some(y)) => match x.cmp(&y) {
                core::cmp::Ordering::Equal => continue,
                ord => return ord,
            },
        }
    }
}

/// Pull the next non-ignorable primary weight from `iter`. If a codepoint
/// expands into two weights, the second is stashed into `extra` and
/// returned by the next call.
fn next_weight(
    iter: &mut core::str::Chars<'_>,
    extra: &mut Option<u16>,
) -> Option<u16> {
    for ch in iter.by_ref() {
        let (p1, p2) = lookup_primary(ch as u32);
        if p1 == 0 {
            continue;
        }
        if p2 != 0 {
            *extra = Some(p2);
        }
        return Some(p1);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cmp::Ordering;

    #[test]
    fn latin_basics() {
        assert_eq!(compare_primary("a", "b"), Ordering::Less);
        assert_eq!(compare_primary("a", "A"), Ordering::Equal); // L1 ignores case
        assert_eq!(compare_primary("apfel", "äpfel"), Ordering::Equal);
        assert_eq!(compare_primary("apfel", "banane"), Ordering::Less);
    }

    #[test]
    fn sharp_s_expands_to_ss() {
        assert_eq!(compare_primary("straße", "strasse"), Ordering::Equal);
        assert_eq!(compare_primary("straße", "strasze"), Ordering::Less);
    }
}
