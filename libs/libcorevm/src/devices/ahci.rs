//! AHCI (Advanced Host Controller Interface) 1.3.1 controller emulation.
//!
//! Emulates an Intel ICH9-class SATA controller in AHCI mode. Supports up to
//! 6 ports, each with independent command list and received FIS structures.
//! Implements FIS-based ATA/ATAPI command processing for disk and CD-ROM
//! devices.
//!
//! PCI identity: class 01:06:01 (Mass Storage / SATA / AHCI 1.0)
//! Vendor/Device: 8086:2922 (Intel ICH9 AHCI)

use alloc::vec;
use alloc::vec::Vec;
use crate::error::Result;
use crate::memory::mmio::MmioHandler;

// ── AHCI HBA Generic Host Control registers (offsets from ABAR) ──

const HBA_CAP: u64 = 0x00;
const HBA_GHC: u64 = 0x04;
const HBA_IS: u64 = 0x08;
const HBA_PI: u64 = 0x0C;
const HBA_VS: u64 = 0x10;
const HBA_CCC_CTL: u64 = 0x14;
const HBA_CCC_PORTS: u64 = 0x18;
const HBA_EM_LOC: u64 = 0x1C;
const HBA_EM_CTL: u64 = 0x20;
const HBA_CAP2: u64 = 0x24;
const HBA_BOHC: u64 = 0x28;

// ── Per-port register offsets (base = 0x100 + port*0x80) ──

const PORT_CLB: u64 = 0x00;
const PORT_CLBU: u64 = 0x04;
const PORT_FB: u64 = 0x08;
const PORT_FBU: u64 = 0x0C;
const PORT_IS: u64 = 0x10;
const PORT_IE: u64 = 0x14;
const PORT_CMD: u64 = 0x18;
const PORT_TFD: u64 = 0x20;
const PORT_SIG: u64 = 0x24;
const PORT_SSTS: u64 = 0x28;
const PORT_SCTL: u64 = 0x2C;
const PORT_SERR: u64 = 0x30;
const PORT_SACT: u64 = 0x34;
const PORT_CI: u64 = 0x38;
const PORT_SNTF: u64 = 0x3C;
const PORT_FBS: u64 = 0x40;

const PORT_CMD_ST: u32 = 1 << 0;
const PORT_CMD_FRE: u32 = 1 << 4;
const PORT_CMD_FR: u32 = 1 << 14;
const PORT_CMD_CR: u32 = 1 << 15;
const PORT_CMD_ICC_ACTIVE: u32 = 1 << 28;

const PORT_IS_DHRS: u32 = 1 << 0;
const PORT_IS_TFES: u32 = 1 << 30; // Task File Error Status

const GHC_HR: u32 = 1 << 0;
const GHC_IE: u32 = 1 << 1;
const GHC_AE: u32 = 1u32 << 31;

const TFD_STS_DRDY: u32 = 1 << 6;
const TFD_STS_DSC: u32 = 1 << 4;

const SSTS_DET_PRESENT: u32 = 0x3;
const SSTS_SPD_GEN1: u32 = 0x1 << 4;
const SSTS_IPM_ACTIVE: u32 = 0x1 << 8;

const FIS_TYPE_REG_H2D: u8 = 0x27;
const FIS_TYPE_REG_D2H: u8 = 0x34;

const ATA_CMD_IDENTIFY: u8 = 0xEC;
const ATA_CMD_IDENTIFY_PACKET: u8 = 0xA1;
const ATA_CMD_READ_DMA: u8 = 0xC8;
const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
const ATA_CMD_WRITE_DMA: u8 = 0xCA;
const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;
const ATA_CMD_SET_FEATURES: u8 = 0xEF;
const ATA_CMD_FLUSH: u8 = 0xE7;
const ATA_CMD_FLUSH_EXT: u8 = 0xEA;
const ATA_CMD_PACKET: u8 = 0xA0;
const ATA_CMD_READ_SECTORS: u8 = 0x20;
const ATA_CMD_READ_SECTORS_EXT: u8 = 0x24;
const ATA_CMD_WRITE_SECTORS: u8 = 0x30;
const ATA_CMD_WRITE_SECTORS_EXT: u8 = 0x34;

const MAX_PORTS: usize = 6;
/// MMIO region size for AHCI HBA.
pub const AHCI_MMIO_SIZE: u64 = 0x1100;
const SECTOR_SIZE: usize = 512;
const ATAPI_SECTOR_SIZE: usize = 2048;

// ── Guest DMA helper (borrowck-friendly) ──

struct GuestDma {
    ptr: *mut u8,
    len: usize,
}

/// PCI hole constants for guest physical → host offset translation.
/// Guest RAM > 3.5GB is split: 0..0xE0000000 and 0x100000000..
/// Host memory is contiguous, so GPA 0x100000000+ maps to host offset 0xE0000000+.
const PCI_HOLE_START: u64 = 0xE000_0000;
const PCI_HOLE_END: u64   = 0x1_0000_0000;

impl GuestDma {
    /// Translate guest physical address to host memory offset,
    /// accounting for the PCI hole (0xE0000000–0xFFFFFFFF).
    #[inline]
    fn gpa_to_offset(&self, gpa: u64) -> Option<usize> {
        if gpa < PCI_HOLE_START {
            Some(gpa as usize)
        } else if gpa >= PCI_HOLE_END {
            // Above-4G RAM is stored contiguously after the below-hole RAM
            Some((PCI_HOLE_START + (gpa - PCI_HOLE_END)) as usize)
        } else {
            None // PCI hole — not RAM
        }
    }

    fn read_bytes(&self, addr: u64, count: usize) -> Option<Vec<u8>> {
        let a = self.gpa_to_offset(addr)?;
        if count == 0 || a.checked_add(count).map_or(true, |end| end > self.len) { return None; }
        let mut buf = vec![0u8; count];
        unsafe { core::ptr::copy_nonoverlapping(self.ptr.add(a), buf.as_mut_ptr(), count); }
        Some(buf)
    }

    fn write_bytes(&self, addr: u64, data: &[u8]) {
        let a = match self.gpa_to_offset(addr) { Some(o) => o, None => return };
        if data.is_empty() || a.checked_add(data.len()).map_or(true, |end| end > self.len) { return; }
        unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr.add(a), data.len()); }
    }

    fn write_u32(&self, addr: u64, val: u32) {
        self.write_bytes(addr, &val.to_le_bytes());
    }

    fn write_prdt(&self, prdt_base: u64, prdtl: u32, data: &[u8]) {
        let mut offset = 0usize;
        for i in 0..prdtl as u64 {
            if offset >= data.len() { break; }
            let ea = prdt_base + i * 16;
            if let Some(e) = self.read_bytes(ea, 16) {
                let dba = u32::from_le_bytes([e[0], e[1], e[2], e[3]]) as u64
                    | (u32::from_le_bytes([e[4], e[5], e[6], e[7]]) as u64) << 32;
                let bc = (u32::from_le_bytes([e[12], e[13], e[14], e[15]]) & 0x3FFFFF) + 1;
                let n = (bc as usize).min(data.len() - offset);
                self.write_bytes(dba, &data[offset..offset + n]);
                offset += n;
            }
        }
    }

    fn read_prdt(&self, prdt_base: u64, prdtl: u32, buf: &mut [u8]) {
        let mut offset = 0usize;
        for i in 0..prdtl as u64 {
            if offset >= buf.len() { break; }
            let ea = prdt_base + i * 16;
            if let Some(e) = self.read_bytes(ea, 16) {
                let dba = u32::from_le_bytes([e[0], e[1], e[2], e[3]]) as u64
                    | (u32::from_le_bytes([e[4], e[5], e[6], e[7]]) as u64) << 32;
                let bc = (u32::from_le_bytes([e[12], e[13], e[14], e[15]]) & 0x3FFFFF) + 1;
                let n = (bc as usize).min(buf.len() - offset);
                if let Some(chunk) = self.read_bytes(dba, n) {
                    buf[offset..offset + n].copy_from_slice(&chunk);
                } else {
                    #[cfg(feature = "std")]
                    eprintln!("[ahci] PRDT READ FAILED: dba=0x{:X} bc={} prdtl={} i={} ram_len=0x{:X}",
                        dba, bc, prdtl, i, self.len);
                }
                offset += n;
            }
        }
    }
}

// ── Drive ──

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AhciDriveKind {
    AtaDisk,
    AtapiCdrom,
}

struct AhciDrive {
    kind: AhciDriveKind,
    disk: Vec<u8>,
    disk_fd: i32,
    total_bytes: u64,
    present: bool,
    cache: super::disk_cache::DiskCache,
}

impl AhciDrive {
    fn new() -> Self {
        AhciDrive {
            kind: AhciDriveKind::AtaDisk, disk: Vec::new(), disk_fd: -1,
            total_bytes: 0, present: false,
            cache: super::disk_cache::DiskCache::new(0, super::disk_cache::CacheMode::None),
        }
    }

    fn sector_size(&self) -> usize {
        match self.kind { AhciDriveKind::AtaDisk => SECTOR_SIZE, AhciDriveKind::AtapiCdrom => ATAPI_SECTOR_SIZE }
    }

    fn total_sectors(&self) -> u64 { self.total_bytes / self.sector_size() as u64 }

    fn seek_to(&self, byte_offset: u64) {
        #[cfg(feature = "host_test")]
        {
            use crate::syscall;
            let fd = self.disk_fd as u32;
            syscall::lseek(fd, 0, 0); // rewind
            let mut remaining = byte_offset;
            const MAX_CHUNK: u64 = 0x7FFF_FFFF;
            while remaining > MAX_CHUNK {
                syscall::lseek(fd, MAX_CHUNK as i32, 1);
                remaining -= MAX_CHUNK;
            }
            if remaining > 0 {
                syscall::lseek(fd, remaining as i32, 1);
            }
        }
        #[cfg(not(feature = "host_test"))]
        { let _ = byte_offset; }
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) {
        // Try cache first
        if self.cache.read(offset, buf) {
            return;
        }
        // Cache miss — read from host
        self.read_at_host(offset, buf);
        // Populate cache with the block we just read (if block-aligned and fits)
        if self.cache.enabled() && buf.len() <= 4096 {
            let block_start = offset & !4095;
            let mut block_buf = [0u8; 4096];
            if block_start == offset && buf.len() == 4096 {
                block_buf.copy_from_slice(buf);
            } else {
                // Read the full aligned block for caching
                self.read_at_host(block_start, &mut block_buf);
            }
            self.cache.populate_read(block_start, &block_buf);
        }
    }

    fn read_at_host(&self, offset: u64, buf: &mut [u8]) {
        if self.disk_fd >= 0 {
            #[cfg(feature = "std")]
            {
                use std::io::{Read, Seek, SeekFrom};
                let mut file = unsafe { Self::borrow_file(self.disk_fd) };
                let _ = file.seek(SeekFrom::Start(offset));
                let mut total = 0usize;
                while total < buf.len() {
                    match file.read(&mut buf[total..]) {
                        Ok(0) => break,
                        Ok(n) => total += n,
                        Err(_) => break,
                    }
                }
                if total < buf.len() { buf[total..].fill(0); }
                core::mem::forget(file);
                return;
            }
            #[cfg(feature = "host_test")]
            {
                self.seek_to(offset);
                use crate::syscall;
                let n = syscall::read(self.disk_fd as u32, buf);
                let read_len = if n == u32::MAX { 0 } else { (n as usize).min(buf.len()) };
                if read_len < buf.len() { buf[read_len..].fill(0); }
                return;
            }
            #[allow(unreachable_code)]
            { buf.fill(0); }
        } else {
            let s = offset as usize;
            let e = s.checked_add(buf.len()).unwrap_or(self.disk.len()).min(self.disk.len());
            if s < self.disk.len() {
                buf[..e - s].copy_from_slice(&self.disk[s..e]);
                for b in buf[e - s..].iter_mut() { *b = 0; }
            } else {
                for b in buf.iter_mut() { *b = 0; }
            }
        }
    }

    fn write_at(&mut self, offset: u64, buf: &[u8]) {
        // Try cache — returns true if write was absorbed (write-back mode)
        if self.cache.write(offset, buf) {
            return; // Write-back: data is in cache, will be flushed later
        }
        // Write-through or uncached: write to host now
        self.write_at_host(offset, buf);
    }

    fn write_at_host(&mut self, offset: u64, buf: &[u8]) {
        if self.disk_fd >= 0 {
            #[cfg(feature = "std")]
            {
                use std::io::{Write, Seek, SeekFrom};
                let mut file = unsafe { Self::borrow_file(self.disk_fd) };
                if let Err(_e) = file.seek(SeekFrom::Start(offset)) {
                    eprintln!("[ahci] WRITE SEEK ERROR at offset=0x{:X}: {}", offset, _e);
                }
                if let Err(_e) = file.write_all(buf) {
                    eprintln!("[ahci] WRITE ERROR at offset=0x{:X} len={}: {}", offset, buf.len(), _e);
                }
                core::mem::forget(file);
                return;
            }
            #[cfg(feature = "host_test")]
            {
                self.seek_to(offset);
                use crate::syscall;
                syscall::write(self.disk_fd as u32, buf);
                return;
            }
        } else {
            let s = offset as usize;
            if let Some(e) = s.checked_add(buf.len()) {
                if e <= self.disk.len() {
                    self.disk[s..e].copy_from_slice(buf);
                } else if s < self.disk.len() {
                    let dl = self.disk.len();
                    self.disk[s..].copy_from_slice(&buf[..dl - s]);
                }
            }
        }
    }

    /// Borrow a file descriptor/handle as a `std::fs::File` without taking ownership.
    /// Caller MUST `core::mem::forget` the returned File to prevent closing the fd.
    #[cfg(feature = "std")]
    unsafe fn borrow_file(fd: i32) -> std::fs::File {
        #[cfg(unix)]
        {
            use std::os::unix::io::FromRawFd;
            std::fs::File::from_raw_fd(fd)
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::{FromRawHandle, RawHandle};
            std::fs::File::from_raw_handle(fd as isize as RawHandle)
        }
    }

    fn build_identify(&self) -> [u8; 512] {
        let mut id = [0u8; 512];
        let sectors = self.total_sectors();
        if self.kind == AhciDriveKind::AtapiCdrom {
            write_word(&mut id, 0, 0x85C0);
        } else {
            write_word(&mut id, 0, 0x0040);
            // CHS geometry (words 1, 3, 6) — required for INT 13h / bootmgr.
            // Use standard translation: 16383 cylinders, 16 heads, 63 sectors.
            let chs_sectors = sectors.min(16383 * 16 * 63);
            let heads = 16u16;
            let spt = 63u16;
            let cyls = (chs_sectors / (heads as u64 * spt as u64)).min(16383) as u16;
            write_word(&mut id, 1, cyls);  // word 1: cylinders
            write_word(&mut id, 3, heads); // word 3: heads
            write_word(&mut id, 6, spt);   // word 6: sectors per track
        }
        write_ata_string(&mut id, 10, b"COREVM_AHCI_0000", 20);
        write_ata_string(&mut id, 23, b"1.0     ", 8);
        write_ata_string(&mut id, 27, b"CoreVM AHCI Virtual Disk            ", 40);
        write_word(&mut id, 47, 0x8010);
        write_word(&mut id, 49, 0x0F00);
        write_word(&mut id, 53, 0x0006);
        write_word(&mut id, 59, 0x0010);
        let lba28 = if sectors > 0x0FFF_FFFF { 0x0FFF_FFFF } else { sectors as u32 };
        write_word(&mut id, 60, (lba28 & 0xFFFF) as u16);
        write_word(&mut id, 61, (lba28 >> 16) as u16);
        write_word(&mut id, 63, 0x0407);
        write_word(&mut id, 64, 0x0003);
        write_word(&mut id, 75, 31);
        write_word(&mut id, 76, 0x000E);
        write_word(&mut id, 80, 0x01F0);
        write_word(&mut id, 82, 0x7C6B);
        write_word(&mut id, 83, 0x7400 | (1 << 10));
        write_word(&mut id, 85, 0x7C69);
        write_word(&mut id, 86, 0x7400 | (1 << 10));
        write_word(&mut id, 88, 0x407F);
        write_word(&mut id, 100, (sectors & 0xFFFF) as u16);
        write_word(&mut id, 101, ((sectors >> 16) & 0xFFFF) as u16);
        write_word(&mut id, 102, ((sectors >> 32) & 0xFFFF) as u16);
        write_word(&mut id, 103, ((sectors >> 48) & 0xFFFF) as u16);
        write_word(&mut id, 106, 0x4000); // Bit 14=1 (word valid), 512-byte logical sectors, no multi-sector
        write_word(&mut id, 217, 0x0001);
        id
    }
}

// ── Port ──

struct AhciPort {
    clb: u64, fb: u64,
    is: u32, ie: u32, cmd: u32, tfd: u32, sig: u32,
    ssts: u32, sctl: u32, serr: u32, sact: u32, ci: u32, sntf: u32, fbs: u32,
    drive: AhciDrive,
}

impl AhciPort {
    fn new() -> Self {
        AhciPort {
            clb: 0, fb: 0, is: 0, ie: 0, cmd: 0,
            tfd: TFD_STS_DRDY | TFD_STS_DSC, sig: 0xFFFF_FFFF,
            ssts: 0, sctl: 0, serr: 0, sact: 0, ci: 0, sntf: 0, fbs: 0,
            drive: AhciDrive::new(),
        }
    }

    fn update_presence(&mut self) {
        if self.drive.present {
            self.ssts = SSTS_DET_PRESENT | SSTS_SPD_GEN1 | SSTS_IPM_ACTIVE;
            self.sig = match self.drive.kind {
                AhciDriveKind::AtaDisk => 0x0000_0101,
                AhciDriveKind::AtapiCdrom => 0xEB14_0101,
            };
            self.tfd = TFD_STS_DRDY | TFD_STS_DSC;
        } else {
            self.ssts = 0;
            self.sig = 0xFFFF_FFFF;
            self.tfd = 0x7F;
        }
    }
}

// ── AHCI HBA ──

pub struct Ahci {
    cap: u32, ghc: u32, is: u32, pi: u32, vs: u32,
    ccc_ctl: u32, ccc_ports: u32, em_loc: u32, em_ctl: u32, cap2: u32, bohc: u32,
    ports: [AhciPort; MAX_PORTS],
    irq_pending: bool,
    /// MSI state — updated when guest writes PCI config space MSI registers.
    pub msi_enabled: bool,
    pub msi_address: u64,
    pub msi_data: u32,
    guest_mem_ptr: *mut u8,
    guest_mem_len: usize,
}

unsafe impl Send for Ahci {}

impl Ahci {
    pub fn new(num_ports: u8) -> Self {
        let np = (num_ports as usize).min(MAX_PORTS);
        // PI starts at 0; bits are set when drives are attached.
        let pi = 0u32;
        let cap = (np as u32 - 1)
            | (31 << 8)        // 32 command slots
            | (1 << 13)        // PSC
            | (1 << 14)        // SSC
            | (1 << 20)        // SAL
            | (1 << 21)        // ISS Gen1
            | (1 << 24)        // SNCQ
            | (1u32 << 31);   // S64A

        Ahci {
            cap, ghc: GHC_AE, is: 0, pi, vs: 0x0001_0301,
            ccc_ctl: 0, ccc_ports: 0, em_loc: 0, em_ctl: 0, cap2: 0, bohc: 0,
            ports: core::array::from_fn(|_| AhciPort::new()),
            irq_pending: false,
            msi_enabled: false,
            msi_address: 0,
            msi_data: 0,
            guest_mem_ptr: core::ptr::null_mut(),
            guest_mem_len: 0,
        }
    }

    pub fn attach_disk(&mut self, port: usize, image: Vec<u8>, kind: AhciDriveKind) {
        if port >= MAX_PORTS { return; }
        let total = image.len() as u64;
        self.ports[port].drive = AhciDrive { kind, disk: image, disk_fd: -1, total_bytes: total, present: true, cache: super::disk_cache::DiskCache::new(0, super::disk_cache::CacheMode::None) };
        self.ports[port].update_presence();
        self.pi |= 1u32 << port;
    }

    pub fn attach_disk_fd(&mut self, port: usize, fd: i32, size: u64, kind: AhciDriveKind) {
        if port >= MAX_PORTS { return; }
        self.ports[port].drive = AhciDrive { kind, disk: Vec::new(), disk_fd: fd, total_bytes: size, present: true, cache: super::disk_cache::DiskCache::new(0, super::disk_cache::CacheMode::None) };
        self.ports[port].update_presence();
        self.pi |= 1u32 << port;
    }

    pub fn irq_raised(&self) -> bool { self.irq_pending }
    pub fn clear_irq(&mut self) { self.irq_pending = false; }

    pub fn set_guest_memory(&mut self, ptr: *mut u8, len: usize) {
        self.guest_mem_ptr = ptr;
        self.guest_mem_len = len;
    }

    /// Configure disk cache for a specific port.
    /// `cache_mb`: cache size in MiB (0 = disabled).
    /// `mode`: caching strategy.
    pub fn configure_cache(&mut self, port: usize, cache_mb: u32, mode: super::disk_cache::CacheMode) {
        if port < MAX_PORTS {
            self.ports[port].drive.cache = super::disk_cache::DiskCache::new(cache_mb, mode);
        }
    }

    /// Flush dirty cache blocks to host for all ports.
    /// Should be called periodically from the VM loop (e.g. every 100-500ms).
    pub fn flush_caches(&mut self) {
        for port in &mut self.ports {
            if !port.drive.present { continue; }
            if port.drive.cache.dirty_count() == 0 { continue; }
            let dirty = port.drive.cache.collect_dirty();
            for (offset, data) in dirty {
                port.drive.write_at_host(offset, &data);
            }
        }
    }

    /// Check if any port needs a cache flush.
    pub fn any_cache_needs_flush(&self) -> bool {
        self.ports.iter().any(|p| p.drive.cache.needs_flush())
    }

    fn dma(&self) -> GuestDma {
        GuestDma { ptr: self.guest_mem_ptr, len: self.guest_mem_len }
    }

    fn process_commands(&mut self, port_idx: usize) {
        if port_idx >= MAX_PORTS { return; }
        if self.guest_mem_ptr.is_null() { return; }

        let dma = self.dma();
        let port = &mut self.ports[port_idx];
        if !port.drive.present || port.cmd & PORT_CMD_ST == 0 {
            return;
        }

        let ci = port.ci;
        if ci == 0 { return; }

        for slot in 0..32u32 {
            if ci & (1 << slot) == 0 { continue; }

            let cmd_hdr_addr = port.clb + (slot as u64) * 32;
            let header = match dma.read_bytes(cmd_hdr_addr, 32) {
                Some(h) => h,
                None => {
                    port.ci &= !(1 << slot);
                    continue;
                }
            };

            let dw0 = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
            let prdtl = (dw0 >> 16) & 0xFFFF;

            let ctba = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as u64
                | (u32::from_le_bytes([header[12], header[13], header[14], header[15]]) as u64) << 32;

            let cfis = match dma.read_bytes(ctba, 64) {
                Some(f) => f,
                None => {
                    port.ci &= !(1 << slot);
                    continue;
                }
            };
            if cfis[0] != FIS_TYPE_REG_H2D {
                port.ci &= !(1 << slot);
                continue;
            }

            let command = cfis[2];
            // H2D FIS layout:
            //   [0] FIS type (0x27)  [1] Flags  [2] Command  [3] Features
            //   [4] LBA Low          [5] LBA Mid [6] LBA High [7] Device
            //   [8] LBA Low (exp)    [9] LBA Mid (exp) [10] LBA High (exp) [11] Features (exp)
            //   [12] Count Low       [13] Count High
            //
            // AHCI uses 48-bit FIS format, but for 28-bit commands the guest
            // driver may not clear cfis[8-10]. Use all 6 bytes for 48-bit
            // commands, only lower 4 bytes for 28-bit commands.
            let is_ext = matches!(command,
                ATA_CMD_READ_DMA_EXT | ATA_CMD_WRITE_DMA_EXT |
                ATA_CMD_READ_SECTORS_EXT | ATA_CMD_WRITE_SECTORS_EXT |
                ATA_CMD_FLUSH_EXT | 0x27 /* READ NATIVE MAX ADDRESS EXT */);
            let lba = if is_ext {
                // 48-bit: use all 6 LBA bytes
                cfis[4] as u64 | (cfis[5] as u64) << 8 | (cfis[6] as u64) << 16
                    | (cfis[8] as u64) << 24 | (cfis[9] as u64) << 32 | (cfis[10] as u64) << 40
            } else {
                // 28-bit: only use cfis[4-6] + device register bits 3:0
                cfis[4] as u64 | (cfis[5] as u64) << 8
                    | (cfis[6] as u64) << 16
                    | ((cfis[7] & 0x0F) as u64) << 24
            };
            let mut count = (cfis[13] as u32) << 8 | cfis[12] as u32;
            if count == 0 { count = 256; }
            // Cap count to prevent excessive memory allocation (max 256 sectors = 128KB for ATA)
            if count > 0xFFFF { count = 0xFFFF; }

            let prdt_base = ctba + 0x80;


            match command {
                ATA_CMD_IDENTIFY => {
                    if port.drive.kind == AhciDriveKind::AtapiCdrom {
                        // ATAPI devices must abort IDENTIFY DEVICE (0xEC)
                        // and only respond to IDENTIFY PACKET DEVICE (0xA1).
                        // Set the signature in TFD so the driver can re-issue 0xA1.
                        port.tfd = TFD_STS_DRDY | TFD_STS_DSC | (0x04 << 8) | 1; // ABRT
                        port.is |= PORT_IS_DHRS;
                    } else {
                        let id = port.drive.build_identify();
                        if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &id); }
                        dma.write_u32(cmd_hdr_addr + 4, 512);
                        port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
                        port.is |= PORT_IS_DHRS;
                    }
                }
                ATA_CMD_IDENTIFY_PACKET => {
                    if port.drive.kind == AhciDriveKind::AtaDisk {
                        // ATA disks must abort IDENTIFY PACKET DEVICE (0xA1)
                        // and only respond to IDENTIFY DEVICE (0xEC).
                        port.tfd = TFD_STS_DRDY | TFD_STS_DSC | (0x04 << 8) | 1; // ABRT
                        port.is |= PORT_IS_DHRS;
                    } else {
                        let id = port.drive.build_identify();
                        if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &id); }
                        dma.write_u32(cmd_hdr_addr + 4, 512);
                        port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
                        port.is |= PORT_IS_DHRS;
                    }
                }
                ATA_CMD_READ_DMA | ATA_CMD_READ_DMA_EXT
                | ATA_CMD_READ_SECTORS | ATA_CMD_READ_SECTORS_EXT => {
                    let ss = port.drive.sector_size();
                    let total = count as usize * ss;
                    let mut buf = vec![0u8; total];
                    port.drive.read_at(lba * ss as u64, &mut buf);
                    if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &buf); }
                    dma.write_u32(cmd_hdr_addr + 4, total as u32);
                    port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
                    port.is |= PORT_IS_DHRS;
                }
                ATA_CMD_WRITE_DMA | ATA_CMD_WRITE_DMA_EXT
                | ATA_CMD_WRITE_SECTORS | ATA_CMD_WRITE_SECTORS_EXT => {
                    let ss = port.drive.sector_size();
                    let total = count as usize * ss;
                    let mut buf = vec![0u8; total];
                    if prdtl > 0 { dma.read_prdt(prdt_base, prdtl, &mut buf); }
                    port.drive.write_at(lba * ss as u64, &buf);
                    dma.write_u32(cmd_hdr_addr + 4, total as u32);
                    port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
                    port.is |= PORT_IS_DHRS;
                }
                ATA_CMD_FLUSH | ATA_CMD_FLUSH_EXT => {
                    // Flush write cache — call fsync on the host fd
                    port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
                    port.is |= PORT_IS_DHRS;
                    #[cfg(feature = "std")]
                    if port.drive.disk_fd >= 0 {
                        let file = unsafe { AhciDrive::borrow_file(port.drive.disk_fd) };
                        let _ = file.sync_all();
                        core::mem::forget(file);
                    }
                }
                ATA_CMD_SET_FEATURES
                | 0xF5 // SECURITY FREEZE LOCK — no-op for VMs
                | 0x27 // READ NATIVE MAX ADDRESS EXT — return success
                | 0xE0 | 0xE1 // STANDBY IMMEDIATE / IDLE IMMEDIATE
                | 0xE5 | 0xE6 // CHECK POWER MODE / SLEEP
                | 0x91 // INITIALIZE DEVICE PARAMETERS
                => {
                    port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
                    port.is |= PORT_IS_DHRS;
                }
                0xB0 => { // SMART
                    let sub = cfis[3]; // features register = subcommand
                    match sub {
                        0xD0 | 0xD1 => { // SMART READ DATA / SMART READ THRESHOLDS
                            let mut data = [0u8; 512];
                            if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &data); }
                            dma.write_u32(cmd_hdr_addr + 4, 512);
                            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
                        }
                        0xD8 | 0xD9 | 0xDA => { // SMART ENABLE/DISABLE OPERATIONS / RETURN STATUS
                            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
                        }
                        _ => {
                            // Unsupported SMART sub — just succeed
                            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
                        }
                    }
                    port.is |= PORT_IS_DHRS;
                }
                ATA_CMD_PACKET => {
                    if port.drive.kind != AhciDriveKind::AtapiCdrom {
                        // ATA disks don't support PACKET command
                        port.tfd = TFD_STS_DRDY | TFD_STS_DSC | (0x04 << 8) | 1; // ABRT
                        port.is |= PORT_IS_DHRS;
                    } else if let Some(acmd) = dma.read_bytes(ctba + 0x40, 16) {
                        process_atapi(port, &dma, &acmd, prdt_base, prdtl, cmd_hdr_addr);
                    } else {
                        port.tfd = TFD_STS_DRDY | TFD_STS_DSC | 1;
                        port.is |= PORT_IS_DHRS;
                    }
                }
                _ => {
                    #[cfg(feature = "std")]
                    eprintln!("[ahci] UNSUPPORTED ATA cmd=0x{:02X}", command);
                    // Return ABRT error for unsupported commands
                    port.tfd = TFD_STS_DRDY | TFD_STS_DSC | (0x04 << 8) | 1; // ERR + ABRT
                    port.is |= PORT_IS_DHRS;
                }
            }

            // Post D2H FIS
            if port.fb != 0 && port.cmd & PORT_CMD_FRE != 0 {
                let mut d2h = [0u8; 20];
                d2h[0] = FIS_TYPE_REG_D2H;
                d2h[1] = 0x40;
                d2h[2] = (port.tfd & 0xFF) as u8;
                d2h[3] = ((port.tfd >> 8) & 0xFF) as u8;
                dma.write_bytes(port.fb + 0x40, &d2h);
            }

            port.ci &= !(1 << slot);
            port.sact &= !(1 << slot);
        }

        // Update global IS
        let port = &self.ports[port_idx];
        if port.is & port.ie != 0 {
            self.is |= 1 << port_idx;
            if self.ghc & GHC_IE != 0 {
                self.irq_pending = true;
            }
        }
    }

    fn port_read(&self, idx: usize, off: u64) -> u32 {
        if idx >= MAX_PORTS { return 0xFFFFFFFF; }
        let p = &self.ports[idx];
        match off {
            PORT_CLB => p.clb as u32, PORT_CLBU => (p.clb >> 32) as u32,
            PORT_FB => p.fb as u32, PORT_FBU => (p.fb >> 32) as u32,
            PORT_IS => p.is, PORT_IE => p.ie,
            PORT_CMD => {
                let mut c = p.cmd;
                if c & PORT_CMD_ST != 0 { c |= PORT_CMD_CR; }
                if c & PORT_CMD_FRE != 0 { c |= PORT_CMD_FR; }
                c
            }
            PORT_TFD => p.tfd, PORT_SIG => p.sig, PORT_SSTS => p.ssts,
            PORT_SCTL => p.sctl, PORT_SERR => p.serr, PORT_SACT => p.sact,
            PORT_CI => p.ci, PORT_SNTF => p.sntf, PORT_FBS => p.fbs,
            _ => 0,
        }
    }

    fn port_write(&mut self, idx: usize, off: u64, val: u32) {
        if idx >= MAX_PORTS { return; }
        match off {
            PORT_CLB => { let hi = self.ports[idx].clb & !0xFFFF_FFFF; self.ports[idx].clb = hi | (val as u64 & !0x3FF); }
            PORT_CLBU => { let lo = self.ports[idx].clb & 0xFFFF_FFFF; self.ports[idx].clb = (val as u64) << 32 | lo; }
            PORT_FB => { let hi = self.ports[idx].fb & !0xFFFF_FFFF; self.ports[idx].fb = hi | (val as u64 & !0xFF); }
            PORT_FBU => { let lo = self.ports[idx].fb & 0xFFFF_FFFF; self.ports[idx].fb = (val as u64) << 32 | lo; }
            PORT_IS => { self.ports[idx].is &= !val; if self.ports[idx].is == 0 { self.is &= !(1 << idx); } }
            PORT_IE => { self.ports[idx].ie = val; }
            PORT_CMD => {
                let old = self.ports[idx].cmd;
                self.ports[idx].cmd = val & !(PORT_CMD_CR | PORT_CMD_FR);
                // When ST is set (start), clear TFD error bits and process pending commands.
                // SeaBIOS writes PORT_CI first, then sets PORT_CMD.ST — so commands
                // queued before ST was set must be processed now.
                if val & PORT_CMD_ST != 0 && old & PORT_CMD_ST == 0 {
                    self.ports[idx].tfd = TFD_STS_DRDY | TFD_STS_DSC;
                    if self.ports[idx].ci != 0 {
                        self.process_commands(idx);
                    }
                }
            }
            PORT_SCTL => {
                let old = self.ports[idx].sctl & 0x0F;
                self.ports[idx].sctl = val;
                if old == 1 && val & 0x0F == 0 { self.ports[idx].update_presence(); }
            }
            PORT_SERR => { self.ports[idx].serr &= !val; }
            PORT_SACT => { self.ports[idx].sact |= val; }
            PORT_CI => {
                self.ports[idx].ci |= val;
                self.process_commands(idx);
            }
            PORT_FBS => { self.ports[idx].fbs = val; }
            _ => {}
        }
    }
}

/// Ensure all disk file descriptors are synced and closed when AHCI is dropped.
#[cfg(feature = "std")]
impl Drop for Ahci {
    fn drop(&mut self) {
        for port in &mut self.ports {
            if port.drive.disk_fd >= 0 {
                // Flush any pending cache writes
                if port.drive.cache.dirty_count() > 0 {
                    let dirty = port.drive.cache.collect_dirty();
                    for (offset, data) in dirty {
                        port.drive.write_at_host(offset, &data);
                    }
                }
                // Sync and close the fd
                #[cfg(feature = "std")]
                {
                    // SAFETY: taking ownership of fd; File::drop will close it
                    let file = unsafe { AhciDrive::borrow_file(port.drive.disk_fd) };
                    // NOTE: we do NOT mem::forget here — let File drop to close the fd
                    let _ = file.sync_all();
                    // File drops here and closes the fd
                }
                #[cfg(not(feature = "std"))]
                {
                    // fd leaks on non-std — acceptable for no_std targets
                }
                port.drive.disk_fd = -1;
            }
        }
    }
}

impl MmioHandler for Ahci {
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        let val = match offset {
            HBA_CAP => self.cap, HBA_GHC => self.ghc, HBA_IS => self.is,
            HBA_PI => self.pi, HBA_VS => self.vs, HBA_CCC_CTL => self.ccc_ctl,
            HBA_CCC_PORTS => self.ccc_ports, HBA_EM_LOC => self.em_loc,
            HBA_EM_CTL => self.em_ctl, HBA_CAP2 => self.cap2, HBA_BOHC => self.bohc,
            0x100..=0x10FF => {
                let po = offset - 0x100;
                self.port_read((po / 0x80) as usize, po % 0x80)
            }
            _ => 0,
        };
        Ok(match size {
            1 => (val >> ((offset & 3) * 8)) as u64 & 0xFF,
            2 => (val >> ((offset & 2) * 8)) as u64 & 0xFFFF,
            _ => val as u64,
        })
    }

    fn write(&mut self, offset: u64, _size: u8, val: u64) -> Result<()> {
        let v = val as u32;
        match offset {
            HBA_GHC => {
                if v & GHC_HR != 0 {
                    for p in self.ports.iter_mut() {
                        p.is = 0; p.ie = 0; p.cmd = 0; p.ci = 0; p.sact = 0; p.serr = 0;
                        p.update_presence();
                    }
                    self.is = 0; self.ghc = GHC_AE; self.irq_pending = false;
                    #[cfg(feature = "std")]
                    eprintln!("[ahci] HBA RESET");
                } else {
                    let old = self.ghc;
                    self.ghc = v | GHC_AE;
                    #[cfg(feature = "std")]
                    if (v & GHC_IE != 0) && (old & GHC_IE == 0) {
                        eprintln!("[ahci] GHC_IE enabled (GHC=0x{:08X})", self.ghc);
                    }
                }
            }
            HBA_IS => { self.is &= !v; if self.is == 0 { self.irq_pending = false; } }
            HBA_CCC_CTL => self.ccc_ctl = v, HBA_CCC_PORTS => self.ccc_ports = v,
            HBA_EM_CTL => self.em_ctl = v, HBA_BOHC => self.bohc = v,
            0x100..=0x10FF => {
                let po = offset - 0x100;
                self.port_write((po / 0x80) as usize, po % 0x80, v);
            }
            _ => {}
        }
        Ok(())
    }
}

// ── ATAPI command processing (free function to avoid borrow issues) ──

fn process_atapi(port: &mut AhciPort, dma: &GuestDma, acmd: &[u8], prdt_base: u64, prdtl: u32, cmd_hdr_addr: u64) {
    match acmd[0] {
        0x00 => { // TEST UNIT READY
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x03 => { // REQUEST SENSE
            let len = (acmd[4] as usize).min(18);
            let mut s = [0u8; 18];
            s[0] = 0x70; s[7] = 10;
            if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &s[..len]); }
            dma.write_u32(cmd_hdr_addr + 4, len as u32);
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x12 => { // INQUIRY
            let evpd = acmd[1] & 0x01 != 0;
            let page_code = acmd[2];
            let al = ((acmd[3] as usize) << 8) | acmd[4] as usize;
            if evpd {
                match page_code {
                    0x00 => { // Supported VPD pages
                        let d = [0x05u8, 0x00, 0x00, 0x03, 0x00, 0x80, 0x83];
                        let len = al.min(d.len());
                        if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
                        dma.write_u32(cmd_hdr_addr + 4, len as u32);
                    }
                    0x80 => { // Unit Serial Number
                        let d = [0x05u8, 0x80, 0x00, 0x08,
                            b'C', b'V', b'M', b'0', b'0', b'0', b'0', b'1'];
                        let len = al.min(d.len());
                        if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
                        dma.write_u32(cmd_hdr_addr + 4, len as u32);
                    }
                    0x83 => { // Device Identification
                        let d = [0x05u8, 0x83, 0x00, 0x00]; // empty
                        let len = al.min(d.len());
                        if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
                        dma.write_u32(cmd_hdr_addr + 4, len as u32);
                    }
                    _ => {
                        // Unsupported VPD page — return CHECK CONDITION
                        port.tfd = TFD_STS_DRDY | TFD_STS_DSC | 1;
                        port.is |= PORT_IS_DHRS;
                        return;
                    }
                }
            } else {
                let mut d = [0u8; 36];
                d[0] = 0x05; d[1] = 0x80; d[2] = 0x05; d[3] = 0x32; d[4] = 31;
                d[8..16].copy_from_slice(b"CoreVM  ");
                d[16..32].copy_from_slice(b"Virtual CD-ROM  ");
                d[32..36].copy_from_slice(b"1.0 ");
                let len = al.min(36);
                if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
                dma.write_u32(cmd_hdr_addr + 4, len as u32);
            }
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x25 => { // READ CAPACITY
            let secs = port.drive.total_sectors();
            let last = if secs > 0 { (secs - 1) as u32 } else { 0 };
            let bs = port.drive.sector_size() as u32;
            let mut d = [0u8; 8];
            d[0..4].copy_from_slice(&last.to_be_bytes());
            d[4..8].copy_from_slice(&bs.to_be_bytes());
            if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d); }
            dma.write_u32(cmd_hdr_addr + 4, 8);
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x28 => { // READ (10)
            let lba = u32::from_be_bytes([acmd[2], acmd[3], acmd[4], acmd[5]]) as u64;
            let cnt = (u16::from_be_bytes([acmd[7], acmd[8]]) as u32).min(256);
            let ss = port.drive.sector_size();
            let total = cnt as usize * ss;
            let mut buf = vec![0u8; total];
            port.drive.read_at(lba * ss as u64, &mut buf);
            if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &buf); }
            dma.write_u32(cmd_hdr_addr + 4, total as u32);
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x43 => { // READ TOC
            let msf = acmd[1] & 0x02 != 0;
            let format = acmd[2] & 0x0F; // or (acmd[9] >> 6) for some variants
            let al = u16::from_be_bytes([acmd[7], acmd[8]]) as usize;
            let secs = port.drive.total_sectors() as u32;
            match format {
                0 => { // TOC
                    let mut d = vec![0u8; 20];
                    d[0] = 0; d[1] = 18; d[2] = 1; d[3] = 1;
                    d[5] = 0x14; d[6] = 1;
                    if msf { let (m,s,f) = lba_to_msf(0); d[9]=m; d[10]=s; d[11]=f; }
                    d[13] = 0x14; d[14] = 0xAA;
                    if msf { let (m,s,f) = lba_to_msf(secs); d[17]=m; d[18]=s; d[19]=f; }
                    else { d[16..20].copy_from_slice(&secs.to_be_bytes()); }
                    let len = al.min(d.len());
                    if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
                    dma.write_u32(cmd_hdr_addr + 4, len as u32);
                }
                1 => { // Multisession info
                    let mut d = vec![0u8; 12];
                    d[0] = 0; d[1] = 10; // length
                    d[2] = 1; d[3] = 1; // first/last session
                    d[5] = 0x14; d[6] = 1; // track 1
                    // Start address of first track in last session = 0
                    let len = al.min(d.len());
                    if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
                    dma.write_u32(cmd_hdr_addr + 4, len as u32);
                }
                2 => { // Raw TOC (Full TOC)
                    let mut d = vec![0u8; 48];
                    d[0] = 0; d[1] = 46; // length
                    d[2] = 1; d[3] = 1; // first/last session
                    // Point A0 — first track
                    d[5] = 0x01; d[7] = 0xA0; d[11] = 0x01; d[13] = 0x00;
                    // Point A1 — last track
                    d[16+5-11] = 0x01; d[16+7-11] = 0xA1; d[16+11-11] = 0x01;
                    // Point A2 — lead-out
                    let (m,s,f) = lba_to_msf(secs);
                    d[32+5-22] = 0x01; d[32+7-22] = 0xA2; d[32+11-22] = m; d[32+12-22] = s; d[32+13-22] = f;
                    let len = al.min(d.len());
                    if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
                    dma.write_u32(cmd_hdr_addr + 4, len as u32);
                }
                _ => {
                    // Just return format 0 as fallback
                    let mut d = vec![0u8; 20];
                    d[0] = 0; d[1] = 18; d[2] = 1; d[3] = 1;
                    d[5] = 0x14; d[6] = 1;
                    d[13] = 0x14; d[14] = 0xAA;
                    d[16..20].copy_from_slice(&secs.to_be_bytes());
                    let len = al.min(d.len());
                    if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
                    dma.write_u32(cmd_hdr_addr + 4, len as u32);
                }
            }
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x46 => { // GET CONFIGURATION
            let al = u16::from_be_bytes([acmd[7], acmd[8]]) as usize;
            let mut d = [0u8; 8];
            d[3] = 4; d[7] = 0x08;
            let len = al.min(8);
            if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
            dma.write_u32(cmd_hdr_addr + 4, len as u32);
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x4A => { // GET EVENT STATUS
            let mut d = [0u8; 8];
            d[1] = 6; d[2] = 0x04; d[3] = 0x10; d[4] = 0x02;
            if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d); }
            dma.write_u32(cmd_hdr_addr + 4, 8);
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x1A => { // MODE SENSE (6)
            let d = [0u8; 8];
            if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d); }
            dma.write_u32(cmd_hdr_addr + 4, 8);
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x1B | 0x1E => { // START STOP / PREVENT ALLOW
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x51 => { // READ DISC INFORMATION
            let al = u16::from_be_bytes([acmd[7], acmd[8]]) as usize;
            let mut d = [0u8; 34];
            d[0] = 0; d[1] = 32; // length
            d[2] = 0x0E; // last session complete, disc finalized
            d[3] = 1; // first track
            d[4] = 1; // number of sessions (LSB)
            d[5] = 1; // first track in last session (LSB)
            d[6] = 1; // last track in last session (LSB)
            d[7] = 0x20; // unrestricted use
            d[8] = 0x00; // CD-ROM disc type
            let len = al.min(34);
            if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
            dma.write_u32(cmd_hdr_addr + 4, len as u32);
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x5A => { // MODE SENSE (10) — handled separately for proper response
            let page = acmd[2] & 0x3F;
            let al = u16::from_be_bytes([acmd[7], acmd[8]]) as usize;
            let mut d = [0u8; 28];
            // Mode parameter header (8 bytes for MODE SENSE 10)
            d[1] = 6; // mode data length (excluding first 2 bytes)
            d[2] = 0x00; // medium type (CD-ROM)
            // Return minimal response
            let len = al.min(8);
            if prdtl > 0 { dma.write_prdt(prdt_base, prdtl, &d[..len]); }
            dma.write_u32(cmd_hdr_addr + 4, len as u32);
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        0x2A => { // WRITE (10) — CD-ROM is read-only, return error
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC | 1; // ERR bit set
            port.is |= PORT_IS_DHRS;
        }
        0x35 => { // SYNCHRONIZE CACHE
            port.tfd = TFD_STS_DRDY | TFD_STS_DSC;
            port.is |= PORT_IS_DHRS;
        }
        _ => {
            #[cfg(feature = "std")]
            {
                static ATAPI_UNK_DBG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                if ATAPI_UNK_DBG.fetch_add(1, core::sync::atomic::Ordering::Relaxed) < 5 {
                    eprintln!("[ahci-atapi] UNSUPPORTED opcode 0x{:02X}", acmd[0]);
                }
            }
            // Return CHECK CONDITION with sense: ILLEGAL REQUEST (0x05) /
            // INVALID COMMAND OPERATION CODE (ASC=0x20, ASCQ=0x00).
            // TFD byte 0 = Status (CHK=1, DRDY=1), byte 1 = Error (Sense Key << 4)
            port.tfd = ((0x05 << 4) << 8) | TFD_STS_DRDY | 1; // Error=SenseKey=5, Status=DRDY|CHK
            port.is |= PORT_IS_DHRS | PORT_IS_TFES;
        }
    }
}

// ── Helpers ──

fn write_word(buf: &mut [u8], word_idx: usize, val: u16) {
    let off = word_idx * 2;
    if off + 1 < buf.len() { buf[off] = val as u8; buf[off + 1] = (val >> 8) as u8; }
}

fn write_ata_string(buf: &mut [u8], word_idx: usize, s: &[u8], max_bytes: usize) {
    let off = word_idx * 2;
    for i in (0..max_bytes).step_by(2) {
        let b0 = if i < s.len() { s[i] } else { b' ' };
        let b1 = if i + 1 < s.len() { s[i + 1] } else { b' ' };
        if off + i + 1 < buf.len() { buf[off + i] = b1; buf[off + i + 1] = b0; }
    }
}

fn lba_to_msf(lba: u32) -> (u8, u8, u8) {
    let t = lba + 150;
    ((t / 75 / 60) as u8, ((t / 75) % 60) as u8, (t % 75) as u8)
}

/// Create a PCI device entry for the AHCI controller.
/// MSI capability offset in PCI config space for the AHCI controller.
pub const AHCI_MSI_CAP_OFFSET: usize = 0x80;

pub fn create_ahci_pci_device(mmio_base: u32) -> crate::devices::bus::PciDevice {
    let mut dev = crate::devices::bus::PciDevice::new(0x8086, 0x2922, 0x01, 0x06, 0x01);
    dev.set_subsystem(0x8086, 0x2922);
    dev.set_interrupt(11, 1);
    dev.set_bar(5, mmio_base, 0x1000, true);
    // Add MSI capability at offset 0x80 (standard for Intel AHCI controllers)
    dev.add_msi_capability(AHCI_MSI_CAP_OFFSET);
    dev
}
