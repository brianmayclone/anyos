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

const LARGE_VALUE_BUF: usize = 2 * 1024 * 1024;

dynlink::dll_exports! {
    lib_path: "/Libraries/libdb.so",
    lib_struct: LibDb,
    symbols: {
        libdb_open(path: *const u8, len: u32) -> u32,
        libdb_close(handle: u32) -> (),
        libdb_error(handle: u32, buf: *mut u8, buf_len: u32) -> u32,
        libdb_exec(handle: u32, sql: *const u8, sql_len: u32) -> u32,
        libdb_query(handle: u32, sql: *const u8, sql_len: u32) -> u32,
        libdb_result_row_count(id: u32) -> u32,
        libdb_result_col_count(id: u32) -> u32,
        libdb_result_col_name(id: u32, col: u32, buf: *mut u8, buf_len: u32) -> u32,
        libdb_result_get_int(id: u32, row: u32, col: u32) -> u32,
        libdb_result_get_int_hi(id: u32, row: u32, col: u32) -> u32,
        libdb_result_get_text(id: u32, row: u32, col: u32, buf: *mut u8, buf_len: u32) -> u32,
        libdb_result_get_blob(id: u32, row: u32, col: u32, buf: *mut u8, buf_len: u32) -> u32,
        libdb_result_is_null(id: u32, row: u32, col: u32) -> u32,
        libdb_result_free(id: u32) -> (),
        libdb_flush(handle: u32) -> u32,
    }
}

// ── Database ─────────────────────────────────────────────────────────────────

/// An open database handle.
pub struct Database {
    handle: u32,
}

impl Database {
    /// Open (or create) a database file.
    pub fn open(path: &str) -> Option<Database> {
        let h = (lib().libdb_open)(path.as_ptr(), path.len() as u32);
        if h == 0 {
            None
        } else {
            Some(Database { handle: h })
        }
    }

    /// Open an ephemeral in-memory database with no backing file.
    pub fn open_in_memory() -> Option<Database> {
        Self::open(":memory:")
    }

    /// Execute a non-query SQL statement (CREATE, DROP, INSERT, UPDATE, DELETE).
    /// Returns the number of rows affected, or an error message.
    pub fn exec(&self, sql: &str) -> Result<u32, String> {
        let result = (lib().libdb_exec)(self.handle, sql.as_ptr(), sql.len() as u32);
        if result == u32::MAX {
            Err(self.last_error())
        } else {
            Ok(result)
        }
    }

    /// Execute a SELECT query. Returns a `QueryResult` for iterating rows.
    pub fn query(&self, sql: &str) -> Result<QueryResult, String> {
        let id = (lib().libdb_query)(self.handle, sql.as_ptr(), sql.len() as u32);
        if id == 0 {
            Err(self.last_error())
        } else {
            Ok(QueryResult { id })
        }
    }

    /// Get the last error message (empty string if no error).
    pub fn last_error(&self) -> String {
        let mut buf = [0u8; 256];
        let n = (lib().libdb_error)(self.handle, buf.as_mut_ptr(), 256);
        if n == 0 {
            String::from("Unknown error")
        } else {
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("?");
            String::from(s)
        }
    }

    /// Flush all dirty cached pages to disk.
    pub fn flush(&self) -> Result<(), String> {
        let result = (lib().libdb_flush)(self.handle);
        if result == u32::MAX {
            Err(self.last_error())
        } else {
            Ok(())
        }
    }

    /// Return the stored row count for a table.
    pub fn table_row_count(&self, table: &str) -> Option<u32> {
        let table_row_count = libdb_table_row_count()?;
        let result = table_row_count(self.handle, table.as_ptr(), table.len() as u32);
        if result == u32::MAX {
            None
        } else {
            Some(result)
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
            (lib().libdb_close)(self.handle);
        }
    }
}

#[cfg(not(feature = "host"))]
fn libdb_table_row_count() -> Option<extern "C" fn(u32, *const u8, u32) -> u32> {
    let ptr = dynlink::dl_sym(&lib()._handle, "libdb_table_row_count")?;
    Some(unsafe {
        core::mem::transmute_copy::<*const (), extern "C" fn(u32, *const u8, u32) -> u32>(&ptr)
    })
}

#[cfg(feature = "host")]
fn libdb_table_row_count() -> Option<extern "C" fn(u32, *const u8, u32) -> u32> {
    None
}

// ── QueryResult ──────────────────────────────────────────────────────────────

/// A query result set returned by SELECT.
pub struct QueryResult {
    id: u32,
}

impl QueryResult {
    /// Number of rows in the result.
    pub fn row_count(&self) -> u32 {
        (lib().libdb_result_row_count)(self.id)
    }

    /// Number of columns in the result.
    pub fn col_count(&self) -> u32 {
        (lib().libdb_result_col_count)(self.id)
    }

    /// Get a column name by index.
    pub fn col_name(&self, col: u32) -> String {
        let mut buf = [0u8; 64];
        let n = (lib().libdb_result_col_name)(self.id, col, buf.as_mut_ptr(), 64);
        let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("?");
        String::from(s)
    }

    /// Get an integer value from a cell. Returns None if null.
    pub fn get_int(&self, row: u32, col: u32) -> Option<i64> {
        if (lib().libdb_result_is_null)(self.id, row, col) == 1 {
            return None;
        }
        let lo = (lib().libdb_result_get_int)(self.id, row, col) as u64;
        let hi = (lib().libdb_result_get_int_hi)(self.id, row, col) as u64;
        Some(((hi << 32) | lo) as i64)
    }

    /// Get a text value from a cell. Returns None if null.
    pub fn get_text(&self, row: u32, col: u32) -> Option<String> {
        if (lib().libdb_result_is_null)(self.id, row, col) == 1 {
            return None;
        }
        let mut buf = alloc::vec![0u8; LARGE_VALUE_BUF];
        let n = (lib().libdb_result_get_text)(
            self.id,
            row,
            col,
            buf.as_mut_ptr(),
            LARGE_VALUE_BUF as u32,
        );
        if n == 0 {
            // Could be empty string or not text
            Some(String::new())
        } else {
            let s = core::str::from_utf8(&buf[..n as usize]).unwrap_or("");
            Some(String::from(s))
        }
    }

    /// Get a blob value from a cell. Returns None if null.
    pub fn get_blob(&self, row: u32, col: u32) -> Option<Vec<u8>> {
        if (lib().libdb_result_is_null)(self.id, row, col) == 1 {
            return None;
        }
        let mut buf = alloc::vec![0u8; LARGE_VALUE_BUF];
        let n = (lib().libdb_result_get_blob)(
            self.id,
            row,
            col,
            buf.as_mut_ptr(),
            LARGE_VALUE_BUF as u32,
        );
        Some(buf[..n as usize].to_vec())
    }

    /// Check if a cell is NULL.
    pub fn is_null(&self, row: u32, col: u32) -> bool {
        (lib().libdb_result_is_null)(self.id, row, col) == 1
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
            (lib().libdb_result_free)(self.id);
        }
    }
}
