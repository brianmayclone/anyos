//! VMware SVGA II GPU driver.
//!
//! PCI device: vendor 0x15AD, device 0x0405. Provides 2D acceleration
//! (RECT_FILL, RECT_COPY), hardware cursor, and FIFO command queue for
//! GPU communication. References: VMware SVGA Device Developer Kit,
//! QEMU hw/display/vmware_vga.c.

use super::GpuDriver;
use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::drivers::pci::PciDevice;
use crate::memory::address::VirtAddr;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

// ──────────────────────────────────────────────
// SVGA Register indices (I/O indexed access)
// ──────────────────────────────────────────────
const SVGA_REG_ID: u32 = 0;
const SVGA_REG_ENABLE: u32 = 1;
const SVGA_REG_WIDTH: u32 = 2;
const SVGA_REG_HEIGHT: u32 = 3;
const SVGA_REG_MAX_WIDTH: u32 = 4;
const SVGA_REG_MAX_HEIGHT: u32 = 5;
const SVGA_REG_BPP: u32 = 7;
const SVGA_REG_FB_START: u32 = 13;
const SVGA_REG_FB_OFFSET: u32 = 14;
const SVGA_REG_VRAM_SIZE: u32 = 15;
const SVGA_REG_FB_SIZE: u32 = 16;
const SVGA_REG_CAPABILITIES: u32 = 17;
const SVGA_REG_FIFO_START: u32 = 18;
const SVGA_REG_FIFO_SIZE: u32 = 19;
const SVGA_REG_CONFIG_DONE: u32 = 20;
const SVGA_REG_SYNC: u32 = 21;
const SVGA_REG_BUSY: u32 = 22;
const SVGA_REG_BYTES_PER_LINE: u32 = 12;

// Version negotiation IDs
const SVGA_ID_2: u32 = 0x9000_0002;

// Capabilities (from VMware svga_reg.h)
const SVGA_CAP_RECT_FILL: u32          = 1 << 0;
const SVGA_CAP_RECT_COPY: u32          = 1 << 1;
const SVGA_CAP_CURSOR: u32             = 1 << 5;
const SVGA_CAP_CURSOR_BYPASS: u32      = 1 << 6;
const SVGA_CAP_CURSOR_BYPASS_2: u32    = 1 << 7;
const SVGA_CAP_8BIT_EMULATION: u32     = 1 << 8;
const SVGA_CAP_ALPHA_CURSOR: u32       = 1 << 9;
const SVGA_CAP_3D: u32                 = 1 << 14;
const SVGA_CAP_EXTENDED_FIFO: u32      = 1 << 15;
const SVGA_CAP_PITCHLOCK: u32          = 1 << 17;
const SVGA_CAP_IRQMASK: u32            = 1 << 18;
const SVGA_CAP_GMR: u32                = 1 << 20;
const SVGA_CAP_TRACES: u32             = 1 << 21;
const SVGA_CAP_GMR2: u32               = 1 << 22;
const SVGA_CAP_SCREEN_OBJECT_2: u32    = 1 << 23;

// Screen Object flags
const SVGA_SCREEN_MUST_BE_SET: u32     = 1 << 0;
const SVGA_SCREEN_IS_PRIMARY: u32      = 1 << 1;

// Special GMR IDs
const SVGA_GMR_FRAMEBUFFER: u32        = 0xFFFF_FFFE;

// FIFO register offsets (in u32 units)
const SVGA_FIFO_MIN: usize = 0;
const SVGA_FIFO_MAX: usize = 1;
const SVGA_FIFO_NEXT_CMD: usize = 2;
const SVGA_FIFO_STOP: usize = 3;
const SVGA_FIFO_CAPABILITIES: usize = 4;
const SVGA_FIFO_FLAGS: usize = 5;

// FIFO extended registers
const SVGA_FIFO_FENCE: usize = 6;
#[allow(dead_code)]
const SVGA_FIFO_3D_HWVERSION: usize = 7;

// FIFO cursor bypass registers (host writes cursor position here)
// Note: in extended FIFO mode, cursor registers are at offsets 9-12
const SVGA_FIFO_CURSOR_ON: usize = 9;
const SVGA_FIFO_CURSOR_X: usize = 10;
const SVGA_FIFO_CURSOR_Y: usize = 11;
const SVGA_FIFO_CURSOR_COUNT: usize = 12;
const SVGA_FIFO_FENCE_GOAL: usize = 289;
const SVGA_FIFO_BUSY: usize = 290;

// FIFO capability flags
const SVGA_FIFO_CAP_FENCE: u32          = 1 << 0;
#[allow(dead_code)]
const SVGA_FIFO_CAP_ACCELFRONT: u32     = 1 << 1;
const SVGA_FIFO_CAP_CURSOR_BYPASS_3: u32 = 1 << 4;
const SVGA_FIFO_CAP_RESERVE: u32        = 1 << 6;
const SVGA_FIFO_CAP_SCREEN_OBJECT: u32  = 1 << 7;
const SVGA_FIFO_CAP_GMR2: u32           = 1 << 8;
const SVGA_FIFO_CAP_SCREEN_OBJECT_2: u32 = 1 << 9;

// Cursor I/O registers (guest writes cursor position here for cursor bypass 1/2)
const SVGA_REG_CURSOR_ID: u32 = 24;
const SVGA_REG_CURSOR_X: u32 = 25;
const SVGA_REG_CURSOR_Y: u32 = 26;
const SVGA_REG_CURSOR_ON: u32 = 27;

// FIFO reserved registers
const SVGA_FIFO_NUM_REGS: usize = 293;

// ── Cursor bypass global state ──────────────────────
/// FIFO virtual address for direct cursor register access (bypasses GPU trait)
static SVGA_FIFO_VIRT: AtomicU64 = AtomicU64::new(0);
/// Whether FIFO cursor bypass 3 is available (host writes cursor pos to FIFO)
static CURSOR_BYPASS_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Last seen CURSOR_COUNT value (to detect position changes)
static LAST_CURSOR_COUNT: AtomicU32 = AtomicU32::new(0);

// ── IRQ handler static state ─────────────────────
/// I/O base address for IRQ handler to read/write IRQSTATUS port.
static SVGA_IO_BASE_IRQ: AtomicU32 = AtomicU32::new(0);
/// Pending IRQ flags (accumulated by handler, consumed by fence sync).
static SVGA_IRQ_PENDING: AtomicU32 = AtomicU32::new(0);
/// Next fence value to insert (monotonically increasing, starts at 1).
static SVGA_NEXT_FENCE: AtomicU32 = AtomicU32::new(1);
/// TID of thread waiting for fence completion (0 = none).
static SVGA_FENCE_WAITER: AtomicU32 = AtomicU32::new(0);
/// Whether IRQ-driven sync is available and active.
static SVGA_IRQ_ENABLED: AtomicBool = AtomicBool::new(false);

// Additional registers
const SVGA_REG_IRQMASK: u32 = 33;
const SVGA_REG_GMR_MAX_IDS: u32 = 43;
const SVGA_REG_GMR_MAX_DESCRIPTOR_LENGTH: u32 = 44;
const SVGA_REG_GMRS_MAX_PAGES: u32 = 46;

// IRQ status port offset from io_base (read to get pending, write to acknowledge)
const SVGA_IRQSTATUS_OFFSET: u16 = 0x8;

// IRQ flag bits
const SVGA_IRQFLAG_ANY_FENCE: u32      = 0x1;
const SVGA_IRQFLAG_FIFO_PROGRESS: u32  = 0x2;
const SVGA_IRQFLAG_FENCE_GOAL: u32     = 0x4;

// FIFO command opcodes
const SVGA_CMD_UPDATE: u32 = 1;
const SVGA_CMD_RECT_FILL: u32 = 2;
const SVGA_CMD_RECT_COPY: u32 = 3;
const SVGA_CMD_DEFINE_CURSOR: u32 = 19;
const SVGA_CMD_DEFINE_ALPHA_CURSOR: u32 = 22;
const SVGA_CMD_FENCE: u32 = 30;
const SVGA_CMD_DEFINE_SCREEN: u32 = 34;
#[allow(dead_code)]
const SVGA_CMD_DESTROY_SCREEN: u32 = 35;
const SVGA_CMD_DEFINE_GMRFB: u32 = 36;
const SVGA_CMD_BLIT_GMRFB_TO_SCREEN: u32 = 37;
#[allow(dead_code)]
const SVGA_CMD_BLIT_SCREEN_TO_GMRFB: u32 = 38;
const SVGA_CMD_DEFINE_GMR2: u32 = 41;
const SVGA_CMD_REMAP_GMR2: u32 = 42;

// ── SVGA3D Command opcodes ──────────────────────────────
const SVGA_3D_CMD_SURFACE_DEFINE: u32     = 1040;
const SVGA_3D_CMD_SURFACE_DESTROY: u32    = 1041;
const SVGA_3D_CMD_SURFACE_COPY: u32       = 1042;
const SVGA_3D_CMD_SURFACE_STRETCHBLT: u32 = 1043;
const SVGA_3D_CMD_SURFACE_DMA: u32        = 1044;
const SVGA_3D_CMD_CONTEXT_DEFINE: u32     = 1045;
const SVGA_3D_CMD_CONTEXT_DESTROY: u32    = 1046;
const SVGA_3D_CMD_SETTRANSFORM: u32       = 1047;
const SVGA_3D_CMD_SETZRANGE: u32          = 1048;
const SVGA_3D_CMD_SETRENDERSTATE: u32     = 1049;
const SVGA_3D_CMD_SETRENDERTARGET: u32    = 1050;
const SVGA_3D_CMD_SETTEXTURESTATE: u32    = 1051;
const SVGA_3D_CMD_SETVIEWPORT: u32        = 1055;
const SVGA_3D_CMD_CLEAR: u32              = 1057;
const SVGA_3D_CMD_PRESENT: u32            = 1058;
const SVGA_3D_CMD_SHADER_DEFINE: u32      = 1059;
const SVGA_3D_CMD_SHADER_DESTROY: u32     = 1060;
const SVGA_3D_CMD_SET_SHADER: u32         = 1061;
const SVGA_3D_CMD_SET_SHADER_CONST: u32   = 1062;
const SVGA_3D_CMD_DRAW_PRIMITIVES: u32    = 1063;
const SVGA_3D_CMD_SETSCISSORRECT: u32     = 1064;
#[allow(dead_code)]
const SVGA_3D_CMD_PRESENT_READBACK: u32   = 1070;

// Valid range for SVGA3D commands (used for validation in syscall handler)
pub const SVGA_3D_CMD_MIN: u32 = 1040;
pub const SVGA_3D_CMD_MAX: u32 = 1099;

// Virtual address for FIFO mapping (kernel higher-half MMIO region)
const FIFO_VIRT_BASE: u64 = 0xFFFF_FFFF_D002_0000;
const FIFO_MAP_PAGES: usize = 64; // 256 KiB

// FIFO bounce buffer size for reserve/commit (identity-mapped DMA memory)
const FIFO_BOUNCE_SIZE: usize = 16384; // 16 KiB = 4 pages
const FIFO_BOUNCE_PAGES: usize = FIFO_BOUNCE_SIZE / 4096;

/// VMware SVGA II GPU driver state, including I/O base, FIFO mapping, and capabilities.
pub struct VmwareSvgaGpu {
    io_base: u16,
    fb_phys: u32,
    fifo_phys: u32,
    fifo_size: u32,
    fifo_virt: u64,
    capabilities: u32,
    width: u32,
    height: u32,
    pitch: u32,
    vram_size_bytes: u32,
    supported: Vec<(u32, u32)>,

    // FIFO capabilities (extended FIFO register at offset 4)
    fifo_caps: u32,

    // Fence synchronization
    has_fences: bool,

    // IRQ support
    irq: u8,
    irq_active: bool,

    // FIFO reserve/commit (bounce buffer for atomic multi-word commands)
    has_fifo_reserve: bool,
    bounce_phys: u64,
    bounce_virt: u64,
    reserved_size: u32,
    using_bounce: bool,

    // Screen Objects (required for GMRFB blits)
    has_screen_object: bool,

    // GMR2 (Guest Memory Regions for DMA)
    has_gmr2: bool,
    gmr_max_ids: u32,
    gmr_max_pages: u32,
    back_buffer_gmr: Option<u32>,
    back_buffer_offset: u32,
    gmr_pages: Vec<u64>,
    next_gmr_id: u32,
}

// ── IRQ handler (free function for register_irq_chain) ────────────

/// VMware SVGA IRQ handler. Reads and acknowledges the IRQSTATUS port,
/// accumulates pending flags, and wakes any thread waiting on a fence.
fn svga_irq_handler(_irq: u8) {
    let io_base = SVGA_IO_BASE_IRQ.load(Ordering::Relaxed) as u16;
    if io_base == 0 {
        return;
    }

    // Read pending IRQ flags from IRQSTATUS port
    let irq_status = unsafe {
        crate::arch::x86::port::inl(io_base + SVGA_IRQSTATUS_OFFSET)
    };
    if irq_status == 0 {
        return; // Not our interrupt (shared IRQ line)
    }

    // Acknowledge by writing back the same flags
    unsafe {
        crate::arch::x86::port::outl(io_base + SVGA_IRQSTATUS_OFFSET, irq_status);
    }

    // Accumulate pending flags
    SVGA_IRQ_PENDING.fetch_or(irq_status, Ordering::Release);

    // If fence-related, wake the waiter
    if irq_status & (SVGA_IRQFLAG_ANY_FENCE | SVGA_IRQFLAG_FENCE_GOAL) != 0 {
        let tid = SVGA_FENCE_WAITER.load(Ordering::Acquire);
        if tid != 0 {
            if !crate::task::scheduler::try_wake_thread(tid) {
                crate::task::scheduler::deferred_wake(tid);
            }
        }
    }
}

impl VmwareSvgaGpu {
    fn reg_write(&self, index: u32, value: u32) {
        unsafe {
            crate::arch::x86::port::outl(self.io_base, index);
            crate::arch::x86::port::outl(self.io_base + 1, value);
        }
    }

    fn reg_read(&self, index: u32) -> u32 {
        unsafe {
            crate::arch::x86::port::outl(self.io_base, index);
            crate::arch::x86::port::inl(self.io_base + 1)
        }
    }

    fn fifo_ptr(&self) -> *mut u32 {
        self.fifo_virt as *mut u32
    }

    fn fifo_read(&self, index: usize) -> u32 {
        unsafe { core::ptr::read_volatile(self.fifo_ptr().add(index)) }
    }

    fn fifo_write_reg(&self, index: usize, value: u32) {
        unsafe { core::ptr::write_volatile(self.fifo_ptr().add(index), value); }
    }

    /// Check if a FIFO register index is valid (within the FIFO_MIN boundary).
    fn is_fifo_reg_valid(&self, reg: usize) -> bool {
        self.fifo_read(SVGA_FIFO_MIN) > (reg * 4) as u32
    }

    /// Initialize the FIFO
    fn init_fifo(&self) {
        let min = SVGA_FIFO_NUM_REGS * 4;
        self.fifo_write_reg(SVGA_FIFO_MIN, min as u32);
        self.fifo_write_reg(SVGA_FIFO_MAX, self.fifo_size);
        self.fifo_write_reg(SVGA_FIFO_NEXT_CMD, min as u32);
        self.fifo_write_reg(SVGA_FIFO_STOP, min as u32);

        // Enable FIFO
        self.reg_write(SVGA_REG_CONFIG_DONE, 1);
    }

    // ── FIFO Reserve/Commit ──────────────────────────────

    /// Ring the doorbell: notify the device that new commands are pending.
    /// Uses the FIFO_BUSY protocol to avoid redundant SYNC writes.
    fn ring_doorbell(&self) {
        if self.is_fifo_reg_valid(SVGA_FIFO_BUSY) {
            if self.fifo_read(SVGA_FIFO_BUSY) == 0 {
                self.fifo_write_reg(SVGA_FIFO_BUSY, 1);
                self.reg_write(SVGA_REG_SYNC, 1);
            }
        } else {
            self.reg_write(SVGA_REG_SYNC, 1);
        }
    }

    /// Reserve space in the FIFO for a command. Returns a raw pointer to write into.
    /// After writing, call `fifo_commit_all()`.
    fn fifo_reserve(&mut self, bytes: u32) -> *mut u8 {
        let bytes = (bytes + 3) & !3; // Align to 4 bytes

        let min = self.fifo_read(SVGA_FIFO_MIN);
        let max = self.fifo_read(SVGA_FIFO_MAX);
        let fifo_len = max - min;

        if bytes > fifo_len {
            // Command too large for FIFO — sync and retry
            self.sync_fifo();
        }

        if self.has_fifo_reserve {
            let next_cmd = self.fifo_read(SVGA_FIFO_NEXT_CMD);
            let stop = self.fifo_read(SVGA_FIFO_STOP);

            // Check contiguous space available after NEXT_CMD
            let can_fit = if next_cmd >= stop {
                // Free region wraps: [next_cmd..max) + [min..stop)
                if next_cmd + bytes < max || (next_cmd + bytes == max && stop > min) {
                    true
                } else if (max - next_cmd) + (stop - min) > bytes {
                    // Enough total space but wraps — use bounce buffer
                    false
                } else {
                    self.sync_fifo();
                    true // After sync there's always space
                }
            } else {
                // Free region: [next_cmd..stop)
                if next_cmd + bytes < stop {
                    true
                } else {
                    self.sync_fifo();
                    true
                }
            };

            if can_fit && !self.using_bounce {
                let next_cmd = self.fifo_read(SVGA_FIFO_NEXT_CMD);
                self.reserved_size = bytes;
                self.using_bounce = false;
                return (self.fifo_virt + next_cmd as u64) as *mut u8;
            }
        }

        // Fallback: use bounce buffer
        if self.bounce_virt != 0 && (bytes as usize) <= FIFO_BOUNCE_SIZE {
            self.reserved_size = bytes;
            self.using_bounce = true;
            return self.bounce_virt as *mut u8;
        }

        // Ultra-fallback: sync FIFO and write directly
        self.sync_fifo();
        let next_cmd = self.fifo_read(SVGA_FIFO_NEXT_CMD);
        self.reserved_size = bytes;
        self.using_bounce = false;
        (self.fifo_virt + next_cmd as u64) as *mut u8
    }

    /// Commit a reserved FIFO region, making the command visible to the device.
    fn fifo_commit(&mut self, bytes: u32) {
        let bytes = (bytes + 3) & !3;
        let min = self.fifo_read(SVGA_FIFO_MIN);
        let max = self.fifo_read(SVGA_FIFO_MAX);

        if self.using_bounce {
            // Copy from bounce buffer to FIFO word-by-word with wrapping
            let mut next_cmd = self.fifo_read(SVGA_FIFO_NEXT_CMD);
            let src = self.bounce_virt as *const u32;
            let words = bytes as usize / 4;

            for i in 0..words {
                // Wait for space if FIFO is full
                loop {
                    let stop = self.fifo_read(SVGA_FIFO_STOP);
                    let next_next = if next_cmd + 4 >= max { min } else { next_cmd + 4 };
                    if next_next != stop {
                        break;
                    }
                    self.sync_fifo();
                }

                unsafe {
                    let word = core::ptr::read_volatile(src.add(i));
                    let dst = (self.fifo_virt + next_cmd as u64) as *mut u32;
                    core::ptr::write_volatile(dst, word);
                }
                next_cmd = if next_cmd + 4 >= max { min } else { next_cmd + 4 };
                // Commit each word individually (no FIFO_CAP_RESERVE)
                self.fifo_write_reg(SVGA_FIFO_NEXT_CMD, next_cmd);
            }
        } else {
            // Direct reservation: advance NEXT_CMD atomically
            let mut next_cmd = self.fifo_read(SVGA_FIFO_NEXT_CMD);
            next_cmd += bytes;
            if next_cmd >= max {
                next_cmd -= max - min;
            }
            self.fifo_write_reg(SVGA_FIFO_NEXT_CMD, next_cmd);
        }

        self.reserved_size = 0;
        self.using_bounce = false;
    }

    /// Commit the entire reserved region and ring the doorbell.
    fn fifo_commit_all(&mut self) {
        let size = self.reserved_size;
        if size > 0 {
            self.fifo_commit(size);
            self.ring_doorbell();
        }
    }

    /// Write words to the FIFO command buffer (uses reserve/commit internally).
    fn fifo_write_cmd(&mut self, words: &[u32]) {
        let bytes = (words.len() * 4) as u32;
        let ptr = self.fifo_reserve(bytes);
        unsafe {
            core::ptr::copy_nonoverlapping(
                words.as_ptr() as *const u8,
                ptr,
                bytes as usize,
            );
        }
        self.fifo_commit_all();
    }

    /// Synchronize: wait for GPU to process all FIFO commands (legacy busy-wait).
    fn sync_fifo(&self) {
        self.reg_write(SVGA_REG_SYNC, 1);
        while self.reg_read(SVGA_REG_BUSY) != 0 {
            core::hint::spin_loop();
        }
    }

    // ── Fence Synchronization ────────────────────────────

    /// Insert a fence marker into the FIFO command stream.
    /// Returns the fence ID (monotonically increasing u32).
    fn fence_insert(&mut self) -> u32 {
        if !self.has_fences {
            return 0;
        }
        let mut fence_id = SVGA_NEXT_FENCE.fetch_add(1, Ordering::Relaxed);
        // Fence 0 is reserved as "no fence" — skip it
        if fence_id == 0 {
            fence_id = SVGA_NEXT_FENCE.fetch_add(1, Ordering::Relaxed);
        }
        self.fifo_write_cmd(&[SVGA_CMD_FENCE, fence_id]);
        fence_id
    }

    /// Check if a fence has been passed (completed) by the GPU.
    /// Uses signed comparison for u32 wraparound safety.
    fn fence_has_passed(&self, fence_id: u32) -> bool {
        if fence_id == 0 || !self.has_fences {
            return true;
        }
        let fifo_fence = self.fifo_read(SVGA_FIFO_FENCE);
        (fifo_fence.wrapping_sub(fence_id) as i32) >= 0
    }

    /// Wait for a specific fence to complete. Uses IRQ-driven sleep
    /// if available, falls back to polling.
    fn fence_sync(&mut self, fence_id: u32) {
        if fence_id == 0 || self.fence_has_passed(fence_id) {
            return;
        }

        if self.irq_active && self.is_fifo_reg_valid(SVGA_FIFO_FENCE_GOAL) {
            // Set FENCE_GOAL and enable fence IRQ
            self.fifo_write_reg(SVGA_FIFO_FENCE_GOAL, fence_id);
            self.reg_write(SVGA_REG_IRQMASK,
                SVGA_IRQFLAG_FENCE_GOAL | SVGA_IRQFLAG_ANY_FENCE);

            // Clear stale pending flags
            SVGA_IRQ_PENDING.store(0, Ordering::Release);

            // Check again (might have passed between first check and enabling IRQ)
            if self.fence_has_passed(fence_id) {
                return;
            }

            // Ring doorbell to wake the device
            self.ring_doorbell();

            // Check once more (might have passed after doorbell)
            if self.fence_has_passed(fence_id) {
                return;
            }

            // Block current thread — IRQ handler will wake us
            let tid = crate::task::scheduler::current_tid();
            if tid > 0 {
                SVGA_FENCE_WAITER.store(tid, Ordering::Release);
                crate::task::scheduler::block_current_thread();
                SVGA_FENCE_WAITER.store(0, Ordering::Release);

                if self.fence_has_passed(fence_id) {
                    return;
                }
            }
        }

        // Fallback: polling with SYNC/BUSY
        self.reg_write(SVGA_REG_SYNC, 1);
        let mut busy = true;
        while !self.fence_has_passed(fence_id) && busy {
            busy = self.reg_read(SVGA_REG_BUSY) != 0;
            core::hint::spin_loop();
        }
    }

    // ── GMR2 (Guest Memory Regions) ──────────────────────

    /// Define a GMR2 with the given physical pages.
    /// Returns the GMR ID, or None on failure.
    fn gmr2_define(&mut self, phys_pages: &[u64]) -> Option<u32> {
        if !self.has_gmr2 || phys_pages.is_empty() {
            return None;
        }
        let gmr_id = self.next_gmr_id;
        if gmr_id >= self.gmr_max_ids {
            crate::serial_println!("  SVGA: GMR ID limit reached ({})", self.gmr_max_ids);
            return None;
        }
        let num_pages = phys_pages.len() as u32;
        if num_pages > self.gmr_max_pages {
            crate::serial_println!("  SVGA: GMR too large ({} > {} pages)", num_pages, self.gmr_max_pages);
            return None;
        }
        self.next_gmr_id += 1;

        // 1. DEFINE_GMR2: declare the region's total page count
        self.fifo_write_cmd(&[SVGA_CMD_DEFINE_GMR2, gmr_id, num_pages]);

        // 2. REMAP_GMR2: map physical pages (32-bit PPNs)
        // Format: [cmd, gmrId, flags=0, offsetPages=0, numPages, ppn0, ppn1, ...]
        let cmd_words = 5 + phys_pages.len();
        let bytes = (cmd_words * 4) as u32;
        let ptr = self.fifo_reserve(bytes);
        unsafe {
            let dst = ptr as *mut u32;
            core::ptr::write_volatile(dst.add(0), SVGA_CMD_REMAP_GMR2);
            core::ptr::write_volatile(dst.add(1), gmr_id);
            core::ptr::write_volatile(dst.add(2), 0); // flags: PPN32
            core::ptr::write_volatile(dst.add(3), 0); // offsetPages
            core::ptr::write_volatile(dst.add(4), num_pages);
            for (i, &phys) in phys_pages.iter().enumerate() {
                let ppn = (phys >> 12) as u32;
                core::ptr::write_volatile(dst.add(5 + i), ppn);
            }
        }
        self.fifo_commit_all();

        Some(gmr_id)
    }

    /// Undefine a GMR2 (set numPages=0 to release).
    fn gmr2_undefine(&mut self, gmr_id: u32) {
        self.fifo_write_cmd(&[SVGA_CMD_DEFINE_GMR2, gmr_id, 0]);
    }

    /// Set the GMRFB (off-screen surface pointer) for blit operations.
    fn define_gmrfb(&mut self, gmr_id: u32, offset: u32, bytes_per_line: u32, bpp: u32) {
        // SVGAGMRImageFormat: bitsPerPixel[7:0] | colorDepth[15:8]
        // For XRGB8888: bpp=32, colorDepth=24 (R8G8B8 + 8 unused)
        let color_depth = if bpp == 32 { 24u32 } else { bpp };
        let format = (bpp & 0xFF) | ((color_depth & 0xFF) << 8);
        self.fifo_write_cmd(&[
            SVGA_CMD_DEFINE_GMRFB,
            gmr_id, offset,          // SVGAGuestPtr
            bytes_per_line,           // bytesPerLine
            format,                   // SVGAGMRImageFormat
        ]);
    }

    /// Define Screen Object 0 (primary display).
    /// Required before SVGA_CMD_BLIT_GMRFB_TO_SCREEN can target it.
    fn define_screen_0(&mut self) {
        if !self.has_screen_object {
            return;
        }

        if self.fifo_caps & SVGA_FIFO_CAP_SCREEN_OBJECT_2 != 0 {
            // Screen Object v2: full struct with backing store → VRAM
            self.fifo_write_cmd(&[
                SVGA_CMD_DEFINE_SCREEN,
                44,                                         // structSize
                0,                                          // id = 0 (primary)
                SVGA_SCREEN_MUST_BE_SET | SVGA_SCREEN_IS_PRIMARY,
                self.width,
                self.height,
                0, 0,                                       // root.x, root.y
                SVGA_GMR_FRAMEBUFFER,                       // backingStore → BAR1 VRAM
                0,                                          // offset
                self.pitch,                                 // bytesPerLine
                0,                                          // cloneCount
            ]);
        } else {
            // Screen Object v1: truncated struct (no backing store)
            self.fifo_write_cmd(&[
                SVGA_CMD_DEFINE_SCREEN,
                28,                                         // structSize
                0,                                          // id = 0
                SVGA_SCREEN_MUST_BE_SET | SVGA_SCREEN_IS_PRIMARY,
                self.width,
                self.height,
                0, 0,                                       // root.x, root.y
            ]);
        }
        self.sync_fifo();
    }

    /// Blit from GMRFB to screen.
    fn blit_gmrfb_to_screen(&mut self, src_x: i32, src_y: i32,
                             dst_left: i32, dst_top: i32,
                             dst_right: i32, dst_bottom: i32) {
        self.fifo_write_cmd(&[
            SVGA_CMD_BLIT_GMRFB_TO_SCREEN,
            src_x as u32, src_y as u32,     // srcOrigin
            dst_left as u32, dst_top as u32, // destRect left, top
            dst_right as u32, dst_bottom as u32, // destRect right, bottom
            0, // destScreenId
        ]);
    }

}

impl GpuDriver for VmwareSvgaGpu {
    fn name(&self) -> &str {
        "VMware SVGA II"
    }

    fn set_mode(&mut self, width: u32, height: u32, bpp: u32) -> Option<(u32, u32, u32, u32)> {
        self.reg_write(SVGA_REG_WIDTH, width);
        self.reg_write(SVGA_REG_HEIGHT, height);
        self.reg_write(SVGA_REG_BPP, bpp);
        self.reg_write(SVGA_REG_ENABLE, 1);

        let actual_w = self.reg_read(SVGA_REG_WIDTH);
        let actual_h = self.reg_read(SVGA_REG_HEIGHT);
        let pitch = self.reg_read(SVGA_REG_BYTES_PER_LINE);
        let fb = self.reg_read(SVGA_REG_FB_START);

        self.width = actual_w;
        self.height = actual_h;
        self.pitch = pitch;
        self.fb_phys = fb;

        crate::serial_println!(
            "  SVGA II: mode set to {}x{}x{} (pitch={}, fb={:#x})",
            actual_w, actual_h, bpp, pitch, fb
        );

        // Redefine Screen Object 0 with new dimensions
        self.define_screen_0();

        Some((actual_w, actual_h, pitch, fb))
    }

    fn get_mode(&self) -> (u32, u32, u32, u32) {
        (self.width, self.height, self.pitch, self.fb_phys)
    }

    fn supported_modes(&self) -> &[(u32, u32)] {
        &self.supported
    }

    fn has_accel(&self) -> bool {
        (self.capabilities & SVGA_CAP_RECT_FILL) != 0
            || (self.capabilities & SVGA_CAP_RECT_COPY) != 0
    }

    fn accel_fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32) -> bool {
        if self.capabilities & SVGA_CAP_RECT_FILL == 0 {
            return false;
        }
        self.fifo_write_cmd(&[SVGA_CMD_RECT_FILL, color, x, y, w, h]);
        self.update_rect(x, y, w, h);
        true
    }

    fn accel_copy_rect(&mut self, sx: u32, sy: u32, dx: u32, dy: u32, w: u32, h: u32) -> bool {
        if self.capabilities & SVGA_CAP_RECT_COPY == 0 {
            return false;
        }
        self.fifo_write_cmd(&[SVGA_CMD_RECT_COPY, sx, sy, dx, dy, w, h]);
        self.update_rect(dx, dy, w, h);
        true
    }

    fn update_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if self.has_screen_object {
            // Screen Object mode: blit VRAM region → screen
            self.define_gmrfb(SVGA_GMR_FRAMEBUFFER, 0, self.pitch, 32);
            self.blit_gmrfb_to_screen(
                x as i32, y as i32,
                x as i32, y as i32,
                (x + w) as i32, (y + h) as i32,
            );
        } else {
            self.fifo_write_cmd(&[SVGA_CMD_UPDATE, x, y, w, h]);
        }
    }

    fn transfer_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if self.has_screen_object {
            if let Some(gmr_id) = self.back_buffer_gmr {
                // DMA blit: GMR back-buffer → Screen Object 0
                // back_buffer_offset accounts for sub-page alignment of Vec<u32>
                self.define_gmrfb(gmr_id, self.back_buffer_offset, self.width * 4, 32);
            } else {
                // No GMR: blit from VRAM → Screen Object 0
                // (SVGA_CMD_UPDATE is deprecated in Screen Object mode)
                self.define_gmrfb(SVGA_GMR_FRAMEBUFFER, 0, self.pitch, 32);
            }
            self.blit_gmrfb_to_screen(
                x as i32, y as i32,
                x as i32, y as i32,
                (x + w) as i32, (y + h) as i32,
            );
            return;
        }
        // Legacy UPDATE (no Screen Objects — e.g. QEMU)
        self.fifo_write_cmd(&[SVGA_CMD_UPDATE, x, y, w, h]);
    }

    fn has_hw_cursor(&self) -> bool {
        (self.capabilities & (SVGA_CAP_CURSOR | SVGA_CAP_ALPHA_CURSOR)) != 0
    }

    fn define_cursor(&mut self, w: u32, h: u32, hotx: u32, hoty: u32, pixels: &[u32]) {
        if w == 0 || h == 0 || pixels.len() != (w * h) as usize {
            return;
        }

        // Prefer ALPHA_CURSOR (ARGB, full color + transparency) over monochrome.
        if self.capabilities & SVGA_CAP_ALPHA_CURSOR != 0 {
            // SVGA_CMD_DEFINE_ALPHA_CURSOR: [cmd, id, hotx, hoty, w, h, pixels...]
            let mut cmd = alloc::vec![SVGA_CMD_DEFINE_ALPHA_CURSOR, 0, hotx, hoty, w, h];
            cmd.extend_from_slice(pixels);
            self.fifo_write_cmd(&cmd);
            self.sync_fifo();
            return;
        }

        if self.capabilities & SVGA_CAP_CURSOR == 0 {
            return;
        }

        // Fallback: monochrome cursor (bpp=1) — AND + XOR masks.
        let bitmap_u32s = ((w + 31) / 32 * h) as usize;
        let bpl = ((w + 7) / 8) as usize;

        let mut cmd = alloc::vec![SVGA_CMD_DEFINE_CURSOR, 0, hotx, hoty, w, h, 1, 1];

        let total_bytes = bitmap_u32s * 4;
        let mut and_bytes = alloc::vec![0xFFu8; total_bytes];
        let mut xor_bytes = alloc::vec![0u8; total_bytes];

        for row in 0..h as usize {
            for col in 0..w as usize {
                let idx = row * w as usize + col;
                let pixel = if idx < pixels.len() { pixels[idx] } else { 0 };
                let alpha = (pixel >> 24) & 0xFF;

                if alpha >= 128 {
                    let byte_idx = row * bpl + col / 8;
                    let bit = 0x80u8 >> (col % 8);
                    and_bytes[byte_idx] &= !bit;
                    let r = (pixel >> 16) & 0xFF;
                    let g = (pixel >> 8) & 0xFF;
                    let b = pixel & 0xFF;
                    if r > 128 || g > 128 || b > 128 {
                        xor_bytes[byte_idx] |= bit;
                    }
                }
            }
        }

        for i in 0..bitmap_u32s {
            let off = i * 4;
            let word = and_bytes[off] as u32
                | ((and_bytes.get(off + 1).copied().unwrap_or(0) as u32) << 8)
                | ((and_bytes.get(off + 2).copied().unwrap_or(0) as u32) << 16)
                | ((and_bytes.get(off + 3).copied().unwrap_or(0) as u32) << 24);
            cmd.push(word);
        }

        for i in 0..bitmap_u32s {
            let off = i * 4;
            let word = xor_bytes[off] as u32
                | ((xor_bytes.get(off + 1).copied().unwrap_or(0) as u32) << 8)
                | ((xor_bytes.get(off + 2).copied().unwrap_or(0) as u32) << 16)
                | ((xor_bytes.get(off + 3).copied().unwrap_or(0) as u32) << 24);
            cmd.push(word);
        }

        self.fifo_write_cmd(&cmd);
        self.sync_fifo();
    }

    fn move_cursor(&mut self, x: u32, y: u32) {
        // QEMU uses I/O registers for cursor bypass, not FIFO memory
        self.reg_write(SVGA_REG_CURSOR_X, x);
        self.reg_write(SVGA_REG_CURSOR_Y, y);
        self.reg_write(SVGA_REG_CURSOR_ON, 1); // SVGA_CURSOR_ON_SHOW
    }

    fn show_cursor(&mut self, visible: bool) {
        self.reg_write(SVGA_REG_CURSOR_ON, if visible { 1 } else { 0 });
    }

    // VMware SVGA II does NOT use double-buffering — it uses FIFO + UPDATE
    fn has_double_buffer(&self) -> bool { false }

    fn sync(&mut self) {
        if self.has_fences {
            let fence = self.fence_insert();
            self.fence_sync(fence);
        } else {
            self.sync_fifo();
        }
    }

    fn vram_size(&self) -> u32 {
        self.vram_size_bytes
    }

    fn register_back_buffer(&mut self, phys_pages: &[u64], sub_page_offset: u32) -> bool {
        if !self.has_gmr2 || !self.has_screen_object || phys_pages.is_empty() {
            return false;
        }
        // Undefine previous GMR if any
        if let Some(old_id) = self.back_buffer_gmr {
            self.gmr2_undefine(old_id);
            self.back_buffer_gmr = None;
            self.gmr_pages.clear();
        }
        // Define new GMR with the provided pages
        match self.gmr2_define(phys_pages) {
            Some(gmr_id) => {
                // Sync: ensure DEFINE+REMAP commands are processed before first blit
                self.sync_fifo();
                self.gmr_pages = phys_pages.to_vec();
                self.back_buffer_gmr = Some(gmr_id);
                self.back_buffer_offset = sub_page_offset;
                crate::serial_println!("  SVGA: GMR {} defined ({} pages, offset={})", gmr_id, phys_pages.len(), sub_page_offset);
                true
            }
            None => false,
        }
    }

    fn has_dma_back_buffer(&self) -> bool {
        self.back_buffer_gmr.is_some()
    }

    // ── 3D Acceleration ──────────────────────────────────

    fn has_3d(&self) -> bool {
        self.capabilities & SVGA_CAP_3D != 0
    }

    fn hw_version_3d(&self) -> u32 {
        if self.has_3d() && self.is_fifo_reg_valid(SVGA_FIFO_3D_HWVERSION) {
            self.fifo_read(SVGA_FIFO_3D_HWVERSION)
        } else {
            0
        }
    }

    fn submit_3d_commands(&mut self, words: &[u32]) -> bool {
        if !self.has_3d() || words.is_empty() {
            return false;
        }
        self.fifo_write_cmd(words);
        true
    }

    fn dma_surface_upload(&mut self, sid: u32, data: &[u8], width: u32, height: u32) -> bool {
        if !self.has_3d() || !self.has_gmr2 || data.is_empty() {
            return false;
        }

        // Allocate contiguous physical pages for the data
        let num_pages = (data.len() + 4095) / 4096;
        let phys = match crate::memory::physical::alloc_contiguous(num_pages) {
            Some(p) => p,
            None => return false,
        };

        // Copy data to the identity-mapped physical pages
        let virt = phys.as_u64() as *mut u8;
        unsafe {
            core::ptr::copy_nonoverlapping(data.as_ptr(), virt, data.len());
            // Zero remaining bytes in the last page
            let remainder = num_pages * 4096 - data.len();
            if remainder > 0 {
                core::ptr::write_bytes(virt.add(data.len()), 0, remainder);
            }
        }

        // Build physical page list for GMR
        let phys_pages: alloc::vec::Vec<u64> = (0..num_pages)
            .map(|i| phys.as_u64() + (i * 4096) as u64)
            .collect();

        // Define a temporary GMR
        let gmr_id = match self.gmr2_define(&phys_pages) {
            Some(id) => id,
            None => {
                for i in 0..num_pages {
                    crate::memory::physical::free_frame(
                        crate::memory::address::PhysAddr::new(phys.as_u64() + (i * 4096) as u64)
                    );
                }
                return false;
            }
        };

        // Issue SURFACE_DMA: GMR → surface (WRITE_HOST_VRAM)
        let pitch = width * 4;
        let max_offset = height * pitch;
        let cmd_words = [
            SVGA_3D_CMD_SURFACE_DMA,
            (20 * 4) as u32, // size_bytes: 20 payload u32s
            // guest image: { gmr_id, offset, pitch }
            gmr_id, 0, pitch,
            // host image: { sid, face, mipmap }
            sid, 0, 0,
            // transfer type: WRITE_HOST_VRAM = 1
            1,
            // copy box: { x, y, z, w, h, d, srcx, srcy, srcz }
            0, 0, 0, width, height, 1, 0, 0, 0,
            // suffix: { suffixSize, maximumOffset, flags }
            12, max_offset, 0,
        ];
        self.fifo_write_cmd(&cmd_words);

        // Sync to ensure GPU has read the data before we free the pages
        self.sync_fifo();

        // Clean up: undefine GMR and free physical pages
        self.gmr2_undefine(gmr_id);
        for i in 0..num_pages {
            crate::memory::physical::free_frame(
                crate::memory::address::PhysAddr::new(phys.as_u64() + (i * 4096) as u64)
            );
        }

        true
    }
}

// ──────────────────────────────────────────────
// Initialization
// ──────────────────────────────────────────────

/// Initialize and register the VMware SVGA II GPU driver.
/// Called from HAL factory during PCI probe.
pub fn init_and_register(pci_dev: &PciDevice) -> bool {
    // BAR0 = I/O port base
    let bar0 = pci_dev.bars[0];
    if bar0 == 0 || bar0 & 1 == 0 {
        crate::serial_println!("  SVGA II: BAR0 is not I/O space ({:#x})", bar0);
        return false;
    }
    let io_base = (bar0 & !0x3) as u16;

    // BAR1 = Framebuffer physical address
    let fb_phys = pci_dev.bars[1] & !0xF;
    if fb_phys == 0 {
        crate::serial_println!("  SVGA II: BAR1 (framebuffer) is zero");
        return false;
    }

    // BAR2 = FIFO physical address
    let fifo_phys = pci_dev.bars[2] & !0xF;
    if fifo_phys == 0 {
        crate::serial_println!("  SVGA II: BAR2 (FIFO) is zero");
        return false;
    }

    // Enable PCI bus mastering + I/O + memory
    let cmd = crate::drivers::pci::pci_config_read32(pci_dev.bus, pci_dev.device, pci_dev.function, 0x04);
    crate::drivers::pci::pci_config_write32(
        pci_dev.bus, pci_dev.device, pci_dev.function,
        0x04, cmd | 0x07,
    );

    let mut gpu = VmwareSvgaGpu {
        io_base,
        fb_phys,
        fifo_phys,
        fifo_size: 0,
        fifo_virt: 0,
        capabilities: 0,
        width: 0,
        height: 0,
        pitch: 0,
        vram_size_bytes: 0,
        supported: Vec::new(),
        fifo_caps: 0,
        has_fences: false,
        irq: 0,
        irq_active: false,
        has_fifo_reserve: false,
        bounce_phys: 0,
        bounce_virt: 0,
        reserved_size: 0,
        using_bounce: false,
        has_screen_object: false,
        has_gmr2: false,
        gmr_max_ids: 0,
        gmr_max_pages: 0,
        back_buffer_gmr: None,
        back_buffer_offset: 0,
        gmr_pages: Vec::new(),
        next_gmr_id: 0,
    };

    // 1. Version negotiation
    gpu.reg_write(SVGA_REG_ID, SVGA_ID_2);
    let id = gpu.reg_read(SVGA_REG_ID);
    if id != SVGA_ID_2 {
        crate::serial_println!("  SVGA II: version negotiation failed (got {:#x})", id);
        return false;
    }

    // 2. Read capabilities
    gpu.capabilities = gpu.reg_read(SVGA_REG_CAPABILITIES);
    crate::serial_println!("  SVGA II: capabilities = {:#010x}", gpu.capabilities);

    // Log all capability flags
    let cap_flags: &[(u32, &str)] = &[
        (SVGA_CAP_RECT_FILL,       "RECT_FILL"),
        (SVGA_CAP_RECT_COPY,       "RECT_COPY"),
        (SVGA_CAP_CURSOR,          "CURSOR"),
        (SVGA_CAP_CURSOR_BYPASS,   "CURSOR_BYPASS"),
        (SVGA_CAP_CURSOR_BYPASS_2, "CURSOR_BYPASS_2"),
        (SVGA_CAP_8BIT_EMULATION,  "8BIT_EMULATION"),
        (SVGA_CAP_ALPHA_CURSOR,    "ALPHA_CURSOR"),
        (SVGA_CAP_3D,              "3D"),
        (SVGA_CAP_EXTENDED_FIFO,   "EXTENDED_FIFO"),
        (SVGA_CAP_PITCHLOCK,       "PITCHLOCK"),
        (SVGA_CAP_IRQMASK,         "IRQMASK"),
        (SVGA_CAP_GMR,             "GMR"),
        (SVGA_CAP_TRACES,          "TRACES"),
        (SVGA_CAP_GMR2,            "GMR2"),
        (SVGA_CAP_SCREEN_OBJECT_2, "SCREEN_OBJECT_2"),
    ];
    for &(flag, name) in cap_flags {
        if gpu.capabilities & flag != 0 {
            crate::serial_println!("    - {}", name);
        }
    }

    // Log SVGA3D hardware version if 3D is supported
    if gpu.has_3d() {
        let hw_ver = gpu.hw_version_3d();
        crate::serial_println!("  SVGA II: 3D hardware version = {:#x}", hw_ver);
    }

    // 3. Enable SVGA FIRST — must be enabled before setting mode or reading FB info
    gpu.reg_write(SVGA_REG_ENABLE, 1);

    // 4. Set desired mode (match VESA VBE boot mode)
    gpu.reg_write(SVGA_REG_WIDTH, 1024);
    gpu.reg_write(SVGA_REG_HEIGHT, 768);
    gpu.reg_write(SVGA_REG_BPP, 32);

    // 5. Read back actual mode and VRAM info
    gpu.width = gpu.reg_read(SVGA_REG_WIDTH);
    gpu.height = gpu.reg_read(SVGA_REG_HEIGHT);
    gpu.pitch = gpu.reg_read(SVGA_REG_BYTES_PER_LINE);
    gpu.fb_phys = gpu.reg_read(SVGA_REG_FB_START);
    gpu.vram_size_bytes = gpu.reg_read(SVGA_REG_VRAM_SIZE);
    crate::serial_println!("  SVGA II: VRAM size = {} KiB ({} MiB)", gpu.vram_size_bytes / 1024, gpu.vram_size_bytes / (1024 * 1024));

    // 6. Query hardware max resolution and build supported mode list
    let max_w = gpu.reg_read(SVGA_REG_MAX_WIDTH);
    let max_h = gpu.reg_read(SVGA_REG_MAX_HEIGHT);
    crate::serial_println!("  SVGA II: max resolution {}x{}", max_w, max_h);
    gpu.supported = super::COMMON_MODES.iter()
        .copied()
        .filter(|&(w, h)| w <= max_w && h <= max_h)
        .collect();

    // 7. Read FIFO info and map FIFO memory
    gpu.fifo_size = gpu.reg_read(SVGA_REG_FIFO_SIZE);
    crate::serial_println!(
        "  SVGA II: IO={:#x} FB={:#x} FIFO={:#x} (size={}K)",
        io_base, fb_phys, fifo_phys, gpu.fifo_size / 1024
    );

    let pages = (gpu.fifo_size as usize + 4095) / 4096;
    let pages = pages.min(FIFO_MAP_PAGES);
    for i in 0..pages {
        let virt = VirtAddr::new(FIFO_VIRT_BASE + (i as u64) * 4096);
        let phys = crate::memory::address::PhysAddr::new(fifo_phys as u64 + (i as u64) * 4096);
        crate::memory::virtual_mem::map_page(virt, phys, 0x03); // R/W, present
    }
    gpu.fifo_virt = FIFO_VIRT_BASE;

    // 7. Initialize FIFO and signal CONFIG_DONE
    gpu.init_fifo();

    // 8. Read FIFO capabilities
    SVGA_FIFO_VIRT.store(gpu.fifo_virt, Ordering::Relaxed);
    let fifo_caps = gpu.fifo_read(SVGA_FIFO_CAPABILITIES);
    gpu.fifo_caps = fifo_caps;
    crate::serial_println!("  SVGA II: FIFO caps = {:#06x}", fifo_caps);

    if fifo_caps & SVGA_FIFO_CAP_CURSOR_BYPASS_3 != 0 {
        CURSOR_BYPASS_ACTIVE.store(true, Ordering::Relaxed);
        crate::serial_println!("    - CURSOR_BYPASS_3");
    }

    // 8a. Fence support
    gpu.has_fences = fifo_caps & SVGA_FIFO_CAP_FENCE != 0;
    if gpu.has_fences {
        crate::serial_println!("    - FENCE");
    }

    // 8b. FIFO reserve/commit support + bounce buffer allocation
    gpu.has_fifo_reserve = fifo_caps & SVGA_FIFO_CAP_RESERVE != 0;
    if gpu.has_fifo_reserve {
        crate::serial_println!("    - RESERVE");
    }
    // Always allocate bounce buffer (needed for large commands even without RESERVE)
    match crate::memory::physical::alloc_contiguous(FIFO_BOUNCE_PAGES) {
        Some(p) => {
            let phys = p.as_u64();
            unsafe { core::ptr::write_bytes(phys as *mut u8, 0, FIFO_BOUNCE_SIZE); }
            gpu.bounce_phys = phys;
            gpu.bounce_virt = phys; // identity-mapped
        }
        None => {
            crate::serial_println!("    SVGA: bounce buffer alloc failed");
            gpu.has_fifo_reserve = false;
        }
    }

    // 8c. GMR2 support
    gpu.has_gmr2 = (gpu.capabilities & SVGA_CAP_GMR2 != 0)
        && (fifo_caps & SVGA_FIFO_CAP_GMR2 != 0);
    if gpu.has_gmr2 {
        gpu.gmr_max_ids = gpu.reg_read(SVGA_REG_GMR_MAX_IDS);
        gpu.gmr_max_pages = gpu.reg_read(SVGA_REG_GMRS_MAX_PAGES);
        crate::serial_println!("    - GMR2 (max {} IDs, {} pages)", gpu.gmr_max_ids, gpu.gmr_max_pages);
    }

    // 8d. Screen Object support (required for GMRFB blits)
    gpu.has_screen_object = (fifo_caps & SVGA_FIFO_CAP_SCREEN_OBJECT != 0)
        || (fifo_caps & SVGA_FIFO_CAP_SCREEN_OBJECT_2 != 0);
    if gpu.has_screen_object {
        let ver = if fifo_caps & SVGA_FIFO_CAP_SCREEN_OBJECT_2 != 0 { "v2" } else { "v1" };
        crate::serial_println!("    - SCREEN_OBJECT ({})", ver);
        gpu.define_screen_0();
    }

    // 9. IRQ setup (if device supports IRQMASK)
    let irq = pci_dev.interrupt_line;
    gpu.irq = irq;

    if gpu.capabilities & SVGA_CAP_IRQMASK != 0 && irq > 0 && irq < 32 {
        // Store I/O base for IRQ handler access
        SVGA_IO_BASE_IRQ.store(io_base as u32, Ordering::Relaxed);

        // Disable all IRQ sources initially
        gpu.reg_write(SVGA_REG_IRQMASK, 0);

        // Clear any pending IRQ flags
        unsafe {
            let status = crate::arch::x86::port::inl(io_base + SVGA_IRQSTATUS_OFFSET);
            crate::arch::x86::port::outl(io_base + SVGA_IRQSTATUS_OFFSET, status);
        }
        SVGA_IRQ_PENDING.store(0, Ordering::Relaxed);

        // Register IRQ handler (use chain since SVGA may share IRQ with other devices)
        crate::arch::x86::irq::register_irq_chain(irq, svga_irq_handler);
        if crate::arch::x86::apic::is_initialized() {
            crate::arch::x86::ioapic::unmask_irq(irq);
        } else {
            crate::arch::x86::pic::unmask(irq);
        }

        // Enable fence-related IRQs
        gpu.reg_write(SVGA_REG_IRQMASK,
            SVGA_IRQFLAG_ANY_FENCE | SVGA_IRQFLAG_FENCE_GOAL);

        gpu.irq_active = true;
        SVGA_IRQ_ENABLED.store(true, Ordering::Release);
        crate::serial_println!("    SVGA IRQ {} registered (fence-driven sync)", irq);
    }

    // 10. Initial full-screen UPDATE
    gpu.fifo_write_cmd(&[SVGA_CMD_UPDATE, 0, 0, gpu.width, gpu.height]);
    gpu.sync_fifo();

    crate::serial_println!(
        "  SVGA II: initialized {}x{} (pitch={}, fb={:#x})",
        gpu.width, gpu.height, gpu.pitch, gpu.fb_phys
    );

    super::register(Box::new(gpu));
    true
}

/// Probe: initialize VMware SVGA II and return a HAL driver.
pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    init_and_register(pci);
    super::create_hal_driver("VMware SVGA II")
}

// ── FIFO Cursor Bypass (absolute mouse from host) ────────────

/// Poll the VMware SVGA FIFO for absolute cursor position.
/// Returns `Some((x, y))` if cursor position changed since last poll.
/// The host (QEMU/VBox) writes cursor position to FIFO memory when
/// cursor bypass 3 is active. This replaces PS/2 for mouse input.
pub fn poll_cursor() -> Option<(i32, i32)> {
    if !CURSOR_BYPASS_ACTIVE.load(Ordering::Relaxed) {
        return None;
    }
    let fifo_virt = SVGA_FIFO_VIRT.load(Ordering::Relaxed);
    if fifo_virt == 0 {
        return None;
    }

    let fifo = fifo_virt as *const u32;

    // Read CURSOR_COUNT — host increments this on each position update
    let count = unsafe { core::ptr::read_volatile(fifo.add(SVGA_FIFO_CURSOR_COUNT)) };
    let last = LAST_CURSOR_COUNT.load(Ordering::Relaxed);
    if count == last {
        return None;
    }
    LAST_CURSOR_COUNT.store(count, Ordering::Relaxed);

    let x = unsafe { core::ptr::read_volatile(fifo.add(SVGA_FIFO_CURSOR_X)) } as i32;
    let y = unsafe { core::ptr::read_volatile(fifo.add(SVGA_FIFO_CURSOR_Y)) } as i32;

    Some((x, y))
}

/// Check if SVGA cursor bypass is available.
pub fn has_cursor_bypass() -> bool {
    CURSOR_BYPASS_ACTIVE.load(Ordering::Relaxed)
}
