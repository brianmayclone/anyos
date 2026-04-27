//! AMD/ATI GPU framebuffer driver.
//!
//! Provides basic framebuffer access for AMD Radeon GPUs.
//! Like the Intel driver, this inherits the firmware-configured display mode
//! (UEFI GOP or VBE) and exposes it as a linear framebuffer.
//!
//! Covers PCI vendor 0x1002 (AMD/ATI), class 03:00 (VGA).
//! Supports: Radeon HD 5000+, R7/R9, RX 400/500, RX 5000 (Navi), RX 6000/7000.
//!
//! # Strategy
//! AMD GPUs have extremely complex register sets (MMHUB, DCN, GFX, etc.).
//! A full amdgpu/DC driver is thousands of lines. Instead, we:
//! 1. Read the VRAM BAR (BAR0) as the framebuffer base
//! 2. Read CRTC/display controller registers for current mode (if MMIO mapped)
//! 3. Fall back to boot framebuffer info (UEFI GOP) for dimensions
//! 4. Support basic modesetting via ATOMBIOS if available
//!
//! This gives us a working display on virtually all AMD hardware that has
//! firmware-initialized video output (i.e., every system with a monitor).

use super::GpuDriver;
use crate::drivers::pci::PciDevice;
use crate::memory::address::PhysAddr;
use crate::memory::virtual_mem;
use alloc::boxed::Box;

// ── MMIO Register Offsets ───────────────────────────────────────────────────

// Display Controller registers (DCE 6.x+)
const CRTC_STATUS: u32 = 0x6E8C; // CRTC status
const CRTC_H_TOTAL: u32 = 0x6D00; // Horizontal total
const CRTC_V_TOTAL: u32 = 0x6D08; // Vertical total
const GRPH_ENABLE: u32 = 0x6800; // Graphics enable
const GRPH_PRIMARY_SURFACE: u32 = 0x6810; // Primary surface address
const GRPH_PITCH: u32 = 0x6820; // Surface pitch (in pixels)
const GRPH_SURFACE_SIZE: u32 = 0x6830; // Surface size (w/h)
const GRPH_CONTROL: u32 = 0x6804; // Surface format control

// DCN (Display Core Next) registers for newer GPUs (Navi+)
const HUBP_SURFACE_ADDR: u32 = 0x15D4;
const HUBP_SURFACE_PITCH: u32 = 0x15DC;

// ── Driver State ────────────────────────────────────────────────────────────

pub struct AmdFbGpu {
    mmio_base: u64,
    fb_phys: u64,
    fb_phys_u32: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    vram_size: u32,
    family: AmdFamily,
}

#[derive(Debug, Clone, Copy)]
enum AmdFamily {
    PreGcn, // HD 5000/6000
    Gcn1,   // HD 7000, R7/R9 200
    Gcn2,   // R7/R9 300
    Gcn4,   // RX 400/500 (Polaris)
    Gcn5,   // Vega
    Rdna1,  // RX 5000 (Navi 10/14)
    Rdna2,  // RX 6000 (Navi 21/22/23)
    Rdna3,  // RX 7000 (Navi 31/32/33)
    Unknown,
}

fn detect_family(device_id: u16) -> AmdFamily {
    match device_id {
        // Polaris (RX 460/470/480/560/570/580)
        0x67DF | 0x67EF | 0x6FDF | 0x699F => AmdFamily::Gcn4,
        // Vega
        0x6860 | 0x6861 | 0x6862 | 0x6863 | 0x687F | 0x69AF => AmdFamily::Gcn5,
        // Navi 10 (RX 5600/5700)
        0x7310 | 0x7312 | 0x7318 | 0x731F | 0x7340 => AmdFamily::Rdna1,
        // Navi 14 (RX 5300/5500)
        0x7340 | 0x7341 | 0x7347 => AmdFamily::Rdna1,
        // Navi 21 (RX 6800/6900)
        0x73A5 | 0x73AB | 0x73AF | 0x73BF => AmdFamily::Rdna2,
        // Navi 22 (RX 6700)
        0x73DF | 0x73E3 | 0x73FF => AmdFamily::Rdna2,
        // Navi 23 (RX 6600)
        0x73E0 | 0x73E1 => AmdFamily::Rdna2,
        // Navi 31 (RX 7900)
        0x744C | 0x7448 => AmdFamily::Rdna3,
        // Navi 32 (RX 7700/7800)
        0x747E => AmdFamily::Rdna3,
        // Navi 33 (RX 7600)
        0x7480 | 0x7483 => AmdFamily::Rdna3,
        // Older GCN
        0x6798 | 0x679A | 0x679E | 0x6818 | 0x6819 => AmdFamily::Gcn1,
        _ => AmdFamily::Unknown,
    }
}

impl AmdFbGpu {
    #[inline]
    unsafe fn rd(&self, reg: u32) -> u32 {
        if self.mmio_base == 0 {
            return 0;
        }
        core::ptr::read_volatile((self.mmio_base + reg as u64) as *const u32)
    }

    fn read_display_config(&mut self) {
        if self.mmio_base == 0 {
            return;
        }
        unsafe {
            // Try DCE-style registers first
            let grph_en = self.rd(GRPH_ENABLE);
            if grph_en & 1 != 0 {
                let surface = self.rd(GRPH_PRIMARY_SURFACE);
                if surface != 0 {
                    self.fb_phys_u32 = surface;
                }

                let pitch_px = self.rd(GRPH_PITCH);
                if pitch_px > 0 {
                    self.pitch = pitch_px * 4;
                } // Convert pixels → bytes

                let size = self.rd(GRPH_SURFACE_SIZE);
                if size != 0 {
                    let w = size & 0x3FFF;
                    let h = (size >> 16) & 0x3FFF;
                    if w > 0 && h > 0 {
                        self.width = w;
                        self.height = h;
                    }
                }
            }
        }
    }
}

impl GpuDriver for AmdFbGpu {
    fn name(&self) -> &str {
        "AMD Radeon"
    }
    fn driver_type_name(&self) -> &str {
        "amd"
    }

    fn set_mode(&mut self, width: u32, height: u32, bpp: u32) -> Option<(u32, u32, u32, u32)> {
        // For firmware-initialized framebuffer, we can adjust pitch/size
        // but full modesetting requires ATOMBIOS (complex, not implemented)
        if self.mmio_base != 0 {
            let new_pitch = width * (bpp / 8);
            unsafe {
                let base = self.mmio_base;
                core::ptr::write_volatile((base + GRPH_PITCH as u64) as *mut u32, width);
                core::ptr::write_volatile(
                    (base + GRPH_SURFACE_SIZE as u64) as *mut u32,
                    (width & 0x3FFF) | ((height & 0x3FFF) << 16),
                );
                // Trigger update
                core::ptr::write_volatile(
                    (base + GRPH_PRIMARY_SURFACE as u64) as *mut u32,
                    self.fb_phys_u32,
                );
            }
            self.width = width;
            self.height = height;
            self.pitch = new_pitch;
            self.bpp = bpp;
            Some((width, height, new_pitch, self.fb_phys_u32))
        } else {
            None
        }
    }

    fn get_mode(&self) -> (u32, u32, u32, u32) {
        (self.width, self.height, self.pitch, self.fb_phys_u32)
    }

    fn has_accel(&self) -> bool {
        false
    }
    fn has_hw_cursor(&self) -> bool {
        false
    }
    fn has_double_buffer(&self) -> bool {
        false
    }

    fn vram_size(&self) -> u32 {
        self.vram_size
    }
}

// ── Initialization ──────────────────────────────────────────────────────────

pub fn init_and_register(pci: &PciDevice) -> bool {
    let family = detect_family(pci.device_id);
    crate::serial_println!(
        "  AMD GPU: device={:#06x} family={:?}",
        pci.device_id,
        family
    );

    // BAR0 = VRAM (framebuffer), BAR2 = MMIO doorbell, BAR5 = MMIO registers
    // Layout varies by generation — try common patterns
    let bar0 = (pci.bars[0] & !0xF) as u64;
    let bar0_hi = if pci.bars[0] & 0x04 != 0 {
        (pci.bars[1] as u64) << 32
    } else {
        0
    };
    let vram_phys = bar0 | bar0_hi;

    let bar2 = (pci.bars[2] & !0xF) as u64;
    let bar2_hi = if pci.bars[2] & 0x04 != 0 {
        (pci.bars[3] as u64) << 32
    } else {
        0
    };
    let doorbell_phys = bar2 | bar2_hi;

    let bar5 = (pci.bars[4] & !0xF) as u64;
    let bar5_hi = if pci.bars[4] & 0x04 != 0 {
        (pci.bars[5] as u64) << 32
    } else {
        0
    };
    let mmio_phys = bar5 | bar5_hi;

    // Use BAR2 as MMIO if BAR5 is 0 (some GPUs use different BAR layouts)
    let reg_phys = if mmio_phys != 0 {
        mmio_phys
    } else {
        doorbell_phys
    };

    crate::serial_println!(
        "  AMD GPU: VRAM={:#014x} MMIO={:#014x}",
        vram_phys,
        reg_phys
    );

    // Enable memory space + bus mastering
    let cmd = crate::drivers::pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    crate::drivers::pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    // Map MMIO registers (256 KiB should cover DCE registers)
    let mmio_base = if reg_phys != 0 {
        match virtual_mem::map_mmio(PhysAddr::new(reg_phys), 64) {
            Some(v) => v.as_u64(),
            None => 0,
        }
    } else {
        0
    };

    // Get framebuffer info from boot
    let (boot_fb, boot_w, boot_h, boot_pitch) = crate::drivers::framebuffer::info()
        .map(|fb| (fb.addr as u32, fb.width, fb.height, fb.pitch))
        .unwrap_or((vram_phys as u32, 1024, 768, 4096));

    let mut gpu = AmdFbGpu {
        mmio_base,
        fb_phys: vram_phys,
        fb_phys_u32: boot_fb,
        width: boot_w,
        height: boot_h,
        pitch: boot_pitch,
        bpp: 32,
        vram_size: 256 * 1024 * 1024, // Conservative estimate
        family,
    };

    // Try reading actual config from hardware
    gpu.read_display_config();

    crate::serial_println!(
        "[OK] AMD Radeon: {}x{} pitch={} fb={:#x} family={:?}",
        gpu.width,
        gpu.height,
        gpu.pitch,
        gpu.fb_phys_u32,
        family
    );

    super::register(Box::new(gpu));
    true
}

pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    if init_and_register(pci) {
        super::create_hal_driver("AMD Radeon Graphics")
    } else {
        None
    }
}
