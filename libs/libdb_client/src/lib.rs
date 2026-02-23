//! libdb_client — Safe Rust wrapper for the libdb shared library.
//!
//! Loads `libdb.so` via `dl_open`/`dl_sym` and provides ergonomic Rust types
//! (`Database`, `QueryResult`) for database operations.
//!
//! # Usage
//! ```rust
//! libdb_client::init();
//! let db = libdb_client::Database::open("/data/settings.db").unwrap();
//! db.exec("CREATE TABLE prefs (key TEXT, value TEXT)").unwrap();
//! db.exec("INSERT INTO prefs (key, value) VALUES ('theme', 'dark')").unwrap();
//! let result = db.query("SELECT * FROM prefs").unwrap();
//! // ... iterate result ...
//! ```

#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use dynlink::{dl_open, dl_sym, DlHandle};

// ── Function pointer cache ───────────────────────────────────────────────────

struct LibDb {
    _handle: DlHandle,
    // Lifecycle
    open: extern "C" fn(*const u8, u32) -> u32,
    close: extern "C" fn(u32),
    error: extern "C" fn(u32, *mut u8, u32) -> u32,
    // Execute
    exec: extern "C" fn(u32, *const u8, u32) -> u32,
    // Query
    query: extern "C" fn(u32, *const u8, u32) -> u32,
    result_row_count: extern "C" fn(u32) -> u32,
    result_col_count: extern "C" fn(u32) -> u32,
    result_col_name: extern "C" fn(u32, u32, *mut u8, u32) -> u32,
    result_get_int: extern "C" fn(u32, u32, u32) -> u32,
    result_get_int_hi: extern "C" fn(u32, u32, u32) -> u32,
    result_get_text: extern "C" fn(u32, u32, u32, *mut u8, u32) -> u32,
    result_is_null: extern "C" fn(u32, u32, u32) -> u32,
    result_free: extern "C" fn(u32),
}

static mut LIB: Option<LibDb> = None;

fn lib() -> &'static LibDb {
    unsafe { LIB.as_ref().expect("libdb not loaded — call init() first") }
}

/// Resolve a function pointer from the loaded library.
///
/// # Safety
/// The caller must ensure T has the correct function signature.
unsafe fn resolve<T: Copy>(handle: &DlHandle, name: &str) -> T {
    let ptr = dl_sym(handle, name)
        .unwrap_or_else(|| panic!("libdb: symbol not found: {}", name));
    unsafe { core::mem::transmute_copy::<*const (), T>(&ptr) }
}

// ── Initialization ───────────────────────────────────────────────────────────

/// Load libdb.so and cache all function pointers. Returns true on success.
/// Must be called once before any database operations.
pub fn init() -> bool {
    let handle = match dl_open("/Libraries/libdb.so") {
        Some(h) => h,
        None => return false,
    };

    unsafe {
        let lib = LibDb {
            open: resolve(&handle, "libdb_open"),
            close: resolve(&handle, "libdb_close"),
            error: resolve(&handle, "libdb_error"),
            exec: resolve(&handle, "libdb_exec"),
            query: resolve(&handle, "libdb_query"),
            result_row_count: resolve(&handle, "libdb_result_row_count"),
            result_col_count: resolve(&handle, "libdb_result_col_count"),
            result_col_name: resolve(&handle, "libdb_result_col_name"),
            result_get_int: resolve(&handle, "libdb_result_get_int"),
            result_get_int_hi: resolve(&handle, "libdb_result_get_int_hi"),
            result_get_text: resolve(&handle, "libdb_result_get_text"),
            result_is_null: resolve(&handle, "libdb_result_is_null"),
            result_free: resolve(&handle, "libdb_result_free"),
            _handle: handle,
        };
        LIB = Some(lib);
    }
    true
}

// ── Database ─────────────────────────────────────────────────────────────────

/// An open database handle.
pub struct Database {
    handle: u32,
}

impl Database {
    /// Open (or create) a database file.
    pub fn open(path: &str) -> Option<Database> {
        let h = (lib().open)(path.as_ptr(), path.len() as u32);
        if h == 0 { None } else { Some(Database { handle: h }) }
    }

    /// Execute a non-query SQL statement (CREATE, DROP, INSERT, UPDATE, DELETE).
    /// Returns the number of rows affected, or an error message.
    pub fn exec(&self, sql: &str) -> Result<u32, String> {
        let result = (lib().exec)(self.handle, sql.as_ptr(), sql.len() as u32);
        if result == u32::MAX {
            Err(self.last_error())
        } else {
            Ok(result)
        }
    }

    /// Execute a SELECT query. Returns a `QueryResult` for iterating rows.
    pub fn query(&self, sql: &str) -> Result<QueryResult, String> {
        let id = (lib().query)(self.handle, sql.as_ptr(), sql.len() as u32);
        if id == 0 {
            Err(self.last_error())
        } else {
            Ok(QueryResult { id })
        }
    }

    /// Get the last error message (empty string if no error).
    pub fn last_error(&self) -> String {
        let mut buf = [0u8; 256];
        let n = (lib().error)(self.handle, buf.as_mut_ptr(), 256);
        if n == 0 {
            String::from("Unknown error")
        } else {
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("?");
            String::from(s)
        }
    }

    /// Close the database explicitly.
    pub fn close(self) {
        // Drop will handle it
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        if self.handle != 0 {
            (lib().close)(self.handle);
        }
    }
}

// ── QueryResult ──────────────────────────────────────────────────────────────

/// A query result set returned by SELECT.
pub struct QueryResult {
    id: u32,
}

impl QueryResult {
    /// Number of rows in the result.
    pub fn row_count(&self) -> u32 {
        (lib().result_row_count)(self.id)
    }

    /// Number of columns in the result.
    pub fn col_count(&self) -> u32 {
        (lib().result_col_count)(self.id)
    }

    /// Get a column name by index.
    pub fn col_name(&self, col: u32) -> String {
        let mut buf = [0u8; 64];
        let n = (lib().result_col_name)(self.id, col, buf.as_mut_ptr(), 64);
        let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("?");
        String::from(s)
    }

    /// Get an integer value from a cell. Returns None if null.
    pub fn get_int(&self, row: u32, col: u32) -> Option<i64> {
        if (lib().result_is_null)(self.id, row, col) == 1 {
            return None;
        }
        let lo = (lib().result_get_int)(self.id, row, col) as u64;
        let hi = (lib().result_get_int_hi)(self.id, row, col) as u64;
        Some(((hi << 32) | lo) as i64)
    }

    /// Get a text value from a cell. Returns None if null.
    pub fn get_text(&self, row: u32, col: u32) -> Option<String> {
        if (lib().result_is_null)(self.id, row, col) == 1 {
            return None;
        }
        let mut buf = [0u8; 256];
        let n = (lib().result_get_text)(self.id, row, col, buf.as_mut_ptr(), 256);
        if n == 0 {
            // Could be empty string or not text
            Some(String::new())
        } else {
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
            Some(String::from(s))
        }
    }

    /// Check if a cell is NULL.
    pub fn is_null(&self, row: u32, col: u32) -> bool {
        (lib().result_is_null)(self.id, row, col) == 1
    }

    /// Get all column names.
    pub fn col_names(&self) -> Vec<String> {
        let cc = self.col_count();
        let mut names = Vec::with_capacity(cc as usize);
        for i in 0..cc {
            names.push(self.col_name(i));
        }
        names
    }
}

impl Drop for QueryResult {
    fn drop(&mut self) {
        if self.id != 0 {
            (lib().result_free)(self.id);
        }
    }
}
