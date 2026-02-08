//! User program loader: reads ELF or flat binaries from the filesystem, creates an
//! isolated per-process page directory, maps code/stack/DLL pages, and spawns a kernel
//! trampoline thread that transitions to Ring 3 via `iret`.

use crate::memory::address::VirtAddr;
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::sync::spinlock::Spinlock;

/// Default load address for flat binaries (128 MiB).
/// ELF binaries use their own vaddr from program headers.
const PROGRAM_LOAD_ADDR: u32 = 0x0800_0000;

/// User stack is allocated below this address (192 MiB).
/// Stack grows downward.
const USER_STACK_TOP: u32 = 0x0C00_0000;

/// Number of pages for the user stack (64 KiB = 16 pages).
const USER_STACK_PAGES: u32 = 16;

const PAGE_SIZE: u32 = 4096;
const PAGE_WRITABLE: u32 = 0x02;
const PAGE_USER: u32 = 0x04;

/// Max concurrent pending programs (no heap allocation needed).
const MAX_PENDING: usize = 16;

/// Slot holding the entry point and stack pointer for a newly spawned user thread.
///
/// The trampoline thread looks up its TID in this table to learn where to jump
/// after the context switch into the new address space.
struct PendingSlot {
    tid: u32,
    entry: u32,
    user_stack: u32,
    used: bool,
}

impl PendingSlot {
    const fn empty() -> Self {
        PendingSlot { tid: 0, entry: 0, user_stack: 0, used: false }
    }
}

static PENDING_PROGRAMS: Spinlock<[PendingSlot; MAX_PENDING]> =
    Spinlock::new([
        PendingSlot::empty(), PendingSlot::empty(), PendingSlot::empty(), PendingSlot::empty(),
        PendingSlot::empty(), PendingSlot::empty(), PendingSlot::empty(), PendingSlot::empty(),
        PendingSlot::empty(), PendingSlot::empty(), PendingSlot::empty(), PendingSlot::empty(),
        PendingSlot::empty(), PendingSlot::empty(), PendingSlot::empty(), PendingSlot::empty(),
    ]);

// =========================================================================
// ELF32 structures
// =========================================================================

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const PT_LOAD: u32 = 1;

/// ELF32 file header layout (52 bytes, packed to match on-disk format).
#[repr(C, packed)]
struct Elf32Header {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u32,
    e_phoff: u32,
    e_shoff: u32,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

/// ELF32 program header layout (32 bytes, packed to match on-disk format).
#[repr(C, packed)]
struct Elf32Phdr {
    p_type: u32,
    p_offset: u32,
    p_vaddr: u32,
    p_paddr: u32,
    p_filesz: u32,
    p_memsz: u32,
    p_flags: u32,
    p_align: u32,
}

/// Result of loading an ELF: entry point and brk address.
struct ElfLoadResult {
    entry: u32,
    brk: u32,
}

/// Load an ELF32 binary into a user page directory.
/// Returns the entry point and the brk (end of last segment, page-aligned).
fn load_elf(data: &[u8], pd_phys: crate::memory::address::PhysAddr) -> Result<ElfLoadResult, &'static str> {
    if data.len() < 52 {
        return Err("ELF file too small");
    }

    let hdr = unsafe { &*(data.as_ptr() as *const Elf32Header) };

    // Verify magic
    if hdr.e_ident[0..4] != ELF_MAGIC {
        return Err("Invalid ELF magic");
    }
    // Must be ELF32 (class 1)
    if hdr.e_ident[4] != 1 {
        return Err("Not ELF32");
    }
    // Must be little-endian
    if hdr.e_ident[5] != 1 {
        return Err("Not little-endian ELF");
    }

    let entry = hdr.e_entry;
    let ph_off = hdr.e_phoff as usize;
    let ph_size = hdr.e_phentsize as usize;
    let ph_num = hdr.e_phnum as usize;

    crate::serial_println!("  ELF: entry={:#010x}, {} program headers", entry, ph_num);

    let mut max_vaddr_end: u32 = 0;

    // Iterate program headers and load PT_LOAD segments
    for i in 0..ph_num {
        let ph_offset = ph_off + i * ph_size;
        if ph_offset + ph_size > data.len() {
            return Err("ELF program header out of bounds");
        }
        let phdr = unsafe { &*(data.as_ptr().add(ph_offset) as *const Elf32Phdr) };

        if phdr.p_type != PT_LOAD {
            continue;
        }

        let vaddr = phdr.p_vaddr;
        let memsz = phdr.p_memsz;
        let filesz = phdr.p_filesz;
        let offset = phdr.p_offset;

        if memsz == 0 {
            continue;
        }

        crate::serial_println!(
            "  ELF PT_LOAD: vaddr={:#010x} filesz={:#x} memsz={:#x}",
            vaddr, filesz, memsz
        );

        // Validate: vaddr must be in user space (not kernel)
        if vaddr >= 0xC000_0000 {
            return Err("ELF segment in kernel space");
        }

        // Allocate pages for this segment
        let page_start = vaddr & !0xFFF;
        let page_end = (vaddr + memsz + PAGE_SIZE - 1) & !0xFFF;
        let num_pages = (page_end - page_start) / PAGE_SIZE;

        for p in 0..num_pages {
            let page_virt = VirtAddr::new(page_start + p * PAGE_SIZE);
            // Only map if not already mapped (segments can overlap pages)
            if !virtual_mem::is_mapped_in_pd(pd_phys, page_virt) {
                let phys = physical::alloc_frame()
                    .ok_or("Failed to allocate frame for ELF segment")?;
                virtual_mem::map_page_in_pd(pd_phys, page_virt, phys, PAGE_WRITABLE | PAGE_USER);
            }
        }

        let seg_end = vaddr + memsz;
        if seg_end > max_vaddr_end {
            max_vaddr_end = seg_end;
        }
    }

    // Now switch to user PD and copy data
    unsafe {
        let old_cr3 = virtual_mem::current_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u32());

        for i in 0..ph_num {
            let ph_offset = ph_off + i * ph_size;
            let phdr = &*(data.as_ptr().add(ph_offset) as *const Elf32Phdr);

            if phdr.p_type != PT_LOAD || phdr.p_memsz == 0 {
                continue;
            }

            let vaddr = phdr.p_vaddr;
            let filesz = phdr.p_filesz as usize;
            let memsz = phdr.p_memsz as usize;
            let offset = phdr.p_offset as usize;

            // Zero all allocated pages first. alloc_frame() returns unzeroed
            // frames that may contain stale data from previous use. This
            // prevents info leaks between processes and ensures BSS is clean
            // even if there are page-alignment gaps.
            let page_start = (vaddr & !0xFFF) as usize;
            let page_end = (vaddr as usize + memsz + 0xFFF) & !0xFFF;
            core::ptr::write_bytes(page_start as *mut u8, 0, page_end - page_start);

            // Copy file data over the zeroed pages
            if filesz > 0 && offset + filesz <= data.len() {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(offset),
                    vaddr as *mut u8,
                    filesz,
                );
            }
        }

        core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
    }

    // brk starts at end of last segment, page-aligned up
    let brk = (max_vaddr_end + PAGE_SIZE - 1) & !0xFFF;

    Ok(ElfLoadResult { entry, brk })
}

/// Check if data starts with ELF magic bytes.
fn is_elf(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == ELF_MAGIC
}

/// Load a flat binary from the filesystem and run it in Ring 3.
/// Creates a per-process page directory with isolated user-space mappings.
/// Returns the TID of the spawned thread.
pub fn load_and_run(path: &str, name: &str) -> Result<u32, &'static str> {
    load_and_run_with_args(path, name, "")
}

/// Load a flat binary or ELF and run it with command-line arguments.
pub fn load_and_run_with_args(path: &str, name: &str, args: &str) -> Result<u32, &'static str> {
    // Read the binary from the filesystem
    let data = crate::fs::vfs::read_file_to_vec(path)
        .map_err(|_| "Failed to read program file")?;

    if data.is_empty() {
        return Err("Program file is empty");
    }

    crate::serial_println!("  Loading program '{}' ({} bytes)", path, data.len());

    // Create per-process page directory (clones kernel mappings, empty user space)
    let pd_phys = virtual_mem::create_user_page_directory()
        .ok_or("Failed to create user page directory")?;

    let (entry_point, brk);

    if is_elf(&data) {
        // ---- ELF binary path ----
        crate::serial_println!("  Detected ELF binary");

        // Allocate and map stack pages
        let stack_bottom = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE;
        for i in 0..USER_STACK_PAGES {
            let virt = VirtAddr::new(stack_bottom + i * PAGE_SIZE);
            let phys = physical::alloc_frame()
                .ok_or("Failed to allocate frame for user stack")?;
            virtual_mem::map_page_in_pd(pd_phys, virt, phys, PAGE_WRITABLE | PAGE_USER);
        }

        // Map DLLs
        crate::task::dll::map_all_dlls_into(pd_phys);

        // Load ELF segments (allocates pages, copies data)
        let elf_result = load_elf(&data, pd_phys)?;
        entry_point = elf_result.entry;
        brk = elf_result.brk;

        // Zero user stack
        unsafe {
            let old_cr3 = virtual_mem::current_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u32());
            core::ptr::write_bytes(stack_bottom as *mut u8, 0, (USER_STACK_PAGES * PAGE_SIZE) as usize);
            core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
        }

        crate::serial_println!(
            "  ELF: PD={:#010x}, entry={:#010x}, brk={:#010x}, stack={:#010x}-{:#010x}",
            pd_phys.as_u32(), entry_point, brk,
            stack_bottom, USER_STACK_TOP
        );
    } else {
        // ---- Flat binary path ----
        let code_pages = (data.len() as u32 + PAGE_SIZE - 1) / PAGE_SIZE;
        for i in 0..code_pages {
            let virt = VirtAddr::new(PROGRAM_LOAD_ADDR + i * PAGE_SIZE);
            let phys = physical::alloc_frame()
                .ok_or("Failed to allocate frame for program code")?;
            virtual_mem::map_page_in_pd(pd_phys, virt, phys, PAGE_WRITABLE | PAGE_USER);
        }

        let stack_bottom = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE;
        for i in 0..USER_STACK_PAGES {
            let virt = VirtAddr::new(stack_bottom + i * PAGE_SIZE);
            let phys = physical::alloc_frame()
                .ok_or("Failed to allocate frame for user stack")?;
            virtual_mem::map_page_in_pd(pd_phys, virt, phys, PAGE_WRITABLE | PAGE_USER);
        }

        crate::task::dll::map_all_dlls_into(pd_phys);

        unsafe {
            let old_cr3 = virtual_mem::current_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u32());

            // Zero all code pages first (alloc_frame returns unzeroed frames)
            let dest = PROGRAM_LOAD_ADDR as *mut u8;
            core::ptr::write_bytes(dest, 0, (code_pages * PAGE_SIZE) as usize);

            // Copy program data over zeroed pages
            core::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());

            core::ptr::write_bytes(stack_bottom as *mut u8, 0, (USER_STACK_PAGES * PAGE_SIZE) as usize);

            core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
        }

        entry_point = PROGRAM_LOAD_ADDR;
        brk = PROGRAM_LOAD_ADDR + code_pages * PAGE_SIZE;

        crate::serial_println!(
            "  PD={:#010x}, {} code pages at {:#010x}, {} stack pages at {:#010x}-{:#010x}, brk={:#010x}",
            pd_phys.as_u32(), code_pages, PROGRAM_LOAD_ADDR,
            USER_STACK_PAGES, stack_bottom, USER_STACK_TOP, brk
        );
    }

    // Disable interrupts to prevent the timer from scheduling the new thread
    // before we set its CR3 to the user PD (would page fault at 0x08000000).
    // Preserve caller's interrupt state so we don't re-enable interrupts
    // if the caller intentionally had them disabled.
    let flags: u32;
    unsafe { core::arch::asm!("pushfd; pop {}", out(reg) flags); }
    unsafe { core::arch::asm!("cli"); }

    let tid = crate::task::scheduler::spawn(user_thread_trampoline, 200, name);
    crate::task::scheduler::set_thread_user_info(tid, pd_phys, brk);

    // Store pending program info keyed by TID (after spawn so we know the TID).
    // Uses fixed-size array — no heap allocation.
    {
        let mut slots = PENDING_PROGRAMS.lock();
        let slot = slots.iter_mut().find(|s| !s.used)
            .expect("Too many pending programs");
        slot.tid = tid;
        slot.entry = entry_point;
        slot.user_stack = USER_STACK_TOP;
        slot.used = true;
    }
    if !args.is_empty() {
        crate::task::scheduler::set_thread_args(tid, args);
    }

    crate::serial_println!("  Spawn complete: TID={}, about to restore interrupts (flags={:#x})", tid, flags);

    // Restore caller's interrupt state
    if flags & 0x200 != 0 {
        unsafe { core::arch::asm!("sti"); }
    }

    crate::serial_println!("  load_and_run returning TID={}", tid);

    Ok(tid)
}

/// Trampoline: runs as a kernel thread, then transitions to user mode.
/// At this point, context_switch.asm has already loaded our CR3 (user PD).
extern "C" fn user_thread_trampoline() {
    let tid = crate::task::scheduler::current_tid();
    let (entry, user_stack) = {
        let mut slots = PENDING_PROGRAMS.lock();
        let slot = slots.iter_mut().find(|s| s.used && s.tid == tid)
            .expect("No pending program for trampoline");
        let e = slot.entry;
        let s = slot.user_stack;
        slot.used = false; // Free the slot
        (e, s)
    };

    crate::serial_println!(
        "  User trampoline: entering Ring 3 at {:#010x}, stack={:#010x}",
        entry, user_stack
    );

    // Jump to user mode via iret
    unsafe {
        jump_to_user_mode(entry, user_stack);
    }
}

/// Transition to Ring 3 by setting up an iret frame.
/// User code segment = 0x1B (GDT entry 3 | RPL=3)
/// User data segment = 0x23 (GDT entry 4 | RPL=3)
unsafe fn jump_to_user_mode(entry: u32, user_stack: u32) -> ! {
    core::arch::asm!(
        // Set data segment registers to user data segment
        // Use a dedicated register operand to avoid clobbering user_esp/entry
        "mov ds, {seg:x}",
        "mov es, {seg:x}",
        "mov fs, {seg:x}",
        "mov gs, {seg:x}",
        // Build iret frame on the kernel stack
        "push {seg:e}",   // SS = user data segment (0x23)
        "push {user_esp}", // ESP = user stack pointer
        "pushfd",          // EFLAGS
        "or dword ptr [esp], 0x200", // Set IF (interrupts enabled)
        "push 0x1B",       // CS = user code segment
        "push {entry}",    // EIP = program entry point
        "iretd",           // Enter Ring 3!
        seg = in(reg) 0x23u32,
        user_esp = in(reg) user_stack,
        entry = in(reg) entry,
        options(noreturn)
    );
}
