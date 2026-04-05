//! Block storage device drivers with backend dispatch.
//!
//! Routes read/write requests to the active storage backend (ATA PIO or AHCI DMA).
//! The backend is selected at boot based on hardware detection.
//!
//! All I/O is serialized via `IO_LOCK` — a yielding lock that gives up the CPU
//! time slice instead of busy-spinning when contended.  Reads are accelerated
//! by the global block cache (`fs::blockcache`).

pub mod ata;
pub mod ahci;
pub mod atapi;
pub mod blockdev;
pub mod nvme;
pub mod lsi_scsi;
pub mod sdhci;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use crate::sync::spinlock::Spinlock;

// ── Per-Device I/O Override ──────────────────────────────────────────────────
// Secondary storage drivers (USB mass storage, etc.) register their own
// read/write handlers for specific disk IDs, bypassing the global backend.

/// I/O handler override for a specific disk.
pub(crate) struct DeviceIoHandler {
    pub disk_id: u8,
    pub read_fn: fn(u8, u32, u32, &mut [u8]) -> bool,  // (disk_id, lba, count, buf)
    pub write_fn: fn(u8, u32, u32, &[u8]) -> bool,      // (disk_id, lba, count, buf)
}

pub(crate) static IO_OVERRIDES: Spinlock<Vec<DeviceIoHandler>> = Spinlock::new(Vec::new());

/// Register a per-device I/O handler (called by USB storage driver, etc.).
pub fn register_device_io(
    disk_id: u8,
    read_fn: fn(u8, u32, u32, &mut [u8]) -> bool,
    write_fn: fn(u8, u32, u32, &[u8]) -> bool,
) {
    IO_OVERRIDES.lock().push(DeviceIoHandler { disk_id, read_fn, write_fn });
}

/// Remove a per-device I/O handler (for hot-unplug).
pub fn unregister_device_io(disk_id: u8) {
    IO_OVERRIDES.lock().retain(|h| h.disk_id != disk_id);
}

/// Read sectors via a per-device I/O override (for ISO 9660 USB CDROM integration).
pub fn read_via_override(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    let overrides = IO_OVERRIDES.lock();
    if let Some(handler) = overrides.iter().find(|h| h.disk_id == disk_id) {
        return (handler.read_fn)(disk_id, lba, count, buf);
    }
    false
}

#[derive(Copy, Clone, PartialEq)]
enum StorageBackend {
    Ata,
    Ahci,
    Nvme,
    LsiScsi,
}

static mut BACKEND: StorageBackend = StorageBackend::Ata;

/// Yielding lock for serializing disk I/O.  Does NOT disable interrupts.
/// When contended, yields the CPU time slice instead of busy-spinning,
/// allowing other threads to make progress while waiting for I/O.
static IO_LOCK: AtomicBool = AtomicBool::new(false);

/// Counter for I/O operations in progress (for statistics).
static IO_OPS_TOTAL: AtomicU32 = AtomicU32::new(0);

/// Adaptive readahead state — tracks the last read position to detect
/// sequential access patterns and scale readahead accordingly.
static LAST_READ_END_LBA: AtomicU32 = AtomicU32::new(0);
/// Current readahead level (in sectors). Doubles on sequential hits,
/// resets to minimum on random access. Range: 16..512 sectors (8-256 KiB).
static READAHEAD_LEVEL: AtomicU32 = AtomicU32::new(64);
const READAHEAD_MIN: u32 = 16;   //   8 KiB
const READAHEAD_MAX: u32 = 512;  // 256 KiB

#[inline]
fn io_lock_acquire() {
    // Fast path: try once without yielding
    if IO_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_ok()
    {
        return;
    }
    // Slow path: yield between attempts (avoids burning CPU cycles)
    io_lock_acquire_slow();
}

#[cold]
fn io_lock_acquire_slow() {
    let can_yield = crate::task::scheduler::current_tid() > 0;
    loop {
        // Brief spin (8 iterations) before yielding — handles very short holds
        for _ in 0..8 {
            if IO_LOCK
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return;
            }
            core::hint::spin_loop();
        }
        // Yield CPU time slice so other threads can run (only if scheduler is active)
        if can_yield {
            crate::task::scheduler::schedule();
        }
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

/// Switch the active storage backend to NVMe (called after NVMe init succeeds).
pub fn set_backend_nvme() {
    unsafe { BACKEND = StorageBackend::Nvme; }
}

/// Switch the active storage backend to LSI Logic SCSI.
pub fn set_backend_lsi() {
    unsafe { BACKEND = StorageBackend::LsiScsi; }
}

/// Read `count` sectors starting at `lba` into `buf`.
///
/// First checks the global block cache — sectors already in RAM are served
/// directly without touching the disk.  Cache misses are read from the backend
/// and then populated into the cache for future reads.
///
/// For sequential access patterns, extra sectors are read ahead (up to 128
/// sectors / 64 KiB) to amortize disk latency.
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    read_sectors_on_disk(0, lba, count, buf)
}

/// Read `count` sectors from a specific physical disk into `buf`.
///
/// Disk 0 uses the active global backend. Other disks use registered per-device
/// overrides when available. The block cache and adaptive readahead are keyed
/// by `disk_id` so mounted secondary volumes benefit from the same fast path.
pub fn read_sectors_on_disk(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    if count == 0 { return true; }
    crate::debug_println!("  [storage] read_sectors: disk={} lba={} count={}", disk_id, lba, count);
    IO_OPS_TOTAL.fetch_add(1, Ordering::Relaxed);

    // ── Check if block cache is available ──────────────────────────────
    let cache_active = crate::fs::blockcache::is_ready();

    // ── Fast path: try to serve entirely from block cache ──────────────
    let cached = if cache_active {
        crate::fs::blockcache::cached_read(disk_id, lba, count, buf)
    } else { 0 };
    if cached == count {
        crate::debug_println!("  [storage] read_sectors: fully cached disk={} lba={} count={}", disk_id, lba, count);
        return true;
    }

    // ── Partial or full miss: read remaining sectors from disk ─────────
    let miss_lba = lba + cached;
    let miss_count = count - cached;
    let miss_offset = cached as usize * 512;

    // Adaptive readahead (only when cache is active)
    let readahead = if cache_active && miss_count <= 64 {
        let last_end = LAST_READ_END_LBA.load(Ordering::Relaxed);
        let is_sequential = miss_lba == last_end || (miss_lba > 0 && miss_lba <= last_end + 8);
        if is_sequential {
            let level = READAHEAD_LEVEL.load(Ordering::Relaxed);
            let new_level = (level * 2).min(READAHEAD_MAX);
            READAHEAD_LEVEL.store(new_level, Ordering::Relaxed);
            new_level
        } else {
            READAHEAD_LEVEL.store(READAHEAD_MIN, Ordering::Relaxed);
            READAHEAD_MIN
        }
    } else {
        0
    };

    let total_fetch = miss_count + readahead;
    let fetch_bytes = total_fetch as usize * 512;
    if cache_active {
        LAST_READ_END_LBA.store(miss_lba + total_fetch, Ordering::Relaxed);
    }

    // We need a potentially larger buffer for readahead
    let result = if readahead > 0 {
        let mut big_buf = alloc::vec![0u8; fetch_bytes];
        io_lock_acquire();
        let ok = read_sectors_raw_for_disk(disk_id, miss_lba, total_fetch, &mut big_buf);
        io_lock_release();
        if ok {
            // Copy requested portion to caller buffer
            let needed = miss_count as usize * 512;
            let copy_end = needed.min(buf.len() - miss_offset);
            buf[miss_offset..miss_offset + copy_end]
                .copy_from_slice(&big_buf[..copy_end]);
            // Populate cache with ALL fetched sectors (requested + readahead)
            crate::fs::blockcache::populate(disk_id, miss_lba, total_fetch, &big_buf);
            true
        } else {
            false
        }
    } else {
        // No readahead: read directly into caller buffer
        io_lock_acquire();
        let ok = read_sectors_raw_for_disk(disk_id, miss_lba, miss_count, &mut buf[miss_offset..]);
        io_lock_release();
        if ok && cache_active {
            // Populate cache with fetched sectors
            let fetched_bytes = miss_count as usize * 512;
            if buf.len() >= miss_offset + fetched_bytes {
                crate::fs::blockcache::populate(disk_id, miss_lba, miss_count,
                    &buf[miss_offset..miss_offset + fetched_bytes]);
            }
        }
        ok
    };

    crate::debug_println!("  [storage] read_sectors: done result={}", result);
    result
}

/// Raw read without cache — dispatches to the active backend.
fn read_sectors_raw(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    read_sectors_raw_for_disk(0, lba, count, buf)
}

fn read_sectors_raw_for_disk(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    if disk_id != 0 {
        let overrides = IO_OVERRIDES.lock();
        if let Some(handler) = overrides.iter().find(|h| h.disk_id == disk_id) {
            let f = handler.read_fn;
            drop(overrides);
            return f(disk_id, lba, count, buf);
        }
    }
    match unsafe { BACKEND } {
        StorageBackend::Ata => {
            let mut offset = 0usize;
            let mut remaining = count;
            let mut cur_lba = lba;
            while remaining > 0 {
                let batch = remaining.min(255) as u8;
                if !ata::read_sectors(cur_lba, batch, &mut buf[offset..]) {
                    return false;
                }
                offset += batch as usize * 512;
                cur_lba += batch as u32;
                remaining -= batch as u32;
            }
            true
        }
        StorageBackend::Ahci => ahci::read_sectors(lba, count, buf),
        StorageBackend::Nvme => nvme::read_sectors(lba, count, buf),
        StorageBackend::LsiScsi => lsi_scsi::read_sectors(lba, count, buf),
    }
}

/// Write `count` sectors starting at `lba` from `buf`.
///
/// Uses write-back caching: data is stored in the block cache as dirty sectors
/// and written to disk lazily during writeback. This makes writes extremely fast
/// (RAM speed) while maintaining read coherency.
///
/// For large writes (>256 sectors / 128 KiB), data goes directly to disk to
/// avoid flooding the cache and evicting useful read-cache entries.
pub fn write_sectors(lba: u32, count: u32, buf: &[u8]) -> bool {
    write_sectors_on_disk(0, lba, count, buf)
}

/// Write `count` sectors to a specific physical disk from `buf`.
pub fn write_sectors_on_disk(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    if count == 0 { return true; }
    IO_OPS_TOTAL.fetch_add(1, Ordering::Relaxed);

    let cache_active = crate::fs::blockcache::is_ready();
    let data_len = count as usize * 512;

    // Write-back path: small writes go to cache, large writes bypass
    if cache_active && count <= 256 && buf.len() >= data_len {
        crate::fs::blockcache::write_back(disk_id, lba, count, &buf[..data_len]);
        return true;
    }

    // Large write or no cache: go directly to disk
    io_lock_acquire();
    let result = write_sectors_raw_for_disk(disk_id, lba, count, buf);
    io_lock_release();
    if result && cache_active {
        // Update cache so reads see the new data
        if buf.len() >= data_len {
            crate::fs::blockcache::populate(disk_id, lba, count, &buf[..data_len]);
        } else {
            crate::fs::blockcache::invalidate(disk_id, lba, count);
        }
    }
    result
}

/// Raw write without cache — dispatches to the active backend.
fn write_sectors_raw(lba: u32, count: u32, buf: &[u8]) -> bool {
    write_sectors_raw_for_disk(0, lba, count, buf)
}

fn write_sectors_raw_for_disk(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    if disk_id != 0 {
        let overrides = IO_OVERRIDES.lock();
        if let Some(handler) = overrides.iter().find(|h| h.disk_id == disk_id) {
            let f = handler.write_fn;
            drop(overrides);
            return f(disk_id, lba, count, buf);
        }
    }
    match unsafe { BACKEND } {
        StorageBackend::Ata => {
            let mut offset = 0usize;
            let mut remaining = count;
            let mut cur_lba = lba;
            while remaining > 0 {
                let batch = remaining.min(255) as u8;
                if !ata::write_sectors(cur_lba, batch, &buf[offset..]) {
                    return false;
                }
                offset += batch as usize * 512;
                cur_lba += batch as u32;
                remaining -= batch as u32;
            }
            true
        }
        StorageBackend::Ahci => ahci::write_sectors(lba, count, buf),
        StorageBackend::Nvme => nvme::write_sectors(lba, count, buf),
        StorageBackend::LsiScsi => lsi_scsi::write_sectors(lba, count, buf),
    }
}

/// Write sectors directly to disk, bypassing the block cache.
/// Used by the cache writeback mechanism itself to avoid recursion.
pub fn write_sectors_direct(lba: u32, count: u32, buf: &[u8]) -> bool {
    write_sectors_direct_on_disk(0, lba, count, buf)
}

/// Write sectors directly to a specific disk, bypassing the block cache.
pub fn write_sectors_direct_on_disk(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    if count == 0 { return true; }
    io_lock_acquire();
    let result = write_sectors_raw_for_disk(disk_id, lba, count, buf);
    io_lock_release();
    result
}

/// Flush storage write cache to persistent media.
pub fn flush() {
    io_lock_acquire();
    match unsafe { BACKEND } {
        StorageBackend::Ahci => { ahci::flush(); }
        _ => {} // ATA/NVMe/SCSI: no explicit flush needed or not supported
    }
    io_lock_release();
}

// ── HAL integration ─────────────────────────────────────────────────────────

use alloc::boxed::Box;
use crate::drivers::hal::{Driver, DriverType, DriverError};

struct StorageHalDriver {
    name: &'static str,
}

impl Driver for StorageHalDriver {
    fn name(&self) -> &str { self.name }
    fn driver_type(&self) -> DriverType { DriverType::Block }
    fn init(&mut self) -> Result<(), DriverError> { Ok(()) }
    fn read(&self, offset: usize, buf: &mut [u8]) -> Result<usize, DriverError> {
        let lba = (offset / 512) as u32;
        let count = ((buf.len() + 511) / 512) as u32;
        if read_sectors(lba, count, buf) { Ok(count as usize * 512) } else { Err(DriverError::IoError) }
    }
    fn write(&self, offset: usize, buf: &[u8]) -> Result<usize, DriverError> {
        let lba = (offset / 512) as u32;
        let count = ((buf.len() + 511) / 512) as u32;
        if write_sectors(lba, count, buf) { Ok(count as usize * 512) } else { Err(DriverError::IoError) }
    }
    fn ioctl(&mut self, _cmd: u32, _arg: u32) -> Result<u32, DriverError> {
        Err(DriverError::NotSupported)
    }
}

/// Create a HAL Driver wrapper for the storage subsystem (called from driver probe).
pub(crate) fn create_hal_driver(name: &'static str) -> Option<Box<dyn Driver>> {
    Some(Box::new(StorageHalDriver { name }))
}

/// Probe for IDE controller (uses default ATA PIO backend).
pub fn ide_probe(_pci: &crate::drivers::pci::PciDevice) -> Option<Box<dyn Driver>> {
    create_hal_driver("IDE Controller")
}
