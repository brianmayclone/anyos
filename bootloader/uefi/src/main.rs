//! anyOS UEFI Bootloader
//!
//! Loads `/system/kernel.bin` from the FAT data partition, sets up page tables,
//! fills the BootInfo struct (identical to the BIOS Stage 2 format), and jumps
//! to the kernel entry point.
//!
//! Boot flow:
//!   1. UEFI firmware loads this EFI application from ESP
//!   2. Query/set GOP framebuffer (1024x768x32)
//!   3. Find data partition, read kernel.bin (fallback: kernel.bak)
//!   4. Copy flat binary to 0x100000
//!   5. Convert UEFI memory map -> E820 format
//!   6. Fill BootInfo at 0x9000
//!   7. Build 4-level page tables (identity + higher-half)
//!   8. ExitBootServices, load CR3, jump to kernel

#![no_std]
#![no_main]

extern crate alloc;

use core::arch::asm;
use uefi::prelude::*;
use uefi::boot::{self, AllocateType, MemoryType};
use uefi::mem::memory_map::MemoryMap;
use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};
use uefi::proto::media::file::{File, FileAttribute, FileMode, FileInfo};
use uefi::proto::media::fs::SimpleFileSystem;

// -- Constants (must match kernel expectations) --------------------------------

/// Physical address where kernel flat binary is loaded (1 MiB mark).
const KERNEL_LOAD_ADDR: u64 = 0x0010_0000;

/// Physical address for the BootInfo struct.
const BOOT_INFO_ADDR: u64 = 0x9000;

/// Physical address for the E820 memory map entries.
const MEMORY_MAP_ADDR: u64 = 0x1000;

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

/// Address for the trampoline code (between page tables and BootInfo).
const TRAMPOLINE_ADDR: u64 = 0x8000;

/// Page table entry flags.
const PT_PRESENT: u64 = 0x01;
const PT_RW: u64 = 0x02;
const PT_PS: u64 = 0x80; // 2 MiB page
const PT_BASE_FLAGS: u64 = PT_PRESENT | PT_RW;
const PT_PAGE_FLAGS: u64 = PT_PRESENT | PT_RW | PT_PS;

/// Preferred video mode.
const PREFERRED_WIDTH: usize = 1024;
const PREFERRED_HEIGHT: usize = 768;

/// Kernel file paths on data partition.
const KERNEL_PATH: &str = "\\system\\kernel.bin";
const KERNEL_FALLBACK: &str = "\\system\\kernel.bak";

/// Maximum kernel size: 8 MiB.
const MAX_KERNEL_SIZE: usize = 8 * 1024 * 1024;

/// Serial port for debug output.
const COM1: u16 = 0x3F8;

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
            let c = if nibble < 10 { b'0' + nibble } else { b'A' + nibble - 10 };
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
        unsafe { asm!("cli; hlt"); }
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
    let (fb_addr, fb_width, fb_height, fb_pitch, fb_bpp) = setup_gop();
    serial_print("[UEFI] Framebuffer: ");
    serial_print_hex(fb_addr as u64);
    serial_print(" ");
    serial_print_hex(fb_width as u64);
    serial_print("x");
    serial_print_hex(fb_height as u64);
    serial_print("\n");

    // -- Step 2: Load kernel from data partition ------------------------------
    serial_print("[UEFI] Loading kernel...\n");
    let kernel_size = load_kernel();
    serial_print("[UEFI] Kernel loaded, size=");
    serial_print_hex(kernel_size as u64);
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
    boot_info.kernel_phys_start = KERNEL_LOAD_ADDR as u32;
    boot_info.kernel_phys_end = KERNEL_LOAD_ADDR as u32 + kernel_size as u32;
    boot_info.rsdp_addr = rsdp_addr;

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
    build_page_tables(fb_addr);

    // -- Step 7: Enable FPU/SSE, load CR3, jump to kernel ---------------------
    serial_print("[UEFI] Jumping to kernel...\n");
    unsafe {
        jump_to_kernel();
    }
}

// -- GOP setup ----------------------------------------------------------------

fn setup_gop() -> (u32, u32, u32, u32, u8) {
    let gop_handle = boot::get_handle_for_protocol::<GraphicsOutput>()
        .expect("GOP not available");

    let mut gop = boot::open_protocol_exclusive::<GraphicsOutput>(gop_handle)
        .expect("Failed to open GOP");

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

    (fb_base as u32, w as u32, h as u32, stride as u32 * 4, bpp)
}

// -- Kernel loading -----------------------------------------------------------

fn load_kernel() -> usize {
    let fs_handles = boot::find_handles::<SimpleFileSystem>()
        .expect("No filesystem handles found");

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
            if let Some(size) = try_load_kernel_from(&mut root, path) {
                return size;
            }
        }
    }

    panic!("Kernel not found on any partition!");
}

fn try_load_kernel_from(
    root: &mut uefi::proto::media::file::Directory,
    path: &str,
) -> Option<usize> {
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

    // Allocate pages at the kernel load address
    let pages_needed = (file_size + 4095) / 4096;
    boot::allocate_pages(
        AllocateType::Address(KERNEL_LOAD_ADDR),
        MemoryType::LOADER_DATA,
        pages_needed,
    )
    .expect("Failed to allocate kernel memory at 0x100000");

    // Read kernel into memory
    let kernel_buf =
        unsafe { core::slice::from_raw_parts_mut(KERNEL_LOAD_ADDR as *mut u8, file_size) };

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

    Some(file_size)
}

// -- Page table construction --------------------------------------------------

fn build_page_tables(fb_addr: u32) {
    // Clear page table area (0x3000..0x8000 = 5 pages = 20 KiB)
    unsafe {
        let base = PD_FB_ADDR as *mut u8;
        core::ptr::write_bytes(base, 0, 5 * 4096);
    }

    let write64 = |addr: u64, val: u64| unsafe {
        core::ptr::write_volatile(addr as *mut u64, val);
    };

    // PML4[0] -> PDPT_LOW (identity map)
    write64(PML4_ADDR + 0 * 8, PDPT_LOW_ADDR | PT_BASE_FLAGS);
    // PML4[511] -> PDPT_HIGH (higher-half kernel)
    write64(PML4_ADDR + 511 * 8, PDPT_HIGH_ADDR | PT_BASE_FLAGS);

    // PDPT_LOW[0] -> PD_LOW (first 1 GiB)
    write64(PDPT_LOW_ADDR + 0 * 8, PD_LOW_ADDR | PT_BASE_FLAGS);

    // PD_LOW: identity map first 16 MiB with 2 MiB pages
    for i in 0u64..8 {
        write64(PD_LOW_ADDR + i * 8, (i * 0x20_0000) | PT_PAGE_FLAGS);
    }

    // PDPT_HIGH[510] -> PD_LOW (higher-half kernel reuses identity PD)
    write64(PDPT_HIGH_ADDR + 510 * 8, PD_LOW_ADDR | PT_BASE_FLAGS);

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
        let target_pd = if pdpt_index == 0 { PD_LOW_ADDR } else { PD_FB_ADDR };

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

unsafe fn jump_to_kernel() -> ! {
    // Build a small trampoline at TRAMPOLINE_ADDR (0x8000), which is within
    // the identity-mapped first 16 MiB. We MUST switch CR3 from code that is
    // mapped in BOTH the old (UEFI) and new (our) page tables. The UEFI
    // bootloader's code is at a UEFI-allocated address (likely above 16 MiB)
    // which is NOT in our identity map — switching CR3 here would triple-fault.
    //
    // Trampoline expects:
    //   RDI = PML4 physical address (for CR3)
    //   RSI = new stack pointer
    //   RDX = boot_info address (passed to kernel in RDI)
    //   RCX = kernel entry address
    //
    // Trampoline code (10 bytes):
    //   mov cr3, rdi      ; 0F 22 DF     — switch to our page tables
    //   mov rsp, rsi      ; 48 89 F4     — set up kernel stack
    //   mov rdi, rdx      ; 48 89 D7     — RDI = boot_info for kernel
    //   jmp rcx           ; FF E1        — jump to kernel entry
    let trampoline: [u8; 11] = [
        0x0F, 0x22, 0xDF,       // mov cr3, rdi
        0x48, 0x89, 0xF4,       // mov rsp, rsi
        0x48, 0x89, 0xD7,       // mov rdi, rdx
        0xFF, 0xE1,             // jmp rcx
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
        in("rcx") KERNEL_LOAD_ADDR,
        trampoline = in(reg) TRAMPOLINE_ADDR,
        options(noreturn),
    );
}
