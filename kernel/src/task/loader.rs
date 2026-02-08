use crate::memory::address::VirtAddr;
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::sync::spinlock::Spinlock;

/// User programs are loaded at this virtual address (128 MiB).
/// Each process has its own page directory, so multiple programs
/// can coexist at the same virtual address in different address spaces.
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

/// Pending user program info per TID, consumed by the trampoline.
/// Uses a fixed-size array to avoid heap allocation inside Spinlock.
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

/// Load a flat binary from the filesystem and run it in Ring 3.
/// Creates a per-process page directory with isolated user-space mappings.
/// Returns the TID of the spawned thread.
pub fn load_and_run(path: &str, name: &str) -> Result<u32, &'static str> {
    load_and_run_with_args(path, name, "")
}

/// Load a flat binary and run it with command-line arguments.
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

    // Allocate and map code pages in the user's page directory
    let code_pages = (data.len() as u32 + PAGE_SIZE - 1) / PAGE_SIZE;
    for i in 0..code_pages {
        let virt = VirtAddr::new(PROGRAM_LOAD_ADDR + i * PAGE_SIZE);
        let phys = physical::alloc_frame()
            .ok_or("Failed to allocate frame for program code")?;
        virtual_mem::map_page_in_pd(pd_phys, virt, phys, PAGE_WRITABLE | PAGE_USER);
    }

    // Allocate and map stack pages in the user's page directory
    let stack_bottom = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE;
    for i in 0..USER_STACK_PAGES {
        let virt = VirtAddr::new(stack_bottom + i * PAGE_SIZE);
        let phys = physical::alloc_frame()
            .ok_or("Failed to allocate frame for user stack")?;
        virtual_mem::map_page_in_pd(pd_phys, virt, phys, PAGE_WRITABLE | PAGE_USER);
    }

    // Map all loaded DLLs into the new process page directory
    crate::task::dll::map_all_dlls_into(pd_phys);

    // Temporarily switch to user PD to copy program data and zero stack.
    // This works because the user PD has all kernel mappings cloned,
    // so kernel heap (where `data` lives) is still accessible.
    unsafe {
        let old_cr3 = virtual_mem::current_cr3();
        core::arch::asm!("mov cr3, {}", in(reg) pd_phys.as_u32());

        // Copy program binary to load address
        let dest = PROGRAM_LOAD_ADDR as *mut u8;
        core::ptr::copy_nonoverlapping(data.as_ptr(), dest, data.len());
        // Zero-fill remainder of last page
        let remainder = (code_pages * PAGE_SIZE) as usize - data.len();
        if remainder > 0 {
            core::ptr::write_bytes(dest.add(data.len()), 0, remainder);
        }

        // Zero the user stack
        core::ptr::write_bytes(stack_bottom as *mut u8, 0, (USER_STACK_PAGES * PAGE_SIZE) as usize);

        // Switch back to kernel PD
        core::arch::asm!("mov cr3, {}", in(reg) old_cr3);
    }

    let brk = PROGRAM_LOAD_ADDR + code_pages * PAGE_SIZE;

    crate::serial_println!(
        "  PD={:#010x}, {} code pages at {:#010x}, {} stack pages at {:#010x}-{:#010x}, brk={:#010x}",
        pd_phys.as_u32(), code_pages, PROGRAM_LOAD_ADDR,
        USER_STACK_PAGES, stack_bottom, USER_STACK_TOP, brk
    );

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
        slot.entry = PROGRAM_LOAD_ADDR;
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
