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

// Capabilities
const SVGA_CAP_RECT_FILL: u32 = 1 << 0;
const SVGA_CAP_RECT_COPY: u32 = 1 << 1;
const SVGA_CAP_CURSOR: u32 = 1 << 5;
const SVGA_CAP_CURSOR_BYPASS: u32 = 1 << 6;
const SVGA_CAP_CURSOR_BYPASS_2: u32 = 1 << 7;
const SVGA_CAP_ALPHA_CURSOR: u32 = 1 << 23;

// FIFO register offsets (in u32 units)
const SVGA_FIFO_MIN: usize = 0;
const SVGA_FIFO_MAX: usize = 1;
const SVGA_FIFO_NEXT_CMD: usize = 2;
const SVGA_FIFO_STOP: usize = 3;
const SVGA_FIFO_CAPABILITIES: usize = 4;
const SVGA_FIFO_FLAGS: usize = 5;

// FIFO cursor bypass registers (host writes cursor position here)
const SVGA_FIFO_CURSOR_ON: usize = 6;
const SVGA_FIFO_CURSOR_X: usize = 7;
const SVGA_FIFO_CURSOR_Y: usize = 8;
const SVGA_FIFO_CURSOR_COUNT: usize = 9;

// FIFO capability flags
const SVGA_FIFO_CAP_CURSOR_BYPASS_3: u32 = 1 << 4;

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

// FIFO command opcodes
const SVGA_CMD_UPDATE: u32 = 1;
const SVGA_CMD_RECT_FILL: u32 = 2;
const SVGA_CMD_RECT_COPY: u32 = 3;
const SVGA_CMD_DEFINE_CURSOR: u32 = 19;
const SVGA_CMD_DEFINE_ALPHA_CURSOR: u32 = 22;

// Virtual address for FIFO mapping (kernel higher-half MMIO region)
const FIFO_VIRT_BASE: u64 = 0xFFFF_FFFF_D002_0000;
const FIFO_MAP_PAGES: usize = 64; // 256 KiB

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

    /// Write words to the FIFO command buffer
    fn fifo_write_cmd(&self, words: &[u32]) {
        let min = self.fifo_read(SVGA_FIFO_MIN);
        let max = self.fifo_read(SVGA_FIFO_MAX);

        let mut next_cmd = self.fifo_read(SVGA_FIFO_NEXT_CMD);

        for &word in words {
            let stop = self.fifo_read(SVGA_FIFO_STOP);
            let next_next = if next_cmd + 4 >= max { min } else { next_cmd + 4 };

            // If FIFO is full, sync and retry
            if next_next == stop {
                self.sync_fifo();
            }

            unsafe {
                let ptr = (self.fifo_virt + next_cmd as u64) as *mut u32;
                core::ptr::write_volatile(ptr, word);
            }

            next_cmd = if next_cmd + 4 >= max { min } else { next_cmd + 4 };
        }

        self.fifo_write_reg(SVGA_FIFO_NEXT_CMD, next_cmd);
    }

    /// Synchronize: wait for GPU to process all FIFO commands
    fn sync_fifo(&self) {
        self.reg_write(SVGA_REG_SYNC, 1);
        // Busy-wait for GPU to finish
        while self.reg_read(SVGA_REG_BUSY) != 0 {
            core::hint::spin_loop();
        }
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
        // Also issue an UPDATE so the display reflects the change
        self.fifo_write_cmd(&[SVGA_CMD_UPDATE, x, y, w, h]);
        true
    }

    fn accel_copy_rect(&mut self, sx: u32, sy: u32, dx: u32, dy: u32, w: u32, h: u32) -> bool {
        if self.capabilities & SVGA_CAP_RECT_COPY == 0 {
            return false;
        }
        self.fifo_write_cmd(&[SVGA_CMD_RECT_COPY, sx, sy, dx, dy, w, h]);
        // UPDATE the destination
        self.fifo_write_cmd(&[SVGA_CMD_UPDATE, dx, dy, w, h]);
        true
    }

    fn update_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
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
        self.sync_fifo();
    }

    fn vram_size(&self) -> u32 {
        self.vram_size_bytes
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
    if gpu.capabilities & SVGA_CAP_RECT_FILL != 0 {
        crate::serial_println!("    - RECT_FILL");
    }
    if gpu.capabilities & SVGA_CAP_RECT_COPY != 0 {
        crate::serial_println!("    - RECT_COPY");
    }
    if gpu.capabilities & SVGA_CAP_CURSOR != 0 {
        crate::serial_println!("    - CURSOR");
    }
    if gpu.capabilities & SVGA_CAP_CURSOR_BYPASS != 0 {
        crate::serial_println!("    - CURSOR_BYPASS");
    }
    if gpu.capabilities & SVGA_CAP_ALPHA_CURSOR != 0 {
        crate::serial_println!("    - ALPHA_CURSOR");
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

    // 8. Check for FIFO cursor bypass 3 (host writes cursor pos to FIFO memory)
    SVGA_FIFO_VIRT.store(gpu.fifo_virt, Ordering::Relaxed);
    let fifo_caps = gpu.fifo_read(SVGA_FIFO_CAPABILITIES);
    if fifo_caps & SVGA_FIFO_CAP_CURSOR_BYPASS_3 != 0 {
        CURSOR_BYPASS_ACTIVE.store(true, Ordering::Relaxed);
        crate::serial_println!("  SVGA II: FIFO cursor bypass 3 active (absolute mouse from host)");
    } else {
        crate::serial_println!("  SVGA II: FIFO caps={:#x} (no cursor bypass 3)", fifo_caps);
    }

    // 9. Send initial full-screen UPDATE + SYNC so QEMU displays the framebuffer
    gpu.fifo_write_cmd(&[SVGA_CMD_UPDATE, 0, 0, gpu.width, gpu.height]);
    gpu.sync();

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
