//! Superblock-Magic-Detection für CoreFS-Partitionen.
//!
//! Der Boot-Pfad probiert jede Partition mit [`detect`] — wenn `true`
//! zurückkommt, liegt an dieser Startposition ein CoreFS-Volume und
//! der Mount-Pfad konstruiert einen [`super::CoreFsDriver`] darüber.
//!
//! # On-Disk-Layout
//!
//! CoreFS legt den Primär-Superblock auf **Block 1** ab (Block 0 ist
//! reserviert). Die logische Block-Größe beträgt 4 KiB, der AnyOS-Storage-
//! Stack adressiert in 512-Byte-Sektoren — ein 4-KiB-Block entspricht 8
//! Sektoren.
//!
//! Wir lesen 8 Sektoren ab Partition-relativer LBA 8 und vergleichen die
//! ersten 8 Bytes gegen [`corefs_core::storage::ondisk::layout::ODF_MAGIC`]
//! (`"COREFSDF"` little-endian).

use corefs_core::storage::ondisk::layout::{BLOCK_SIZE, ODF_MAGIC, PRIMARY_SUPERBLOCK_BLOCK};

/// Ein AnyOS-Sektor in Bytes (siehe `drivers::storage`).
const SECTOR_SIZE: u64 = 512;

/// Wie viele Sektoren ein CoreFS-Block umfasst (4096 / 512 = 8).
const SECTORS_PER_BLOCK: u32 = (BLOCK_SIZE / SECTOR_SIZE) as u32;

/// Partition-relativer LBA-Offset des Primär-Superblocks.
const PRIMARY_SB_LBA_OFFSET: u32 = (PRIMARY_SUPERBLOCK_BLOCK as u32) * SECTORS_PER_BLOCK;

/// Prüft, ob die Partition bei `partition_lba` ein gültiges CoreFS-Volume
/// beherbergt.
///
/// Führt **einen** 4-KiB-Disk-Read aus und vergleicht das Magic-Feld.
/// Die volle Superblock-Validierung (Version, CRC, Geometrie) übernimmt
/// später [`super::CoreFsDriver::mount_read_only`] beim eigentlichen Mount.
pub fn detect(disk_id: u8, partition_lba: u32) -> bool {
    let abs_lba = partition_lba.saturating_add(PRIMARY_SB_LBA_OFFSET);
    let mut buf = [0u8; BLOCK_SIZE as usize];
    if !crate::drivers::storage::read_sectors_on_disk(disk_id, abs_lba, SECTORS_PER_BLOCK, &mut buf)
    {
        return false;
    }
    magic_matches(&buf)
}

/// Prüft, ob die ersten 8 Bytes eines Superblock-Blocks das ODF-Magic enthalten.
fn magic_matches(block: &[u8]) -> bool {
    if block.len() < 8 {
        return false;
    }
    let mut m = [0u8; 8];
    m.copy_from_slice(&block[..8]);
    u64::from_le_bytes(m) == ODF_MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odf_magic_is_corefsdf_ascii() {
        assert_eq!(ODF_MAGIC, u64::from_le_bytes(*b"COREFSDF"));
    }

    #[test]
    fn magic_matches_accepts_genuine_magic() {
        let mut block = [0u8; BLOCK_SIZE as usize];
        block[..8].copy_from_slice(&ODF_MAGIC.to_le_bytes());
        assert!(magic_matches(&block));
    }

    #[test]
    fn magic_matches_rejects_zero_block() {
        let block = [0u8; BLOCK_SIZE as usize];
        assert!(!magic_matches(&block));
    }

    #[test]
    fn magic_matches_rejects_garbage() {
        let mut block = [0u8; BLOCK_SIZE as usize];
        block[..8].copy_from_slice(b"NOTCOREF");
        assert!(!magic_matches(&block));
    }

    #[test]
    fn magic_matches_rejects_short_buf() {
        let short = [0u8; 4];
        assert!(!magic_matches(&short));
    }

    #[test]
    fn primary_sb_lba_offset_is_eight() {
        // Block-size 4096 / sector-size 512 = 8 sectors per block,
        // primary SB at block 1 → absolute partition-relative LBA 8.
        assert_eq!(PRIMARY_SB_LBA_OFFSET, 8);
    }

    #[test]
    fn sectors_per_block_is_eight() {
        assert_eq!(SECTORS_PER_BLOCK, 8);
    }
}
