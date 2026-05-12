//! anyOS UEFI Bootloader
//!
//! Loads `/System/kernel.bin` from the EFI System Partition, sets up page tables,
//! fills the BootInfo struct (identical to the BIOS Stage 2 format), and jumps
//! to the kernel entry point.
//!
//! Boot flow:
//!   1. UEFI firmware loads this EFI application from ESP
//!   2. Query/set GOP framebuffer (1024x768x32)
//!   3. Find data partition, read kernel.bin (fallback: kernel.bak)
//!   4. Copy flat binary to a low physical address compatible with the kernel LMA
//!   5. Convert UEFI memory map -> E820 format
//!   6. Fill BootInfo at 0x9000
//!   7. Build 4-level page tables (identity + higher-half)
//!   8. ExitBootServices, load CR3, jump to kernel

#![no_std]
#![no_main]

extern crate alloc;

use core::{arch::asm, fmt::Write, time::Duration};
use uefi::boot::{self, AllocateType, MemoryType};
use uefi::mem::memory_map::MemoryMap;
use uefi::prelude::*;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};
use uefi::proto::console::text::{Key, ScanCode};
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::system;

// -- Constants (must match kernel expectations) --------------------------------

/// Preferred physical address where the kernel flat binary is loaded.
const KERNEL_LOAD_ADDR: u64 = 0x0010_0000;

/// Link-time physical base used by kernel/link.ld.
const KERNEL_LINK_PHYS_BASE: u64 = 0x0010_0000;

/// Physical address for the BootInfo struct.
const BOOT_INFO_ADDR: u64 = 0x9000;

/// Physical address for the E820 memory map entries.
///
/// Keep this away from page-table pages (0x3000..0xAFFF), the trampoline
/// (0x8000), and BootInfo (0x9000). The first 2 MiB are reserved by the kernel
/// physical allocator, so this remains stable after handoff.
const MEMORY_MAP_ADDR: u64 = 0x1_0000;

/// Maximum number of E820 entries we can store (fits in 0x1000..0x9000 = 32 KiB).
const MAX_E820_ENTRIES: usize = 1024;

/// BootInfo magic value ("ANYO").
const BOOT_INFO_MAGIC: u32 = 0x414E594F;

/// Page table physical addresses (same as BIOS protected_mode.asm).
const PML4_ADDR: u64 = 0x4000;
const PDPT_LOW_ADDR: u64 = 0x5000;
const PD_LOW_ADDR: u64 = 0x6000;
const PDPT_HIGH_ADDR: u64 = 0x7000;
const PD_FB_ADDR: u64 = 0x3000;
const PD_HIGH_ADDR: u64 = 0xA000;

/// Address for the trampoline code (between page tables and BootInfo).
const TRAMPOLINE_ADDR: u64 = 0x8000;

/// Page table entry flags.
const PT_PRESENT: u64 = 0x01;
const PT_RW: u64 = 0x02;
const PT_PS: u64 = 0x80; // 2 MiB page
const PT_BASE_FLAGS: u64 = PT_PRESENT | PT_RW;
const PT_PAGE_FLAGS: u64 = PT_PRESENT | PT_RW | PT_PS;

/// Amount of low physical memory identity-mapped for early execution.
const LOW_IDENTITY_MAP_SIZE: u64 = 128 * 1024 * 1024;

/// Size of the kernel higher-half physical window.
const KERNEL_HIGH_MAP_SIZE: u64 = 128 * 1024 * 1024;

/// Candidate load addresses. They are all congruent to 1 MiB modulo 2 MiB so
/// the higher-half 2 MiB page mapping can preserve the kernel's 1 MiB LMA.
const KERNEL_LOAD_CANDIDATES: [u64; 4] = [KERNEL_LOAD_ADDR, 0x0110_0000, 0x0210_0000, 0x0410_0000];

/// Preferred video mode.
const PREFERRED_WIDTH: usize = 1024;
const PREFERRED_HEIGHT: usize = 768;

/// Splash timeout before booting the default entry.
const BOOT_TIMEOUT_SECONDS: u64 = 5;

/// Kernel file paths (looked up on all volumes — ESP and data partition).
const KERNEL_PATH: &str = "\\System\\kernel.bin";
const KERNEL_FALLBACK: &str = "\\System\\kernel.bak";

/// Maximum kernel size: keep this comfortably above release kernels with
/// embedded assets and debug metadata stripped by mkimage's ELF-to-flat pass.
const MAX_KERNEL_SIZE: usize = 64 * 1024 * 1024;

/// Serial port for debug output.
const COM1: u16 = 0x3F8;

/// Embedded boot logo: u32 width, u32 height, then RGBA8888 pixels.
const BOOT_LOGO: &[u8] = include_bytes!("../../../kernel/src/graphics/boot_logo.bin");

/// 8x16 bitmap font for framebuffer-rendered boot menu text.
const FONT_DATA: &[u8] = include_bytes!("../../../kernel/src/graphics/font_8x16.bin");
const FONT_WIDTH: u32 = 8;
const FONT_HEIGHT: u32 = 16;

// -- E820 entry (matches kernel/src/boot_info.rs) -----------------------------

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct E820Entry {
    base_addr: u64,
    length: u64,
    entry_type: u32,
    acpi_extended: u32,
}

const E820_USABLE: u32 = 1;
const E820_RESERVED: u32 = 2;
const E820_ACPI_RECLAIMABLE: u32 = 3;

// -- BootInfo (matches kernel/src/boot_info.rs) -------------------------------

#[repr(C, packed)]
struct BootInfo {
    magic: u32,
    memory_map_addr: u32,
    memory_map_count: u32,
    framebuffer_addr: u32,
    framebuffer_pitch: u32,
    framebuffer_width: u32,
    framebuffer_height: u32,
    framebuffer_bpp: u8,
    boot_drive: u8,
    boot_mode: u8,
    _padding: u8,
    kernel_phys_start: u32,
    kernel_phys_end: u32,
    rsdp_addr: u32,
    boot_params: [u8; 64],
    edid_data: [u8; 128],
    edid_valid: u8,
    _padding2: [u8; 3],
}

struct KernelImage {
    phys_start: u64,
    size: usize,
}

#[derive(Copy, Clone)]
enum FramebufferFormat {
    Rgb,
    Bgr,
}

struct BootEntry {
    title: &'static str,
    params: &'static [u8],
}

struct TextBuffer {
    bytes: [u8; 128],
    len: usize,
}

impl TextBuffer {
    fn new() -> Self {
        Self {
            bytes: [0; 128],
            len: 0,
        }
    }

    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}

impl Write for TextBuffer {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let remaining = self.bytes.len().saturating_sub(self.len);
        let copy_len = remaining.min(s.len());
        self.bytes[self.len..self.len + copy_len].copy_from_slice(&s.as_bytes()[..copy_len]);
        self.len += copy_len;
        Ok(())
    }
}

const BOOT_ENTRIES: [BootEntry; 4] = [
    BootEntry {
        title: "anyOS",
        params: b"",
    },
    BootEntry {
        title: "anyOS (Verbose)",
        params: b"verbose",
    },
    BootEntry {
        title: "anyOS (Textmode)",
        params: b"nogui",
    },
    BootEntry {
        title: "anyOS (Verbose + 1920x1080)",
        params: b"verbose res=1920x1080",
    },
];

enum BootDecision {
    Default,
    Menu,
    Entry(usize),
}

// -- Serial debug output ------------------------------------------------------

fn serial_init() {
    unsafe {
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x80);
        outb(COM1 + 0, 0x01);
        outb(COM1 + 1, 0x00);
        outb(COM1 + 3, 0x03);
        outb(COM1 + 2, 0xC7);
        outb(COM1 + 4, 0x0B);
    }
}

fn serial_write_byte(b: u8) {
    unsafe {
        loop {
            if (inb(COM1 + 5) & 0x20) != 0 {
                break;
            }
        }
        outb(COM1, b);
    }
}

fn serial_print(s: &str) {
    for b in s.bytes() {
        if b == b'\n' {
            serial_write_byte(b'\r');
        }
        serial_write_byte(b);
    }
}

fn serial_print_hex(val: u64) {
    serial_print("0x");
    let mut started = false;
    for i in (0..16).rev() {
        let nibble = ((val >> (i * 4)) & 0xF) as u8;
        if nibble != 0 || started || i == 0 {
            started = true;
            let c = if nibble < 10 {
                b'0' + nibble
            } else {
                b'A' + nibble - 10
            };
            serial_write_byte(c);
        }
    }
}

unsafe fn outb(port: u16, val: u8) {
    asm!("out dx, al", in("dx") port, in("al") val, options(nomem, nostack));
}

unsafe fn inb(port: u16) -> u8 {
    let val: u8;
    asm!("in al, dx", out("al") val, in("dx") port, options(nomem, nostack));
    val
}

// -- Panic handler (serial output, works after ExitBootServices) --------------

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_print("\n[UEFI] PANIC: ");
    if let Some(loc) = info.location() {
        serial_print(loc.file());
        serial_print(":");
        serial_print_hex(loc.line() as u64);
    }
    serial_print("\n");
    loop {
        unsafe {
            asm!("cli; hlt");
        }
    }
}

// -- UEFI Entry Point ---------------------------------------------------------

#[entry]
fn main() -> Status {
    uefi::helpers::init().unwrap();

    serial_init();
    serial_print("\n[UEFI] anyOS UEFI Bootloader starting...\n");

    // -- Step 1: Set up GOP framebuffer ---------------------------------------
    serial_print("[UEFI] Querying GOP...\n");
    let (fb_addr, fb_width, fb_height, fb_pitch, fb_bpp, fb_format) = setup_gop();
    serial_print("[UEFI] Framebuffer: ");
    serial_print_hex(fb_addr as u64);
    serial_print(" ");
    serial_print_hex(fb_width as u64);
    serial_print("x");
    serial_print_hex(fb_height as u64);
    serial_print("\n");

    let selected_entry = run_boot_menu(fb_addr, fb_width, fb_height, fb_pitch, fb_format);
    serial_print("[UEFI] Boot entry: ");
    serial_print(BOOT_ENTRIES[selected_entry].title);
    if !BOOT_ENTRIES[selected_entry].params.is_empty() {
        serial_print(" params=\"");
        if let Ok(params) = core::str::from_utf8(BOOT_ENTRIES[selected_entry].params) {
            serial_print(params);
        }
        serial_print("\"");
    }
    serial_print("\n");

    // -- Step 2: Load kernel from data partition ------------------------------
    serial_print("[UEFI] Loading kernel...\n");
    let kernel = load_kernel();
    serial_print("[UEFI] Kernel loaded, size=");
    serial_print_hex(kernel.size as u64);
    serial_print(" addr=");
    serial_print_hex(kernel.phys_start);
    serial_print("\n");

    // -- Step 2b: Find ACPI RSDP (must be before ExitBootServices) ------------
    let rsdp_addr = find_rsdp();
    if rsdp_addr != 0 {
        serial_print("[UEFI] RSDP found at ");
        serial_print_hex(rsdp_addr as u64);
        serial_print("\n");
    } else {
        serial_print("[UEFI] RSDP not found\n");
    }

    // -- Step 3: Fill BootInfo (before ExitBootServices) ----------------------
    let boot_info = unsafe { &mut *(BOOT_INFO_ADDR as *mut BootInfo) };
    boot_info.magic = BOOT_INFO_MAGIC;
    boot_info.memory_map_addr = MEMORY_MAP_ADDR as u32;
    boot_info.memory_map_count = 0; // filled after ExitBootServices
    boot_info.framebuffer_addr = fb_addr;
    boot_info.framebuffer_pitch = fb_pitch;
    boot_info.framebuffer_width = fb_width;
    boot_info.framebuffer_height = fb_height;
    boot_info.framebuffer_bpp = fb_bpp;
    boot_info.boot_drive = 0;
    boot_info.boot_mode = 1; // UEFI
    boot_info._padding = 0;
    boot_info.kernel_phys_start = kernel.phys_start as u32;
    boot_info.kernel_phys_end = kernel.phys_start as u32 + kernel.size as u32;
    boot_info.rsdp_addr = rsdp_addr;
    boot_info.boot_params = [0u8; 64];
    copy_boot_params(
        &mut boot_info.boot_params,
        BOOT_ENTRIES[selected_entry].params,
    );
    boot_info.edid_data = [0u8; 128];
    boot_info.edid_valid = 0;
    boot_info._padding2 = [0u8; 3];

    // -- Step 4: ExitBootServices ---------------------------------------------
    serial_print("[UEFI] Calling ExitBootServices...\n");
    let memory_map = unsafe { boot::exit_boot_services(Some(MemoryType::LOADER_DATA)) };

    // Now we have no more UEFI boot services. Only serial port for debug.

    // -- Step 5: Convert memory map to E820 -----------------------------------
    let e820_entries = unsafe {
        core::slice::from_raw_parts_mut(MEMORY_MAP_ADDR as *mut E820Entry, MAX_E820_ENTRIES)
    };

    let mut e820_count: u32 = 0;
    for desc in memory_map.entries() {
        if e820_count as usize >= MAX_E820_ENTRIES {
            break;
        }

        let e820_type = if desc.ty == MemoryType::CONVENTIONAL
            || desc.ty == MemoryType::BOOT_SERVICES_CODE
            || desc.ty == MemoryType::BOOT_SERVICES_DATA
        {
            E820_USABLE
        } else if desc.ty == MemoryType::ACPI_RECLAIM {
            E820_ACPI_RECLAIMABLE
        } else {
            E820_RESERVED
        };

        let base = desc.phys_start;
        let length = desc.page_count * 4096;

        if length == 0 {
            continue;
        }

        e820_entries[e820_count as usize] = E820Entry {
            base_addr: base,
            length,
            entry_type: e820_type,
            acpi_extended: 0,
        };
        e820_count += 1;
    }

    // Update BootInfo with final memory map count
    unsafe {
        let bi = &mut *(BOOT_INFO_ADDR as *mut BootInfo);
        bi.memory_map_count = e820_count;
    }

    serial_print("[UEFI] E820 entries: ");
    serial_print_hex(e820_count as u64);
    serial_print("\n");

    // -- Step 6: Build page tables --------------------------------------------
    serial_print("[UEFI] Building page tables...\n");
    build_page_tables(fb_addr, kernel.phys_start);

    // -- Step 7: Enable FPU/SSE, load CR3, jump to kernel ---------------------
    serial_print("[UEFI] Jumping to kernel...\n");
    unsafe {
        jump_to_kernel(kernel.phys_start);
    }
}

// -- GOP setup ----------------------------------------------------------------

fn setup_gop() -> (u32, u32, u32, u32, u8, FramebufferFormat) {
    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>().expect("GOP not available");

    let mut gop =
        boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle).expect("Failed to open GOP");

    // Try to find 1024x768x32 mode
    let mut best_mode = None;

    for mode in gop.modes() {
        let info = mode.info();
        let (w, h) = info.resolution();
        let fmt = info.pixel_format();

        if w == PREFERRED_WIDTH && h == PREFERRED_HEIGHT {
            match fmt {
                PixelFormat::Bgr | PixelFormat::Rgb => {
                    best_mode = Some(mode);
                    break;
                }
                _ => {}
            }
        }
    }

    if let Some(mode) = best_mode {
        gop.set_mode(&mode).expect("Failed to set GOP mode");
    }

    let mode_info = gop.current_mode_info();
    let (w, h) = mode_info.resolution();
    let stride = mode_info.stride();
    let fb_base = gop.frame_buffer().as_mut_ptr() as u64;
    let bpp = 32u8;
    let format = match mode_info.pixel_format() {
        PixelFormat::Rgb => FramebufferFormat::Rgb,
        _ => FramebufferFormat::Bgr,
    };

    (
        fb_base as u32,
        w as u32,
        h as u32,
        stride as u32 * 4,
        bpp,
        format,
    )
}

// -- Boot menu ----------------------------------------------------------------

fn run_boot_menu(
    fb_addr: u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
) -> usize {
    draw_boot_background(fb_addr, fb_width, fb_height, fb_pitch, fb_format);
    draw_splash_text(fb_addr, fb_width, fb_height, fb_pitch, fb_format, BOOT_TIMEOUT_SECONDS);
    flush_keyboard();

    match wait_for_splash_choice(fb_addr, fb_width, fb_height, fb_pitch, fb_format) {
        BootDecision::Default => 0,
        BootDecision::Entry(index) => index,
        BootDecision::Menu => {
            show_interactive_menu(fb_addr, fb_width, fb_height, fb_pitch, fb_format)
        }
    }
}

fn wait_for_splash_choice(
    fb_addr: u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
) -> BootDecision {
    for remaining in (1..=BOOT_TIMEOUT_SECONDS).rev() {
        draw_splash_text(fb_addr, fb_width, fb_height, fb_pitch, fb_format, remaining);
        for _ in 0..10 {
            if let Some(decision) = poll_splash_key() {
                return decision;
            }
            boot::stall(Duration::from_millis(100));
        }
    }

    BootDecision::Default
}

fn poll_splash_key() -> Option<BootDecision> {
    system::with_stdin(|stdin| match stdin.read_key().ok().flatten() {
        Some(Key::Special(ScanCode::ESCAPE)) => Some(BootDecision::Menu),
        Some(Key::Printable(ch)) => match char::from(ch) {
            '\r' | '\n' => Some(BootDecision::Default),
            'v' | 'V' => Some(BootDecision::Entry(1)),
            'n' | 'N' => Some(BootDecision::Entry(2)),
            '1' => Some(BootDecision::Entry(0)),
            '2' => Some(BootDecision::Entry(1)),
            '3' => Some(BootDecision::Entry(2)),
            '4' => Some(BootDecision::Entry(3)),
            _ => None,
        },
        _ => None,
    })
}

fn show_interactive_menu(
    fb_addr: u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
) -> usize {
    let mut selected = 0usize;
    loop {
        draw_boot_background(fb_addr, fb_width, fb_height, fb_pitch, fb_format);
        draw_menu_text(fb_addr, fb_width, fb_height, fb_pitch, fb_format, selected);

        match wait_for_menu_key() {
            MenuKey::Up => {
                selected = selected.saturating_sub(1);
            }
            MenuKey::Down => {
                if selected + 1 < BOOT_ENTRIES.len() {
                    selected += 1;
                }
            }
            MenuKey::Select => return selected,
            MenuKey::Entry(index) => return index,
            MenuKey::Cancel => return 0,
            MenuKey::Ignored => {}
        }
    }
}

enum MenuKey {
    Up,
    Down,
    Select,
    Cancel,
    Entry(usize),
    Ignored,
}

fn wait_for_menu_key() -> MenuKey {
    loop {
        if let Some(key) = system::with_stdin(|stdin| stdin.read_key().ok().flatten()) {
            match key {
                Key::Special(ScanCode::UP) => return MenuKey::Up,
                Key::Special(ScanCode::DOWN) => return MenuKey::Down,
                Key::Special(ScanCode::ESCAPE) => return MenuKey::Cancel,
                Key::Printable(ch) => match char::from(ch) {
                    '\r' | '\n' => return MenuKey::Select,
                    '1' => return MenuKey::Entry(0),
                    '2' => return MenuKey::Entry(1),
                    '3' => return MenuKey::Entry(2),
                    '4' => return MenuKey::Entry(3),
                    _ => return MenuKey::Ignored,
                },
                _ => return MenuKey::Ignored,
            }
        }
        boot::stall(Duration::from_millis(20));
    }
}

fn flush_keyboard() {
    loop {
        let had_key = system::with_stdin(|stdin| stdin.read_key().ok().flatten().is_some());
        if !had_key {
            break;
        }
    }
}

fn draw_boot_background(
    fb_addr: u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
) {
    clear_framebuffer(fb_addr, fb_width, fb_height, fb_pitch);
    draw_logo(fb_addr, fb_width, fb_height, fb_pitch, fb_format);
}

fn clear_framebuffer(fb_addr: u32, fb_width: u32, fb_height: u32, fb_pitch: u32) {
    for y in 0..fb_height as usize {
        let row = (fb_addr as usize + y * fb_pitch as usize) as *mut u32;
        for x in 0..fb_width as usize {
            unsafe {
                row.add(x).write_volatile(0);
            }
        }
    }
}

fn draw_logo(
    fb_addr: u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
) {
    if BOOT_LOGO.len() < 8 {
        return;
    }

    let logo_w = u32::from_le_bytes([BOOT_LOGO[0], BOOT_LOGO[1], BOOT_LOGO[2], BOOT_LOGO[3]]);
    let logo_h = u32::from_le_bytes([BOOT_LOGO[4], BOOT_LOGO[5], BOOT_LOGO[6], BOOT_LOGO[7]]);
    let pixel_bytes = logo_w as usize * logo_h as usize * 4;
    if logo_w == 0 || logo_h == 0 || BOOT_LOGO.len() < 8 + pixel_bytes {
        return;
    }

    let dst_x = fb_width.saturating_sub(logo_w) / 2;
    let dst_y = (fb_height.saturating_sub(logo_h) / 2).saturating_sub(fb_height / 10);
    let src = &BOOT_LOGO[8..8 + pixel_bytes];
    let draw_w = logo_w.min(fb_width.saturating_sub(dst_x));
    let draw_h = logo_h.min(fb_height.saturating_sub(dst_y));

    for y in 0..draw_h as usize {
        let dst_row = (fb_addr as usize + (dst_y as usize + y) * fb_pitch as usize) as *mut u32;
        for x in 0..draw_w as usize {
            let si = (y * logo_w as usize + x) * 4;
            let r = src[si];
            let g = src[si + 1];
            let b = src[si + 2];
            let a = src[si + 3];
            if a == 0 {
                continue;
            }
            let pixel = if a == 255 {
                pack_pixel(fb_format, r, g, b)
            } else {
                pack_pixel(
                    fb_format,
                    blend_over_black(r, a),
                    blend_over_black(g, a),
                    blend_over_black(b, a),
                )
            };
            unsafe {
                dst_row.add(dst_x as usize + x).write_volatile(pixel);
            }
        }
    }
}

fn blend_over_black(channel: u8, alpha: u8) -> u8 {
    ((channel as u16 * alpha as u16) / 255) as u8
}

fn pack_pixel(format: FramebufferFormat, r: u8, g: u8, b: u8) -> u32 {
    match format {
        // UEFI RGB means byte order R,G,B,Reserved; as little-endian u32 that is
        // 0x00BBGGRR. BGR is the reverse and matches the old VESA path.
        FramebufferFormat::Rgb => ((b as u32) << 16) | ((g as u32) << 8) | r as u32,
        FramebufferFormat::Bgr => ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
    }
}

fn draw_splash_text(
    fb_addr: u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
    remaining: u64,
) {
    let base_y = fb_height.saturating_sub(FONT_HEIGHT * 6);

    draw_centered_text(
        fb_addr,
        fb_width,
        fb_pitch,
        fb_format,
        base_y,
        (0xD0, 0xD0, 0xD0),
        "anyOS UEFI Bootloader",
    );
    draw_centered_text(
        fb_addr,
        fb_width,
        fb_pitch,
        fb_format,
        base_y + FONT_HEIGHT * 2,
        (0x68, 0x72, 0x80),
        "Esc Boot Menu   V Verbose   N Textmode   Enter Boot",
    );

    let countdown_y = base_y + FONT_HEIGHT * 3;
    fill_rect(
        fb_addr,
        fb_pitch,
        fb_format,
        0,
        countdown_y,
        fb_width,
        FONT_HEIGHT,
        (0, 0, 0),
    );
    let mut text = TextBuffer::new();
    let _ = write!(&mut text, "Booting default in {}s", remaining);
    draw_centered_text(
        fb_addr,
        fb_width,
        fb_pitch,
        fb_format,
        countdown_y,
        (0x68, 0x72, 0x80),
        text.as_str(),
    );
}

fn draw_menu_text(
    fb_addr: u32,
    fb_width: u32,
    fb_height: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
    selected: usize,
) {
    let start_y = fb_height / 2;

    draw_centered_text(
        fb_addr,
        fb_width,
        fb_pitch,
        fb_format,
        start_y.saturating_sub(FONT_HEIGHT * 2),
        (0xFF, 0xFF, 0xFF),
        "anyOS Boot Menu",
    );

    for (index, entry) in BOOT_ENTRIES.iter().enumerate() {
        let y = start_y + index as u32 * FONT_HEIGHT;
        let mut label = TextBuffer::new();
        let _ = write!(&mut label, "{}. {}", index + 1, entry.title);

        if index == selected {
            fill_rect(
                fb_addr,
                fb_pitch,
                fb_format,
                0,
                y,
                fb_width,
                FONT_HEIGHT,
                (0x00, 0xAA, 0xAA),
            );
            draw_centered_text(
                fb_addr,
                fb_width,
                fb_pitch,
                fb_format,
                y,
                (0x00, 0x00, 0x00),
                label.as_str(),
            );
        } else {
            draw_centered_text(
                fb_addr,
                fb_width,
                fb_pitch,
                fb_format,
                y,
                (0xD0, 0xD0, 0xD0),
                label.as_str(),
            );
        }
    }

    draw_centered_text(
        fb_addr,
        fb_width,
        fb_pitch,
        fb_format,
        fb_height.saturating_sub(FONT_HEIGHT * 2),
        (0x68, 0x72, 0x80),
        "Up/Down Select   Enter Boot   Esc Default",
    );
}

fn draw_centered_text(
    fb_addr: u32,
    fb_width: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
    y: u32,
    color: (u8, u8, u8),
    text: &str,
) {
    let text_width = text.len() as u32 * FONT_WIDTH;
    let x = fb_width.saturating_sub(text_width) / 2;
    draw_text(fb_addr, fb_width, fb_pitch, fb_format, x, y, color, text);
}

fn draw_text(
    fb_addr: u32,
    fb_width: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
    mut x: u32,
    y: u32,
    color: (u8, u8, u8),
    text: &str,
) {
    for byte in text.bytes() {
        if x + FONT_WIDTH > fb_width {
            break;
        }
        draw_char(fb_addr, fb_pitch, fb_format, x, y, color, byte);
        x += FONT_WIDTH;
    }
}

fn draw_char(
    fb_addr: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
    x: u32,
    y: u32,
    color: (u8, u8, u8),
    ch: u8,
) {
    if !(32..=126).contains(&ch) {
        return;
    }

    let glyph_offset = (ch - 32) as usize * FONT_HEIGHT as usize;
    if glyph_offset + FONT_HEIGHT as usize > FONT_DATA.len() {
        return;
    }

    let pixel = pack_pixel(fb_format, color.0, color.1, color.2);
    for row in 0..FONT_HEIGHT {
        let bits = FONT_DATA[glyph_offset + row as usize];
        let dst_row = (fb_addr as usize + (y + row) as usize * fb_pitch as usize) as *mut u32;
        for col in 0..FONT_WIDTH {
            if bits & (0x80 >> col) != 0 {
                unsafe {
                    dst_row.add((x + col) as usize).write_volatile(pixel);
                }
            }
        }
    }
}

fn fill_rect(
    fb_addr: u32,
    fb_pitch: u32,
    fb_format: FramebufferFormat,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    color: (u8, u8, u8),
) {
    let pixel = pack_pixel(fb_format, color.0, color.1, color.2);
    for row in 0..height {
        let dst_row = (fb_addr as usize + (y + row) as usize * fb_pitch as usize) as *mut u32;
        for col in 0..width {
            unsafe {
                dst_row.add((x + col) as usize).write_volatile(pixel);
            }
        }
    }
}

fn copy_boot_params(dst: &mut [u8; 64], params: &[u8]) {
    *dst = [0u8; 64];
    let len = core::cmp::min(params.len(), dst.len() - 1);
    dst[..len].copy_from_slice(&params[..len]);
}

// -- Kernel loading -----------------------------------------------------------

fn load_kernel() -> KernelImage {
    let fs_handles = boot::find_handles::<SimpleFileSystem>().expect("No filesystem handles found");

    for handle in &fs_handles {
        let mut fs = match boot::open_protocol_exclusive::<SimpleFileSystem>(*handle) {
            Ok(fs) => fs,
            Err(_) => continue,
        };

        let mut root = match fs.open_volume() {
            Ok(r) => r,
            Err(_) => continue,
        };

        for path in &[KERNEL_PATH, KERNEL_FALLBACK] {
            if let Some(kernel) = try_load_kernel_from(&mut root, path) {
                return kernel;
            }
        }
    }

    panic!("Kernel not found on any partition!");
}

fn try_load_kernel_from(
    root: &mut uefi::proto::media::file::Directory,
    path: &str,
) -> Option<KernelImage> {
    // Convert path to UCS-2
    let mut path_buf = [0u16; 64];
    for (i, b) in path.bytes().enumerate() {
        if i >= path_buf.len() - 1 {
            break;
        }
        path_buf[i] = b as u16;
    }

    let path_cstr = uefi::CStr16::from_u16_with_nul(&path_buf[..path.len() + 1]).ok()?;

    let file_handle = root
        .open(path_cstr, FileMode::Read, FileAttribute::empty())
        .ok()?;

    let mut file = match file_handle.into_type().ok()? {
        uefi::proto::media::file::FileType::Regular(f) => f,
        _ => return None,
    };

    // Get file size
    let mut info_buf = [0u8; 256];
    let info = file.get_info::<FileInfo>(&mut info_buf).ok()?;
    let file_size = info.file_size() as usize;

    if file_size == 0 || file_size > MAX_KERNEL_SIZE {
        return None;
    }

    serial_print("[UEFI] Found kernel: ");
    serial_print(path);
    serial_print(" (");
    serial_print_hex(file_size as u64);
    serial_print(" bytes)\n");

    let load_addr = allocate_kernel_region(file_size)?;
    serial_print("[UEFI] Kernel load address: ");
    serial_print_hex(load_addr);
    serial_print("\n");

    // Read kernel into memory
    let kernel_buf = unsafe { core::slice::from_raw_parts_mut(load_addr as *mut u8, file_size) };

    let mut total_read = 0;
    while total_read < file_size {
        let n = file
            .read(&mut kernel_buf[total_read..])
            .expect("Failed to read kernel");
        if n == 0 {
            break;
        }
        total_read += n;
    }

    if total_read != file_size {
        serial_print("[UEFI] WARNING: short read!\n");
    }

    Some(KernelImage {
        phys_start: load_addr,
        size: file_size,
    })
}

fn allocate_kernel_region(file_size: usize) -> Option<u64> {
    let pages_needed = (file_size + 4095) / 4096;

    for &addr in &KERNEL_LOAD_CANDIDATES {
        if !kernel_load_addr_is_valid(addr, file_size) {
            continue;
        }

        if boot::allocate_pages(
            AllocateType::Address(addr),
            MemoryType::LOADER_DATA,
            pages_needed,
        )
        .is_ok()
        {
            return Some(addr);
        }
    }

    allocate_relocatable_kernel_region(file_size, pages_needed)
}

fn allocate_relocatable_kernel_region(file_size: usize, pages_needed: usize) -> Option<u64> {
    let slack_pages = (0x20_0000 / 4096) + 1;
    let raw_pages = pages_needed.checked_add(slack_pages)?;
    let raw = boot::allocate_pages(
        AllocateType::MaxAddress(LOW_IDENTITY_MAP_SIZE - 1),
        MemoryType::LOADER_DATA,
        raw_pages,
    )
    .ok()?;

    let raw_start = raw.as_ptr() as u64;
    let raw_end = raw_start + raw_pages as u64 * 4096;
    let load_addr = align_kernel_load_addr(raw_start);
    let load_end = load_addr.checked_add(file_size as u64)?;

    if load_addr >= raw_start
        && load_end <= raw_end
        && kernel_load_addr_is_valid(load_addr, file_size)
    {
        return Some(load_addr);
    }

    None
}

fn kernel_load_addr_is_valid(addr: u64, file_size: usize) -> bool {
    let end = match addr.checked_add(file_size as u64) {
        Some(end) => end,
        None => return false,
    };

    addr >= KERNEL_LINK_PHYS_BASE
        && end <= LOW_IDENTITY_MAP_SIZE
        && ((addr - KERNEL_LINK_PHYS_BASE) & 0x1F_FFFF) == 0
}

fn align_kernel_load_addr(addr: u64) -> u64 {
    let base = addr & !0x1F_FFFF;
    let candidate = base + KERNEL_LINK_PHYS_BASE;
    if candidate >= addr {
        candidate
    } else {
        candidate + 0x20_0000
    }
}

// -- Page table construction --------------------------------------------------

fn build_page_tables(fb_addr: u32, kernel_phys_start: u64) {
    let kernel_high_phys_base = kernel_phys_start - KERNEL_LINK_PHYS_BASE;
    if (kernel_high_phys_base & 0x1F_FFFF) != 0 {
        panic!("Kernel physical load address is not compatible with 2 MiB higher-half mapping");
    }

    // Clear page table pages. Keep this explicit because the low boot layout is
    // intentionally sparse around the trampoline and BootInfo pages.
    zero_page(PD_FB_ADDR);
    zero_page(PML4_ADDR);
    zero_page(PDPT_LOW_ADDR);
    zero_page(PD_LOW_ADDR);
    zero_page(PDPT_HIGH_ADDR);
    zero_page(PD_HIGH_ADDR);

    let write64 = |addr: u64, val: u64| unsafe {
        core::ptr::write_volatile(addr as *mut u64, val);
    };

    // PML4[0] -> PDPT_LOW (identity map)
    write64(PML4_ADDR + 0 * 8, PDPT_LOW_ADDR | PT_BASE_FLAGS);
    // PML4[511] -> PDPT_HIGH (higher-half kernel)
    write64(PML4_ADDR + 511 * 8, PDPT_HIGH_ADDR | PT_BASE_FLAGS);

    // PDPT_LOW[0] -> PD_LOW (first 1 GiB)
    write64(PDPT_LOW_ADDR + 0 * 8, PD_LOW_ADDR | PT_BASE_FLAGS);

    // PD_LOW: identity map first 128 MiB with 2 MiB pages. This matches the
    // BIOS Stage 2 window and leaves room for the current kernel plus BSS/stack.
    for i in 0u64..(LOW_IDENTITY_MAP_SIZE / 0x20_0000) {
        write64(PD_LOW_ADDR + i * 8, (i * 0x20_0000) | PT_PAGE_FLAGS);
    }

    // PDPT_HIGH[510] -> dedicated kernel high-half PD.
    write64(PDPT_HIGH_ADDR + 510 * 8, PD_HIGH_ADDR | PT_BASE_FLAGS);

    // Higher-half kernel mapping. The kernel is linked as if its physical LMA
    // starts at 1 MiB, but UEFI may reserve that exact address. Map the linked
    // virtual window onto the actual selected physical load region.
    for i in 0u64..(KERNEL_HIGH_MAP_SIZE / 0x20_0000) {
        write64(
            PD_HIGH_ADDR + i * 8,
            (kernel_high_phys_base + i * 0x20_0000) | PT_PAGE_FLAGS,
        );
    }

    // Framebuffer mapping: dynamically determine the correct PDPT entry.
    // BIOS VBE typically returns ~0xFD000000 (PDPT[3], 3-4 GiB range),
    // but OVMF GOP returns 0x80000000 (PDPT[2], 2-3 GiB range).
    if fb_addr != 0 {
        let pdpt_index = (fb_addr as u64) >> 30; // which 1 GiB block

        // Link PD_FB to the correct PDPT entry (skip if PDPT[0] — PD_LOW is there)
        if pdpt_index > 0 {
            write64(PDPT_LOW_ADDR + pdpt_index * 8, PD_FB_ADDR | PT_BASE_FLAGS);
        }

        let fb_aligned = (fb_addr as u64) & 0xFFE0_0000; // 2 MiB align down
        let pd_index = ((fb_addr as u64) & 0x3FFF_FFFF) >> 21;

        // Use PD_FB for non-zero PDPT entries, PD_LOW for PDPT[0]
        let target_pd = if pdpt_index == 0 {
            PD_LOW_ADDR
        } else {
            PD_FB_ADDR
        };

        // Map 8 × 2 MiB = 16 MiB of VRAM
        for i in 0u64..8 {
            let idx = pd_index + i;
            if idx < 512 {
                write64(
                    target_pd + idx * 8,
                    (fb_aligned + i * 0x20_0000) | PT_PAGE_FLAGS,
                );
            }
        }
    }
}

fn zero_page(addr: u64) {
    unsafe {
        core::ptr::write_bytes(addr as *mut u8, 0, 4096);
    }
}

// -- ACPI RSDP discovery ------------------------------------------------------

/// ACPI 1.0 RSDP GUID: eb9d2d30-2d88-11d3-9a16-0090273fc14d
const ACPI_GUID: uefi::Guid = uefi::Guid::parse_or_panic("eb9d2d30-2d88-11d3-9a16-0090273fc14d");

/// ACPI 2.0+ RSDP GUID: 8868e871-e4f1-11d3-bc22-0080c73c8881
const ACPI2_GUID: uefi::Guid = uefi::Guid::parse_or_panic("8868e871-e4f1-11d3-bc22-0080c73c8881");

/// Find the ACPI RSDP from UEFI configuration tables.
/// Returns the physical address of the RSDP, or 0 if not found.
fn find_rsdp() -> u32 {
    let st = uefi::table::system_table_raw().expect("No system table");
    let st = unsafe { st.as_ref() };

    let count = st.number_of_configuration_table_entries;
    if count == 0 {
        return 0;
    }

    let entries = st.configuration_table;
    if entries.is_null() {
        return 0;
    }

    // Prefer ACPI 2.0 (XSDT), fall back to 1.0 (RSDT)
    let mut rsdp1: u32 = 0;

    for i in 0..count {
        let entry = unsafe { &*entries.add(i) };
        if entry.vendor_guid == ACPI2_GUID {
            return entry.vendor_table as u32;
        }
        if entry.vendor_guid == ACPI_GUID {
            rsdp1 = entry.vendor_table as u32;
        }
    }

    rsdp1
}

// -- Jump to kernel -----------------------------------------------------------

unsafe fn jump_to_kernel(kernel_entry_phys: u64) -> ! {
    asm!("cli", options(nomem, nostack, preserves_flags));

    // Build a small trampoline at TRAMPOLINE_ADDR (0x8000), which is within
    // the identity-mapped first 128 MiB. We MUST switch CR3 from code that is
    // mapped in BOTH the old (UEFI) and new (our) page tables. The UEFI
    // bootloader's code is at a UEFI-allocated address (likely above 128 MiB)
    // which is NOT in our identity map — switching CR3 here would triple-fault.
    //
    // Trampoline expects:
    //   RDI = PML4 physical address (for CR3)
    //   RSI = new stack pointer
    //   RDX = boot_info address (passed to kernel in RDI)
    //   RCX = kernel entry address
    //
    // Trampoline code:
    //   cli               ; FA           — firmware must not leak IF=1 to kernel
    //   mov cr3, rdi      ; 0F 22 DF     — switch to our page tables
    //   mov rsp, rsi      ; 48 89 F4     — set up kernel stack
    //   mov rdi, rdx      ; 48 89 D7     — RDI = boot_info for kernel
    //   jmp rcx           ; FF E1        — jump to kernel entry
    let trampoline: [u8; 12] = [
        0xFA, // cli
        0x0F, 0x22, 0xDF, // mov cr3, rdi
        0x48, 0x89, 0xF4, // mov rsp, rsi
        0x48, 0x89, 0xD7, // mov rdi, rdx
        0xFF, 0xE1, // jmp rcx
    ];

    core::ptr::copy_nonoverlapping(
        trampoline.as_ptr(),
        TRAMPOLINE_ADDR as *mut u8,
        trampoline.len(),
    );

    // CR0: clear EM (bit 2) and TS (bit 3), set MP (bit 1) and NE (bit 5)
    asm!(
        "mov rax, cr0",
        "and eax, ~0x0C",
        "or eax, 0x22",
        "mov cr0, rax",
        out("rax") _,
        options(nomem, nostack),
    );

    // CR4: set OSFXSR (bit 9) and OSXMMEXCPT (bit 10)
    asm!(
        "mov rax, cr4",
        "or eax, 0x600",
        "mov cr4, rax",
        out("rax") _,
        options(nomem, nostack),
    );

    // Initialize FPU
    asm!("fninit", options(nomem, nostack));

    // Jump to trampoline — it will switch CR3 and jump to the kernel.
    // Use explicit register constraints so the compiler knows exactly which
    // registers are in use and won't allocate conflicting operands.
    asm!(
        "jmp {trampoline}",
        in("rdi") PML4_ADDR,
        in("rsi") 0x200000u64,
        in("rdx") BOOT_INFO_ADDR,
        in("rcx") kernel_entry_phys,
        trampoline = in(reg) TRAMPOLINE_ADDR,
        options(noreturn),
    );
}
