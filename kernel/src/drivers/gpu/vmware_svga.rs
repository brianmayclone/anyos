/// VMware SVGA II GPU driver
///
/// PCI device: vendor 0x15AD, device 0x0405
/// Provides 2D acceleration (RECT_FILL, RECT_COPY), hardware cursor,
/// and FIFO command queue for GPU communication.
///
/// References:
/// - VMware SVGA Device Developer Kit
/// - QEMU hw/display/vmware_vga.c

use super::GpuDriver;
use alloc::boxed::Box;
use crate::drivers::pci::PciDevice;
use crate::memory::address::VirtAddr;

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

// FIFO register offsets (in u32 units)
const SVGA_FIFO_MIN: usize = 0;
const SVGA_FIFO_MAX: usize = 1;
const SVGA_FIFO_NEXT_CMD: usize = 2;
const SVGA_FIFO_STOP: usize = 3;

// Cursor I/O registers (QEMU uses I/O ports, not FIFO memory, for cursor bypass)
const SVGA_REG_CURSOR_ID: u32 = 24;
const SVGA_REG_CURSOR_X: u32 = 25;
const SVGA_REG_CURSOR_Y: u32 = 26;
const SVGA_REG_CURSOR_ON: u32 = 27;

// FIFO reserved registers
const SVGA_FIFO_NUM_REGS: usize = 293;

// FIFO command opcodes
const SVGA_CMD_UPDATE: u32 = 1;
const SVGA_CMD_RECT_FILL: u32 = 2;
const SVGA_CMD_RECT_COPY: u32 = 3;
const SVGA_CMD_DEFINE_CURSOR: u32 = 19;

// Virtual address for FIFO mapping (after E1000 MMIO at 0xD000_0000, 128K)
const FIFO_VIRT_BASE: u32 = 0xD002_0000;
const FIFO_MAP_PAGES: usize = 64; // 256 KiB

pub struct VmwareSvgaGpu {
    io_base: u16,
    fb_phys: u32,
    fifo_phys: u32,
    fifo_size: u32,
    fifo_virt: u32,
    capabilities: u32,
    width: u32,
    height: u32,
    pitch: u32,
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
                self.sync();
            }

            unsafe {
                let ptr = (self.fifo_virt + next_cmd) as *mut u32;
                core::ptr::write_volatile(ptr, word);
            }

            next_cmd = if next_cmd + 4 >= max { min } else { next_cmd + 4 };
        }

        self.fifo_write_reg(SVGA_FIFO_NEXT_CMD, next_cmd);
    }

    /// Synchronize: wait for GPU to process all FIFO commands
    fn sync(&self) {
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
        (self.capabilities & SVGA_CAP_CURSOR) != 0
    }

    fn define_cursor(&mut self, w: u32, h: u32, hotx: u32, hoty: u32, pixels: &[u32]) {
        if self.capabilities & SVGA_CAP_CURSOR == 0 {
            return;
        }

        // DEFINE_CURSOR command:
        // opcode, id, hotx, hoty, w, h, depth_and, depth_xor, and_mask..., xor_mask...
        // For 32bpp ARGB cursor: depth_and=1, depth_xor=32
        // and_mask: 1bpp transparency mask (1=transparent, 0=opaque)
        // xor_mask: 32bpp ARGB pixel data

        let and_mask_size = ((w + 31) / 32) * h; // in u32 words
        let xor_mask_size = w * h; // in u32 words (32bpp)

        let mut cmd = alloc::vec![SVGA_CMD_DEFINE_CURSOR, 0, hotx, hoty, w, h, 1, 32];

        // Generate AND mask from alpha channel.
        // QEMU reads mask as uint8_t*: mask[col/8] & (0x80 >> (col%8)).
        // On little-endian, byte 0 of a u32 holds bits 0-7, byte 1 holds bits 8-15, etc.
        // (col % 32) ^ 7 maps pixel column to the correct bit position so that
        // byte 0 = pixels 0-7, byte 1 = pixels 8-15, etc.
        for row in 0..h {
            let mut and_word: u32 = 0;
            for col in 0..w {
                let idx = (row * w + col) as usize;
                let alpha = if idx < pixels.len() { (pixels[idx] >> 24) & 0xFF } else { 0 };
                if alpha < 128 {
                    // Transparent pixel
                    and_word |= 1 << ((col % 32) ^ 7);
                }
                if col % 32 == 31 || col == w - 1 {
                    cmd.push(and_word);
                    and_word = 0;
                }
            }
        }

        // XOR mask: pixel data with alpha stripped (QEMU uses mask & 0xFFFFFF)
        for row in 0..h {
            for col in 0..w {
                let idx = (row * w + col) as usize;
                let pixel = if idx < pixels.len() { pixels[idx] & 0x00FFFFFF } else { 0 };
                cmd.push(pixel);
            }
        }

        self.fifo_write_cmd(&cmd);
        self.sync(); // Ensure QEMU processes the cursor definition before show/move
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

    // 3. Enable SVGA FIRST — must be enabled before setting mode or reading FB info
    gpu.reg_write(SVGA_REG_ENABLE, 1);

    // 4. Set desired mode (match VESA VBE boot mode)
    gpu.reg_write(SVGA_REG_WIDTH, 1024);
    gpu.reg_write(SVGA_REG_HEIGHT, 768);
    gpu.reg_write(SVGA_REG_BPP, 32);

    // 5. Read back actual mode (device may adjust)
    gpu.width = gpu.reg_read(SVGA_REG_WIDTH);
    gpu.height = gpu.reg_read(SVGA_REG_HEIGHT);
    gpu.pitch = gpu.reg_read(SVGA_REG_BYTES_PER_LINE);
    gpu.fb_phys = gpu.reg_read(SVGA_REG_FB_START);

    // 6. Read FIFO info and map FIFO memory
    gpu.fifo_size = gpu.reg_read(SVGA_REG_FIFO_SIZE);
    crate::serial_println!(
        "  SVGA II: IO={:#x} FB={:#x} FIFO={:#x} (size={}K)",
        io_base, fb_phys, fifo_phys, gpu.fifo_size / 1024
    );

    let pages = (gpu.fifo_size as usize + 4095) / 4096;
    let pages = pages.min(FIFO_MAP_PAGES);
    for i in 0..pages {
        let virt = VirtAddr::new(FIFO_VIRT_BASE + (i as u32) * 4096);
        let phys = crate::memory::address::PhysAddr::new(fifo_phys + (i as u32) * 4096);
        crate::memory::virtual_mem::map_page(virt, phys, 0x03); // R/W, present
    }
    gpu.fifo_virt = FIFO_VIRT_BASE;

    // 7. Initialize FIFO and signal CONFIG_DONE
    gpu.init_fifo();

    // 8. Send initial full-screen UPDATE + SYNC so QEMU displays the framebuffer
    gpu.fifo_write_cmd(&[SVGA_CMD_UPDATE, 0, 0, gpu.width, gpu.height]);
    gpu.sync();

    crate::serial_println!(
        "  SVGA II: initialized {}x{} (pitch={}, fb={:#x})",
        gpu.width, gpu.height, gpu.pitch, gpu.fb_phys
    );

    super::register(Box::new(gpu));
    true
}
