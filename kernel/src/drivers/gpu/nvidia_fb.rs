//! NVIDIA GPU basic framebuffer driver.
//!
//! Provides a firmware-inherited linear framebuffer for NVIDIA GeForce GPUs.
//! This is NOT a full nouveau driver — no 3D, no modesetting, no power
//! management. It simply uses the display mode configured by UEFI GOP or
//! the VBIOS and exposes the framebuffer for 2D compositor use.
//!
//! Covers PCI vendor 0x10DE (NVIDIA), class 03:00 or 03:02 (VGA/3D).
//! Supports: GeForce 600+ (Kepler), 700 (Kepler), 900 (Maxwell),
//! GTX 1000 (Pascal), RTX 2000 (Turing), RTX 3000 (Ampere), RTX 4000 (Ada).
//!
//! # Strategy
//! NVIDIA GPUs have a proprietary register set. However, the UEFI firmware
//! (or VBIOS) always initializes a working display via GOP. We:
//! 1. Read BAR0 (MMIO) and BAR1 (VRAM aperture) from PCI config
//! 2. Read the display controller registers for the active head (PCRTC)
//! 3. Fall back to boot framebuffer info if register read fails
//! 4. Expose as a simple linear framebuffer — no modesetting
//!
//! This is enough for a working desktop with compositor at native resolution.

use super::GpuDriver;
use crate::drivers::pci::PciDevice;
use crate::memory::address::PhysAddr;
use crate::memory::virtual_mem;
use alloc::boxed::Box;

// ── NVIDIA MMIO Register Offsets ────────────────────────────────────────────
// These are stable across Kepler → Ada architectures for display readback.

// PMC (Power Management Controller)
const NV_PMC_BOOT_0: u32 = 0x000000; // Chip revision / architecture ID

// PCRTC (Pixel Clock / CRT Controller) — Head 0
const NV_PCRTC_START: u32 = 0x600800; // Scanout start address in VRAM
const NV_PCRTC_CONFIG: u32 = 0x600804; // Display config (pixel format)

// PDISPLAY (Display Engine) — varies by generation but these are common
const NV_PDISPLAY_FE_OFFSET: u32 = 0x610000;

// Pre-Kepler (NV50/Fermi) scanout registers
const NV50_PDISPLAY_CRTC_C: u32 = 0x610B58; // Active display size (Fermi)
const NV50_PDISPLAY_PITCH: u32 = 0x610B20; // Display pitch

// Kepler+ display registers
const GK_HEAD_SET_OFFSET: u32 = 0x616300; // Head 0 offset
const GK_HEAD_SET_SIZE: u32 = 0x616308; // Head 0 size (width/height)
const GK_HEAD_SET_PITCH: u32 = 0x616310; // Head 0 pitch (storage pitch)
const GK_HEAD_SET_SURFACE: u32 = 0x616320; // Head 0 surface address

// ── Architecture Detection ──────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
enum NvArch {
    Fermi,   // GF1xx (GeForce 400/500)
    Kepler,  // GK1xx (GeForce 600/700)
    Maxwell, // GM1xx/GM2xx (GeForce 900)
    Pascal,  // GP1xx (GeForce 1000)
    Volta,   // GV1xx (Titan V, Quadro GV100)
    Turing,  // TU1xx (GeForce RTX 2000)
    Ampere,  // GA1xx (GeForce RTX 3000)
    Ada,     // AD1xx (GeForce RTX 4000)
    Unknown,
}

fn detect_arch(device_id: u16) -> NvArch {
    // Rough classification by device ID ranges
    match device_id >> 4 {
        // Fermi (0x0DC0-0x0FFF, 0x1040-0x10FF)
        0x0DC..=0x0FF | 0x104..=0x10F => NvArch::Fermi,
        // Kepler (0x1180-0x11FF, 0x1280-0x12FF)
        0x118..=0x11F | 0x128..=0x12F | 0x0FC..=0x0FE => NvArch::Kepler,
        // Maxwell (0x1340-0x13FF, 0x1740-0x17FF)
        0x134..=0x13F | 0x174..=0x17F | 0x139..=0x139 => NvArch::Maxwell,
        // Pascal (0x15F0-0x15FF, 0x1B00-0x1BFF, 0x1C00-0x1CFF, 0x1D00-0x1DFF)
        0x15F..=0x15F | 0x1B0..=0x1BF | 0x1C0..=0x1CF | 0x1D0..=0x1DF => NvArch::Pascal,
        // Turing (0x1E00-0x1EFF, 0x1F00-0x1FFF, 0x2180-0x21FF)
        0x1E0..=0x1EF | 0x1F0..=0x1FF | 0x218..=0x21F => NvArch::Turing,
        // Ampere (0x2200-0x22FF, 0x2480-0x24FF, 0x2500-0x25FF)
        0x220..=0x22F | 0x248..=0x24F | 0x250..=0x25F => NvArch::Ampere,
        // Ada (0x2680-0x26FF, 0x2780-0x27FF, 0x2800-0x28FF)
        0x268..=0x26F | 0x278..=0x27F | 0x280..=0x28F => NvArch::Ada,
        _ => NvArch::Unknown,
    }
}

// ── Driver State ────────────────────────────────────────────────────────────

pub struct NvidiaFbGpu {
    mmio_base: u64,
    fb_phys: u32,
    width: u32,
    height: u32,
    pitch: u32,
    bpp: u32,
    vram_bar: u64,
    arch: NvArch,
}

impl NvidiaFbGpu {
    #[inline]
    unsafe fn rd(&self, reg: u32) -> u32 {
        if self.mmio_base == 0 {
            return 0;
        }
        core::ptr::read_volatile((self.mmio_base + reg as u64) as *const u32)
    }

    /// Try to read display configuration from GPU registers.
    fn read_display_config(&mut self) {
        if self.mmio_base == 0 {
            return;
        }

        unsafe {
            // Try Kepler+ registers first
            let size = self.rd(GK_HEAD_SET_SIZE);
            if size != 0 && size != 0xFFFFFFFF {
                let w = (size & 0xFFFF) as u32;
                let h = (size >> 16) as u32;
                if w >= 640 && w <= 7680 && h >= 480 && h <= 4320 {
                    self.width = w;
                    self.height = h;
                }
            }

            let pitch = self.rd(GK_HEAD_SET_PITCH);
            if pitch > 0 && pitch < 0x10000 {
                self.pitch = pitch;
            }

            let surface = self.rd(GK_HEAD_SET_SURFACE);
            if surface != 0 && surface != 0xFFFFFFFF {
                self.fb_phys = surface;
            }

            // Try NV50/Fermi registers as fallback
            if self.width == 0 || self.height == 0 {
                let c = self.rd(NV50_PDISPLAY_CRTC_C);
                if c != 0 && c != 0xFFFFFFFF {
                    let h = (c & 0xFFFF) as u32;
                    let w = (c >> 16) as u32;
                    if w >= 640 && h >= 480 {
                        self.width = w;
                        self.height = h;
                    }
                }
                let p = self.rd(NV50_PDISPLAY_PITCH);
                if p > 0 && p < 0x10000 {
                    self.pitch = p;
                }
            }

            // Read scanout address
            let start = self.rd(NV_PCRTC_START);
            if start != 0 && start != 0xFFFFFFFF && self.fb_phys == 0 {
                // This is an offset within VRAM, add VRAM BAR base
                self.fb_phys = self.vram_bar as u32 + start;
            }
        }
    }
}

impl GpuDriver for NvidiaFbGpu {
    fn name(&self) -> &str {
        "NVIDIA GeForce"
    }
    fn driver_type_name(&self) -> &str {
        "nvidia"
    }

    fn set_mode(&mut self, _width: u32, _height: u32, _bpp: u32) -> Option<(u32, u32, u32, u32)> {
        // NVIDIA modesetting requires programming the display engine (EVO/NVDisplay)
        // which is extremely complex and GPU-generation-specific.
        // We only support the firmware-configured mode.
        None
    }

    fn get_mode(&self) -> (u32, u32, u32, u32) {
        (self.width, self.height, self.pitch, self.fb_phys)
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
        // NVIDIA VRAM size can be read from PMC, but varies by generation.
        // Use a safe estimate based on the pitch and height.
        self.pitch * self.height * 2
    }
}

// ── Initialization ──────────────────────────────────────────────────────────

pub fn init_and_register(pci: &PciDevice) -> bool {
    let arch = detect_arch(pci.device_id);
    crate::serial_println!(
        "  NVIDIA GPU: device={:#06x} arch={:?}",
        pci.device_id,
        arch
    );

    // BAR0 = MMIO registers (16-32 MiB)
    let bar0 = (pci.bars[0] & !0xF) as u64;
    let bar0_hi = if pci.bars[0] & 0x04 != 0 {
        (pci.bars[1] as u64) << 32
    } else {
        0
    };
    let mmio_phys = bar0 | bar0_hi;

    // BAR1 = VRAM aperture (framebuffer, 256 MiB - 16 GiB)
    let bar1 = (pci.bars[2] & !0xF) as u64;
    let bar1_hi = if pci.bars[2] & 0x04 != 0 {
        (pci.bars[3] as u64) << 32
    } else {
        0
    };
    let vram_phys = bar1 | bar1_hi;

    crate::serial_println!(
        "  NVIDIA GPU: MMIO={:#014x} VRAM={:#014x}",
        mmio_phys,
        vram_phys
    );

    // Enable memory space + bus mastering
    let cmd = crate::drivers::pci::pci_config_read32(pci.bus, pci.device, pci.function, 0x04);
    crate::drivers::pci::pci_config_write32(pci.bus, pci.device, pci.function, 0x04, cmd | 0x07);

    // Map MMIO registers (map 1 MiB — display engine registers are at 0x600000+)
    let mmio_base = if mmio_phys != 0 {
        match virtual_mem::map_mmio(PhysAddr::new(mmio_phys), 256) {
            Some(v) => v.as_u64(),
            None => 0,
        }
    } else {
        0
    };

    // Get framebuffer info from boot (UEFI GOP)
    let (boot_fb, boot_w, boot_h, boot_pitch) = crate::drivers::framebuffer::info()
        .map(|fb| (fb.addr as u32, fb.width, fb.height, fb.pitch))
        .unwrap_or((vram_phys as u32, 1024, 768, 4096));

    let mut gpu = NvidiaFbGpu {
        mmio_base,
        fb_phys: boot_fb,
        width: boot_w,
        height: boot_h,
        pitch: boot_pitch,
        bpp: 32,
        vram_bar: vram_phys,
        arch,
    };

    // Try to read actual display config from GPU registers
    gpu.read_display_config();

    // Use boot info as authoritative if GPU registers gave garbage
    if gpu.width == 0 || gpu.height == 0 || gpu.pitch == 0 {
        gpu.width = boot_w;
        gpu.height = boot_h;
        gpu.pitch = boot_pitch;
        gpu.fb_phys = boot_fb;
    }

    crate::serial_println!(
        "[OK] NVIDIA GeForce: {}x{} pitch={} fb={:#x} arch={:?}",
        gpu.width,
        gpu.height,
        gpu.pitch,
        gpu.fb_phys,
        arch
    );

    super::register(Box::new(gpu));
    true
}

pub fn probe(pci: &PciDevice) -> Option<Box<dyn crate::drivers::hal::Driver>> {
    if init_and_register(pci) {
        super::create_hal_driver("NVIDIA GeForce")
    } else {
        None
    }
}
