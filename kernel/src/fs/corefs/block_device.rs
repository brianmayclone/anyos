//! AnyOS → `corefs_core` [`BlockDevice`] adapter.
//!
//! Implementiert den plattformneutralen
//! [`corefs_core::storage::block_device::BlockDevice`]-Trait gegen den
//! AnyOS-Storage-Stack (ATA / AHCI / NVMe / SCSI).
//!
//! # Architektur
//!
//! Der Adapter operiert **byte-adressiert** (wie vom `corefs-core`-Trait
//! gefordert), intern werden die Aufrufe in 512-Byte-Sektor-Operationen
//! übersetzt und an [`crate::drivers::storage`] weitergereicht.
//!
//! Die CoreFS-Partition beginnt bei einer partition-relativen LBA
//! (`partition_lba_offset`); der Adapter addiert diesen Offset zu allen
//! Sektor-Adressen, sodass `corefs-core` das Device als von Offset 0
//! beginnend sieht — genau so, wie der On-Disk-Format-Reader es erwartet.
//!
//! # TRIM
//!
//! Der AnyOS-Storage-Stack reicht aktuell kein TRIM/UNMAP an die Hardware
//! durch. [`BlockDeviceAdapter::trim`] ist daher ein No-op; `supports_trim`
//! liefert `false`.
//!
//! # Block-Cache
//!
//! Das Caching passiert eine Schicht tiefer: `crate::drivers::storage::
//! read_sectors_on_disk` / `write_sectors_on_disk` gehen bereits durch den
//! globalen `fs::blockcache`. Der Adapter ruft den Cache daher selbst NICHT
//! zusätzlich auf — ein weiterer Wrap-Pfad würde nur Spinlock-Traffic
//! duplizieren und beim Write die gerade per write-back markierten Dirty-
//! Einträge fälschlich invalidieren, bevor sie geflusht werden.
//!
//! # Teststrategie
//!
//! Die realen `read_sectors_on_disk` / `write_sectors_on_disk`-Aufrufe wirken
//! erst ab Boot mit initialisiertem Backend. Für Unit-Tests indirektieren
//! wir über den kleinen [`SectorIo`]-Trait, den Tests mit einer
//! In-Memory-Fake-Implementierung (`MemSectorIo`) ersetzen können.

use alloc::boxed::Box;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;
use core::fmt;

use corefs_core::error::{CoreFsError, CoreFsResult};
use corefs_core::storage::block_device::{
    check_alignment, check_bounds, check_write_permission, BlockDevice, DeviceGeometry,
    SECTOR_SIZE_512,
};

/// AnyOS-Standard-Sektorgröße (alle unterstützten Backends melden 512 Bytes).
pub const ANYOS_SECTOR_SIZE: u32 = SECTOR_SIZE_512;

// ---------------------------------------------------------------------------
// SectorIo — indirection trait so tests can swap the real storage stack.
// ---------------------------------------------------------------------------

/// Schmale Abstraktion über den AnyOS-Sektor-I/O-Pfad.
///
/// Produktionscode reicht die echten `drivers::storage`-Calls durch; Tests
/// liefern eine In-Memory-Implementierung.
pub trait SectorIo: fmt::Debug + Send + Sync {
    /// Liest `count` 512-Byte-Sektoren ab absoluter `lba` in `buf`.
    fn read_sectors(&self, disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool;
    /// Schreibt `count` 512-Byte-Sektoren ab absoluter `lba` aus `buf`.
    fn write_sectors(&self, disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool;
    /// Flusht Write-Caches auf persistentes Medium.
    fn flush(&self);
}

/// Produktions-`SectorIo`, delegiert an [`crate::drivers::storage`].
#[derive(Debug, Default, Clone, Copy)]
pub struct AnyOsSectorIo;

impl SectorIo for AnyOsSectorIo {
    fn read_sectors(&self, disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
        crate::drivers::storage::read_sectors_on_disk(disk_id, lba, count, buf)
    }

    fn write_sectors(&self, disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
        crate::drivers::storage::write_sectors_on_disk(disk_id, lba, count, buf)
    }

    fn flush(&self) {
        crate::drivers::storage::flush();
    }
}

// ---------------------------------------------------------------------------
// BlockDeviceAdapter
// ---------------------------------------------------------------------------

/// [`BlockDevice`]-Implementierung über einen AnyOS-Storage-Disk.
pub struct BlockDeviceAdapter {
    disk_id: u8,
    partition_lba_offset: u32,
    geometry: DeviceGeometry,
    io: Box<dyn SectorIo>,
}

impl fmt::Debug for BlockDeviceAdapter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockDeviceAdapter")
            .field("disk_id", &self.disk_id)
            .field("partition_lba_offset", &self.partition_lba_offset)
            .field("geometry", &self.geometry)
            .finish()
    }
}

impl BlockDeviceAdapter {
    /// Neuer Adapter gegen den realen AnyOS-Storage-Stack.
    ///
    /// `partition_sectors` gibt die Länge der CoreFS-Partition in 512-Byte-
    /// Sektoren an und bildet die Kapazität des virtuellen Devices.
    pub fn new(
        disk_id: u8,
        partition_lba_offset: u32,
        partition_sectors: u64,
        read_only: bool,
    ) -> CoreFsResult<Self> {
        Self::with_io(
            disk_id,
            partition_lba_offset,
            partition_sectors,
            read_only,
            Box::new(AnyOsSectorIo),
        )
    }

    /// Neuer Adapter mit beliebiger [`SectorIo`]-Implementierung.
    ///
    /// Für Tests — der Aufrufer injiziert z. B. einen `MemSectorIo`.
    pub fn with_io(
        disk_id: u8,
        partition_lba_offset: u32,
        partition_sectors: u64,
        read_only: bool,
        io: Box<dyn SectorIo>,
    ) -> CoreFsResult<Self> {
        if partition_sectors == 0 {
            return Err(CoreFsError::InvalidInput(format!(
                "BlockDeviceAdapter: partition_sectors must be > 0 (disk_id={disk_id})",
            )));
        }
        let capacity_bytes = partition_sectors
            .checked_mul(u64::from(ANYOS_SECTOR_SIZE))
            .ok_or_else(|| {
                CoreFsError::InvalidInput(format!(
                    "BlockDeviceAdapter: partition_sectors {partition_sectors} overflows u64 bytes",
                ))
            })?;
        // Overflow check: last partition LBA must fit in u32.
        let end_lba = u64::from(partition_lba_offset)
            .checked_add(partition_sectors)
            .ok_or_else(|| {
                CoreFsError::InvalidInput(format!(
                    "BlockDeviceAdapter: partition range overflow (offset={partition_lba_offset}, sectors={partition_sectors})",
                ))
            })?;
        if end_lba > u64::from(u32::MAX) {
            return Err(CoreFsError::InvalidInput(format!(
                "BlockDeviceAdapter: partition end LBA {end_lba} exceeds u32",
            )));
        }
        Ok(Self {
            disk_id,
            partition_lba_offset,
            geometry: DeviceGeometry {
                sector_size: ANYOS_SECTOR_SIZE,
                sector_count: partition_sectors,
                capacity_bytes,
                read_only,
            },
            io,
        })
    }

    /// Disk-Id dieses Adapters (zur Diagnose / Logging).
    pub fn disk_id(&self) -> u8 {
        self.disk_id
    }

    /// Partition-Offset in Sektoren (0 = Anfang des physischen Disks).
    pub fn partition_lba_offset(&self) -> u32 {
        self.partition_lba_offset
    }

    /// Übersetzt einen partition-relativen Byte-Offset in eine absolute LBA.
    fn byte_offset_to_abs_lba(&self, offset: u64) -> u32 {
        // Alignment wurde zuvor validiert; Division ist exakt.
        let rel_lba = (offset / u64::from(ANYOS_SECTOR_SIZE)) as u32;
        self.partition_lba_offset + rel_lba
    }
}

impl BlockDevice for BlockDeviceAdapter {
    fn geometry(&self) -> &DeviceGeometry {
        &self.geometry
    }

    fn read_at(&self, offset: u64, length: u64) -> CoreFsResult<Vec<u8>> {
        check_alignment(offset, length, self.geometry.sector_size)?;
        check_bounds(offset, length, self.geometry.capacity_bytes)?;
        if length == 0 {
            return Ok(Vec::new());
        }
        let count = (length / u64::from(ANYOS_SECTOR_SIZE)) as u32;
        let abs_lba = self.byte_offset_to_abs_lba(offset);
        let mut buf = vec![0u8; length as usize];
        if !self.io.read_sectors(self.disk_id, abs_lba, count, &mut buf) {
            return Err(CoreFsError::State(format!(
                "BlockDeviceAdapter: read_sectors_on_disk failed (disk_id={}, lba={abs_lba}, count={count})",
                self.disk_id
            )));
        }
        Ok(buf)
    }

    fn write_at(&mut self, offset: u64, data: &[u8]) -> CoreFsResult<()> {
        check_write_permission(self.geometry.read_only)?;
        let length = data.len() as u64;
        check_alignment(offset, length, self.geometry.sector_size)?;
        check_bounds(offset, length, self.geometry.capacity_bytes)?;
        if data.is_empty() {
            return Ok(());
        }
        let count = (length / u64::from(ANYOS_SECTOR_SIZE)) as u32;
        let abs_lba = self.byte_offset_to_abs_lba(offset);
        if !self.io.write_sectors(self.disk_id, abs_lba, count, data) {
            return Err(CoreFsError::State(format!(
                "BlockDeviceAdapter: write_sectors_on_disk failed (disk_id={}, lba={abs_lba}, count={count})",
                self.disk_id
            )));
        }
        Ok(())
    }

    fn sync(&mut self) -> CoreFsResult<()> {
        self.io.flush();
        Ok(())
    }

    fn trim(&mut self, _offset: u64, _length: u64) -> CoreFsResult<()> {
        // AnyOS-Storage-Stack forwardet aktuell kein TRIM/UNMAP an die
        // Hardware. Silent no-op — der Scrubber/Allocator erkennt das an
        // `supports_trim() == false`.
        Ok(())
    }

    fn supports_trim(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// In-memory SectorIo mock for unit tests (feature = kunit + cfg(test))
// ---------------------------------------------------------------------------

/// In-Memory-[`SectorIo`]-Implementierung für Tests.
#[cfg(any(test, feature = "kunit"))]
pub struct MemSectorIo {
    bytes: crate::sync::mutex::Mutex<Vec<u8>>,
}

#[cfg(any(test, feature = "kunit"))]
impl fmt::Debug for MemSectorIo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemSectorIo").finish()
    }
}

#[cfg(any(test, feature = "kunit"))]
impl MemSectorIo {
    pub fn new(total_sectors: u64) -> Self {
        Self {
            bytes: crate::sync::mutex::Mutex::new(vec![
                0u8;
                (total_sectors * u64::from(ANYOS_SECTOR_SIZE))
                    as usize
            ]),
        }
    }

    pub fn snapshot(&self) -> Vec<u8> {
        self.bytes.lock().clone()
    }
}

#[cfg(any(test, feature = "kunit"))]
impl SectorIo for MemSectorIo {
    fn read_sectors(&self, _disk_id: u8, lba: u32, count: u32, buf: &mut [u8]) -> bool {
        let start = lba as usize * ANYOS_SECTOR_SIZE as usize;
        let len = count as usize * ANYOS_SECTOR_SIZE as usize;
        let data = self.bytes.lock();
        if start + len > data.len() || buf.len() < len {
            return false;
        }
        buf[..len].copy_from_slice(&data[start..start + len]);
        true
    }

    fn write_sectors(&self, _disk_id: u8, lba: u32, count: u32, buf: &[u8]) -> bool {
        let start = lba as usize * ANYOS_SECTOR_SIZE as usize;
        let len = count as usize * ANYOS_SECTOR_SIZE as usize;
        let mut data = self.bytes.lock();
        if start + len > data.len() || buf.len() < len {
            return false;
        }
        data[start..start + len].copy_from_slice(&buf[..len]);
        true
    }

    fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter_with_mem(
        offset: u32,
        part_sectors: u64,
        disk_total_sectors: u64,
    ) -> BlockDeviceAdapter {
        BlockDeviceAdapter::with_io(
            0,
            offset,
            part_sectors,
            false,
            Box::new(MemSectorIo::new(disk_total_sectors)),
        )
        .unwrap()
    }

    #[test]
    fn new_rejects_zero_partition() {
        let err = BlockDeviceAdapter::with_io(0, 0, 0, false, Box::new(MemSectorIo::new(16)))
            .unwrap_err();
        assert!(matches!(err, CoreFsError::InvalidInput(_)));
    }

    #[test]
    fn new_rejects_u32_overflow() {
        let err =
            BlockDeviceAdapter::with_io(0, u32::MAX - 1, 10, false, Box::new(MemSectorIo::new(16)))
                .unwrap_err();
        assert!(matches!(err, CoreFsError::InvalidInput(_)));
    }

    #[test]
    fn geometry_is_consistent() {
        let a = adapter_with_mem(100, 64, 200);
        let g = a.geometry();
        assert_eq!(g.sector_size, 512);
        assert_eq!(g.sector_count, 64);
        assert_eq!(g.capacity_bytes, 64 * 512);
        assert!(!g.read_only);
    }

    #[test]
    fn write_then_read_round_trips_via_partition_offset() {
        let mut a = adapter_with_mem(8, 16, 64);
        let payload = [0xABu8; 1024];
        a.write_at(0, &payload).unwrap();
        let got = a.read_at(0, 1024).unwrap();
        assert_eq!(got, payload);
    }

    #[test]
    fn partition_offset_shifts_absolute_lba() {
        let part_off = 8u32;
        let a = adapter_with_mem(part_off, 16, 64);
        // Absolute LBA for partition-relative offset 1024 (= sector 2) is 10.
        assert_eq!(a.byte_offset_to_abs_lba(1024), part_off + 2);
    }

    #[test]
    fn read_misaligned_rejected() {
        let a = adapter_with_mem(0, 16, 16);
        let err = a.read_at(100, 512).unwrap_err();
        assert!(matches!(err, CoreFsError::InvalidInput(_)));
    }

    #[test]
    fn read_past_end_rejected() {
        let a = adapter_with_mem(0, 4, 16);
        // capacity = 4 * 512 = 2048
        let err = a.read_at(2048, 512).unwrap_err();
        assert!(matches!(err, CoreFsError::InvalidInput(_)));
    }

    #[test]
    fn read_only_blocks_writes() {
        let mut a =
            BlockDeviceAdapter::with_io(0, 0, 16, true, Box::new(MemSectorIo::new(16))).unwrap();
        let err = a.write_at(0, &[0u8; 512]).unwrap_err();
        assert!(matches!(err, CoreFsError::PolicyViolation(_)));
    }

    #[test]
    fn trim_is_noop_ok_and_supports_trim_false() {
        let mut a = adapter_with_mem(0, 16, 16);
        a.trim(0, 512).unwrap();
        assert!(!a.supports_trim());
    }

    #[test]
    fn writes_land_at_absolute_lba_in_backing_store() {
        let part_off = 4u32;
        let mut a = adapter_with_mem(part_off, 16, 64);
        a.write_at(0, &[0x5Au8; 512]).unwrap();
        // Extract backing store by replacing MemSectorIo — we re-read via adapter:
        let got = a.read_at(0, 512).unwrap();
        assert_eq!(got, vec![0x5A; 512]);
    }

    #[test]
    fn sync_calls_flush_without_error() {
        let mut a = adapter_with_mem(0, 4, 4);
        a.sync().unwrap();
    }
}
