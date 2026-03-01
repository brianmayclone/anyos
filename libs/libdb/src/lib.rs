//! libdb — File-based SQL database library for anyOS.
//!
//! Provides a simple page-based database engine with SQL query support.
//! Built as a `.so` shared library loaded via `dl_open`/`dl_sym`.
//!
//! # Architecture
//! - Single file per database, page-based layout (4096-byte pages)
//! - Table directory in page 0, data pages in linked chains
//! - SQL subset: CREATE/DROP TABLE, INSERT, SELECT, UPDATE, DELETE
//! - 13 C ABI exports for use via dynlink
//!
//! # Export Convention
//! All public functions are `extern "C"` with `#[no_mangle]` for use via `dl_sym()`.

#![no_std]
#![no_main]

extern crate alloc;

mod types;
mod parser;
mod schema;
mod engine;
mod executor;
pub mod syscall;

use alloc::vec::Vec;
use crate::types::*;
use crate::engine::Database;

// ── Allocator ────────────────────────────────────────────────────────────────

libheap::dll_allocator!(crate::syscall::sbrk, crate::syscall::mmap, crate::syscall::munmap);

// ── Panic handler ────────────────────────────────────────────────────────────

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::exit(1);
}

// ── Global state ─────────────────────────────────────────────────────────────

/// Maximum concurrent open databases.
const MAX_HANDLES: usize = 8;

/// Maximum concurrent result sets.
const MAX_RESULTS: usize = 16;

struct GlobalState {
    handles: Vec<Option<Database>>,
    results: Vec<Option<ResultSet>>,
}

static mut STATE: Option<GlobalState> = None;

fn state() -> &'static mut GlobalState {
    unsafe {
        if STATE.is_none() {
            STATE = Some(GlobalState {
                handles: Vec::new(),
                results: Vec::new(),
            });
        }
        STATE.as_mut().unwrap()
    }
}

/// Allocate a handle slot, returns handle (1-based) or 0 on failure.
fn alloc_handle(db: Database) -> u32 {
    let s = state();
    // Find an empty slot
    for (i, slot) in s.handles.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(db);
            return (i + 1) as u32;
        }
    }
    // No empty slot — push if under limit
    if s.handles.len() < MAX_HANDLES {
        s.handles.push(Some(db));
        return s.handles.len() as u32;
    }
    0
}

/// Get a mutable reference to a database by handle.
fn get_db(handle: u32) -> Option<&'static mut Database> {
    let s = state();
    let idx = handle as usize;
    if idx == 0 || idx > s.handles.len() { return None; }
    s.handles[idx - 1].as_mut()
}

/// Allocate a result slot, returns result_id (1-based) or 0 on failure.
fn alloc_result(rs: ResultSet) -> u32 {
    let s = state();
    for (i, slot) in s.results.iter_mut().enumerate() {
        if slot.is_none() {
            *slot = Some(rs);
            return (i + 1) as u32;
        }
    }
    if s.results.len() < MAX_RESULTS {
        s.results.push(Some(rs));
        return s.results.len() as u32;
    }
    0
}

/// Get a reference to a result set by id.
fn get_result(result_id: u32) -> Option<&'static ResultSet> {
    let s = state();
    let idx = result_id as usize;
    if idx == 0 || idx > s.results.len() { return None; }
    s.results[idx - 1].as_ref()
}

// ══════════════════════════════════════════════════════════════════════════════
//  Exported C API
// ══════════════════════════════════════════════════════════════════════════════

/// Open (or create) a database file. Returns handle (1+), or 0 on error.
#[no_mangle]
pub extern "C" fn libdb_open(path_ptr: *const u8, path_len: u32) -> u32 {
    let path = unsafe {
        let slice = core::slice::from_raw_parts(path_ptr, path_len as usize);
        core::str::from_utf8(slice).unwrap_or("")
    };
    if path.is_empty() { return 0; }

    match Database::open(path) {
        Ok(db) => alloc_handle(db),
        Err(e) => {
            // Store error for later retrieval
            let handle = alloc_handle(match Database::open("/dev/null") {
                Ok(db) => db,
                Err(_) => return 0,
            });
            if handle != 0 {
                if let Some(db) = get_db(handle) {
                    db.last_error = e.message();
                }
            }
            0
        }
    }
}

/// Close a database handle.
#[no_mangle]
pub extern "C" fn libdb_close(handle: u32) {
    let s = state();
    let idx = handle as usize;
    if idx > 0 && idx <= s.handles.len() {
        if let Some(mut db) = s.handles[idx - 1].take() {
            db.close();
        }
    }
}

/// Get the last error message for a handle.
/// Returns number of bytes written to buf, or 0 if no error.
#[no_mangle]
pub extern "C" fn libdb_error(handle: u32, buf_ptr: *mut u8, buf_len: u32) -> u32 {
    if let Some(db) = get_db(handle) {
        if db.last_error.is_empty() { return 0; }
        let bytes = db.last_error.as_bytes();
        let copy_len = bytes.len().min(buf_len as usize);
        unsafe {
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf_ptr, copy_len);
        }
        copy_len as u32
    } else {
        0
    }
}

/// Execute a non-query SQL statement (CREATE, DROP, INSERT, UPDATE, DELETE).
/// Returns rows affected, or u32::MAX on error.
#[no_mangle]
pub extern "C" fn libdb_exec(handle: u32, sql_ptr: *const u8, sql_len: u32) -> u32 {
    let sql = unsafe {
        let slice = core::slice::from_raw_parts(sql_ptr, sql_len as usize);
        core::str::from_utf8(slice).unwrap_or("")
    };

    let db = match get_db(handle) {
        Some(db) => db,
        None => return u32::MAX,
    };

    db.last_error.clear();

    let stmt = match parser::parse_sql(sql) {
        Ok(s) => s,
        Err(e) => {
            db.last_error = e.message();
            return u32::MAX;
        }
    };

    match executor::exec(db, stmt) {
        Ok(count) => count,
        Err(e) => {
            db.last_error = e.message();
            u32::MAX
        }
    }
}

/// Execute a SELECT query. Returns result_id (1+), or 0 on error.
#[no_mangle]
pub extern "C" fn libdb_query(handle: u32, sql_ptr: *const u8, sql_len: u32) -> u32 {
    let sql = unsafe {
        let slice = core::slice::from_raw_parts(sql_ptr, sql_len as usize);
        core::str::from_utf8(slice).unwrap_or("")
    };

    let db = match get_db(handle) {
        Some(db) => db,
        None => return 0,
    };

    db.last_error.clear();

    let stmt = match parser::parse_sql(sql) {
        Ok(s) => s,
        Err(e) => {
            db.last_error = e.message();
            return 0;
        }
    };

    match executor::query(db, stmt) {
        Ok(rs) => alloc_result(rs),
        Err(e) => {
            db.last_error = e.message();
            0
        }
    }
}

/// Get row count of a result set.
#[no_mangle]
pub extern "C" fn libdb_result_row_count(result_id: u32) -> u32 {
    get_result(result_id).map(|rs| rs.rows.len() as u32).unwrap_or(0)
}

/// Get column count of a result set.
#[no_mangle]
pub extern "C" fn libdb_result_col_count(result_id: u32) -> u32 {
    get_result(result_id).map(|rs| rs.col_names.len() as u32).unwrap_or(0)
}

/// Get column name. Returns bytes written to buf.
#[no_mangle]
pub extern "C" fn libdb_result_col_name(
    result_id: u32,
    col: u32,
    buf_ptr: *mut u8,
    buf_len: u32,
) -> u32 {
    if let Some(rs) = get_result(result_id) {
        if (col as usize) < rs.col_names.len() {
            let name = rs.col_names[col as usize].as_bytes();
            let copy_len = name.len().min(buf_len as usize);
            unsafe {
                core::ptr::copy_nonoverlapping(name.as_ptr(), buf_ptr, copy_len);
            }
            return copy_len as u32;
        }
    }
    0
}

/// Get an integer value from a result cell (low 32 bits).
#[no_mangle]
pub extern "C" fn libdb_result_get_int(result_id: u32, row: u32, col: u32) -> u32 {
    if let Some(rs) = get_result(result_id) {
        if let Some(r) = rs.rows.get(row as usize) {
            if let Some(Value::Integer(v)) = r.values.get(col as usize) {
                return *v as u32;
            }
        }
    }
    0
}

/// Get an integer value from a result cell (high 32 bits).
#[no_mangle]
pub extern "C" fn libdb_result_get_int_hi(result_id: u32, row: u32, col: u32) -> u32 {
    if let Some(rs) = get_result(result_id) {
        if let Some(r) = rs.rows.get(row as usize) {
            if let Some(Value::Integer(v)) = r.values.get(col as usize) {
                return (*v >> 32) as u32;
            }
        }
    }
    0
}

/// Get a text value from a result cell. Returns bytes written to buf.
#[no_mangle]
pub extern "C" fn libdb_result_get_text(
    result_id: u32,
    row: u32,
    col: u32,
    buf_ptr: *mut u8,
    buf_len: u32,
) -> u32 {
    if let Some(rs) = get_result(result_id) {
        if let Some(r) = rs.rows.get(row as usize) {
            if let Some(Value::Text(s)) = r.values.get(col as usize) {
                let bytes = s.as_bytes();
                let copy_len = bytes.len().min(buf_len as usize);
                unsafe {
                    core::ptr::copy_nonoverlapping(bytes.as_ptr(), buf_ptr, copy_len);
                }
                return copy_len as u32;
            }
        }
    }
    0
}

/// Check if a result cell is NULL. Returns 1 if null, 0 otherwise.
#[no_mangle]
pub extern "C" fn libdb_result_is_null(result_id: u32, row: u32, col: u32) -> u32 {
    if let Some(rs) = get_result(result_id) {
        if let Some(r) = rs.rows.get(row as usize) {
            match r.values.get(col as usize) {
                Some(Value::Null) | None => return 1,
                Some(_) => return 0,
            }
        }
    }
    1 // Result or row not found — treat as null
}

/// Free a result set.
#[no_mangle]
pub extern "C" fn libdb_result_free(result_id: u32) {
    let s = state();
    let idx = result_id as usize;
    if idx > 0 && idx <= s.results.len() {
        s.results[idx - 1] = None;
    }
}
