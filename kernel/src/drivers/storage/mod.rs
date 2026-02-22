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
pub mod blockdev;
pub mod nvme;
pub mod lsi_scsi;

use core::sync::atomic::{AtomicBool, Ordering};

#[derive(Copy, Clone, PartialEq)]
enum StorageBackend {
    Ata,
    Ahci,
    Nvme,
    LsiScsi,
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
/// Dispatches to the active backend. For ATA, automatically batches into
/// 255-sector chunks. For AHCI, uses DMA with a bounce buffer.
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    crate::debug_println!("  [storage] read_sectors: lba={} count={} (acquiring io_lock)", lba, count);
    io_lock_acquire();
    crate::debug_println!("  [storage] read_sectors: io_lock acquired, dispatching");
    let result = match unsafe { BACKEND } {
        StorageBackend::Ata => {
            let mut offset = 0usize;
            let mut remaining = count;
            let mut cur_lba = lba;
            let mut ok = true;
            while remaining > 0 {
                let batch = remaining.min(255) as u8;
                crate::debug_println!("  [storage] ATA batch: lba={} count={} remaining={}", cur_lba, batch, remaining);
                if !ata::read_sectors(cur_lba, batch, &mut buf[offset..]) {
                    crate::debug_println!("  [storage] ATA batch FAILED: lba={} count={}", cur_lba, batch);
                    ok = false;
                    break;
                }
                crate::debug_println!("  [storage] ATA batch OK: lba={} count={}", cur_lba, batch);
                offset += batch as usize * 512;
                cur_lba += batch as u32;
                remaining -= batch as u32;
            }
            ok
        }
        StorageBackend::Ahci => ahci::read_sectors(lba, count, buf),
        StorageBackend::Nvme => nvme::read_sectors(lba, count, buf),
        StorageBackend::LsiScsi => lsi_scsi::read_sectors(lba, count, buf),

    };
    crate::debug_println!("  [storage] read_sectors: done result={} releasing io_lock", result);
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
        StorageBackend::Nvme => nvme::write_sectors(lba, count, buf),
        StorageBackend::LsiScsi => lsi_scsi::write_sectors(lba, count, buf),

    };
    io_lock_release();
    result
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
