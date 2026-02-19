//! VirtualBox VGA (VBoxVGA) GPU driver with HGSMI acceleration.
//!
//! PCI device: vendor 0x80EE, device 0xBEEF. Uses DISPI (Bochs Graphics Adapter)
//! registers for mode setting and double-buffering, plus HGSMI (Host-Guest Shared
//! Memory Interface) for hardware cursor and dirty-rectangle notification.
//!
//! HGSMI commands are placed in a 64 KiB heap at the end of VRAM and submitted
//! to the host by writing their VRAM offset to I/O port 0x3D0.

use super::GpuDriver;
use alloc::boxed::Box;
use alloc::vec;
use crate::drivers::pci::PciDevice;
use crate::memory::address::{PhysAddr, VirtAddr};

// ──────────────────────────────────────────────
// DISPI (Bochs Display Interface) registers
// ──────────────────────────────────────────────

const VBE_DISPI_IOPORT_INDEX: u16 = 0x01CE;
const VBE_DISPI_IOPORT_DATA: u16  = 0x01CF;

const VBE_DISPI_INDEX_ID: u16         = 0x00;
const VBE_DISPI_INDEX_XRES: u16       = 0x01;
const VBE_DISPI_INDEX_YRES: u16       = 0x02;
const VBE_DISPI_INDEX_BPP: u16        = 0x03;
const VBE_DISPI_INDEX_ENABLE: u16     = 0x04;
const VBE_DISPI_INDEX_VIRT_WIDTH: u16 = 0x06;
const VBE_DISPI_INDEX_VIRT_HEIGHT: u16 = 0x07;
const VBE_DISPI_INDEX_Y_OFFSET: u16   = 0x09;

const VBE_DISPI_ENABLED: u16     = 0x01;
const VBE_DISPI_LFB_ENABLED: u16 = 0x40;
const VBE_DISPI_NOCLEARMEM: u16  = 0x80;

// VBox-specific DISPI IDs (capability levels)
const VBE_DISPI_ID_VBOX_VIDEO: u16 = 0xBE00;
const VBE_DISPI_ID_HGSMI: u16     = 0xBE01;
const VBE_DISPI_ID_ANYX: u16      = 0xBE02;

// ──────────────────────────────────────────────
// HGSMI protocol constants
// ──────────────────────────────────────────────

/// Guest-to-host HGSMI command port (write VRAM offset of command buffer here)
const HGSMI_PORT_GUEST: u16 = 0x3D0;

/// HGSMI channel for VBVA (Video Buffer Video Acceleration)
const HGSMI_CH_VBVA: u8 = 0x02;

/// VBVA command codes (used as channel_info in HGSMI header)
const VBVA_INFO_SCREEN: u16         = 6;
const VBVA_MOUSE_POINTER_SHAPE: u16 = 8;
const VBVA_INFO_CAPS: u16           = 12;
const VBVA_CURSOR_POSITION: u16     = 21;

/// Mouse pointer shape flags
const VBOX_MOUSE_POINTER_VISIBLE: u32 = 0x01;
const VBOX_MOUSE_POINTER_ALPHA: u32   = 0x02;
const VBOX_MOUSE_POINTER_SHAPE: u32   = 0x04;

// VBVA_INFO_SCREEN flags
const VBVA_SCREEN_F_ACTIVE: u16 = 0x01;

// ──────────────────────────────────────────────
// HGSMI heap layout
// ──────────────────────────────────────────────

/// Kernel virtual base for the HGSMI heap mapping.
/// Reuses VMware SVGA FIFO address — these devices are mutually exclusive.
const HGSMI_HEAP_VIRT_BASE: u64 = 0xFFFF_FFFF_D002_0000;
const HGSMI_HEAP_SIZE: u32 = 65536;     // 64 KiB
const HGSMI_HEAP_PAGES: usize = 16;     // 64 KiB / 4 KiB

// ──────────────────────────────────────────────
// HGSMI data structures (all repr(C, packed))
// ──────────────────────────────────────────────

/// HGSMI buffer header — 16 bytes, placed at the start of each command in VRAM.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct HgsmiBufferHeader {
    data_size: u32,      // payload size in bytes (excludes header + tail)
    flags: u8,           // 0x00 = single (non-fragmented)
    channel: u8,         // HGSMI_CH_VBVA = 0x02
    channel_info: u16,   // VBVA command code
    _reserved: [u8; 8],  // must be zero
}

/// HGSMI buffer tail — 8 bytes, placed after the payload.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct HgsmiBufferTail {
    _reserved: u32,      // must be zero
    checksum: u32,       // Jenkins one-at-a-time hash of header + payload
}

/// VBVA_MOUSE_POINTER_SHAPE payload header (followed by pixel data).
#[repr(C, packed)]
struct VbvaMousePointerShape {
    result: i32,         // set by host on completion
    flags: u32,          // VISIBLE | ALPHA | SHAPE
    hot_x: u32,
    hot_y: u32,
    width: u32,
    height: u32,
    // followed by: AND mask (for non-alpha) then pixel data (ARGB8888)
}

/// VBVA_CURSOR_POSITION payload.
#[repr(C, packed)]
struct VbvaCursorPosition {
    report: u32,   // 1 = report position, 0 = query
    x: u32,
    y: u32,
}

/// VBVA_INFO_SCREEN payload.
#[repr(C, packed)]
struct VbvaInfoScreen {
    view_index: u32,
    origin_x: i32,
    origin_y: i32,
    start_offset: u32,
    line_size: u32,      // pitch in bytes
    width: u32,
    height: u32,
    bpp: u16,
    flags: u16,          // VBVA_SCREEN_F_ACTIVE
}

/// VBVA_INFO_CAPS payload.
#[repr(C, packed)]
struct VbvaInfoCaps {
    flags: u32,
    _reserved: [u32; 3],
}

// ──────────────────────────────────────────────
// DISPI register helpers
// ──────────────────────────────────────────────

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

// ──────────────────────────────────────────────
// Jenkins one-at-a-time hash (HGSMI checksum)
// ──────────────────────────────────────────────

fn hgsmi_checksum(data: &[u8]) -> u32 {
    let mut hash: u32 = 0;
    for &byte in data {
        hash = hash.wrapping_add(byte as u32);
        hash = hash.wrapping_add(hash << 10);
        hash ^= hash >> 6;
    }
    hash = hash.wrapping_add(hash << 3);
    hash ^= hash >> 11;
    hash = hash.wrapping_add(hash << 15);
    hash
}

// ──────────────────────────────────────────────
// VBoxVGA GPU driver
// ──────────────────────────────────────────────

pub struct VBoxVgaGpu {
    fb_phys: u32,
    vram_size: u32,
    width: u32,
    height: u32,
    pitch: u32,
    double_buffered: bool,
    front_page: u32,

    // HGSMI state
    hgsmi_supported: bool,
    hgsmi_heap_virt: u64,     // kernel virtual address of HGSMI heap
    hgsmi_heap_offset: u32,   // offset within VRAM where heap starts
    hgsmi_alloc_offset: u32,  // bump allocator within heap

    // Cursor state
    cursor_defined: bool,
}

impl VBoxVgaGpu {
    /// Allocate an HGSMI buffer in the heap. Returns (kernel_virt_ptr, vram_offset).
    /// Commands are processed synchronously, so we reset the allocator each time.
    fn hgsmi_alloc(&mut self, data_size: u32) -> Option<(u64, u32)> {
        if !self.hgsmi_supported {
            return None;
        }

        let total = 16 + data_size + 8; // header + payload + tail
        let aligned = (total + 7) & !7;  // 8-byte aligned

        if self.hgsmi_alloc_offset + aligned > HGSMI_HEAP_SIZE {
            // Reset — previous commands already processed
            self.hgsmi_alloc_offset = 0;
        }

        let virt = self.hgsmi_heap_virt + self.hgsmi_alloc_offset as u64;
        let vram_off = self.hgsmi_heap_offset + self.hgsmi_alloc_offset;
        self.hgsmi_alloc_offset += aligned;

        Some((virt, vram_off))
    }

    /// Build and submit an HGSMI command with the given VBVA command code and payload.
    fn hgsmi_submit(&mut self, channel_info: u16, payload: &[u8]) {
        let data_size = payload.len() as u32;
        let (virt, vram_off) = match self.hgsmi_alloc(data_size) {
            Some(v) => v,
            None => return,
        };

        // Write header
        let header = HgsmiBufferHeader {
            data_size,
            flags: 0x00, // single (non-fragmented)
            channel: HGSMI_CH_VBVA,
            channel_info,
            _reserved: [0; 8],
        };
        unsafe {
            core::ptr::write_volatile(virt as *mut HgsmiBufferHeader, header);
        }

        // Write payload
        if !payload.is_empty() {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    payload.as_ptr(),
                    (virt + 16) as *mut u8,
                    payload.len(),
                );
            }
        }

        // Calculate checksum over header + payload
        let check_data = unsafe {
            core::slice::from_raw_parts(virt as *const u8, 16 + data_size as usize)
        };
        let checksum = hgsmi_checksum(check_data);

        // Write tail
        let tail = HgsmiBufferTail {
            _reserved: 0,
            checksum,
        };
        unsafe {
            core::ptr::write_volatile(
                (virt + 16 + data_size as u64) as *mut HgsmiBufferTail,
                tail,
            );
        }

        // Memory fence to ensure all VRAM writes are visible before port write
        core::sync::atomic::fence(core::sync::atomic::Ordering::SeqCst);

        // Submit command: write VRAM offset to guest HGSMI port
        unsafe {
            crate::arch::x86::port::outl(HGSMI_PORT_GUEST, vram_off);
        }
    }

    /// Send VBVA_INFO_SCREEN to configure display geometry.
    fn send_info_screen(&mut self) {
        let screen = VbvaInfoScreen {
            view_index: 0,
            origin_x: 0,
            origin_y: 0,
            start_offset: 0,
            line_size: self.pitch,
            width: self.width,
            height: self.height,
            bpp: 32,
            flags: VBVA_SCREEN_F_ACTIVE,
        };
        let payload = unsafe {
            core::slice::from_raw_parts(
                &screen as *const _ as *const u8,
                core::mem::size_of::<VbvaInfoScreen>(),
            )
        };
        self.hgsmi_submit(VBVA_INFO_SCREEN, payload);
    }

    /// Send VBVA_INFO_CAPS to report guest driver capabilities.
    fn send_info_caps(&mut self) {
        let caps = VbvaInfoCaps {
            flags: 0, // no special capabilities for V1
            _reserved: [0; 3],
        };
        let payload = unsafe {
            core::slice::from_raw_parts(
                &caps as *const _ as *const u8,
                core::mem::size_of::<VbvaInfoCaps>(),
            )
        };
        self.hgsmi_submit(VBVA_INFO_CAPS, payload);
    }
}

impl GpuDriver for VBoxVgaGpu {
    fn name(&self) -> &str {
        if self.hgsmi_supported {
            "VBoxVGA (HGSMI)"
        } else {
            "VBoxVGA"
        }
    }

    fn set_mode(&mut self, width: u32, height: u32, bpp: u32) -> Option<(u32, u32, u32, u32)> {
        // Disable VBE
        dispi_write(VBE_DISPI_INDEX_ENABLE, 0);

        // Set resolution
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

        dispi_write(VBE_DISPI_INDEX_Y_OFFSET, 0);

        // Update HGSMI screen info
        if self.hgsmi_supported {
            self.send_info_screen();
        }

        crate::serial_println!(
            "  VBoxVGA: mode set to {}x{}x{} (pitch={}, dblbuf={})",
            actual_w, actual_h, actual_bpp, pitch, self.double_buffered
        );

        Some((actual_w, actual_h, pitch, self.fb_phys))
    }

    fn get_mode(&self) -> (u32, u32, u32, u32) {
        (self.width, self.height, self.pitch, self.fb_phys)
    }

    // ── 2D Acceleration ──────────────────────────────────

    fn has_accel(&self) -> bool {
        self.hgsmi_supported
    }

    fn accel_fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: u32) -> bool {
        if self.fb_phys == 0 || w == 0 || h == 0 {
            return false;
        }
        // Clamp to display bounds
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width.saturating_sub(x));
        let h = h.min(self.height.saturating_sub(y));
        if w == 0 || h == 0 {
            return false;
        }

        // Software fill on VRAM (VBoxVGA has no GPU fill primitive)
        let fb = self.fb_phys as *mut u32;
        let pitch_u32 = (self.pitch / 4) as usize;
        for row in y..(y + h) {
            let offset = (row as usize) * pitch_u32 + (x as usize);
            unsafe {
                let dst = fb.add(offset);
                for col in 0..(w as usize) {
                    core::ptr::write_volatile(dst.add(col), color);
                }
            }
        }
        true
    }

    fn accel_copy_rect(&mut self, sx: u32, sy: u32, dx: u32, dy: u32, w: u32, h: u32) -> bool {
        if self.fb_phys == 0 || w == 0 || h == 0 {
            return false;
        }

        let fb = self.fb_phys as *mut u32;
        let pitch_u32 = (self.pitch / 4) as usize;

        // Copy with correct overlap handling
        if dy <= sy {
            for row in 0..(h as usize) {
                let src_off = (sy as usize + row) * pitch_u32 + sx as usize;
                let dst_off = (dy as usize + row) * pitch_u32 + dx as usize;
                unsafe { core::ptr::copy(fb.add(src_off), fb.add(dst_off), w as usize); }
            }
        } else {
            for row in (0..(h as usize)).rev() {
                let src_off = (sy as usize + row) * pitch_u32 + sx as usize;
                let dst_off = (dy as usize + row) * pitch_u32 + dx as usize;
                unsafe { core::ptr::copy(fb.add(src_off), fb.add(dst_off), w as usize); }
            }
        }
        true
    }

    fn update_rect(&mut self, _x: u32, _y: u32, _w: u32, _h: u32) {
        // Touch the Y_OFFSET register with the current value to signal VBox
        // that the framebuffer has changed. This triggers VBox's display update
        // path rather than relying on periodic full-VRAM scanning.
        let y = self.front_page * self.height;
        dispi_write(VBE_DISPI_INDEX_Y_OFFSET, y as u16);
    }

    // ── Hardware Cursor ──────────────────────────────────

    fn has_hw_cursor(&self) -> bool {
        self.hgsmi_supported
    }

    fn define_cursor(&mut self, w: u32, h: u32, hotx: u32, hoty: u32, pixels: &[u32]) {
        if !self.hgsmi_supported || w == 0 || h == 0 {
            return;
        }
        if pixels.len() != (w * h) as usize {
            return;
        }

        // Build VBVA_MOUSE_POINTER_SHAPE payload:
        // [VbvaMousePointerShape header] [ARGB pixel data]
        let shape_size = core::mem::size_of::<VbvaMousePointerShape>();
        let pixel_bytes = (w * h * 4) as usize;
        let total_payload = shape_size + pixel_bytes;

        let mut payload = vec![0u8; total_payload];

        // Fill shape header
        let shape = VbvaMousePointerShape {
            result: 0,
            flags: VBOX_MOUSE_POINTER_VISIBLE | VBOX_MOUSE_POINTER_ALPHA | VBOX_MOUSE_POINTER_SHAPE,
            hot_x: hotx,
            hot_y: hoty,
            width: w,
            height: h,
        };
        unsafe {
            core::ptr::copy_nonoverlapping(
                &shape as *const _ as *const u8,
                payload.as_mut_ptr(),
                shape_size,
            );
        }

        // Copy ARGB pixel data
        unsafe {
            core::ptr::copy_nonoverlapping(
                pixels.as_ptr() as *const u8,
                payload.as_mut_ptr().add(shape_size),
                pixel_bytes,
            );
        }

        self.hgsmi_submit(VBVA_MOUSE_POINTER_SHAPE, &payload);
        self.cursor_defined = true;
    }

    fn move_cursor(&mut self, x: u32, y: u32) {
        if !self.hgsmi_supported {
            return;
        }

        // Send cursor position via HGSMI
        let pos = VbvaCursorPosition {
            report: 1, // report = set position
            x,
            y,
        };
        let payload = unsafe {
            core::slice::from_raw_parts(
                &pos as *const _ as *const u8,
                core::mem::size_of::<VbvaCursorPosition>(),
            )
        };
        self.hgsmi_submit(VBVA_CURSOR_POSITION, payload);
    }

    fn show_cursor(&mut self, visible: bool) {
        if !self.hgsmi_supported {
            return;
        }

        // Send a minimal VBVA_MOUSE_POINTER_SHAPE with only the visibility flag
        let shape = VbvaMousePointerShape {
            result: 0,
            flags: if visible { VBOX_MOUSE_POINTER_VISIBLE } else { 0 },
            hot_x: 0,
            hot_y: 0,
            width: 0,
            height: 0,
        };
        let payload = unsafe {
            core::slice::from_raw_parts(
                &shape as *const _ as *const u8,
                core::mem::size_of::<VbvaMousePointerShape>(),
            )
        };
        self.hgsmi_submit(VBVA_MOUSE_POINTER_SHAPE, payload);
    }

    // ── Double Buffering ─────────────────────────────────

    fn has_double_buffer(&self) -> bool {
        self.double_buffered
    }

    fn flip(&mut self) {
        if !self.double_buffered {
            return;
        }
        self.front_page ^= 1;
        let y_offset = self.front_page * self.height;
        dispi_write(VBE_DISPI_INDEX_Y_OFFSET, y_offset as u16);
    }

    fn back_buffer_phys(&self) -> Option<u32> {
        if !self.double_buffered {
            return None;
        }
        let back_page = self.front_page ^ 1;
        let offset = back_page * self.height * self.pitch;
        Some(self.fb_phys + offset)
    }
}

// ──────────────────────────────────────────────
// PCI BAR size detection
// ──────────────────────────────────────────────

/// Detect the size of PCI BAR0 by writing all 1s and reading back.
fn detect_bar0_size(bus: u8, device: u8, function: u8) -> u32 {
    use crate::drivers::pci::{pci_config_read32, pci_config_write32};

    let bar_offset: u8 = 0x10; // BAR0

    // Save original BAR0 value
    let original = pci_config_read32(bus, device, function, bar_offset);

    // Disable memory decoding while probing
    let cmd = pci_config_read32(bus, device, function, 0x04);
    pci_config_write32(bus, device, function, 0x04, cmd & !0x03);

    // Write all 1s to BAR0
    pci_config_write32(bus, device, function, bar_offset, 0xFFFF_FFFF);

    // Read back to get size mask
    let mask = pci_config_read32(bus, device, function, bar_offset);

    // Restore original BAR0
    pci_config_write32(bus, device, function, bar_offset, original);

    // Restore command register
    pci_config_write32(bus, device, function, 0x04, cmd);

    if mask == 0 || mask == 0xFFFF_FFFF {
        return 0;
    }

    // For memory BAR: clear lower 4 bits, invert, add 1
    let size = !(mask & !0xF) + 1;
    size
}

// ──────────────────────────────────────────────
// Initialization
// ──────────────────────────────────────────────

/// Initialize and register the VBoxVGA GPU driver.
/// Called from HAL PCI probe when device 80EE:BEEF is found.
pub fn init_and_register(pci_dev: &PciDevice) {
    // Read BAR0 = framebuffer physical address
    let fb_phys = pci_dev.bars[0] & !0xF;
    if fb_phys == 0 {
        crate::serial_println!("  VBoxVGA: BAR0 is zero, cannot init");
        return;
    }

    // Detect VRAM size from BAR0
    let vram_size = detect_bar0_size(pci_dev.bus, pci_dev.device, pci_dev.function);
    if vram_size == 0 {
        crate::serial_println!("  VBoxVGA: Could not detect VRAM size, using 16 MiB default");
    }
    let vram_size = if vram_size > 0 { vram_size } else { 16 * 1024 * 1024 };

    crate::serial_println!(
        "  VBoxVGA: fb_phys={:#010x}, vram_size={} MiB",
        fb_phys, vram_size / (1024 * 1024)
    );

    // Enable PCI memory access + bus mastering
    let cmd = crate::drivers::pci::pci_config_read32(
        pci_dev.bus, pci_dev.device, pci_dev.function, 0x04,
    );
    crate::drivers::pci::pci_config_write32(
        pci_dev.bus, pci_dev.device, pci_dev.function,
        0x04, cmd | 0x07,
    );

    // Negotiate VBox HGSMI capability via DISPI ID register
    let mut hgsmi_supported = false;

    // Try highest first: ANYX (arbitrary resolution)
    dispi_write(VBE_DISPI_INDEX_ID, VBE_DISPI_ID_ANYX);
    let id = dispi_read(VBE_DISPI_INDEX_ID);
    if id == VBE_DISPI_ID_ANYX {
        hgsmi_supported = true;
        crate::serial_println!("  VBoxVGA: DISPI ID {:#06x} (ANYX + HGSMI)", id);
    } else {
        // Try HGSMI
        dispi_write(VBE_DISPI_INDEX_ID, VBE_DISPI_ID_HGSMI);
        let id = dispi_read(VBE_DISPI_INDEX_ID);
        if id == VBE_DISPI_ID_HGSMI {
            hgsmi_supported = true;
            crate::serial_println!("  VBoxVGA: DISPI ID {:#06x} (HGSMI)", id);
        } else {
            // Try VBOX_VIDEO
            dispi_write(VBE_DISPI_INDEX_ID, VBE_DISPI_ID_VBOX_VIDEO);
            let id = dispi_read(VBE_DISPI_INDEX_ID);
            if id == VBE_DISPI_ID_VBOX_VIDEO {
                hgsmi_supported = true;
                crate::serial_println!("  VBoxVGA: DISPI ID {:#06x} (VBOX_VIDEO)", id);
            } else {
                crate::serial_println!("  VBoxVGA: DISPI ID {:#06x} (standard BGA, no HGSMI)", id);
            }
        }
    }

    // Map HGSMI heap at end of VRAM into kernel virtual space
    let heap_offset = vram_size - HGSMI_HEAP_SIZE;
    let heap_phys = fb_phys + heap_offset;

    if hgsmi_supported {
        crate::serial_println!(
            "  VBoxVGA: Mapping HGSMI heap: phys={:#010x}, virt={:#018x}, {} pages",
            heap_phys, HGSMI_HEAP_VIRT_BASE, HGSMI_HEAP_PAGES
        );
        for i in 0..HGSMI_HEAP_PAGES {
            let virt = VirtAddr::new(HGSMI_HEAP_VIRT_BASE + (i as u64) * 4096);
            let phys = PhysAddr::new(heap_phys as u64 + (i as u64) * 4096);
            // Map as uncacheable (0x03 = Present + Writable, no WC)
            // HGSMI control structures need strict ordering
            crate::memory::virtual_mem::map_page(virt, phys, 0x03);
        }

        // Zero the heap area
        unsafe {
            core::ptr::write_bytes(HGSMI_HEAP_VIRT_BASE as *mut u8, 0, HGSMI_HEAP_SIZE as usize);
        }
    }

    // Read current display mode from bootloader/DISPI
    let width = dispi_read(VBE_DISPI_INDEX_XRES) as u32;
    let height = dispi_read(VBE_DISPI_INDEX_YRES) as u32;
    let bpp = dispi_read(VBE_DISPI_INDEX_BPP) as u32;
    let virt_height = dispi_read(VBE_DISPI_INDEX_VIRT_HEIGHT) as u32;
    let pitch = width * (bpp / 8);
    let double_buffered = virt_height >= height * 2;

    crate::serial_println!(
        "  VBoxVGA: Current mode {}x{}x{} (pitch={}, dblbuf={})",
        width, height, bpp, pitch, double_buffered
    );

    let mut gpu = VBoxVgaGpu {
        fb_phys,
        vram_size,
        width,
        height,
        pitch,
        double_buffered,
        front_page: 0,
        hgsmi_supported,
        hgsmi_heap_virt: if hgsmi_supported { HGSMI_HEAP_VIRT_BASE } else { 0 },
        hgsmi_heap_offset: heap_offset,
        hgsmi_alloc_offset: 0,
        cursor_defined: false,
    };

    // Configure display via HGSMI
    if hgsmi_supported {
        gpu.send_info_caps();
        gpu.send_info_screen();
        crate::serial_println!("  VBoxVGA: HGSMI initialized (hw_cursor + update_rect)");
    }

    // Register with GPU subsystem
    super::register(Box::new(gpu));
}
