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

/// Number of pages for the user stack (8 MiB = 2048 pages).
/// Matches the Linux default of 8 MiB.
const USER_STACK_PAGES: u64 = 2048;

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
// fork() child state — saved parent registers for child to resume from
// =========================================================================

/// User-mode register state saved by fork() for the child process.
/// The child's trampoline restores these via IRETQ, with RAX=0.
#[repr(C)]
pub struct ForkChildRegs {
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    // IRETQ frame
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

struct ForkPendingSlot {
    tid: u32,
    used: bool,
    regs: ForkChildRegs,
}

impl ForkPendingSlot {
    const fn empty() -> Self {
        ForkPendingSlot {
            tid: 0,
            used: false,
            regs: ForkChildRegs {
                rbx: 0, rcx: 0, rdx: 0, rsi: 0, rdi: 0, rbp: 0,
                r8: 0, r9: 0, r10: 0, r11: 0, r12: 0, r13: 0, r14: 0, r15: 0,
                rip: 0, cs: 0, rflags: 0, rsp: 0, ss: 0,
            },
        }
    }
}

static PENDING_FORKS: Spinlock<[ForkPendingSlot; MAX_PENDING]> =
    Spinlock::new([
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
        ForkPendingSlot::empty(), ForkPendingSlot::empty(),
    ]);

/// Store the parent's register state for a fork() child to pick up.
pub fn store_pending_fork(tid: u32, regs: ForkChildRegs) {
    let mut slots = PENDING_FORKS.lock();
    let slot = slots.iter_mut().find(|s| !s.used)
        .expect("Too many pending forks");
    slot.tid = tid;
    slot.regs = regs;
    slot.used = true;
}

/// Trampoline for fork() child threads.
/// Wakes up in kernel mode, retrieves saved parent registers, then IRETQ to user
/// mode with RAX=0 (fork child return value).
pub extern "C" fn fork_child_trampoline() {
    let tid = crate::task::scheduler::current_tid();

    // Retrieve saved register state
    let regs = {
        let mut slots = PENDING_FORKS.lock();
        let slot = slots.iter_mut().find(|s| s.used && s.tid == tid)
            .expect("No pending fork state for child trampoline");
        // Copy regs out and free slot
        let r = ForkChildRegs {
            rbx: slot.regs.rbx, rcx: slot.regs.rcx, rdx: slot.regs.rdx,
            rsi: slot.regs.rsi, rdi: slot.regs.rdi, rbp: slot.regs.rbp,
            r8: slot.regs.r8, r9: slot.regs.r9, r10: slot.regs.r10,
            r11: slot.regs.r11, r12: slot.regs.r12, r13: slot.regs.r13,
            r14: slot.regs.r14, r15: slot.regs.r15,
            rip: slot.regs.rip, cs: slot.regs.cs,
            rflags: slot.regs.rflags, rsp: slot.regs.rsp, ss: slot.regs.ss,
        };
        slot.used = false;
        r
    };

    unsafe { fork_return_to_user(&regs); }
}

/// Restore all user-mode registers from a ForkChildRegs struct and IRETQ.
/// Sets RAX=0 (fork child return value). Never returns.
///
/// CRITICAL: Never hardcode register names (ax, rax, etc.) in asm! blocks when
/// `in(reg)` operands exist — LLVM may allocate the same register, causing
/// silent corruption of the pointer operand.
unsafe fn fork_return_to_user(regs: *const ForkChildRegs) -> ! {
    core::arch::asm!(
        "cli",
        // Set data segments for user mode — use {seg} operand, NEVER hardcode ax
        "mov ds, {seg:x}",
        "mov es, {seg:x}",
        "mov fs, {seg:x}",
        "mov gs, {seg:x}",
        // Build IRETQ frame from struct (field offsets in ForkChildRegs):
        // rbx=0, rcx=8, rdx=16, rsi=24, rdi=32, rbp=40,
        // r8=48, r9=56, r10=64, r11=72, r12=80, r13=88, r14=96, r15=104,
        // rip=112, cs=120, rflags=128, rsp=136, ss=144
        "push qword ptr [{p} + 144]",   // SS
        "push qword ptr [{p} + 136]",   // RSP
        "push qword ptr [{p} + 128]",   // RFLAGS
        "or qword ptr [rsp], 0x200",    // Ensure IF set (no hardcoded reg)
        "push qword ptr [{p} + 120]",   // CS
        "push qword ptr [{p} + 112]",   // RIP
        // Restore GPRs — {p} is still live, no hardcoded reg writes allowed
        "mov r15, [{p} + 104]",
        "mov r14, [{p} + 96]",
        "mov r13, [{p} + 88]",
        "mov r12, [{p} + 80]",
        "mov r11, [{p} + 72]",
        "mov r10, [{p} + 64]",
        "mov r9,  [{p} + 56]",
        "mov r8,  [{p} + 48]",
        "mov rbp, [{p} + 40]",
        "mov rdi, [{p} + 32]",
        "mov rsi, [{p} + 24]",
        "mov rdx, [{p} + 16]",
        "mov rcx, [{p} + 8]",
        "mov rbx, [{p}]",
        // {p} is now dead — safe to clobber any register
        "xor eax, eax",             // RAX = 0 (fork child return value)
        "iretq",
        p = in(reg) regs,
        seg = in(reg) 0x23u64,
        options(noreturn)
    );
}

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
    /// Total user pages mapped (code + data + BSS segments).
    pages_mapped: u32,
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



    let mut max_vaddr_end: u64 = 0;
    let mut total_pages: u32 = 0;

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
                total_pages += 1;
            }
        }

        let seg_end = vaddr + memsz;
        if seg_end > max_vaddr_end {
            max_vaddr_end = seg_end;
        }
    }

    // Switch to user PD and copy data (interrupts disabled to prevent
    // timer-driven context switch while CR3 points at the target PD).
    // Save/restore RFLAGS instead of unconditional cli/sti to avoid
    // re-enabling interrupts when caller already had them disabled.
    unsafe {
        let rflags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
        core::arch::asm!("cli", options(nomem, nostack));
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
        core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
    }

    let brk = (max_vaddr_end + PAGE_SIZE - 1) & !0xFFF;
    Ok(ElfLoadResult { entry, brk, pages_mapped: total_pages })
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



    let mut max_vaddr_end: u64 = 0;
    let mut total_pages: u32 = 0;

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
                total_pages += 1;
            }
        }

        let seg_end = vaddr + memsz;
        if seg_end > max_vaddr_end {
            max_vaddr_end = seg_end;
        }
    }

    unsafe {
        let rflags: u64;
        core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
        core::arch::asm!("cli", options(nomem, nostack));
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
        core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
    }

    let brk = (max_vaddr_end + PAGE_SIZE - 1) & !0xFFF;
    Ok(ElfLoadResult { entry, brk, pages_mapped: total_pages })
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

// =========================================================================
// Shared binary loading (used by both spawn and exec)
// =========================================================================

/// Result of loading a binary into a page directory.
pub struct LoadResult {
    pub entry: u64,
    pub brk: u64,
    pub is_compat32: bool,
    pub user_pages: u32,
}

/// Load a binary (ELF64/ELF32/flat) into an already-created page directory.
/// Maps code segments + user stack. Returns entry point, brk, arch mode, page count.
pub fn load_binary_into_pd(
    data: &[u8],
    pd_phys: crate::memory::address::PhysAddr,
) -> Result<LoadResult, &'static str> {
    if data.is_empty() {
        return Err("Program data is empty");
    }

    let mut total_user_pages: u32 = 0;
    let stack_bottom = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE;

    let class = elf_class(data);
    if class == ELFCLASS64 {
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_bottom),
            USER_STACK_PAGES,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;
        let elf_result = load_elf64(data, pd_phys)?;
        total_user_pages += elf_result.pages_mapped + stack_mapped;
        Ok(LoadResult {
            entry: elf_result.entry,
            brk: elf_result.brk,
            is_compat32: false,
            user_pages: total_user_pages,
        })
    } else if class == ELFCLASS32 {
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_bottom),
            USER_STACK_PAGES,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;
        let elf_result = load_elf32(data, pd_phys)?;
        total_user_pages += elf_result.pages_mapped + stack_mapped;
        Ok(LoadResult {
            entry: elf_result.entry,
            brk: elf_result.brk,
            is_compat32: true,
            user_pages: total_user_pages,
        })
    } else if is_elf(data) {
        Err("Unknown ELF class (not ELF32 or ELF64)")
    } else {
        // Flat binary
        let code_pages = (data.len() as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
        let code_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(PROGRAM_LOAD_ADDR),
            code_pages,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_bottom),
            USER_STACK_PAGES,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;

        // Copy binary data into the new address space
        unsafe {
            let rflags: u64;
            core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
            core::arch::asm!("cli", options(nomem, nostack));
            let old_cr3 = virtual_mem::current_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());

            let dest = PROGRAM_LOAD_ADDR as *mut u8;
            core::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());

            core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
            core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
        }

        total_user_pages += code_mapped + stack_mapped;
        Ok(LoadResult {
            entry: PROGRAM_LOAD_ADDR,
            brk: PROGRAM_LOAD_ADDR + code_pages * PAGE_SIZE,
            is_compat32: false,
            user_pages: total_user_pages,
        })
    }
}

// =========================================================================
// exec() — replace current process image
// =========================================================================

/// Replace the current process with a new binary loaded from `data`.
/// On success, never returns (jumps to user mode in new address space).
/// On failure, returns an error string and the old process continues.
pub fn exec_current_process(data: &[u8], args: &str) -> &'static str {
    let tid = crate::task::scheduler::current_tid();

    // Get old PD before we replace it
    let old_pd = match crate::task::scheduler::current_thread_page_directory() {
        Some(pd) => pd,
        None => return "exec: no page directory on current thread",
    };

    // Create fresh page directory
    let new_pd = match virtual_mem::create_user_page_directory() {
        Some(pd) => pd,
        None => return "exec: failed to create page directory (OOM)",
    };

    // Load binary into new PD
    let result = match load_binary_into_pd(data, new_pd) {
        Ok(r) => r,
        Err(e) => {
            virtual_mem::destroy_user_page_directory(new_pd);
            return e;
        }
    };

    // Map DLLs into new address space
    crate::task::dll::map_all_dlls_into(new_pd);

    // Determine architecture mode
    let arch_mode = if result.is_compat32 {
        crate::task::thread::ArchMode::Compat32
    } else {
        crate::task::thread::ArchMode::Native64
    };

    // Update thread metadata (PD, brk, arch_mode, FPU reset, mmap reset)
    crate::task::scheduler::exec_update_thread(
        tid, new_pd, result.brk as u32, arch_mode, result.user_pages,
    );

    // Set new args (clear old args first)
    crate::task::scheduler::set_thread_args(tid, args);

    // Rekey environment from old PD to new PD (move entries in-place)
    crate::task::env::rekey_env(old_pd.0, new_pd.0);

    // Switch CR3 to new address space and destroy old one
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
        core::arch::asm!("mov cr3, {}", in(reg) new_pd.as_u64());
    }

    // Destroy old PD (safe: we're now running on new PD, kernel pages are shared)
    virtual_mem::destroy_user_page_directory(old_pd);

    // Re-enable interrupts and jump to user mode (never returns)
    let user_stack = USER_STACK_TOP - 8;

    let fmt = if result.is_compat32 { "elf32" } else { "elf64" };
    crate::serial_println!("exec: T{} -> ({}, {} pages, entry={:#x})",
        tid, fmt, result.user_pages, result.entry);

    if result.is_compat32 {
        unsafe { jump_to_user_mode_compat32(result.entry, user_stack); }
    } else {
        unsafe { jump_to_user_mode(result.entry, user_stack); }
    }
}

/// Load a flat binary from the filesystem and run it in Ring 3.
/// Creates a per-process PML4 with isolated user-space mappings.
/// Returns the TID of the spawned thread.
pub fn load_and_run(path: &str, name: &str) -> Result<u32, &'static str> {
    load_and_run_with_args(path, name, "")
}

/// Load a flat binary or ELF and run it with command-line arguments.
/// If `path` ends with `.app`, it is treated as a bundle directory:
/// the binary is resolved from Info.conf `exec=` field, or derived from the folder name.
/// The exec binary MUST reside directly inside the .app directory (no subdirectories).
pub fn load_and_run_with_args(path: &str, name: &str, args: &str) -> Result<u32, &'static str> {
    // .app bundle resolution
    let resolved_path: alloc::string::String;
    let bundle_cwd: Option<alloc::string::String>;
    let bundle_caps: Option<crate::task::capabilities::CapSet>;
    let bundle_app_id: Option<alloc::string::String>;
    let actual_path = if path.ends_with(".app") {
        // Parse Info.conf for exec field and working_dir
        let config = crate::task::app_config::parse_info_conf(path);

        // Determine binary name: prefer Info.conf exec=, fallback to folder name
        let binary_name: alloc::string::String = if let Some(ref cfg) = config {
            if let Some(ref exec) = cfg.exec {
                // SECURITY: exec must be a plain filename — no '/' or '..' allowed.
                // The binary MUST reside directly inside the .app directory.
                if exec.contains('/') || exec.contains("..") {
                    return Err(".app exec must be a plain filename (no path separators)");
                }
                alloc::string::String::from(exec.as_str())
            } else {
                // Fallback: derive from folder name minus ".app"
                let folder_name = path.rsplit('/').next().unwrap_or(path);
                alloc::string::String::from(&folder_name[..folder_name.len() - 4])
            }
        } else {
            let folder_name = path.rsplit('/').next().unwrap_or(path);
            alloc::string::String::from(&folder_name[..folder_name.len() - 4])
        };

        if binary_name.is_empty() {
            return Err("Invalid .app bundle: empty exec name");
        }

        resolved_path = alloc::format!("{}/{}", path, binary_name);

        // Determine CWD from working_dir field (default: bundle directory)
        bundle_cwd = if let Some(ref cfg) = config {
            match cfg.working_dir.as_deref() {
                Some("home") => Some(alloc::string::String::from("/")),
                Some(explicit) if explicit != "bundle" => Some(alloc::string::String::from(explicit)),
                _ => Some(alloc::string::String::from(path)), // "bundle" or unset
            }
        } else {
            Some(alloc::string::String::from(path))
        };

        // Extract capabilities from Info.conf
        bundle_caps = if let Some(ref cfg) = config {
            if let Some(ref cap_str) = cfg.capabilities {
                Some(crate::task::capabilities::parse_capabilities(cap_str))
            } else {
                Some(crate::task::capabilities::CAP_DEFAULT)
            }
        } else {
            Some(crate::task::capabilities::CAP_DEFAULT)
        };

        // Extract app_id for permission lookup (id field from Info.conf, or folder name)
        bundle_app_id = if let Some(ref cfg) = config {
            if let Some(ref id) = cfg.id {
                Some(id.clone())
            } else {
                Some(alloc::string::String::from(name))
            }
        } else {
            Some(alloc::string::String::from(name))
        };

        resolved_path.as_str()
    } else {
        bundle_cwd = None;
        bundle_caps = None;
        bundle_app_id = None;
        path
    };

    // Permission check: caller must have read permission on the binary
    if let Ok((uid, gid, mode)) = crate::fs::vfs::get_permissions(actual_path) {
        if !crate::fs::permissions::check_permission(uid, gid, mode, crate::fs::permissions::PERM_READ) {
            return Err("Permission denied");
        }
    }

    // Read the binary from the filesystem
    let data = match crate::fs::vfs::read_file_to_vec(actual_path) {
        Ok(d) => d,
        Err(e) => {
            crate::serial_println!("  load_and_run: read_file_to_vec('{}') failed: {:?}", actual_path, e);
            return Err("Failed to read program file");
        }
    };

    if data.is_empty() {
        return Err("Program file is empty");
    }

    // Create per-process PML4 (clones kernel mappings, empty user space)
    let pd_phys = virtual_mem::create_user_page_directory()
        .ok_or("Failed to create user page directory")?;

    let (entry_point, brk);
    let mut is_compat32 = false;
    let mut total_user_pages: u32 = 0;

    let class = elf_class(&data);
    if class == ELFCLASS64 {
        // ---- ELF64 binary path ----

        // Allocate, map, and zero stack pages (single CR3 switch)
        let stack_bottom = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE;
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_bottom),
            USER_STACK_PAGES,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;

        // Pre-map DLIB shared RO pages into the new address space.
        // Without this, every first DLIB access triggers a page fault inside
        // the kernel's demand-page handler (which holds LOADED_DLLS + ALLOCATOR
        // locks). Pre-mapping avoids that fragile nested-lock path for RO code.
        // Per-process .data/.bss pages are still demand-paged on first access.
        crate::task::dll::map_all_dlls_into(pd_phys);

        // Load ELF64 segments
        let elf_result = load_elf64(&data, pd_phys)?;
        entry_point = elf_result.entry;
        brk = elf_result.brk;
        total_user_pages += elf_result.pages_mapped + stack_mapped;

    } else if class == ELFCLASS32 {
        // ---- ELF32 binary path (32-bit compatibility) ----

        let stack_bottom = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE;
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_bottom),
            USER_STACK_PAGES,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;

        // Pre-map DLIB shared RO pages (same as ELF64 path above)
        crate::task::dll::map_all_dlls_into(pd_phys);

        let elf_result = load_elf32(&data, pd_phys)?;
        entry_point = elf_result.entry;
        brk = elf_result.brk;
        is_compat32 = true;
        total_user_pages += elf_result.pages_mapped + stack_mapped;

    } else if is_elf(&data) {
        return Err("Unknown ELF class (not ELF32 or ELF64)");
    } else {
        // ---- Flat binary path ----
        let code_pages = (data.len() as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
        let code_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(PROGRAM_LOAD_ADDR),
            code_pages,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;

        let stack_bottom = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE;
        let stack_mapped = virtual_mem::map_pages_range_in_pd(
            pd_phys,
            VirtAddr::new(stack_bottom),
            USER_STACK_PAGES,
            PAGE_WRITABLE | PAGE_USER,
            true,
        )?;

        // Pre-map DLIB shared RO pages (same as ELF64/ELF32 paths above)
        crate::task::dll::map_all_dlls_into(pd_phys);

        // Copy binary data (pages already zeroed by map_pages_range_in_pd)
        unsafe {
            let rflags: u64;
            core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
            core::arch::asm!("cli", options(nomem, nostack));
            let old_cr3 = virtual_mem::current_cr3();
            core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u64());

            let dest = PROGRAM_LOAD_ADDR as *mut u8;
            core::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());

            core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
            core::arch::asm!("push {}; popfq", in(reg) rflags, options(nomem));
        }

        entry_point = PROGRAM_LOAD_ADDR;
        brk = PROGRAM_LOAD_ADDR + code_pages * PAGE_SIZE;
        total_user_pages += code_mapped + stack_mapped;

    }

    // Spawn in Blocked state — the thread cannot be picked up by any CPU
    // (including APs) until we explicitly wake it.  This prevents the SMP race
    // where an AP runs the trampoline before we store pending-program data.
    let tid = crate::task::scheduler::spawn_blocked(user_thread_trampoline, 100, name);
    crate::task::scheduler::set_thread_user_info(tid, pd_phys, brk as u32);
    if total_user_pages > 0 {
        crate::task::scheduler::adjust_thread_user_pages(tid, total_user_pages as i32);
    }

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
        // Subtract 8 from stack top: x86_64 ABI requires RSP % 16 == 8 at
        // function entry (as if `call` pushed an 8-byte return address). Since
        // _start is entered via iret (no call), we simulate this alignment.
        slot.user_stack = USER_STACK_TOP - 8;
        slot.is_compat32 = is_compat32;
        slot.used = true;
    }
    if !args.is_empty() {
        crate::task::scheduler::set_thread_args(tid, args);
    }

    // Set CWD for .app bundle processes
    if let Some(ref cwd) = bundle_cwd {
        crate::task::scheduler::set_thread_cwd(tid, cwd);
    }

    // Set capabilities: .app bundles use Info.conf intersected with stored permissions,
    // non-.app binaries inherit parent's caps.
    // The permission boundary is at the .app bundle level — CLI tools and system services
    // inherit whatever their parent has (compositor children get CAP_ALL, etc.).
    let caps = if let Some(declared) = bundle_caps {
        use crate::task::capabilities::*;
        if declared == CAP_ALL {
            // System app (capabilities=all) — full access, no permission restriction
            CAP_ALL
        } else {
            // Intersect declared caps with stored user permissions:
            // - auto-granted caps (DLL, THREAD, SHM, EVENT, PIPE) always apply
            // - sensitive caps only if the user granted them
            let auto = CAP_AUTO_GRANTED;
            let uid = crate::task::scheduler::current_thread_uid();
            let app_id = bundle_app_id.as_deref().unwrap_or(name);
            let granted_sensitive = crate::task::permissions::read_stored_perms(uid, app_id)
                .unwrap_or(0);
            auto | (declared & granted_sensitive)
        }
    } else {
        let parent_caps = crate::task::scheduler::current_thread_capabilities();
        if parent_caps == 0 {
            // Kernel thread spawning user process (e.g. compositor at boot) — full access
            crate::task::capabilities::CAP_ALL
        } else if actual_path == "/System/permdialog" {
            // Kernel allowlist: PermissionDialog needs MANAGE_PERMS + FILESYSTEM
            // regardless of parent's caps, so it can write permission files.
            parent_caps | crate::task::capabilities::CAP_MANAGE_PERMS
                        | crate::task::capabilities::CAP_FILESYSTEM
        } else {
            // Non-.app binary: inherit parent's full capabilities
            parent_caps
        }
    };
    crate::task::scheduler::set_thread_capabilities(tid, caps);

    // Inherit uid/gid from parent thread (processes start with same identity)
    let (parent_uid, parent_gid) = {
        let uid = crate::task::scheduler::current_thread_uid();
        let gid = crate::task::scheduler::current_thread_gid();
        (uid, gid)
    };
    crate::task::scheduler::set_thread_identity(tid, parent_uid, parent_gid);

    let fmt = if is_compat32 { "elf32" } else if is_elf(&data) { "elf64" } else { "flat" };
    crate::serial_println!("spawn: '{}' -> T{} ({}, {} pages, entry={:#x})",
        path, tid, fmt, total_user_pages, entry_point);

    // All setup complete (CR3, pending data, args, CWD, caps). Now make the thread runnable.
    crate::task::scheduler::wake_thread(tid);

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
        unsafe { jump_to_user_mode_compat32(entry, user_stack); }
    } else {
        unsafe { jump_to_user_mode(entry, user_stack); }
    }
}

/// Store a pending entry point and user stack for a new intra-process thread.
/// Called by `scheduler::create_thread_in_current_process()`.
pub fn store_pending_thread(tid: u32, entry: u64, user_stack: u64) {
    let mut slots = PENDING_PROGRAMS.lock();
    let slot = slots.iter_mut().find(|s| !s.used)
        .expect("Too many pending programs");
    slot.tid = tid;
    slot.entry = entry;
    slot.user_stack = user_stack;
    slot.is_compat32 = false;
    slot.used = true;
}

/// Trampoline for intra-process threads created via SYS_THREAD_CREATE.
/// Identical to `user_thread_trampoline` — looks up the pending slot and jumps to user mode.
pub extern "C" fn thread_create_trampoline() {
    let tid = crate::task::scheduler::current_tid();
    let (entry, user_stack, compat32) = {
        let mut slots = PENDING_PROGRAMS.lock();
        let slot = slots.iter_mut().find(|s| s.used && s.tid == tid)
            .expect("No pending program for thread_create trampoline");
        let e = slot.entry;
        let s = slot.user_stack;
        let c = slot.is_compat32;
        slot.used = false;
        (e, s, c)
    };

    if compat32 {
        unsafe { jump_to_user_mode_compat32(entry, user_stack); }
    } else {
        unsafe { jump_to_user_mode(entry, user_stack); }
    }
}

/// Transition to Ring 3 by setting up an iretq frame.
/// User code segment 64-bit = 0x2B (GDT entry 5 | RPL=3)
/// User data segment = 0x23 (GDT entry 4 | RPL=3)
unsafe fn jump_to_user_mode(entry: u64, user_stack: u64) -> ! {
    // Use explicit R14/R15 to avoid `mov ax, 0x23` clobbering an in(reg) operand
    // (MEMORY.md: hardcoded AX in asm! corrupts any in(reg) that the compiler
    //  allocates to RAX — and `pop rax` would clobber it too)
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
        "push r14",        // RSP = user stack pointer
        "pushfq",          // RFLAGS
        "pop rax",
        "or rax, 0x200",   // Set IF (interrupts enabled)
        "push rax",
        "push 0x2B",       // CS = user code 64-bit segment
        "push r15",        // RIP = program entry point
        // Clear all GPRs to prevent kernel address leaks to user mode
        // (critical for exec: INT 0x80 frame leaves kernel values in regs)
        "xor eax, eax",
        "xor ebx, ebx",
        "xor ecx, ecx",
        "xor edx, edx",
        "xor esi, esi",
        "xor edi, edi",
        "xor ebp, ebp",
        "xor r8d, r8d",
        "xor r9d, r9d",
        "xor r10d, r10d",
        "xor r11d, r11d",
        "xor r12d, r12d",
        "xor r13d, r13d",
        "xor r14d, r14d",
        "xor r15d, r15d",
        "iretq",           // Enter Ring 3!
        in("r14") user_stack,
        in("r15") entry,
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
        "push r14",        // RSP = user stack pointer (truncated to 32-bit by compat mode)
        "pushfq",          // RFLAGS
        "pop rax",
        "or rax, 0x200",   // Set IF (interrupts enabled)
        "push rax",
        "push 0x1B",       // CS = user code 32-bit compat segment
        "push r15",        // EIP = program entry point (32-bit)
        // Clear all GPRs to prevent kernel address leaks to user mode
        "xor eax, eax",
        "xor ebx, ebx",
        "xor ecx, ecx",
        "xor edx, edx",
        "xor esi, esi",
        "xor edi, edi",
        "xor ebp, ebp",
        "xor r8d, r8d",
        "xor r9d, r9d",
        "xor r10d, r10d",
        "xor r11d, r11d",
        "xor r12d, r12d",
        "xor r13d, r13d",
        "xor r14d, r14d",
        "xor r15d, r15d",
        "iretq",           // Enter Ring 3 in compatibility mode!
        in("r14") user_stack,
        in("r15") entry,
        options(noreturn)
    );
}
