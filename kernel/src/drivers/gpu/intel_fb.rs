//! Intel integrated GPU framebuffer driver.
//!
//! Provides basic modesetting for Intel HD/UHD/Iris Graphics (Gen 5+).
//! This is NOT a full i915 driver — it only does framebuffer setup using
//! the pre-configured display mode from the UEFI/BIOS GOP or VBE.
//!
//! The driver reads the current display pipe configuration (set by firmware),
//! exposes it as a linear framebuffer, and supports resolution changes via
//! VBE DISPI registers (if available) or by reprogramming the display pipe.
//!
//! Covers PCI class 03:00 with vendor 0x8086 — Intel HD Graphics 3000/4000,
//! Iris Pro, UHD 630, Xe, etc.
//!
//! # Strategy
//! On UEFI systems, the firmware already set up a working display via GOP.
//! We inherit that framebuffer (from boot_info) and simply re-map it.
//! On legacy BIOS, we use VBE/DISPI if available, or fall back to the
//! pre-configured mode.

use super::GpuDriver;
use alloc::boxed::Box;
use alloc::vec::Vec;
use crate::drivers::pci::PciDevice;
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::virtual_mem;

// ── MMIO Register Offsets (Intel Gen 5+) ────────────────────────────────────

// Graphics Memory base
const GTTMMADR_OFFSET: usize = 0;

// Pipe A registers (primary display)
const PIPE_A_CONF: u32     = 0x70008;  // Pipe configuration
const PIPE_A_STATUS: u32   = 0x70024;  // Pipe status
const HTOTAL_A: u32        = 0x60000;  // Horizontal total
const VTOTAL_A: u32        = 0x6000C;  // Vertical total
const DSPCNTR_A: u32       = 0x70180;  // Display/Sprite control
const DSPADDR_A: u32       = 0x70184;  // Display base address
const DSPSTRIDE_A: u32     = 0x70188;  // Display stride (pitch)
const DSPSURF_A: u32       = 0x7019C;  // Display surface address (Gen4+)
const DSPSIZE_A: u32       = 0x70190;  // Display size (height-1 << 16 | width-1)
const DSPOFFSET_A: u32     = 0x701A4;  // Display offset

// Display/Sprite control bits
const DSPCNTR_ENABLE: u32      = 1 << 31;
const DSPCNTR_FORMAT_XRGB8888: u32 = 0x06 << 26; // BGRX 8888
const DSPCNTR_FORMAT_BGRX8888: u32 = 0x06 << 26;

// VBE DISPI registers (for Bochs-compat mode on some Intel devices in QEMU)
const VBE_INDEX: u16 = 0x01CE;
const VBE_DATA: u16  = 0x01CF;

// ── Driver State ────────────────────────────────────────────────────────────

pub struct IntelFbGpu {
    mmio_base: u64,
    mmio_size: usize,
    fb_phys: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    has_vbe: bool,
    gen: u8, // Intel GPU generation (5 = Ironlake, 6 = Sandy Bridge, etc.)
}

impl IntelFbGpu {
    #[inline]
    unsafe fn rd(&self, reg: u32) -> u32 {
        if self.mmio_base == 0 { return 0; }
        core::ptr::read_volatile((self.mmio_base + reg as u64) as *const u32)
    }

    #[inline]
    unsafe fn wr(&self, reg: u32, val: u32) {
        if self.mmio_base == 0 { return; }
        core::ptr::write_volatile((self.mmio_base + reg as u64) as *mut u32, val);
    }

    /// Read current display configuration from pipe A registers.
    fn read_pipe_config(&mut self) {
        unsafe {
            let dspcntr = self.rd(DSPCNTR_A);
            if dspcntr & DSPCNTR_ENABLE == 0 {
                // Pipe not enabled — use boot info values
                return;
            }

            // Read surface address
            let surf = self.rd(DSPSURF_A);
            if surf != 0 {
                self.fb_phys = surf;
            }

            // Read stride
            let stride = self.rd(DSPSTRIDE_A);
            if stride > 0 {
                self.pitch = stride;
            }

            // Read size (if available)
            let size = self.rd(DSPSIZE_A);
            if size != 0 {
                let h = (size >> 16) + 1;
                let w = (size & 0xFFFF) + 1;
                if w > 0 && h > 0 {
                    self.width = w;
                    self.height = h;
                }
            }
        }
    }
}

impl GpuDriver for IntelFbGpu {
    fn name(&self) -> &str { "Intel HD Graphics" }

    fn driver_type_name(&self) -> &str { "intel" }

    fn set_mode(&mut self, width: u32, height: u32, bpp: u32) -> Option<(u32, u32, u32, u32)> {
        // Try VBE DISPI if available (Bochs-compat mode)
        if self.has_vbe {
            use crate::arch::x86::port::{outw, inw};
            unsafe {
                outw(VBE_INDEX, 0x04); outw(VBE_DATA, 0x00); // Disable
                outw(VBE_INDEX, 0x01); outw(VBE_DATA, width as u16);  // XRES
                outw(VBE_INDEX, 0x02); outw(VBE_DATA, height as u16); // YRES
                outw(VBE_INDEX, 0x03); outw(VBE_DATA, bpp as u16);    // BPP
                outw(VBE_INDEX, 0x04); outw(VBE_DATA, 0x41);          // Enable + LFB
                // Read back
                outw(VBE_INDEX, 0x01); let w = inw(VBE_DATA) as u32;
                outw(VBE_INDEX, 0x02); let h = inw(VBE_DATA) as u32;
                self.width = w;
                self.height = h;
                self.pitch = w * (bpp / 8);
                self.bpp = bpp;
            }
            return Some((self.width, self.height, self.pitch, self.fb_phys));
        }

        // For real Intel hardware: reprogram pipe A
        if self.mmio_base != 0 {
            // Update stride
            let new_pitch = width * (bpp / 8);
            unsafe {
                self.wr(DSPSTRIDE_A, new_pitch);
                // Update size register
                self.wr(DSPSIZE_A, ((height - 1) << 16) | (width - 1));
                // Trigger surface update
                self.wr(DSPSURF_A, self.fb_phys);
            }
            self.width = width;
            self.height = height;
            self.pitch = new_pitch;
            self.bpp = bpp;
            return Some((self.width, self.height, self.pitch, self.fb_phys));
        }

        None
    }

    fn get_mode(&self) -> (u32, u32, u32, u32) {
        (self.width, self.height, self.pitch, self.fb_phys)
    }

    fn supported_modes(&self) -> &[(u32, u32)] {
        &super::COMMON_MODES
    }

    fn has_accel(&self) -> bool { false }
    fn has_hw_cursor(&self) -> bool { false }
    fn has_double_buffer(&self) -> bool { false }

    fn vram_size(&self) -> u32 {
        // Estimate based on resolution (aperture size is usually 256 MiB+)
        self.pitch * self.height * 2
    }
}

// ── Detection & Initialization ──────────────────────────────────────────────

/// Detect Intel GPU generation from device ID.
fn detect_gen(device_id: u16) -> u8 {
    match device_id {
        // Gen 5 (Ironlake)
        0x0042 | 0x0046 => 5,
        // Gen 6 (Sandy Bridge)
        0x0102 | 0x0106 | 0x0112 | 0x0116 | 0x0122 | 0x0126 => 6,
        // Gen 7 (Ivy Bridge)
        0x0152 | 0x0156 | 0x0162 | 0x0166 => 7,
        // Gen 7.5 (Haswell)
        0x0402 | 0x0406 | 0x0412 | 0x0416 | 0x041E | 0x0A06 | 0x0A16 | 0x0D12 | 0x0D22 => 7,
        // Gen 8 (Broadwell)
        0x1612 | 0x1616 | 0x161E | 0x1622 | 0x1626 | 0x162B => 8,
        // Gen 9 (Skylake/Kaby Lake/Coffee Lake)
        0x1912 | 0x1916 | 0x191B | 0x191E | 0x1902 |
        0x5912 | 0x5916 | 0x591B | 0x591E |
        0x3E90 | 0x3E91 | 0x3E92 | 0x3E93 | 0x3E94 | 0x3E96 | 0x3E98 |
        0x3EA0 | 0x3EA5 | 0x3EA6 | 0x3EA7 | 0x3EA8 => 9,
        // Gen 11 (Ice Lake)
        0x8A52 | 0x8A5A | 0x8A56 | 0x8A51 | 0x8A50 => 11,
        // Gen 12 (Tiger Lake / Alder Lake / Raptor Lake)
        0x9A49 | 0x9A40 | 0x9A60 | 0x9A68 | 0x9A78 |
        0x4680 | 0x4682 | 0x4688 | 0x468A | 0x4690 | 0x4692 | 0x4693 |
        0xA780 | 0xA782 | 0xA788 | 0xA78B => 12,
        _ => 9, // Default to Gen 9 for unknown IDs
    }
}

pub fn init_and_register(pci: &PciDevice) -> bool {
    let gen = detect_gen(pci.device_id);
    crate::serial_println!("  Intel GPU: device={:#06x} gen={}", pci.device_id, gen);

    // Get GTTMMADR (BAR0) — MMIO register space
    let bar0 = (pci.bars[0] & !0xF) as u64;
    let bar0_hi = if pci.bars[0] & 0x04 != 0 { (pci.bars[1] as u64) << 32 } else { 0 };
    let mmio_phys = bar0 | bar0_hi;

    // Get Graphics Aperture (BAR2) — framebuffer
    let bar2 = (pci.bars[2] & !0xF) as u64;
    let bar2_hi = if pci.bars[2] & 0x04 != 0 { (pci.bars[3] as u64) << 32 } else { 0 };
    let fb_phys = bar2 | bar2_hi;

    crate::serial_println!("  Intel GPU: MMIO={:#014x} FB={:#014x}", mmio_phys, fb_phys);

    // Enable memory space + bus mastering
    let cmd = crate::drivers::pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    crate::drivers::pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    // Map MMIO registers (2 MiB for GTT + registers)
    let mmio_pages = 512; // 2 MiB
    let mmio_base = if mmio_phys != 0 {
        match virtual_mem::map_mmio(PhysAddr::new(mmio_phys), mmio_pages) {
            Some(v) => v.as_u64(),
            None => 0,
        }
    } else { 0 };

    // Get current framebuffer info from boot (UEFI/GOP or VBE)
    let (boot_fb, boot_w, boot_h, boot_pitch) = crate::drivers::framebuffer::info()
        .map(|fb| (fb.addr as u32, fb.width, fb.height, fb.pitch))
        .unwrap_or((fb_phys as u32, 1024, 768, 4096));

    let mut gpu = IntelFbGpu {
        mmio_base,
        mmio_size: mmio_pages * 4096,
        fb_phys: boot_fb,
        width: boot_w,
        height: boot_h,
        pitch: boot_pitch,
        bpp: 32,
        has_vbe: false,
        gen,
    };

    // Try to read actual pipe configuration from hardware
    if mmio_base != 0 {
        gpu.read_pipe_config();
    }

    crate::serial_println!("[OK] Intel HD Graphics: {}x{} pitch={} fb={:#x} gen={}",
        gpu.width, gpu.height, gpu.pitch, gpu.fb_phys, gen);

    super::register(Box::new(gpu));
    true
}

pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    if init_and_register(pci) {
        super::create_hal_driver("Intel HD Graphics")
    } else {
        None
    }
}
