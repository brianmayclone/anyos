//! amid — Anywhere Management Interface daemon.
//!
//! Maintains a system information database at `/System/sysdb/ami.db`
//! with tables for hardware, CPU, memory, threads, devices, disks,
//! network, and services. Refreshes data periodically and provides
//! read-only SQL query access to other apps via named pipe IPC.

#![no_std]
#![no_main]

mod schema;
mod collect;
mod ipc;

anyos_std::entry!(main);

// ── Configuration ────────────────────────────────────────────────────────────

/// Database file path.
const DB_PATH: &str = "/System/sysdb/ami.db";

/// Database directory.
const DB_DIR: &str = "/System/sysdb";

/// Named pipe for incoming SQL queries.
const PIPE_NAME: &str = "ami";

/// Fast refresh interval (memory, CPU, threads) in milliseconds.
const FAST_INTERVAL_MS: u32 = 2000;

/// Slow refresh interval (devices, disks, network, services) in milliseconds.
const SLOW_INTERVAL_MS: u32 = 10000;

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    anyos_std::println!("amid: starting Anywhere Management Interface");

    // Initialize libdb client library
    if !libdb_client::init() {
        anyos_std::println!("amid: failed to load libdb.so");
        return;
    }

    // Ensure database directory exists
    anyos_std::fs::mkdir(DB_DIR);

    // Pre-create the database file if it doesn't exist (using stdlib's
    // proven file I/O so libdb only needs to open an existing file).
    {
        let probe = anyos_std::fs::open(DB_PATH, 0);
        if probe == u32::MAX {
            // File doesn't exist — create it via stdlib
            let fd = anyos_std::fs::open(
                DB_PATH,
                anyos_std::fs::O_WRITE | anyos_std::fs::O_CREATE | anyos_std::fs::O_TRUNC,
            );
            if fd == u32::MAX {
                anyos_std::println!("amid: failed to create database file at {}", DB_PATH);
                return;
            }
            anyos_std::fs::close(fd);
            anyos_std::println!("amid: created database file {}", DB_PATH);
        } else {
            anyos_std::fs::close(probe);
        }
    }

    // Open the database via libdb
    let db = match libdb_client::Database::open(DB_PATH) {
        Some(db) => db,
        None => {
            anyos_std::println!("amid: failed to open database at {}", DB_PATH);
            return;
        }
    };

    // Create all tables (idempotent — ignores "already exists" errors)
    schema::init_tables(&db);

    // Initial data collection — static tables first
    anyos_std::println!("amid: collecting hardware info");
    collect::collect_hw(&db);

    // CPU state for delta-based load calculation
    let mut cpu_state = collect::CpuState::new();

    // Initial population of all dynamic tables
    anyos_std::println!("amid: collecting initial data");
    collect::collect_mem(&db);
    collect::collect_cpu(&db, &mut cpu_state);
    collect::collect_threads(&db);
    collect::collect_devices(&db);
    collect::collect_disks(&db);
    collect::collect_net(&db);
    collect::collect_svc(&db);

    // Create the IPC pipe
    let pipe_id = anyos_std::ipc::pipe_create(PIPE_NAME);
    if pipe_id == 0 {
        anyos_std::println!("amid: failed to create '{}' pipe", PIPE_NAME);
        return;
    }

    anyos_std::println!("amid: ready (pipe='{}', db='{}')", PIPE_NAME, DB_PATH);

    // Pipe read buffer
    let mut pipe_buf = [0u8; 4096];

    // Timer tracking
    let mut last_fast = anyos_std::sys::uptime_ms();
    let mut last_slow = last_fast;

    // ── Main loop ────────────────────────────────────────────────────────
    loop {
        let mut active = false;

        // Handle incoming SQL queries via pipe
        if ipc::handle_requests(&db, pipe_id, &mut pipe_buf) {
            active = true;
        }

        // Check refresh timers
        let now = anyos_std::sys::uptime_ms();

        // Fast refresh: mem, cpu, threads (every 2s)
        if now.wrapping_sub(last_fast) >= FAST_INTERVAL_MS {
            collect::collect_mem(&db);
            collect::collect_cpu(&db, &mut cpu_state);
            collect::collect_threads(&db);
            last_fast = now;
        }

        // Slow refresh: devices, disks, net, svc (every 10s)
        if now.wrapping_sub(last_slow) >= SLOW_INTERVAL_MS {
            collect::collect_devices(&db);
            collect::collect_disks(&db);
            collect::collect_net(&db);
            collect::collect_svc(&db);
            last_slow = now;
        }

        // Sleep to avoid busy-waiting.
        // 200ms idle is fine — ami clients wait up to 3s for responses.
        if active {
            anyos_std::process::sleep(50);
        } else {
            anyos_std::process::sleep(200);
        }
    }
}
