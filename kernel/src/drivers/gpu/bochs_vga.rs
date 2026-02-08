/// Bochs VGA / QEMU Standard VGA driver
///
/// Uses DISPI (Display Interface) registers via I/O ports 0x01CE/0x01CF
/// for mode switching and double-buffering via virtual height + Y offset.
///
/// Loaded dynamically via PCI detection (vendor 0x1234, device 0x1111).

use super::GpuDriver;
use alloc::boxed::Box;

// DISPI I/O ports
const VBE_DISPI_IOPORT_INDEX: u16 = 0x01CE;
const VBE_DISPI_IOPORT_DATA: u16 = 0x01CF;

// DISPI register indices
const VBE_DISPI_INDEX_XRES: u16 = 0x01;
const VBE_DISPI_INDEX_YRES: u16 = 0x02;
const VBE_DISPI_INDEX_BPP: u16 = 0x03;
const VBE_DISPI_INDEX_ENABLE: u16 = 0x04;
const VBE_DISPI_INDEX_VIRT_WIDTH: u16 = 0x06;
const VBE_DISPI_INDEX_VIRT_HEIGHT: u16 = 0x07;
const VBE_DISPI_INDEX_Y_OFFSET: u16 = 0x09;

// DISPI enable flags
const VBE_DISPI_ENABLED: u16 = 0x01;
const VBE_DISPI_LFB_ENABLED: u16 = 0x40;
const VBE_DISPI_NOCLEARMEM: u16 = 0x80;

fn dispi_write(index: u16, value: u16) {
    unsafe {
        crate::arch::x86::port::outw(VBE_DISPI_IOPORT_INDEX, index);
        crate::arch::x86::port::outw(VBE_DISPI_IOPORT_DATA, value);
    }
}

fn dispi_read(index: u16) -> u16 {
    unsafe {
        crate::arch::x86::port::outw(VBE_DISPI_IOPORT_INDEX, index);
        crate::arch::x86::port::inw(VBE_DISPI_IOPORT_DATA)
    }
}

pub struct BochsVgaGpu {
    fb_phys: u32,
    width: u32,
    height: u32,
    pitch: u32,
    double_buffered: bool,
    front_page: u32, // 0 or 1
}

impl BochsVgaGpu {
    fn new(fb_phys: u32, width: u32, height: u32, pitch: u32) -> Self {
        // Try to set up double buffering via virtual height
        let virt_height = dispi_read(VBE_DISPI_INDEX_VIRT_HEIGHT) as u32;
        let double_buffered = virt_height >= height * 2;

        if double_buffered {
            crate::serial_println!("  Bochs VGA: double-buffering enabled (virt_height={})", virt_height);
        }

        BochsVgaGpu {
            fb_phys,
            width,
            height,
            pitch,
            double_buffered,
            front_page: 0,
        }
    }
}

impl GpuDriver for BochsVgaGpu {
    fn name(&self) -> &str {
        "Bochs/QEMU VGA"
    }

    fn set_mode(&mut self, width: u32, height: u32, bpp: u32) -> Option<(u32, u32, u32, u32)> {
        // Disable VBE
        dispi_write(VBE_DISPI_INDEX_ENABLE, 0);

        // Set new resolution
        dispi_write(VBE_DISPI_INDEX_XRES, width as u16);
        dispi_write(VBE_DISPI_INDEX_YRES, height as u16);
        dispi_write(VBE_DISPI_INDEX_BPP, bpp as u16);
        dispi_write(VBE_DISPI_INDEX_VIRT_WIDTH, width as u16);
        dispi_write(VBE_DISPI_INDEX_VIRT_HEIGHT, (height * 2) as u16);

        // Re-enable with LFB
        dispi_write(
            VBE_DISPI_INDEX_ENABLE,
            VBE_DISPI_ENABLED | VBE_DISPI_LFB_ENABLED | VBE_DISPI_NOCLEARMEM,
        );

        // Read back actual values
        let actual_w = dispi_read(VBE_DISPI_INDEX_XRES) as u32;
        let actual_h = dispi_read(VBE_DISPI_INDEX_YRES) as u32;
        let actual_bpp = dispi_read(VBE_DISPI_INDEX_BPP) as u32;
        let actual_virt_h = dispi_read(VBE_DISPI_INDEX_VIRT_HEIGHT) as u32;

        let pitch = actual_w * (actual_bpp / 8);

        self.width = actual_w;
        self.height = actual_h;
        self.pitch = pitch;
        self.double_buffered = actual_virt_h >= actual_h * 2;
        self.front_page = 0;

        // Reset Y offset
        dispi_write(VBE_DISPI_INDEX_Y_OFFSET, 0);

        crate::serial_println!(
            "  Bochs VGA: mode set to {}x{}x{} (pitch={}, dblbuf={})",
            actual_w, actual_h, actual_bpp, pitch, self.double_buffered
        );

        Some((actual_w, actual_h, pitch, self.fb_phys))
    }

    fn get_mode(&self) -> (u32, u32, u32, u32) {
        (self.width, self.height, self.pitch, self.fb_phys)
    }

    fn has_double_buffer(&self) -> bool {
        self.double_buffered
    }

    fn flip(&mut self) {
        if !self.double_buffered {
            return;
        }
        // Toggle front page
        self.front_page ^= 1;
        let y_offset = self.front_page * self.height;
        dispi_write(VBE_DISPI_INDEX_Y_OFFSET, y_offset as u16);
    }

    fn back_buffer_phys(&self) -> Option<u32> {
        if !self.double_buffered {
            return None;
        }
        // Back buffer is the page NOT currently displayed
        let back_page = self.front_page ^ 1;
        let offset = back_page * self.height * self.pitch;
        Some(self.fb_phys + offset)
    }
}

// ──────────────────────────────────────────────
// Public API (used by main.rs and compositor for backwards compat)
// ──────────────────────────────────────────────

/// Initialize and register the Bochs VGA GPU driver.
/// Called from HAL factory or main.rs during Phase 9.
pub fn init_and_register(fb_phys: u32, width: u32, height: u32, pitch: u32) -> bool {
    let gpu = BochsVgaGpu::new(fb_phys, width, height, pitch);
    super::register(Box::new(gpu));
    true
}

/// Legacy init function — calls init_and_register.
pub fn init(fb_phys: u32, width: u32, height: u32, pitch: u32) {
    init_and_register(fb_phys, width, height, pitch);
}

/// Check if double-buffering is active (legacy API).
pub fn is_double_buffered() -> bool {
    super::with_gpu(|g| g.has_double_buffer()).unwrap_or(false)
}

/// Flip front/back buffers (legacy API).
pub fn flip() {
    super::with_gpu(|g| g.flip());
}

/// Get back buffer physical address (legacy API).
pub fn back_buffer_phys() -> Option<u32> {
    super::with_gpu(|g| g.back_buffer_phys()).flatten()
}

/// Get display info: (width, height, pitch, fb_phys) (legacy API).
pub fn display_info() -> Option<(u32, u32, u32, u32)> {
    super::with_gpu(|g| g.get_mode())
}
