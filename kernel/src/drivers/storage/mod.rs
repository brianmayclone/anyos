//! Block storage device drivers with backend dispatch.
//!
//! Routes read/write requests to the active storage backend (ATA PIO or AHCI DMA).
//! The backend is selected at boot based on hardware detection.
//!
//! All I/O is serialized via `IO_LOCK` — ATA PIO and AHCI both use a single
//! channel that cannot handle concurrent commands from multiple CPUs.

pub mod ata;
pub mod ahci;
pub mod atapi;

use core::sync::atomic::{AtomicBool, Ordering};

#[derive(Copy, Clone, PartialEq)]
enum StorageBackend {
    Ata,
    Ahci,
}

static mut BACKEND: StorageBackend = StorageBackend::Ata;

/// Spinlock for serializing all disk I/O.  Does NOT disable interrupts —
/// disk operations may take milliseconds and we must not miss timer ticks.
static IO_LOCK: AtomicBool = AtomicBool::new(false);

#[inline]
fn io_lock_acquire() {
    while IO_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

#[inline]
fn io_lock_release() {
    IO_LOCK.store(false, Ordering::Release);
}

/// Switch the active storage backend to AHCI (called after AHCI init succeeds).
pub fn set_backend_ahci() {
    unsafe { BACKEND = StorageBackend::Ahci; }
}

/// Read `count` sectors starting at `lba` into `buf`.
///
/// Dispatches to the active backend. For ATA, automatically batches into
/// 255-sector chunks. For AHCI, uses DMA with a bounce buffer.
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    io_lock_acquire();
    let result = match unsafe { BACKEND } {
        StorageBackend::Ata => {
            let mut offset = 0usize;
            let mut remaining = count;
            let mut cur_lba = lba;
            let mut ok = true;
            while remaining > 0 {
                let batch = remaining.min(255) as u8;
                if !ata::read_sectors(cur_lba, batch, &mut buf[offset..]) {
                    ok = false;
                    break;
                }
                offset += batch as usize * 512;
                cur_lba += batch as u32;
                remaining -= batch as u32;
            }
            ok
        }
        StorageBackend::Ahci => ahci::read_sectors(lba, count, buf),
    };
    io_lock_release();
    result
}

/// Write `count` sectors starting at `lba` from `buf`.
///
/// Dispatches to the active backend. For ATA, automatically batches into
/// 255-sector chunks. For AHCI, uses DMA with a bounce buffer.
pub fn write_sectors(lba: u32, count: u32, buf: &[u8]) -> bool {
    io_lock_acquire();
    let result = match unsafe { BACKEND } {
        StorageBackend::Ata => {
            let mut offset = 0usize;
            let mut remaining = count;
            let mut cur_lba = lba;
            let mut ok = true;
            while remaining > 0 {
                let batch = remaining.min(255) as u8;
                if !ata::write_sectors(cur_lba, batch, &buf[offset..]) {
                    ok = false;
                    break;
                }
                offset += batch as usize * 512;
                cur_lba += batch as u32;
                remaining -= batch as u32;
            }
            ok
        }
        StorageBackend::Ahci => ahci::write_sectors(lba, count, buf),
    };
    io_lock_release();
    result
}
