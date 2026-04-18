//! searchd — Search Daemon for anyOS.
//!
//! Periodically indexes files in standard directories and provides a
//! search API via named pipe IPC. Uses libdb for persistent storage.
//!
//! Configuration: `/System/etc/searchd.conf`
//! Database: `/System/sysdb/search.db`
//! IPC pipe: `"searchd"`
//!
//! Search API commands (via pipe):
//!   SEARCH {query}    — Freetext search across filenames and content
//!   FIND {filename}   — Search by filename (substring match)
//!   KIND {kind}       — List files of a specific kind
//!   RECENT {count}    — Most recently indexed files
//!   STATS             — Index statistics
//!   REINDEX           — Trigger a full re-index

#![no_std]
#![no_main]

mod config;
mod schema;
mod indexer;
mod ipc;

anyos_std::entry!(main);

// ── Configuration ────────────────────────────────────────────────────────────

const DB_PATH: &str = "/System/sysdb/search.db";
const DB_DIR: &str = "/System/sysdb";
const PIPE_NAME: &str = "searchd";

/// Incremental re-index interval (5 minutes).
const REINDEX_INTERVAL_MS: u32 = 300_000;

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    anyos_std::println!("searchd: starting Search Daemon");

    config::register_manifest();

    // Load configuration
    let cfg = config::Config::load();

    // Initialize libdb
    if !libdb_client::init() {
        anyos_std::println!("searchd: *** FATAL: failed to load libdb.so ***");
        anyos_std::println!("searchd: *** SEARCH DAEMON IS SHUTTING DOWN ***");
        return;
    }

    // Ensure database directory exists
    anyos_std::fs::mkdir(DB_DIR);

    // Pre-create database file if it doesn't exist
    {
        let probe = anyos_std::fs::open(DB_PATH, 0);
        if probe == u32::MAX {
            let fd = anyos_std::fs::open(
                DB_PATH,
                anyos_std::fs::O_WRITE | anyos_std::fs::O_CREATE | anyos_std::fs::O_TRUNC,
            );
            if fd == u32::MAX {
                anyos_std::println!("searchd: *** FATAL: failed to create database at {} ***", DB_PATH);
                anyos_std::println!("searchd: *** SEARCH DAEMON IS SHUTTING DOWN ***");
                return;
            }
            anyos_std::fs::close(fd);
            anyos_std::println!("searchd: created database {}", DB_PATH);
        } else {
            anyos_std::fs::close(probe);
        }
    }

    // Open database
    let db = match libdb_client::Database::open(DB_PATH) {
        Some(db) => db,
        None => {
            anyos_std::println!("searchd: *** FATAL: failed to open database at {} ***", DB_PATH);
            anyos_std::println!("searchd: *** SEARCH DAEMON IS SHUTTING DOWN ***");
            return;
        }
    };

    // Initialize schema
    schema::init_tables(&db);

    // Create IPC pipe
    let pipe_id = anyos_std::ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 {
        anyos_std::println!("searchd: *** FATAL: failed to create '{}' pipe ***", PIPE_NAME);
        anyos_std::println!("searchd: *** SEARCH DAEMON IS SHUTTING DOWN ***");
        return;
    }

    anyos_std::println!("searchd: ready (pipe='{}', db='{}')", PIPE_NAME, DB_PATH);

    // Check if we already have an index from a previous run
    let has_index = indexer::has_existing_index(&db);
    if has_index {
        anyos_std::println!("searchd: existing index found, skipping initial full index");
    } else {
        anyos_std::println!("searchd: no existing index, waiting {}ms before initial index", cfg.idle_timeout_ms);
    }

    let boot_time = anyos_std::sys::uptime_ms();
    let mut initial_index_done = has_index;
    let mut last_reindex = if has_index { boot_time } else { 0u32 };
    let mut reindex_flag = false;
    let mut last_status_print = boot_time;

    let mut pipe_buf = [0u8; 4096];

    // ── Main loop ────────────────────────────────────────────────────────
    loop {
        let mut active = false;

        // Handle search requests
        if ipc::handle_requests(&db, pipe_id, &mut pipe_buf, &mut reindex_flag) {
            active = true;
        }

        let now = anyos_std::sys::uptime_ms();

        // Periodic status output (every 30 seconds)
        if now.wrapping_sub(last_status_print) >= 30_000 {
            if !initial_index_done {
                let elapsed = now.wrapping_sub(boot_time);
                if elapsed < cfg.idle_timeout_ms {
                    let remaining = (cfg.idle_timeout_ms - elapsed) / 1000;
                    anyos_std::println!("searchd: initial index in {}s", remaining);
                }
            } else {
                let elapsed = now.wrapping_sub(last_reindex);
                if elapsed < REINDEX_INTERVAL_MS {
                    let remaining = (REINDEX_INTERVAL_MS - elapsed) / 1000;
                    anyos_std::println!("searchd: next re-index in {}s", remaining);
                }
            }
            last_status_print = now;
        }

        // Initial index after idle timeout
        if !initial_index_done && now.wrapping_sub(boot_time) >= cfg.idle_timeout_ms {
            anyos_std::println!("searchd: starting initial index");
            indexer::index_all(&db, &cfg);
            let _ = db.flush();
            initial_index_done = true;
            last_reindex = now;
            anyos_std::println!("searchd: initial index complete (flushed to disk)");
        }

        // Manual reindex request
        if reindex_flag {
            anyos_std::println!("searchd: reindex requested");
            indexer::index_all(&db, &cfg);
            let _ = db.flush();
            last_reindex = now;
            reindex_flag = false;
            anyos_std::println!("searchd: reindex complete (flushed to disk)");
        }

        // Periodic incremental re-index
        if initial_index_done && now.wrapping_sub(last_reindex) >= REINDEX_INTERVAL_MS {
            anyos_std::println!("searchd: incremental re-index starting");
            indexer::index_incremental(&db, REINDEX_INTERVAL_MS, &cfg);
            let _ = db.flush();
            last_reindex = now;
            anyos_std::println!("searchd: incremental re-index complete (flushed to disk)");
        }

        if active {
            anyos_std::process::sleep(100);
        } else {
            anyos_std::process::sleep(1000);
        }
    }
}
