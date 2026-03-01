//! ARM64 boot information parsing.
//!
//! On ARM64, the bootloader (U-Boot or QEMU -kernel) passes the DTB physical
//! address in X0. This module provides helpers to extract boot information
//! from the Device Tree Blob.

use core::sync::atomic::{AtomicU64, Ordering};

/// Physical address of the DTB, saved at boot entry.
static DTB_ADDR: AtomicU64 = AtomicU64::new(0);

/// Save the DTB address from X0 (called from boot.S entry).
pub fn save_dtb_addr(addr: u64) {
    DTB_ADDR.store(addr, Ordering::Relaxed);
}

/// Get the DTB physical address.
pub fn dtb_addr() -> u64 {
    DTB_ADDR.load(Ordering::Relaxed)
}

/// Minimal DTB header parsing to find memory size.
///
/// The QEMU virt machine typically provides 512 MiB or more of RAM
/// starting at 0x4000_0000 (1 GiB). Without a full FDT parser,
/// we use a reasonable default.
pub fn detect_memory() -> (u64, u64) {
    // Default for QEMU virt: 512 MiB starting at 0x4000_0000
    let base: u64 = 0x4000_0000;
    let size: u64 = 512 * 1024 * 1024; // 512 MiB

    // TODO: Parse DTB to get actual memory regions
    // For now, QEMU -m 512M gives us this range.

    crate::serial_println!("  Memory: {:#010x} - {:#010x} ({} MiB)",
        base, base + size, size / (1024 * 1024));

    (base, size)
}
