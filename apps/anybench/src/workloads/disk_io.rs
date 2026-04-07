// Copyright (c) 2024-2026 Mike Strathmann
// SPDX-License-Identifier: MIT
//! Disk I/O Benchmarks — measures filesystem throughput.
//!
//! Four tests:
//!   1. Sequential Read  — read a large file start-to-end (bytes/s)
//!   2. Sequential Write — write a large file start-to-end (bytes/s)
//!   3. Random Read      — small reads at random offsets (ops/s)
//!   4. File Create      — create and delete many small files (ops/s)

use alloc::vec;
use alloc::format;
use super::DISK_TEST_MS;

const TEST_DIR: &str = "/tmp/anybench_io";
const LARGE_FILE: &str = "/tmp/anybench_io/seq_test.bin";
const CHUNK_SIZE: usize = 4096;

/// Ensure /tmp and test directory exist.
fn setup_dir() {
    anyos_std::fs::mkdir("/tmp");
    anyos_std::fs::mkdir(TEST_DIR);
}

/// Create a test file of known size using write_bytes (reliable, no FD needed).
fn ensure_test_file() {
    // Check if file exists by trying to open it
    let fd = anyos_std::fs::open(LARGE_FILE, 0);
    if fd != 0 && fd != u32::MAX {
        // File exists — check size by reading
        let mut probe = [0u8; 1];
        anyos_std::fs::lseek(fd, 0, anyos_std::fs::SEEK_END);
        let size = anyos_std::fs::lseek(fd, 0, anyos_std::fs::SEEK_SET);
        anyos_std::fs::close(fd);
        // If we got a valid file, assume it's big enough
        return;
    }
    // Create 1 MiB test file via write_bytes (single reliable call)
    setup_dir();
    let data = vec![0xAAu8; 256 * 1024]; // 256 KiB chunk
    // Write 4 chunks = 1 MiB
    let _ = anyos_std::fs::write_bytes(LARGE_FILE, &data);
    // Append 3 more chunks by opening in append mode
    for _ in 0..3 {
        let fd = anyos_std::fs::open(LARGE_FILE, 0x03); // O_WRITE | O_APPEND
        if fd != 0 && fd != u32::MAX {
            anyos_std::fs::write(fd, &data);
            anyos_std::fs::close(fd);
        }
    }
}

/// Sequential Write: write a file in 4 KiB chunks, count iterations.
pub fn bench_seq_write() -> u64 {
    setup_dir();
    let buf = vec![0x55u8; CHUNK_SIZE];
    let mut total_bytes: u64 = 0;
    let start = anyos_std::sys::uptime_ms();

    let fd = anyos_std::fs::open(LARGE_FILE, 0x0D); // O_WRITE | O_CREATE | O_TRUNC
    if fd == 0 || fd == u32::MAX {
        return 0;
    }

    while anyos_std::sys::uptime_ms().wrapping_sub(start) < DISK_TEST_MS {
        // Count bytes by chunk size, not return value (kernel may return 0 on success)
        anyos_std::fs::write(fd, &buf);
        total_bytes += CHUNK_SIZE as u64;
    }
    anyos_std::fs::close(fd);
    total_bytes
}

/// Sequential Read: read the test file in 4 KiB chunks.
pub fn bench_seq_read() -> u64 {
    ensure_test_file();

    let mut buf = vec![0u8; CHUNK_SIZE];
    let mut total_bytes: u64 = 0;
    let start = anyos_std::sys::uptime_ms();

    while anyos_std::sys::uptime_ms().wrapping_sub(start) < DISK_TEST_MS {
        let fd = anyos_std::fs::open(LARGE_FILE, 0); // O_RDONLY
        if fd == 0 || fd == u32::MAX { break; }
        loop {
            let n = anyos_std::fs::read(fd, &mut buf);
            if n == 0 || n == u32::MAX { break; }
            total_bytes += n as u64;
        }
        anyos_std::fs::close(fd);
    }
    total_bytes
}

/// Random Read: seek to random offsets, read 512 bytes.
pub fn bench_random_read() -> u64 {
    ensure_test_file();

    let mut buf = vec![0u8; 512];
    let mut total_ops: u64 = 0;
    let mut rng_state: u32 = 0xDEAD_BEEF;
    let start = anyos_std::sys::uptime_ms();

    while anyos_std::sys::uptime_ms().wrapping_sub(start) < DISK_TEST_MS {
        let fd = anyos_std::fs::open(LARGE_FILE, 0);
        if fd == 0 || fd == u32::MAX { break; }

        // Do 100 random reads per open (amortize open/close overhead)
        for _ in 0..100 {
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 17;
            rng_state ^= rng_state << 5;
            let offset = (rng_state % (256 * 1024 - 512)) as i32;
            anyos_std::fs::lseek(fd, offset, anyos_std::fs::SEEK_SET);
            let n = anyos_std::fs::read(fd, &mut buf);
            if n > 0 && n != u32::MAX {
                total_ops += 1;
            }
        }
        anyos_std::fs::close(fd);
    }
    total_ops
}

/// File Create/Delete: create small files, then delete them.
pub fn bench_file_create() -> u64 {
    setup_dir();
    let data = [0x42u8; 64];
    let mut total_ops: u64 = 0;
    let start = anyos_std::sys::uptime_ms();

    while anyos_std::sys::uptime_ms().wrapping_sub(start) < DISK_TEST_MS {
        // Create batch of 50 files
        for i in 0..50u32 {
            let idx = (total_ops as u32 + i) % 5000;
            let path = format!("/tmp/anybench_io/f{}.tmp", idx);
            let fd = anyos_std::fs::open(&path, 0x0D);
            if fd != 0 && fd != u32::MAX {
                anyos_std::fs::write(fd, &data);
                anyos_std::fs::close(fd);
            }
        }
        // Delete them
        for i in 0..50u32 {
            let idx = (total_ops as u32 + i) % 5000;
            let path = format!("/tmp/anybench_io/f{}.tmp", idx);
            anyos_std::fs::unlink(&path);
        }
        total_ops += 50;
    }
    // Cleanup
    anyos_std::fs::unlink(LARGE_FILE);
    for i in 0..5000u32 {
        let path = format!("/tmp/anybench_io/f{}.tmp", i);
        anyos_std::fs::unlink(&path);
    }
    total_ops
}
