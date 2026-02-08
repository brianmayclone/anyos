use crate::arch::x86::port::{inw, outw};
use crate::sync::spinlock::Spinlock;

// Bochs VBE DISPI I/O ports
const VBE_DISPI_IOPORT_INDEX: u16 = 0x01CE;
const VBE_DISPI_IOPORT_DATA: u16 = 0x01CF;

// DISPI register indices
const VBE_DISPI_INDEX_ID: u16 = 0x00;
const VBE_DISPI_INDEX_XRES: u16 = 0x01;
const VBE_DISPI_INDEX_YRES: u16 = 0x02;
const VBE_DISPI_INDEX_BPP: u16 = 0x03;
const VBE_DISPI_INDEX_ENABLE: u16 = 0x04;
const VBE_DISPI_INDEX_BANK: u16 = 0x05;
const VBE_DISPI_INDEX_VIRT_WIDTH: u16 = 0x06;
const VBE_DISPI_INDEX_VIRT_HEIGHT: u16 = 0x07;
const VBE_DISPI_INDEX_X_OFFSET: u16 = 0x08;
const VBE_DISPI_INDEX_Y_OFFSET: u16 = 0x09;

// DISPI enable flags
const VBE_DISPI_ENABLED: u16 = 0x01;
const VBE_DISPI_LFB_ENABLED: u16 = 0x40;
const VBE_DISPI_NOCLEARMEM: u16 = 0x80;

struct BochsVga {
    fb_phys: u32,
    pitch: u32,
    width: u32,
    height: u32,
    front_page: u8, // 0 or 1
    double_buffer_enabled: bool,
}

static BOCHS_VGA: Spinlock<Option<BochsVga>> = Spinlock::new(None);

fn dispi_read(index: u16) -> u16 {
    unsafe {
        outw(VBE_DISPI_IOPORT_INDEX, index);
        inw(VBE_DISPI_IOPORT_DATA)
    }
}

fn dispi_write(index: u16, value: u16) {
    unsafe {
        outw(VBE_DISPI_IOPORT_INDEX, index);
        outw(VBE_DISPI_IOPORT_DATA, value);
    }
}

/// Initialize Bochs VGA double-buffering.
/// Returns true if double-buffer was enabled.
pub fn init(fb_phys: u32, width: u32, height: u32, pitch: u32) -> bool {
    // Check DISPI ID register (valid range: 0xB0C0 - 0xB0CF)
    let id = dispi_read(VBE_DISPI_INDEX_ID);
    if id < 0xB0C0 || id > 0xB0CF {
        crate::serial_println!("  BochsVGA: not detected (ID={:#06x})", id);
        return false;
    }

    crate::serial_println!("  BochsVGA: detected (ID={:#06x})", id);

    // Request virtual height = 2 * physical height for page flipping
    let virt_height = height as u16 * 2;

    // Re-enable VBE with NOCLEARMEM to preserve current display
    let current_enable = dispi_read(VBE_DISPI_INDEX_ENABLE);
    dispi_write(
        VBE_DISPI_INDEX_ENABLE,
        current_enable | VBE_DISPI_NOCLEARMEM,
    );

    // Set virtual height
    dispi_write(VBE_DISPI_INDEX_VIRT_HEIGHT, virt_height);

    // Verify readback
    let actual_virt_height = dispi_read(VBE_DISPI_INDEX_VIRT_HEIGHT);
    let double_buffer_ok = actual_virt_height >= virt_height;

    if double_buffer_ok {
        // Start displaying page 0, render to page 1
        dispi_write(VBE_DISPI_INDEX_Y_OFFSET, 0);

        crate::serial_println!(
            "  BochsVGA: double-buffer OK (virt_height={}, VRAM >= {} KiB)",
            actual_virt_height,
            (pitch * actual_virt_height as u32) / 1024
        );
    } else {
        crate::serial_println!(
            "  BochsVGA: double-buffer: VRAM too small (requested {}, got {})",
            virt_height, actual_virt_height
        );
    }

    let mut state = BOCHS_VGA.lock();
    *state = Some(BochsVga {
        fb_phys,
        pitch,
        width,
        height,
        front_page: 0,
        double_buffer_enabled: double_buffer_ok,
    });

    double_buffer_ok
}

/// Get the physical address of the back buffer (the page not currently displayed).
pub fn back_buffer_phys() -> Option<u32> {
    let state = BOCHS_VGA.lock();
    if let Some(vga) = state.as_ref() {
        if vga.double_buffer_enabled {
            let offset = if vga.front_page == 0 {
                vga.pitch * vga.height // Page 1
            } else {
                0 // Page 0
            };
            return Some(vga.fb_phys + offset);
        }
    }
    None
}

/// Flip: swap front and back pages. The back page becomes visible.
pub fn flip() {
    let mut state = BOCHS_VGA.lock();
    if let Some(vga) = state.as_mut() {
        if vga.double_buffer_enabled {
            vga.front_page = 1 - vga.front_page;
            let y_offset = if vga.front_page == 0 {
                0
            } else {
                vga.height as u16
            };
            dispi_write(VBE_DISPI_INDEX_Y_OFFSET, y_offset);
        }
    }
}

/// Check if double-buffering is active.
pub fn is_double_buffered() -> bool {
    let state = BOCHS_VGA.lock();
    state.as_ref().map_or(false, |v| v.double_buffer_enabled)
}

/// Get display info: (width, height, pitch, fb_phys).
pub fn display_info() -> Option<(u32, u32, u32, u32)> {
    let state = BOCHS_VGA.lock();
    state
        .as_ref()
        .map(|v| (v.width, v.height, v.pitch, v.fb_phys))
}
