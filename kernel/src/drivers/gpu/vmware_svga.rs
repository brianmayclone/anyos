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

// GMR registers
const SVGA_REG_GMR_ID: u32 = 41;
const SVGA_REG_GMR_MAX_IDS: u32 = 43;
const SVGA_REG_GMRS_MAX_PAGES: u32 = 46;

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
const SVGA_CAP_COMMAND_BUFFERS: u32    = 1 << 24;
const SVGA_CAP_CMD_BUFFERS_2: u32      = 1 << 26;
const SVGA_CAP_GBOBJECTS: u32          = 1 << 27;
const SVGA_CAP_DX: u32                 = 1 << 28;
const SVGA_CAP_CAP2_REGISTER: u32      = 1 << 31;

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
const SVGA_FIFO_CAP_SCREEN_OBJECT: u32 = 1 << 7;
const SVGA_FIFO_CAP_GMR2: u32 = 1 << 8;
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

// FIFO command opcodes
const SVGA_CMD_UPDATE: u32 = 1;
const SVGA_CMD_RECT_FILL: u32 = 2;
const SVGA_CMD_RECT_COPY: u32 = 3;
const SVGA_CMD_DEFINE_CURSOR: u32 = 19;
const SVGA_CMD_DEFINE_ALPHA_CURSOR: u32 = 22;

// GMR2 / Screen Object FIFO commands
const SVGA_CMD_DEFINE_SCREEN: u32 = 34;
const SVGA_CMD_DESTROY_SCREEN: u32 = 35;
const SVGA_CMD_DEFINE_GMRFB: u32 = 36;
const SVGA_CMD_BLIT_GMRFB_TO_SCREEN: u32 = 37;
const SVGA_CMD_DEFINE_GMR2: u32 = 41;
const SVGA_CMD_REMAP_GMR2: u32 = 42;

// Special GMR IDs
const SVGA_GMR_FRAMEBUFFER: u32 = 0xFFFF_FFFF;

// Screen object flags
const SVGA_SCREEN_MUST_BE_SET: u32 = 1 << 0;
const SVGA_SCREEN_IS_PRIMARY: u32 = 1 << 1;

// REMAP_GMR2 flags
const SVGA_REMAP_GMR2_PPN32: u32 = 0; // default: 32-bit PPNs

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
    // GMR2 / Screen Object state
    fifo_caps: u32,
    screen_object_active: bool,
    gmr_back_active: bool,
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

    // ── GMR2 / Screen Object methods ──────────────────────────

    /// Define a primary screen object via SVGA_CMD_DEFINE_SCREEN.
    /// The screen is backed by the current GMRFB (VRAM or GMR).
    fn define_screen_object(&mut self, id: u32, w: u32, h: u32) {
        // SVGAScreenObject: structSize, id, flags, size{w,h}, root{x,y},
        //   backingStore{ptr{gmrId,offset}, pitch}, cloneCount = 11 dwords
        let struct_size: u32 = 11 * 4; // 44 bytes
        let flags = SVGA_SCREEN_MUST_BE_SET | SVGA_SCREEN_IS_PRIMARY;
        let backing_pitch = w * 4; // ARGB8888
        #[allow(unused)]
        let cmd = [
            SVGA_CMD_DEFINE_SCREEN,
            // SVGAScreenObject fields:
            struct_size,
            id,
            flags,
            w,              // size.width
            h,              // size.height
            0,              // root.x
            0,              // root.y
            SVGA_GMR_FRAMEBUFFER, // backingStore.ptr.gmrId
            0,              // backingStore.ptr.offset
            backing_pitch,  // backingStore.pitch
            0,              // cloneCount
        ];
        self.fifo_write_cmd(&cmd);
    }

    /// Define a GMR2 region (declares ID + total page count).
    fn define_gmr2(&self, gmr_id: u32, num_pages: u32) {
        self.fifo_write_cmd(&[SVGA_CMD_DEFINE_GMR2, gmr_id, num_pages]);
    }

    /// Remap GMR2: fill in the physical page numbers (PPNs).
    /// `ppns` contains one u32 per page (phys_addr >> 12).
    fn remap_gmr2(&self, gmr_id: u32, ppns: &[u32]) {
        // Header: cmd, gmrId, flags, offsetPages, numPages
        // Followed by: ppn[0], ppn[1], ...
        let mut cmd = Vec::with_capacity(5 + ppns.len());
        cmd.push(SVGA_CMD_REMAP_GMR2);
        cmd.push(gmr_id);
        cmd.push(SVGA_REMAP_GMR2_PPN32); // flags: 32-bit PPNs
        cmd.push(0);                       // offsetPages
        cmd.push(ppns.len() as u32);       // numPages
        cmd.extend_from_slice(ppns);
        self.fifo_write_cmd(&cmd);
    }

    /// Set the active GMRFB (source for BLIT_GMRFB_TO_SCREEN).
    fn set_gmrfb(&self, gmr_id: u32, offset: u32, pitch: u32) {
        // SVGA_CMD_DEFINE_GMRFB: ptr.gmrId, ptr.offset, bytesPerLine, format
        // Format: bitsPerPixel=32, colorDepth=24 → 32 | (24 << 8) = 0x1820
        let format: u32 = 32 | (24 << 8);
        self.fifo_write_cmd(&[
            SVGA_CMD_DEFINE_GMRFB, gmr_id, offset, pitch, format,
        ]);
    }

    /// DMA blit from the active GMRFB to a screen object.
    fn blit_gmrfb_to_screen(&self, src_x: i32, src_y: i32,
                             left: i32, top: i32, right: i32, bottom: i32,
                             screen_id: u32) {
        self.fifo_write_cmd(&[
            SVGA_CMD_BLIT_GMRFB_TO_SCREEN,
            src_x as u32,       // srcOrigin.x
            src_y as u32,       // srcOrigin.y
            left as u32,        // destRect.left
            top as u32,         // destRect.top
            right as u32,       // destRect.right
            bottom as u32,      // destRect.bottom
            screen_id,          // destScreenId
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

        // Recreate screen object for new resolution
        if self.screen_object_active {
            self.define_screen_object(0, actual_w, actual_h);
            // If GMR was active, invalidate it — compositor must re-register
            if self.gmr_back_active {
                self.gmr_back_active = false;
                self.set_gmrfb(SVGA_GMR_FRAMEBUFFER, 0, pitch);
                crate::serial_println!("  SVGA II: GMR invalidated (resolution change), falling back to VRAM");
            }
            self.blit_gmrfb_to_screen(
                0, 0,
                0, 0, actual_w as i32, actual_h as i32,
                0,
            );
            self.sync_fifo();
            crate::serial_println!("  SVGA II: Screen Object 0 redefined ({}x{})", actual_w, actual_h);
        }

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
        if self.screen_object_active {
            // BLIT from GMRFB (VRAM or GMR) to screen object
            self.blit_gmrfb_to_screen(
                x as i32, y as i32,
                x as i32, y as i32,
                (x + w) as i32, (y + h) as i32,
                0,
            );
        } else {
            self.fifo_write_cmd(&[SVGA_CMD_UPDATE, x, y, w, h]);
        }
    }

    fn transfer_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        if self.screen_object_active {
            // BLIT from GMRFB to screen object — handles both VRAM and GMR mode
            self.blit_gmrfb_to_screen(
                x as i32, y as i32,
                x as i32, y as i32,
                (x + w) as i32, (y + h) as i32,
                0,
            );
        } else {
            self.fifo_write_cmd(&[SVGA_CMD_UPDATE, x, y, w, h]);
        }
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

    fn register_back_buffer(&mut self, phys_pages: &[u64]) -> bool {
        if !self.screen_object_active {
            return false;
        }

        let gmr_id: u32 = 1;
        let num_pages = phys_pages.len() as u32;

        // Convert to 32-bit PPNs (phys >> 12)
        let ppns: Vec<u32> = phys_pages.iter().map(|&p| (p >> 12) as u32).collect();

        // Define and populate GMR2
        self.define_gmr2(gmr_id, num_pages);
        self.remap_gmr2(gmr_id, &ppns);
        self.sync_fifo();

        // Switch GMRFB to point at this GMR
        let pitch = self.width * 4; // ARGB8888
        self.set_gmrfb(gmr_id, 0, pitch);
        self.gmr_back_active = true;

        crate::serial_println!(
            "  SVGA II: GMR {} registered ({} pages, pitch={}), DMA blit active",
            gmr_id, num_pages, pitch
        );
        true
    }

    fn has_dma_back_buffer(&self) -> bool {
        self.gmr_back_active
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
        fifo_caps: 0,
        screen_object_active: false,
        gmr_back_active: false,
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
        (SVGA_CAP_COMMAND_BUFFERS, "COMMAND_BUFFERS"),
        (SVGA_CAP_CMD_BUFFERS_2,   "CMD_BUFFERS_2"),
        (SVGA_CAP_GBOBJECTS,       "GBOBJECTS"),
        (SVGA_CAP_DX,              "DX"),
        (SVGA_CAP_CAP2_REGISTER,   "CAP2_REGISTER"),
    ];
    for &(flag, name) in cap_flags {
        if gpu.capabilities & flag != 0 {
            crate::serial_println!("    - {}", name);
        }
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

    // 8. Read FIFO capabilities and check for cursor bypass 3
    SVGA_FIFO_VIRT.store(gpu.fifo_virt, Ordering::Relaxed);
    gpu.fifo_caps = gpu.fifo_read(SVGA_FIFO_CAPABILITIES);
    crate::serial_println!("  SVGA II: FIFO caps = {:#06x}", gpu.fifo_caps);
    if gpu.fifo_caps & SVGA_FIFO_CAP_CURSOR_BYPASS_3 != 0 {
        CURSOR_BYPASS_ACTIVE.store(true, Ordering::Relaxed);
        crate::serial_println!("    - CURSOR_BYPASS_3");
    }

    // Log all FIFO capability flags
    let fifo_cap_flags: &[(u32, &str)] = &[
        (SVGA_FIFO_CAP_SCREEN_OBJECT,   "SCREEN_OBJECT"),
        (SVGA_FIFO_CAP_GMR2,            "GMR2"),
        (SVGA_FIFO_CAP_SCREEN_OBJECT_2, "SCREEN_OBJECT_2"),
    ];
    for &(flag, name) in fifo_cap_flags {
        if gpu.fifo_caps & flag != 0 {
            crate::serial_println!("    - {}", name);
        }
    }

    // 9. Set up GMR2 + SCREEN_OBJECT_2 if supported
    let has_gmr2 = (gpu.capabilities & SVGA_CAP_GMR2 != 0)
        && (gpu.fifo_caps & SVGA_FIFO_CAP_GMR2 != 0);
    let has_screen_obj2 = (gpu.capabilities & SVGA_CAP_SCREEN_OBJECT_2 != 0)
        && (gpu.fifo_caps & SVGA_FIFO_CAP_SCREEN_OBJECT_2 != 0);

    if has_gmr2 && has_screen_obj2 {
        let max_gmr_ids = gpu.reg_read(SVGA_REG_GMR_MAX_IDS);
        let max_gmr_pages = gpu.reg_read(SVGA_REG_GMRS_MAX_PAGES);
        crate::serial_println!(
            "  SVGA II: GMR2 enabled (max {} IDs, {} pages)",
            max_gmr_ids, max_gmr_pages
        );

        // Define primary screen object backed by VRAM framebuffer
        gpu.define_screen_object(0, gpu.width, gpu.height);
        gpu.set_gmrfb(SVGA_GMR_FRAMEBUFFER, 0, gpu.pitch);
        gpu.screen_object_active = true;

        // Initial blit from VRAM framebuffer to screen
        gpu.blit_gmrfb_to_screen(
            0, 0,
            0, 0, gpu.width as i32, gpu.height as i32,
            0,
        );
        gpu.sync_fifo();
        crate::serial_println!(
            "  SVGA II: Screen Object 0 defined ({}x{}, VRAM-backed)",
            gpu.width, gpu.height
        );
    } else {
        // 9b. Legacy path: send initial full-screen UPDATE
        gpu.fifo_write_cmd(&[SVGA_CMD_UPDATE, 0, 0, gpu.width, gpu.height]);
        gpu.sync_fifo();
    }

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
