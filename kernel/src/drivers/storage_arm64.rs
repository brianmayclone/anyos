//! ARM64 storage facade.
//!
//! Reuses the generic block-device registry while routing I/O into the ARM64
//! VirtIO-BLK transport.

use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

#[path = "storage/blockdev.rs"]
pub mod blockdev;

pub(crate) struct DeviceIoHandler {
    pub disk_id: u8,
    pub read_fn: fn(u8, u32, u32, &mut [u8]) -> bool,
    pub write_fn: fn(u8, u32, u32, &[u8]) -> bool,
}

pub(crate) static IO_OVERRIDES: Spinlock<Vec<DeviceIoHandler>> = Spinlock::new(Vec::new());

const MAX_DISKS: usize = 16;
static DISK_SECTORS: [AtomicU64; MAX_DISKS] = [const { AtomicU64::new(0) }; MAX_DISKS];

pub fn register_device_io(
    disk_id: u8,
    read_fn: fn(u8, u32, u32, &mut [u8]) -> bool,
    write_fn: fn(u8, u32, u32, &[u8]) -> bool,
) {
    IO_OVERRIDES.lock().push(DeviceIoHandler {
        disk_id,
        read_fn,
        write_fn,
    });
}

pub fn unregister_device_io(disk_id: u8) {
    IO_OVERRIDES.lock().retain(|h| h.disk_id != disk_id);
}

pub fn read_via_override(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    let overrides = IO_OVERRIDES.lock();
    if let Some(handler) = overrides.iter().find(|h| h.disk_id == disk_id) {
        return (handler.read_fn)(disk_id, lba, count, buf);
    }
    false
}

pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    read_sectors_on_disk(0, lba, count, buf)
}

pub fn read_sectors_on_disk(disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
    if disk_id != 0 {
        let overrides = IO_OVERRIDES.lock();
        if let Some(handler) = overrides.iter().find(|h| h.disk_id == disk_id) {
            let f = handler.read_fn;
            drop(overrides);
            return f(disk_id, lba, count, buf);
        }
    }
    crate::drivers::arm::storage::read_sectors(lba, count, buf)
}

pub fn write_sectors(lba: u32, count: u32, buf: &[u8]) -> bool {
    write_sectors_on_disk(0, lba, count, buf)
}

pub fn write_sectors_on_disk(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    if disk_id != 0 {
        let overrides = IO_OVERRIDES.lock();
        if let Some(handler) = overrides.iter().find(|h| h.disk_id == disk_id) {
            let f = handler.write_fn;
            drop(overrides);
            return f(disk_id, lba, count, buf);
        }
    }
    crate::drivers::arm::storage::write_sectors(lba, count, buf)
}

pub fn write_sectors_direct_on_disk(disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
    write_sectors_on_disk(disk_id, lba, count, buf)
}

pub fn set_disk_sector_count(disk_id: u8, sector_count: u64) {
    if (disk_id as usize) < MAX_DISKS {
        DISK_SECTORS[disk_id as usize].store(sector_count, Ordering::Relaxed);
    }
}

pub fn disk_sector_count(disk_id: u8) -> u64 {
    if (disk_id as usize) >= MAX_DISKS {
        return 0;
    }
    DISK_SECTORS[disk_id as usize].load(Ordering::Relaxed)
}

/// VirtIO-BLK currently has no explicit flush primitive beyond request
/// completion ordering, so this is intentionally a no-op.
pub fn flush() {}
