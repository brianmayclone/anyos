//! User program loader: reads ELF or flat binaries from the filesystem, creates an
//! isolated per-process PML4, maps code/stack/DLL pages, and spawns a kernel
//! trampoline thread that transitions to Ring 3 via `iretq`.

use crate::memory::address::VirtAddr;
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::sync::spinlock::Spinlock;

/// Default load address for flat binaries (128 MiB).
/// ELF binaries use their own vaddr from program headers.
const PROGRAM_LOAD_ADDR: u64 = 0x0800_0000;

/// User stack is allocated below this address (192 MiB).
/// Stack grows downward.
const USER_STACK_TOP: u64 = 0x0C00_0000;

/// Number of pages for the user stack (64 KiB = 16 pages).
const USER_STACK_PAGES: u64 = 16;

const PAGE_SIZE: u64 = 4096;
const PAGE_WRITABLE: u64 = 0x02;
const PAGE_USER: u64 = 0x04;

/// Max concurrent pending programs (no heap allocation needed).
const MAX_PENDING: usize = 16;

/// Slot holding the entry point and stack pointer for a newly spawned user thread.
///
/// The trampoline thread looks up its TID in this table to learn where to jump
/// after the context switch into the new address space.
struct PendingSlot {
    tid: u32,
    entry: u64,
    user_stack: u64,
    is_compat32: bool,
    used: bool,
}

impl PendingSlot {
    const fn empty() -> Self {
        PendingSlot { tid: 0, entry: 0, user_stack: 0, is_compat32: false, used: false }
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
// ELF structures
// =========================================================================

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const PT_LOAD: u32 = 1;

/// ELF class constants (EI_CLASS byte at offset 4).
const ELFCLASS32: u8 = 1;
const ELFCLASS64: u8 = 2;

/// ELF64 file header layout (64 bytes, packed to match on-disk format).
#[repr(C, packed)]
struct Elf64Header {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

/// ELF64 program header layout (56 bytes, packed to match on-disk format).
#[repr(C, packed)]
struct Elf64Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

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
    entry: u64,
    brk: u64,
}

/// Load an ELF64 binary into a user PML4.
/// Returns the entry point and the brk (end of last segment, page-aligned).
fn load_elf64(data: &[u8], pd_phys: crate::memory::address::PhysAddr) -> Result<ElfLoadResult, &'static str> {
    if data.len() < 64 {
        return Err("ELF64 file too small");
    }

    let hdr = unsafe { &*(data.as_ptr() as *const Elf64Header) };

    let entry = hdr.e_entry;
    let ph_off = hdr.e_phoff as usize;
    let ph_size = hdr.e_phentsize as usize;
    let ph_num = hdr.e_phnum as usize;

    crate::serial_println!("  ELF64: entry={:#018x}, {} program headers", entry, ph_num);

    let mut max_vaddr_end: u64 = 0;

    // Iterate program headers and load PT_LOAD segments
    for i in 0..ph_num {
        let ph_offset = ph_off + i * ph_size;
        if ph_offset + ph_size > data.len() {
            return Err("ELF64 program header out of bounds");
        }
        let phdr = unsafe { &*(data.as_ptr().add(ph_offset) as *const Elf64Phdr) };

        if phdr.p_type != PT_LOAD {
            continue;
        }

        let vaddr = phdr.p_vaddr;
        let memsz = phdr.p_memsz;
        let filesz = phdr.p_filesz;

        if memsz == 0 {
            continue;
        }

        crate::serial_println!(
            "  ELF64 PT_LOAD: vaddr={:#018x} filesz={:#x} memsz={:#x}",
            vaddr, filesz, memsz
        );

        // Validate: vaddr must be in user space (lower canonical half)
        if vaddr >= 0x0000_8000_0000_0000 {
            return Err("ELF64 segment in kernel space");
        }

        // Allocate pages for this segment
        let page_start = vaddr & !0xFFF;
        let page_end = (vaddr + memsz + PAGE_SIZE - 1) & !0xFFF;
        let num_pages = (page_end - page_start) / PAGE_SIZE;

        for p in 0..num_pages {
            let page_virt = VirtAddr::new(page_start + p * PAGE_SIZE);
            if !virtual_mem::is_mapped_in_pd(pd_phys, page_virt) {
                let phys = physical::alloc_frame()
                    .ok_or("Failed to allocate frame for ELF64 segment")?;
                virtual_mem::map_page_in_pd(pd_phys, page_virt, phys, PAGE_WRITABLE | PAGE_USER);
            }
        }

        let seg_end = vaddr + memsz;
        if seg_end > max_vaddr_end {
            max_vaddr_end = seg_end;
        }
    }

    // Switch to user PD and copy data
    unsafe {
        let old_cr3 = virtual_mem::current_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());

        for i in 0..ph_num {
            let ph_offset = ph_off + i * ph_size;
            let phdr = &*(data.as_ptr().add(ph_offset) as *const Elf64Phdr);

            if phdr.p_type != PT_LOAD || phdr.p_memsz == 0 {
                continue;
            }

            let vaddr = phdr.p_vaddr;
            let filesz = phdr.p_filesz as usize;
            let memsz = phdr.p_memsz as usize;
            let offset = phdr.p_offset as usize;

            // Zero all allocated pages first
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

    let brk = (max_vaddr_end + PAGE_SIZE - 1) & !0xFFF;
    Ok(ElfLoadResult { entry, brk })
}

/// Load an ELF32 binary into a user PML4 (for 32-bit compatibility mode).
fn load_elf32(data: &[u8], pd_phys: crate::memory::address::PhysAddr) -> Result<ElfLoadResult, &'static str> {
    if data.len() < 52 {
        return Err("ELF32 file too small");
    }

    let hdr = unsafe { &*(data.as_ptr() as *const Elf32Header) };

    let entry = hdr.e_entry as u64;
    let ph_off = hdr.e_phoff as usize;
    let ph_size = hdr.e_phentsize as usize;
    let ph_num = hdr.e_phnum as usize;

    crate::serial_println!("  ELF32: entry={:#010x}, {} program headers", entry, ph_num);

    let mut max_vaddr_end: u64 = 0;

    for i in 0..ph_num {
        let ph_offset = ph_off + i * ph_size;
        if ph_offset + ph_size > data.len() {
            return Err("ELF32 program header out of bounds");
        }
        let phdr = unsafe { &*(data.as_ptr().add(ph_offset) as *const Elf32Phdr) };

        if phdr.p_type != PT_LOAD {
            continue;
        }

        let vaddr = phdr.p_vaddr as u64;
        let memsz = phdr.p_memsz as u64;
        let filesz = phdr.p_filesz as u64;

        if memsz == 0 {
            continue;
        }

        crate::serial_println!(
            "  ELF32 PT_LOAD: vaddr={:#010x} filesz={:#x} memsz={:#x}",
            vaddr, filesz, memsz
        );

        if vaddr >= 0xC000_0000 {
            return Err("ELF32 segment in kernel space");
        }

        let page_start = vaddr & !0xFFF;
        let page_end = (vaddr + memsz + PAGE_SIZE - 1) & !0xFFF;
        let num_pages = (page_end - page_start) / PAGE_SIZE;

        for p in 0..num_pages {
            let page_virt = VirtAddr::new(page_start + p * PAGE_SIZE);
            if !virtual_mem::is_mapped_in_pd(pd_phys, page_virt) {
                let phys = physical::alloc_frame()
                    .ok_or("Failed to allocate frame for ELF32 segment")?;
                virtual_mem::map_page_in_pd(pd_phys, page_virt, phys, PAGE_WRITABLE | PAGE_USER);
            }
        }

        let seg_end = vaddr + memsz;
        if seg_end > max_vaddr_end {
            max_vaddr_end = seg_end;
        }
    }

    unsafe {
        let old_cr3 = virtual_mem::current_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());

        for i in 0..ph_num {
            let ph_offset = ph_off + i * ph_size;
            let phdr = &*(data.as_ptr().add(ph_offset) as *const Elf32Phdr);

            if phdr.p_type != PT_LOAD || phdr.p_memsz == 0 {
                continue;
            }

            let vaddr = phdr.p_vaddr as u64;
            let filesz = phdr.p_filesz as usize;
            let memsz = phdr.p_memsz as usize;
            let offset = phdr.p_offset as usize;

            let page_start = (vaddr & !0xFFF) as usize;
            let page_end = (vaddr as usize + memsz + 0xFFF) & !0xFFF;
            core::ptr::write_bytes(page_start as *mut u8, 0, page_end - page_start);

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

    let brk = (max_vaddr_end + PAGE_SIZE - 1) & !0xFFF;
    Ok(ElfLoadResult { entry, brk })
}

/// Check if data starts with ELF magic bytes.
fn is_elf(data: &[u8]) -> bool {
    data.len() >= 4 && data[0..4] == ELF_MAGIC
}

/// Return the ELF class (1=ELF32, 2=ELF64) or 0 if not an ELF.
fn elf_class(data: &[u8]) -> u8 {
    if data.len() >= 5 && data[0..4] == ELF_MAGIC {
        data[4]
    } else {
        0
    }
}

/// Load a flat binary from the filesystem and run it in Ring 3.
/// Creates a per-process PML4 with isolated user-space mappings.
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

    // Create per-process PML4 (clones kernel mappings, empty user space)
    let pd_phys = virtual_mem::create_user_page_directory()
        .ok_or("Failed to create user page directory")?;

    let (entry_point, brk);
    let mut is_compat32 = false;

    let class = elf_class(&data);
    if class == ELFCLASS64 {
        // ---- ELF64 binary path ----
        crate::serial_println!("  Detected ELF64 binary");

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

        // Load ELF64 segments
        let elf_result = load_elf64(&data, pd_phys)?;
        entry_point = elf_result.entry;
        brk = elf_result.brk;

        // Zero user stack
        unsafe {
            let old_cr3 = virtual_mem::current_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());
            core::ptr::write_bytes(stack_bottom as *mut u8, 0, (USER_STACK_PAGES * PAGE_SIZE) as usize);
            core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
        }

        crate::serial_println!(
            "  ELF64: PD={:#018x}, entry={:#018x}, brk={:#018x}, stack={:#010x}-{:#010x}",
            pd_phys.as_u64(), entry_point, brk,
            stack_bottom, USER_STACK_TOP
        );
    } else if class == ELFCLASS32 {
        // ---- ELF32 binary path (32-bit compatibility) ----
        crate::serial_println!("  Detected ELF32 binary");

        let stack_bottom = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE;
        for i in 0..USER_STACK_PAGES {
            let virt = VirtAddr::new(stack_bottom + i * PAGE_SIZE);
            let phys = physical::alloc_frame()
                .ok_or("Failed to allocate frame for user stack")?;
            virtual_mem::map_page_in_pd(pd_phys, virt, phys, PAGE_WRITABLE | PAGE_USER);
        }

        crate::task::dll::map_all_dlls_into(pd_phys);

        let elf_result = load_elf32(&data, pd_phys)?;
        entry_point = elf_result.entry;
        brk = elf_result.brk;
        is_compat32 = true;

        unsafe {
            let old_cr3 = virtual_mem::current_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());
            core::ptr::write_bytes(stack_bottom as *mut u8, 0, (USER_STACK_PAGES * PAGE_SIZE) as usize);
            core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
        }

        crate::serial_println!(
            "  ELF32 (compat): PD={:#018x}, entry={:#010x}, brk={:#010x}, stack={:#010x}-{:#010x}",
            pd_phys.as_u64(), entry_point, brk,
            stack_bottom, USER_STACK_TOP
        );
    } else if is_elf(&data) {
        return Err("Unknown ELF class (not ELF32 or ELF64)");
    } else {
        // ---- Flat binary path ----
        let code_pages = (data.len() as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
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
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());

            let dest = PROGRAM_LOAD_ADDR as *mut u8;
            core::ptr::write_bytes(dest, 0, (code_pages * PAGE_SIZE) as usize);
            core::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());
            core::ptr::write_bytes(stack_bottom as *mut u8, 0, (USER_STACK_PAGES * PAGE_SIZE) as usize);

            core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
        }

        entry_point = PROGRAM_LOAD_ADDR;
        brk = PROGRAM_LOAD_ADDR + code_pages * PAGE_SIZE;

        crate::serial_println!(
            "  PD={:#018x}, {} code pages at {:#010x}, brk={:#010x}",
            pd_phys.as_u64(), code_pages, PROGRAM_LOAD_ADDR, brk
        );
    }

    // Disable interrupts to prevent the timer from scheduling the new thread
    // before we set its CR3 to the user PD (would page fault at 0x08000000).
    let flags: u64;
    unsafe { core::arch::asm!("pushfq; pop {}", out(reg) flags); }
    unsafe { core::arch::asm!("cli"); }

    let tid = crate::task::scheduler::spawn(user_thread_trampoline, 200, name);
    crate::task::scheduler::set_thread_user_info(tid, pd_phys, brk as u32);

    // Set architecture mode for compat32 threads
    if is_compat32 {
        crate::task::scheduler::set_thread_arch_mode(
            tid, crate::task::thread::ArchMode::Compat32,
        );
    }

    // Store pending program info keyed by TID (after spawn so we know the TID).
    {
        let mut slots = PENDING_PROGRAMS.lock();
        let slot = slots.iter_mut().find(|s| !s.used)
            .expect("Too many pending programs");
        slot.tid = tid;
        slot.entry = entry_point;
        slot.user_stack = USER_STACK_TOP;
        slot.is_compat32 = is_compat32;
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
    let (entry, user_stack, compat32) = {
        let mut slots = PENDING_PROGRAMS.lock();
        let slot = slots.iter_mut().find(|s| s.used && s.tid == tid)
            .expect("No pending program for trampoline");
        let e = slot.entry;
        let s = slot.user_stack;
        let c = slot.is_compat32;
        slot.used = false; // Free the slot
        (e, s, c)
    };

    if compat32 {
        crate::serial_println!(
            "  User trampoline (compat32): entering Ring 3 at {:#010x}, stack={:#010x}",
            entry, user_stack
        );
        unsafe { jump_to_user_mode_compat32(entry, user_stack); }
    } else {
        crate::serial_println!(
            "  User trampoline: entering Ring 3 at {:#018x}, stack={:#018x}",
            entry, user_stack
        );
        unsafe { jump_to_user_mode(entry, user_stack); }
    }
}

/// Transition to Ring 3 by setting up an iretq frame.
/// User code segment 64-bit = 0x2B (GDT entry 5 | RPL=3)
/// User data segment = 0x23 (GDT entry 4 | RPL=3)
unsafe fn jump_to_user_mode(entry: u64, user_stack: u64) -> ! {
    core::arch::asm!(
        // Set data segment registers to user data segment
        "mov ax, 0x23",
        "mov ds, ax",
        "mov es, ax",
        "mov fs, ax",
        "mov gs, ax",
        // Build iretq frame on the kernel stack:
        //   SS, RSP, RFLAGS, CS, RIP
        "push 0x23",       // SS = user data segment
        "push {user_rsp}", // RSP = user stack pointer
        "pushfq",          // RFLAGS
        "pop rax",
        "or rax, 0x200",   // Set IF (interrupts enabled)
        "push rax",
        "push 0x2B",       // CS = user code 64-bit segment
        "push {entry}",    // RIP = program entry point
        "iretq",           // Enter Ring 3!
        user_rsp = in(reg) user_stack,
        entry = in(reg) entry,
        options(noreturn)
    );
}

/// Transition to Ring 3 in 32-bit compatibility mode via iretq.
/// User code segment 32-bit = 0x1B (GDT entry 3 | RPL=3, L=0, D=1)
/// User data segment = 0x23 (GDT entry 4 | RPL=3)
/// When IRETQ loads CS=0x1B (a 32-bit code segment), the CPU enters
/// compatibility mode: the thread runs 32-bit code under the 64-bit kernel.
unsafe fn jump_to_user_mode_compat32(entry: u64, user_stack: u64) -> ! {
    core::arch::asm!(
        // Set data segment registers to user data segment
        "mov ax, 0x23",
        "mov ds, ax",
        "mov es, ax",
        "mov fs, ax",
        "mov gs, ax",
        // Build iretq frame on the kernel stack:
        //   SS, RSP, RFLAGS, CS, RIP
        "push 0x23",       // SS = user data segment
        "push {user_rsp}", // RSP = user stack pointer (truncated to 32-bit by compat mode)
        "pushfq",          // RFLAGS
        "pop rax",
        "or rax, 0x200",   // Set IF (interrupts enabled)
        "push rax",
        "push 0x1B",       // CS = user code 32-bit compat segment
        "push {entry}",    // EIP = program entry point (32-bit)
        "iretq",           // Enter Ring 3 in compatibility mode!
        user_rsp = in(reg) user_stack,
        entry = in(reg) entry,
        options(noreturn)
    );
}
