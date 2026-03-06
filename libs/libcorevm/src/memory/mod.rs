//! Guest memory subsystem for the x86 virtual machine.
//!
//! This module provides the full address-translation pipeline that an x86
//! CPU uses to convert a logical (segment:offset) reference into a physical
//! address:
//!
//! 1. **Segmentation** (`segment`) — applies the segment base, limit, and
//!    access-rights checks to produce a linear address.
//! 2. **Paging** (`paging`) — walks the guest page tables (2-level, PAE, or
//!    4-level) to convert the linear address to a physical address.
//! 3. **Physical memory** (`flat`) — the flat RAM backing store.
//! 4. **MMIO dispatch** (`mmio`) — intercepts physical addresses that belong
//!    to memory-mapped device regions.
//!
//! [`GuestMemory`] ties everything together: it holds the flat RAM plus
//! registered MMIO regions and implements [`MemoryBus`] with automatic MMIO
//! routing. The [`Mmu`] struct tracks paging configuration derived from
//! CR0/CR4/EFER and exposes the high-level `translate` method.

pub mod flat;
pub mod mmio;
pub mod paging;
pub mod segment;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::{Cell, UnsafeCell};
use core::sync::atomic::{AtomicU32, Ordering};

use crate::error::Result;

// ── ROM region ──

/// A read-only memory region mapped at a fixed physical address.
///
/// ROM regions model firmware ROMs (BIOS, VGA BIOS, option ROMs) that
/// occupy specific locations in the physical address space without
/// requiring the flat RAM allocation to extend to those addresses.
/// Reads return the ROM data; writes are silently ignored.
struct RomRegion {
    /// Base physical address of this ROM.
    base: u64,
    /// ROM content.
    data: Vec<u8>,
}

/// Diagnostic: count reads to unmapped memory (above RAM, no MMIO handler).
static UNMAPPED_READ_COUNT: AtomicU32 = AtomicU32::new(0);

/// Self-modifying code detection: tracks which 4 KiB pages have been written.
/// The CPU loop drains this to invalidate decode-cache entries for those pages.
///
/// Uses a simple ring buffer with deduplication of the last page. Since the
/// emulator is single-threaded, no locking is needed beyond the atomics.
pub mod smc {
    use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

    /// Maximum number of unique dirty pages we track before forcing a full flush.
    const MAX_DIRTY: usize = 256;

    /// Ring buffer of dirty page-aligned physical addresses.
    static mut DIRTY_PAGES: [u64; MAX_DIRTY] = [0; MAX_DIRTY];
    /// Number of dirty entries (saturates at MAX_DIRTY+1 to signal "flush all").
    static DIRTY_COUNT: AtomicU32 = AtomicU32::new(0);
    /// Last page marked dirty — used to deduplicate consecutive writes to the
    /// same page (very common with REP STOS/MOVS).
    static LAST_DIRTY_PAGE: AtomicU64 = AtomicU64::new(u64::MAX);

    /// Record a write to physical address `phys`. Called from memory write paths.
    #[inline]
    pub fn mark_page_dirty(phys: u64) {
        let page = phys & !0xFFF;
        // Fast path: skip if same page as last write.
        if LAST_DIRTY_PAGE.load(Ordering::Relaxed) == page {
            return;
        }
        LAST_DIRTY_PAGE.store(page, Ordering::Relaxed);

        let idx = DIRTY_COUNT.load(Ordering::Relaxed) as usize;
        if idx < MAX_DIRTY {
            unsafe { DIRTY_PAGES[idx] = page; }
            DIRTY_COUNT.store((idx + 1) as u32, Ordering::Relaxed);
        } else {
            // Overflow — signal a full flush.
            DIRTY_COUNT.store((MAX_DIRTY + 1) as u32, Ordering::Relaxed);
        }
    }

    /// Result of draining the dirty page buffer.
    pub enum DrainResult {
        /// No pages were dirtied.
        None,
        /// `n` pages were dirtied (copied into the caller's buffer).
        Pages(usize),
        /// Too many pages — caller should flush the entire cache.
        Overflow,
    }

    /// Drain dirty pages into `buf`. Returns how many were copied, or Overflow.
    pub fn drain_to_buf(buf: &mut [u64; MAX_DIRTY]) -> DrainResult {
        let count = DIRTY_COUNT.swap(0, Ordering::Relaxed) as usize;
        if count == 0 {
            return DrainResult::None;
        }
        LAST_DIRTY_PAGE.store(u64::MAX, Ordering::Relaxed);
        if count > MAX_DIRTY {
            return DrainResult::Overflow;
        }
        for i in 0..count {
            buf[i] = unsafe { DIRTY_PAGES[i] };
        }
        DrainResult::Pages(count)
    }
}
/// Diagnostic: count writes to SeaBIOS mmconfig variable at 0xF4DF8.
static MMCFG_WRITE_COUNT: AtomicU32 = AtomicU32::new(0);
use crate::registers::{
    SegmentDescriptor, CR0_PG, CR0_WP, CR4_PAE, CR4_PSE, EFER_LMA, EFER_NXE,
};

pub use flat::FlatMemory;
pub use mmio::{MmioDispatch, MmioHandler, MmioRegion};
pub use paging::walk_page_tables;
pub use segment::segment_translate;

// ── AccessType ──

/// The type of memory access being performed.
///
/// Used by the segmentation and paging layers to enforce access-rights
/// checks and to generate the correct `#PF` error code on violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessType {
    /// Data read.
    Read,
    /// Data write.
    Write,
    /// Instruction fetch.
    Execute,
}

impl AccessType {
    /// Build the x86 `#PF` error code bits for this access type.
    ///
    /// The error code pushed by the CPU on a `#PF` has the following layout
    /// (Intel SDM Vol. 3A, Table 4-12):
    ///
    /// | Bit | Meaning                            |
    /// |-----|------------------------------------|
    /// |  0  | P: 0 = not-present, 1 = protection |
    /// |  1  | W/R: 0 = read, 1 = write           |
    /// |  2  | U/S: 0 = supervisor, 1 = user       |
    /// |  3  | RSVD: reserved-bit violation        |
    /// |  4  | I/D: 0 = data, 1 = instruction fetch|
    ///
    /// # Parameters
    ///
    /// - `cpl`: Current privilege level (bit 2 set if CPL=3).
    /// - `present`: Whether the page was present (bit 0 set if true).
    pub fn to_pf_error_code(self, cpl: u8, present: bool) -> u32 {
        let mut code: u32 = 0;
        if present {
            code |= 1 << 0; // P bit
        }
        match self {
            AccessType::Write => code |= 1 << 1,   // W/R bit
            AccessType::Execute => code |= 1 << 4,  // I/D bit
            AccessType::Read => {}
        }
        if cpl == 3 {
            code |= 1 << 2; // U/S bit
        }
        code
    }
}

// ── MemoryBus trait ──

/// Trait for reading and writing guest physical memory.
///
/// All multi-byte operations use **little-endian** byte order, matching the
/// x86 memory model. Implementations may be backed by flat RAM, MMIO
/// dispatch, or a combination of both.
pub trait MemoryBus {
    /// Read a single byte from physical address `addr`.
    fn read_u8(&self, addr: u64) -> Result<u8>;

    /// Read a 16-bit little-endian value from physical address `addr`.
    fn read_u16(&self, addr: u64) -> Result<u16>;

    /// Read a 32-bit little-endian value from physical address `addr`.
    fn read_u32(&self, addr: u64) -> Result<u32>;

    /// Read a 64-bit little-endian value from physical address `addr`.
    fn read_u64(&self, addr: u64) -> Result<u64>;

    /// Write a single byte to physical address `addr`.
    fn write_u8(&mut self, addr: u64, val: u8) -> Result<()>;

    /// Write a 16-bit little-endian value to physical address `addr`.
    fn write_u16(&mut self, addr: u64, val: u16) -> Result<()>;

    /// Write a 32-bit little-endian value to physical address `addr`.
    fn write_u32(&mut self, addr: u64, val: u32) -> Result<()>;

    /// Write a 64-bit little-endian value to physical address `addr`.
    fn write_u64(&mut self, addr: u64, val: u64) -> Result<()>;

    /// Read `buf.len()` bytes starting from `addr` into `buf`.
    fn read_bytes(&self, addr: u64, buf: &mut [u8]) -> Result<()>;

    /// Write all bytes from `buf` starting at `addr`.
    fn write_bytes(&mut self, addr: u64, buf: &[u8]) -> Result<()>;
}

// ── GuestMemory ──

/// Composite guest physical memory: flat RAM plus MMIO regions.
///
/// Reads and writes are first checked against registered MMIO regions;
/// if no MMIO region matches, the access falls through to flat RAM.
///
/// `UnsafeCell` is used for the MMIO dispatch because device handlers
/// are stateful (`read`/`write` take `&mut self`), but the `MemoryBus`
/// trait requires `&self` for reads (used by paging, decode, etc.).
/// Safety: the emulator is single-threaded and non-re-entrant.
pub struct GuestMemory {
    /// Flat guest RAM.
    ram: FlatMemory,
    /// MMIO region dispatcher (interior mutability for `&self` read path).
    mmio: UnsafeCell<MmioDispatch>,
    /// Read-only ROM regions (BIOS, VGA BIOS, option ROMs) mapped at
    /// arbitrary physical addresses outside (or overlapping) flat RAM.
    /// Checked after MMIO, before flat RAM on reads. Writes are ignored.
    rom_regions: Vec<RomRegion>,
}

impl GuestMemory {
    /// Create a new guest memory with `ram_size` bytes of zeroed RAM.
    pub fn new(ram_size: usize) -> Self {
        GuestMemory {
            ram: FlatMemory::new(ram_size),
            mmio: UnsafeCell::new(MmioDispatch::new()),
            rom_regions: Vec::new(),
        }
    }

    /// Get a mutable reference to the MMIO dispatch.
    ///
    /// # Safety
    ///
    /// Safe because the emulator is single-threaded and MMIO handlers
    /// are non-re-entrant.
    fn mmio_mut(&self) -> &mut MmioDispatch {
        unsafe { &mut *self.mmio.get() }
    }

    /// Copy `data` into guest RAM starting at `offset`.
    ///
    /// This bypasses MMIO routing and writes directly to the flat RAM
    /// backing store. Used for loading BIOS, kernels, or initial data
    /// before the VM starts executing.
    ///
    /// # Panics
    ///
    /// Panics if `offset + data.len()` exceeds the RAM size.
    pub fn load_at(&mut self, offset: usize, data: &[u8]) {
        self.ram.load_at(offset, data);
    }

    /// Register an MMIO region at `base` with `size` bytes.
    ///
    /// Subsequent reads/writes to physical addresses in `[base, base+size)`
    /// will be routed to `handler` instead of flat RAM.
    pub fn add_mmio(&mut self, base: u64, size: u64, handler: Box<dyn MmioHandler>) {
        self.mmio.get_mut().register(base, size, handler);
    }

    /// Borrow the underlying flat RAM.
    pub fn ram(&self) -> &FlatMemory {
        &self.ram
    }

    /// Mutably borrow the underlying flat RAM.
    pub fn ram_mut(&mut self) -> &mut FlatMemory {
        &mut self.ram
    }

    /// Return the number of registered MMIO regions (diagnostic).
    pub fn mmio_region_count(&self) -> usize {
        // Safety: single-threaded, non-re-entrant.
        unsafe { &*self.mmio.get() }.region_count()
    }

    /// Return the MMIO fast-reject bounds (diagnostic).
    pub fn mmio_bounds(&self) -> (u64, u64) {
        unsafe { &*self.mmio.get() }.bounds()
    }

    /// Map a read-only ROM at the given physical base address.
    ///
    /// Subsequent reads to `[base, base + data.len())` return ROM content.
    /// Writes to this range are silently ignored (ROM is read-only).
    /// Multiple ROMs can be mapped at different addresses; they must not
    /// overlap each other (behavior is undefined if they do).
    ///
    /// This is the mechanism used to place firmware ROMs (SeaBIOS at
    /// 0xFFFC0000, VGA BIOS at 0xC0000, shadow copy at 0xE0000) without
    /// allocating a full 4 GiB flat RAM buffer.
    pub fn add_rom(&mut self, base: u64, data: Vec<u8>) {
        self.rom_regions.push(RomRegion { base, data });
    }

    /// Look up a ROM region containing `addr` and return `(offset, &RomRegion)`.
    #[inline]
    fn find_rom(&self, addr: u64) -> Option<(usize, &RomRegion)> {
        for rom in &self.rom_regions {
            let end = rom.base + rom.data.len() as u64;
            if addr >= rom.base && addr < end {
                return Some(((addr - rom.base) as usize, rom));
            }
        }
        None
    }

    /// Check if a write targets a ROM region (should be silently ignored).
    #[inline]
    fn is_rom_addr(&self, addr: u64) -> bool {
        for rom in &self.rom_regions {
            let end = rom.base + rom.data.len() as u64;
            if addr >= rom.base && addr < end {
                return true;
            }
        }
        false
    }
}

/// Helper: dispatch an MMIO read or fall through to RAM.
///
/// Returns `Some(value)` if the address hit an MMIO region, `None` otherwise.
fn try_mmio_read(mmio: &mut MmioDispatch, addr: u64, size: u8) -> Option<Result<u64>> {
    if let Some(region) = mmio.find(addr) {
        let offset = addr - region.base;
        Some(region.handler.read(offset, size))
    } else {
        None
    }
}

/// Helper: dispatch an MMIO write or fall through to RAM.
///
/// Returns `Some(result)` if the address hit an MMIO region, `None` otherwise.
fn try_mmio_write(
    mmio: &mut MmioDispatch,
    addr: u64,
    size: u8,
    val: u64,
) -> Option<Result<()>> {
    if let Some(region) = mmio.find(addr) {
        let offset = addr - region.base;
        Some(region.handler.write(offset, size, val))
    } else {
        None
    }
}

impl MemoryBus for GuestMemory {
    fn read_u8(&self, addr: u64) -> Result<u8> {
        // 1. MMIO
        if let Some(res) = try_mmio_read(self.mmio_mut(), addr, 1) {
            return Ok(res? as u8);
        }
        // 2. ROM regions
        if let Some((off, rom)) = self.find_rom(addr) {
            return Ok(rom.data[off]);
        }
        // 3. Flat RAM
        self.ram.read_u8(addr)
    }

    fn read_u16(&self, addr: u64) -> Result<u16> {
        // 1. MMIO
        if let Some(res) = try_mmio_read(self.mmio_mut(), addr, 2) {
            return Ok(res? as u16);
        }
        // 2. ROM regions
        if let Some((off, rom)) = self.find_rom(addr) {
            if off + 2 <= rom.data.len() {
                let bytes: [u8; 2] = [rom.data[off], rom.data[off + 1]];
                return Ok(u16::from_le_bytes(bytes));
            }
        }
        // 3. Flat RAM (with diagnostic for unmapped)
        if addr as usize >= self.ram.size() {
            let n = UNMAPPED_READ_COUNT.fetch_add(1, Ordering::Relaxed);
            if n < 50 {
                libsyscall::serial_print(format_args!(
                    "[mem-diag] unmapped read16 #{}: addr=0x{:08X}\n", n, addr
                ));
            }
        }
        self.ram.read_u16(addr)
    }

    fn read_u32(&self, addr: u64) -> Result<u32> {
        // 1. MMIO
        if let Some(res) = try_mmio_read(self.mmio_mut(), addr, 4) {
            return Ok(res? as u32);
        }
        // 2. ROM regions
        if let Some((off, rom)) = self.find_rom(addr) {
            if off + 4 <= rom.data.len() {
                let bytes: [u8; 4] = [
                    rom.data[off], rom.data[off + 1],
                    rom.data[off + 2], rom.data[off + 3],
                ];
                return Ok(u32::from_le_bytes(bytes));
            }
        }
        // 3. Flat RAM (with diagnostic for unmapped)
        if addr as usize >= self.ram.size() {
            let n = UNMAPPED_READ_COUNT.fetch_add(1, Ordering::Relaxed);
            if n < 50 {
                libsyscall::serial_print(format_args!(
                    "[mem-diag] unmapped read32 #{}: addr=0x{:08X}\n", n, addr
                ));
            }
        }
        self.ram.read_u32(addr)
    }

    fn read_u64(&self, addr: u64) -> Result<u64> {
        // 1. MMIO
        if let Some(res) = try_mmio_read(self.mmio_mut(), addr, 8) {
            return res;
        }
        // 2. ROM regions
        if let Some((off, rom)) = self.find_rom(addr) {
            if off + 8 <= rom.data.len() {
                let bytes: [u8; 8] = [
                    rom.data[off], rom.data[off + 1],
                    rom.data[off + 2], rom.data[off + 3],
                    rom.data[off + 4], rom.data[off + 5],
                    rom.data[off + 6], rom.data[off + 7],
                ];
                return Ok(u64::from_le_bytes(bytes));
            }
        }
        // 3. Flat RAM
        self.ram.read_u64(addr)
    }

    fn write_u8(&mut self, addr: u64, val: u8) -> Result<()> {
        if let Some(res) = try_mmio_write(self.mmio_mut(), addr, 1, val as u64) {
            return res;
        }
        // Writes to ROM are silently ignored.
        if self.is_rom_addr(addr) {
            return Ok(());
        }
        let result = self.ram.write_u8(addr, val);
        if result.is_ok() {
            smc::mark_page_dirty(addr);
        }
        result
    }

    fn write_u16(&mut self, addr: u64, val: u16) -> Result<()> {
        if let Some(res) = try_mmio_write(self.mmio_mut(), addr, 2, val as u64) {
            return res;
        }
        if self.is_rom_addr(addr) {
            return Ok(());
        }
        let result = self.ram.write_u16(addr, val);
        if result.is_ok() {
            smc::mark_page_dirty(addr);
        }
        result
    }

    fn write_u32(&mut self, addr: u64, val: u32) -> Result<()> {
        if let Some(res) = try_mmio_write(self.mmio_mut(), addr, 4, val as u64) {
            return res;
        }
        if self.is_rom_addr(addr) {
            return Ok(());
        }
        let result = self.ram.write_u32(addr, val);
        if result.is_ok() {
            smc::mark_page_dirty(addr);
        }
        result
    }

    fn write_u64(&mut self, addr: u64, val: u64) -> Result<()> {
        if let Some(res) = try_mmio_write(self.mmio_mut(), addr, 8, val) {
            return res;
        }
        if self.is_rom_addr(addr) {
            return Ok(());
        }
        let result = self.ram.write_u64(addr, val);
        if result.is_ok() {
            smc::mark_page_dirty(addr);
        }
        result
    }

    fn read_bytes(&self, addr: u64, buf: &mut [u8]) -> Result<()> {
        // Check if the read falls within a ROM region.
        if let Some((off, rom)) = self.find_rom(addr) {
            let avail = rom.data.len() - off;
            if buf.len() <= avail {
                buf.copy_from_slice(&rom.data[off..off + buf.len()]);
                return Ok(());
            }
            // Partial ROM hit — fill what we can, rest from RAM.
            buf[..avail].copy_from_slice(&rom.data[off..]);
            buf[avail..].fill(0xFF);
            return Ok(());
        }
        self.ram.read_bytes(addr, buf)
    }

    fn write_bytes(&mut self, addr: u64, buf: &[u8]) -> Result<()> {
        // Writes to ROM are silently ignored.
        if self.is_rom_addr(addr) {
            return Ok(());
        }
        self.ram.write_bytes(addr, buf)
    }
}

// ── Mmu ──

/// Memory Management Unit state derived from control registers.
///
/// Tracks the paging mode and protection flags that affect address
/// translation. Updated whenever the guest writes to CR0, CR4, or EFER
/// via the `update_from_regs` method.
pub struct Mmu {
    /// CR0.PG — paging is enabled.
    pub paging_enabled: bool,
    /// CR4.PSE — page size extensions (4 MiB pages in 32-bit mode).
    pub pse: bool,
    /// CR4.PAE — physical address extension.
    pub pae: bool,
    /// EFER.LMA — long mode active (64-bit paging).
    pub long_mode: bool,
    /// CR0.WP — write protect (supervisor writes to RO user pages fault).
    pub wp: bool,
    /// EFER.NXE — no-execute enable.
    pub nxe: bool,
    /// Cached CR0/CR4/EFER for change detection.
    cached_cr0: u64,
    /// Cached CR4 value.
    cached_cr4: u64,
    /// Cached EFER value.
    cached_efer: u64,
    /// Last CR3 value seen by the translation fast path.
    tlb_last_cr3: Cell<u64>,
    /// Direct-mapped TLB validity/permission flags per entry.
    tlb_valid: UnsafeCell<[u8; 256]>,
    /// Direct-mapped TLB tag: linear page number.
    tlb_linear_page: UnsafeCell<[u64; 256]>,
    /// Direct-mapped TLB tag: CR3 value used for translation.
    tlb_cr3: UnsafeCell<[u64; 256]>,
    /// Direct-mapped TLB payload: physical page number.
    tlb_phys_page: UnsafeCell<[u64; 256]>,
}

impl Mmu {
    /// Create a new MMU with all features disabled (real-mode defaults).
    pub fn new() -> Self {
        Mmu {
            paging_enabled: false,
            pse: false,
            pae: false,
            long_mode: false,
            wp: false,
            nxe: false,
            cached_cr0: 0,
            cached_cr4: 0,
            cached_efer: 0,
            tlb_last_cr3: Cell::new(0),
            tlb_valid: UnsafeCell::new([0; 256]),
            tlb_linear_page: UnsafeCell::new([0; 256]),
            tlb_cr3: UnsafeCell::new([0; 256]),
            tlb_phys_page: UnsafeCell::new([0; 256]),
        }
    }

    /// Flush the internal linear→physical translation cache.
    #[inline]
    pub fn flush_tlb(&self) {
        unsafe { (*self.tlb_valid.get()).fill(0) };
    }

    /// Synchronize MMU state from the current CR0, CR4, and EFER values.
    ///
    /// Uses cached values to skip the update when nothing changed (which
    /// is the common case — control registers are written very rarely).
    #[inline]
    pub fn update_from_regs(&mut self, cr0: u64, cr4: u64, efer: u64) {
        if cr0 == self.cached_cr0 && cr4 == self.cached_cr4 && efer == self.cached_efer {
            return;
        }
        self.cached_cr0 = cr0;
        self.cached_cr4 = cr4;
        self.cached_efer = efer;
        self.flush_tlb();
        self.paging_enabled = (cr0 & CR0_PG) != 0;
        self.wp = (cr0 & CR0_WP) != 0;
        self.pse = (cr4 & CR4_PSE) != 0;
        self.pae = (cr4 & CR4_PAE) != 0;
        self.long_mode = (efer & EFER_LMA) != 0;
        self.nxe = (efer & EFER_NXE) != 0;
    }

    /// Translate a logical address (segment descriptor + offset) to a physical address.
    ///
    /// This is the main entry point used by the instruction executor:
    ///
    /// 1. Apply segmentation to produce a linear address.
    /// 2. If paging is enabled, walk the page tables to produce a physical address.
    /// 3. Otherwise, the linear address *is* the physical address.
    ///
    /// # Parameters
    ///
    /// - `seg`: The cached segment descriptor for this access.
    /// - `offset`: Offset within the segment.
    /// - `cr3`: Page directory base register (only used when paging is on).
    /// - `access`: Read, Write, or Execute.
    /// - `cpl`: Current privilege level.
    /// - `mem`: Guest physical memory bus (for reading page table entries).
    pub fn translate(
        &self,
        seg: &SegmentDescriptor,
        offset: u64,
        cr3: u64,
        access: AccessType,
        cpl: u8,
        mem: &dyn MemoryBus,
    ) -> Result<u64> {
        let linear = segment_translate(seg, offset, access, cpl, self.long_mode)?;
        self.translate_linear(linear, cr3, access, cpl, mem)
    }

    /// Translate a linear (virtual) address to a physical address via paging.
    ///
    /// If paging is disabled, the linear address is returned unchanged.
    /// Otherwise, the appropriate page-table walker is invoked.
    ///
    /// # Parameters
    ///
    /// - `linear`: Linear address (output of segmentation).
    /// - `cr3`: Page directory base register.
    /// - `access`: Read, Write, or Execute.
    /// - `cpl`: Current privilege level.
    /// - `mem`: Guest physical memory bus.
    #[inline]
    pub fn translate_linear(
        &self,
        linear: u64,
        cr3: u64,
        access: AccessType,
        cpl: u8,
        mem: &dyn MemoryBus,
    ) -> Result<u64> {
        if !self.paging_enabled {
            return Ok(linear);
        }

        // CR3 reload invalidates non-global TLB entries. We model this
        // conservatively by flushing the full software TLB on CR3 change.
        if self.tlb_last_cr3.get() != cr3 {
            self.flush_tlb();
            self.tlb_last_cr3.set(cr3);
        }

        let linear_page = linear >> 12;
        let offset = linear & 0xFFF;
        let idx = ((linear_page ^ (cr3 >> 12)) as usize) & 0xFF;
        let access_mask = match access {
            AccessType::Read => 0x1,
            AccessType::Write => 0x2,
            AccessType::Execute => 0x4,
        };

        unsafe {
            let valid = &*self.tlb_valid.get();
            let lin = &*self.tlb_linear_page.get();
            let tag_cr3 = &*self.tlb_cr3.get();
            let phys = &*self.tlb_phys_page.get();

            if (valid[idx] & access_mask) != 0 && lin[idx] == linear_page && tag_cr3[idx] == cr3 {
                return Ok((phys[idx] << 12) | offset);
            }
        }

        let translated = walk_page_tables(linear, cr3, access, cpl, self, mem)?;
        let phys_page = translated >> 12;

        unsafe {
            let valid = &mut *self.tlb_valid.get();
            let lin = &mut *self.tlb_linear_page.get();
            let tag_cr3 = &mut *self.tlb_cr3.get();
            let phys = &mut *self.tlb_phys_page.get();

            // Keep cached permission bits for the same mapping to avoid
            // unnecessary walk repeats when access type varies.
            if lin[idx] == linear_page && tag_cr3[idx] == cr3 && phys[idx] == phys_page {
                valid[idx] |= access_mask;
            } else {
                lin[idx] = linear_page;
                tag_cr3[idx] = cr3;
                phys[idx] = phys_page;
                valid[idx] = access_mask;
            }
        }

        Ok(translated)
    }
}
