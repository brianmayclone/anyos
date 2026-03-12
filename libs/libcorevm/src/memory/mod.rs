//! Guest memory subsystem for the virtual machine.
//!
//! This module provides:
//!
//! 1. **Physical memory** (`flat`) — the flat RAM backing store.
//! 2. **MMIO dispatch** (`mmio`) — intercepts physical addresses that belong
//!    to memory-mapped device regions.
//! 3. **Segmentation** (`segment`) — segment descriptor utilities.
//!
//! [`GuestMemory`] ties everything together: it holds the flat RAM plus
//! registered MMIO regions and implements [`MemoryBus`] with automatic MMIO
//! routing.

pub mod flat;
pub mod mmio;
pub mod segment;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
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

pub use flat::FlatMemory;
pub use mmio::{MmioDispatch, MmioHandler, MmioRegion};
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

    /// Dispatch an MMIO read to the registered handler.
    ///
    /// Returns the value read, or `None` if no handler covers `addr`.
    /// Used by the VM exit dispatcher for hypervisor MMIO exits.
    pub fn dispatch_mmio_read(&self, addr: u64, size: u8) -> Option<u64> {
        let mmio = self.mmio_mut();
        if let Some(region) = mmio.find(addr) {
            let offset = addr - region.base;
            region.handler.read(offset, size).ok()
        } else {
            None
        }
    }

    /// Dispatch an MMIO write to the registered handler.
    ///
    /// Returns `true` if a handler was found and the write was dispatched.
    /// Used by the VM exit dispatcher for hypervisor MMIO exits.
    pub fn dispatch_mmio_write(&self, addr: u64, size: u8, val: u64) -> bool {
        let mmio = self.mmio_mut();
        if let Some(region) = mmio.find(addr) {
            let offset = addr - region.base;
            let _ = region.handler.write(offset, size, val);
            true
        } else {
            false
        }
    }

    /// Total RAM size in bytes.
    #[inline(always)]
    pub fn ram_size(&self) -> usize {
        self.ram.size()
    }

    /// Direct RAM pointer and size for fast-path access.
    #[inline(always)]
    pub fn ram_ptr(&self) -> (*const u8, usize) {
        (self.ram.as_slice().as_ptr(), self.ram.size())
    }

    /// Direct mutable RAM pointer and size for fast-path access.
    #[inline(always)]
    pub fn ram_mut_ptr(&mut self) -> (*mut u8, usize) {
        (self.ram.as_mut_slice().as_mut_ptr(), self.ram.size())
    }

    /// Check if an address could be in an MMIO region (fast reject).
    #[inline(always)]
    fn is_possible_mmio(&self, addr: u64) -> bool {
        let (min_base, max_end) = unsafe { &*self.mmio.get() }.bounds();
        addr >= min_base && addr < max_end
    }

    /// Fast-path read u8: skip MMIO/ROM checks for plain RAM addresses.
    #[inline(always)]
    pub fn fast_read_u8(&self, addr: u64) -> u8 {
        let a = addr as usize;
        if a < self.ram.size() && !self.is_possible_mmio(addr) {
            unsafe { *self.ram.as_slice().as_ptr().add(a) }
        } else {
            self.read_u8(addr).unwrap_or(0xFF)
        }
    }

    /// Fast-path read u16: skip MMIO/ROM checks for plain RAM addresses.
    #[inline(always)]
    pub fn fast_read_u16(&self, addr: u64) -> u16 {
        let a = addr as usize;
        if a + 2 <= self.ram.size() && !self.is_possible_mmio(addr) {
            unsafe { (self.ram.as_slice().as_ptr().add(a) as *const u16).read_unaligned() }
        } else {
            self.read_u16(addr).unwrap_or(0xFFFF)
        }
    }

    /// Fast-path read u32: skip MMIO/ROM checks for plain RAM addresses.
    #[inline(always)]
    pub fn fast_read_u32(&self, addr: u64) -> u32 {
        let a = addr as usize;
        if a + 4 <= self.ram.size() && !self.is_possible_mmio(addr) {
            unsafe { (self.ram.as_slice().as_ptr().add(a) as *const u32).read_unaligned() }
        } else {
            self.read_u32(addr).unwrap_or(0xFFFF_FFFF)
        }
    }

    /// Fast-path write u8: skip MMIO/ROM checks for plain RAM addresses.
    #[inline(always)]
    pub fn fast_write_u8(&mut self, addr: u64, val: u8) {
        let a = addr as usize;
        if a < self.ram.size() && !self.is_possible_mmio(addr) {
            unsafe { *self.ram.as_mut_slice().as_mut_ptr().add(a) = val; }

        } else {
            let _ = self.write_u8(addr, val);
        }
    }

    /// Fast-path write u16: skip MMIO/ROM checks for plain RAM addresses.
    #[inline(always)]
    pub fn fast_write_u16(&mut self, addr: u64, val: u16) {
        let a = addr as usize;
        if a + 2 <= self.ram.size() && !self.is_possible_mmio(addr) {
            unsafe { (self.ram.as_mut_slice().as_mut_ptr().add(a) as *mut u16).write_unaligned(val); }

        } else {
            let _ = self.write_u16(addr, val);
        }
    }

    /// Fast-path write u32: skip MMIO/ROM checks for plain RAM addresses.
    #[inline(always)]
    pub fn fast_write_u32(&mut self, addr: u64, val: u32) {
        let a = addr as usize;
        if a + 4 <= self.ram.size() && !self.is_possible_mmio(addr) {
            unsafe { (self.ram.as_mut_slice().as_mut_ptr().add(a) as *mut u32).write_unaligned(val); }

        } else {
            let _ = self.write_u32(addr, val);
        }
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
                #[cfg(feature = "anyos")]
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
                #[cfg(feature = "anyos")]
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

