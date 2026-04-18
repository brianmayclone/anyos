//! Database schema for the search index.
//!
//! Tables:
//! - `files`: Every indexed file/directory with metadata and classification.
//! - `content`: Full-text content snippets for text/document files.
//! - `state`: Crawler bookkeeping (last scan timestamp per directory).

use libdb_client::Database;

/// SQL statements to create each table.
const CREATE_STATEMENTS: &[&str] = &[
    // files — one row per filesystem entry
    //   path:     absolute path (unique key)
    //   name:     filename component (for fast name searches)
    //   kind:     classification (see Kind constants below)
    //   size:     file size in bytes
    //   modified: last-modified timestamp (uptime_ms at index time)
    //   parent:   parent directory path
    "CREATE TABLE files (path TEXT, name TEXT, kind TEXT, size INTEGER, modified INTEGER, parent TEXT)",
    // content — text content associated with a file (may have multiple rows per file
    // for chunked content, but libdb 255-byte TEXT limit means we store snippets)
    //   path:    file path (foreign key to files.path)
    //   chunk:   chunk index (0, 1, 2, ...)
    //   body:    text content snippet (up to 250 bytes)
    "CREATE TABLE content (path TEXT, chunk INTEGER, body TEXT)",
    // state — crawler state per watched directory
    //   dir:       directory path
    //   last_scan: uptime_ms of last completed scan
    "CREATE TABLE state (dir TEXT, last_scan INTEGER)",
];

/// Initialize all database tables (idempotent).
pub fn init_tables(db: &Database) {
    for sql in CREATE_STATEMENTS {
        match db.exec(sql) {
            Ok(_) => {}
            Err(e) => {
                // "already exists" is expected on restart — only log real errors
                if !e.contains("exist") {
                    anyos_std::println!("searchd: CREATE TABLE error: {}", e);
                }
            }
        }
    }
}

/// Clear all indexed data (used before full re-index).
pub fn clear_index(db: &Database) {
    let _ = db.exec("DELETE FROM content");
    let _ = db.exec("DELETE FROM files");
}

// ── Kind constants ──────────────────────────────────────────────────────────

/// File kind classification strings stored in the `kind` column.
pub const KIND_DIR: &str = "directory";
pub const KIND_TEXT: &str = "text";
pub const KIND_DOCUMENT: &str = "document";
pub const KIND_IMAGE: &str = "image";
pub const KIND_BINARY: &str = "binary";
pub const KIND_CONFIG: &str = "config";
pub const KIND_SCRIPT: &str = "script";
pub const KIND_LIBRARY: &str = "library";
pub const KIND_LINK: &str = "link";
pub const KIND_URL: &str = "url";
pub const KIND_FONT: &str = "font";
pub const KIND_ARCHIVE: &str = "archive";
pub const KIND_AUDIO: &str = "audio";
pub const KIND_VIDEO: &str = "video";
pub const KIND_DATABASE: &str = "database";
pub const KIND_EXECUTABLE: &str = "executable";
