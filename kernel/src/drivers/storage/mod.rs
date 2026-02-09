//! Block storage device drivers with backend dispatch.
//!
//! Routes read/write requests to the active storage backend (ATA PIO or AHCI DMA).
//! The backend is selected at boot based on hardware detection.

pub mod ata;
pub mod ahci;

#[derive(Copy, Clone, PartialEq)]
enum StorageBackend {
    Ata,
    Ahci,
}

static mut BACKEND: StorageBackend = StorageBackend::Ata;

/// Switch the active storage backend to AHCI (called after AHCI init succeeds).
pub fn set_backend_ahci() {
    unsafe { BACKEND = StorageBackend::Ahci; }
}

/// Read `count` sectors starting at `lba` into `buf`.
///
/// Dispatches to the active backend. For ATA, automatically batches into
/// 255-sector chunks. For AHCI, uses DMA with a bounce buffer.
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
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
    }
}

/// Write `count` sectors starting at `lba` from `buf`.
///
/// Dispatches to the active backend. For ATA, automatically batches into
/// 255-sector chunks. For AHCI, uses DMA with a bounce buffer.
pub fn write_sectors(lba: u32, count: u32, buf: &[u8]) -> bool {
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
    }
}
