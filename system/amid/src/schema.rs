//! Table schema definitions and initialization for the ami database.
//!
//! Defines 8 tables across 3 refresh tiers:
//! - Static: `hw` (populated once at startup)
//! - Fast (2s): `mem`, `cpu`, `threads`
//! - Slow (10s): `devices`, `disks`, `net`, `svc`

use libdb_client::Database;

/// SQL statements to create each table.
const CREATE_STATEMENTS: &[&str] = &[
    "CREATE TABLE hw (key TEXT, value TEXT)",
    "CREATE TABLE mem (key TEXT, value INTEGER)",
    "CREATE TABLE cpu (core INTEGER, load_pct INTEGER)",
    "CREATE TABLE threads (tid INTEGER, name TEXT, state INTEGER, prio INTEGER, arch INTEGER, uid INTEGER, pages INTEGER, ticks INTEGER)",
    "CREATE TABLE devices (path TEXT, driver TEXT, dtype INTEGER)",
    "CREATE TABLE disks (id INTEGER, disk_id INTEGER, part INTEGER, start_lba INTEGER, size_sect INTEGER)",
    "CREATE TABLE net (key TEXT, value TEXT)",
    "CREATE TABLE svc (name TEXT, status TEXT, tid INTEGER)",
];

/// Initialize all database tables.
///
/// Attempts CREATE TABLE for each; ignores "already exists" errors
/// so the daemon can safely restart without losing schema.
pub fn init_tables(db: &Database) {
    for sql in CREATE_STATEMENTS {
        // Ignore errors — table may already exist from a previous run.
        let _ = db.exec(sql);
    }
}
