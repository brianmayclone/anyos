//! ARM64 storage adapter — bridges VirtIO-BLK to the filesystem layer.
//!
//! Provides `read_sectors()` / `write_sectors()` functions with the same
//! signature as `drivers::storage::read_sectors()` on x86, so the FS code
//! can call these from `#[cfg(target_arch = "aarch64")]` stubs.

/// Read sectors from the VirtIO block device.
///
/// `lba`: absolute sector number (512-byte sectors).
/// `count`: number of sectors to read.
/// `buf`: output buffer (must be >= `count * 512` bytes).
pub fn read_sectors(lba: u32, count: u32, buf: &mut [u8]) -> bool {
    super::blk::read_sectors(lba as u64, count, buf)
}

/// Write sectors to the VirtIO block device.
pub fn write_sectors(lba: u32, count: u32, buf: &[u8]) -> bool {
    super::blk::write_sectors(lba as u64, count, buf)
}

/// Initialize the filesystem on ARM64 using VirtIO-BLK.
///
/// This performs the same steps as the x86 Phase 7e filesystem init:
/// 1. Set root partition LBA
/// 2. Init VFS
/// 3. Mount root filesystem
/// 4. Mount devfs
pub fn init_filesystem() {
    use crate::fs;

    if !super::blk::is_available() {
        crate::serial_println!("  [ARM64] No VirtIO block device — skipping filesystem init");
        return;
    }

    let capacity = super::blk::capacity();
    crate::serial_println!("  [ARM64] Disk: {} sectors ({} MiB)",
        capacity, capacity * 512 / 1024 / 1024);

    // Scan partition table (MBR/GPT) from sector 0
    let mut mbr_buf = [0u8; 512];
    if !read_sectors(0, 1, &mut mbr_buf) {
        crate::serial_println!("  [ARM64] Failed to read MBR — cannot mount filesystem");
        return;
    }

    // Check for MBR signature
    if mbr_buf[510] != 0x55 || mbr_buf[511] != 0xAA {
        crate::serial_println!("  [ARM64] No MBR signature found — trying raw FAT at LBA 0");
        // No partition table — try mounting entire disk as FAT
        fs::vfs::set_root_partition_lba(0);
        fs::vfs::init();
        fs::vfs::mount("/", fs::vfs::FsType::Fat, 0);
        fs::vfs::mount_devfs();
        return;
    }

    // Parse MBR partition entries (4 entries at offsets 446, 462, 478, 494)
    let mut root_lba: u32 = 0;
    for i in 0..4u32 {
        let base = 446 + (i as usize) * 16;
        let part_type = mbr_buf[base + 4];
        if part_type == 0 { continue; }
        let start_lba = u32::from_le_bytes([
            mbr_buf[base + 8], mbr_buf[base + 9],
            mbr_buf[base + 10], mbr_buf[base + 11],
        ]);
        let size_sectors = u32::from_le_bytes([
            mbr_buf[base + 12], mbr_buf[base + 13],
            mbr_buf[base + 14], mbr_buf[base + 15],
        ]);
        crate::serial_println!("  [ARM64] Partition {}: type={:#04x} LBA={} size={}",
            i, part_type, start_lba, size_sectors);

        if root_lba == 0 {
            root_lba = start_lba;
        }
    }

    if root_lba == 0 {
        crate::serial_println!("  [ARM64] No partitions found — trying raw FAT at LBA 0");
        root_lba = 0;
    }

    crate::serial_println!("  [ARM64] Root partition LBA: {}", root_lba);
    fs::vfs::set_root_partition_lba(root_lba);

    // Initialize VFS
    fs::vfs::init();

    // Mount root as FAT (auto-detects exFAT/NTFS via VBR)
    fs::vfs::mount("/", fs::vfs::FsType::Fat, 0);

    if fs::vfs::has_root_fs() {
        crate::serial_println!("  [ARM64] Root filesystem mounted successfully");
    } else {
        crate::serial_println!("  [ARM64] Warning: root filesystem mount failed");
    }

    // Mount devfs
    fs::vfs::mount_devfs();
}
