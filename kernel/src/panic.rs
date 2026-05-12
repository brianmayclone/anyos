//! Kernel panic and allocation error handlers.
//!
//! Displays panic information on serial, framebuffer, and VGA text outputs,
//! then halts the CPU.  Dumps CPU/thread/memory diagnostics to serial before
//! halting so that panics are diagnosable from serial logs alone.

use core::panic::PanicInfo;

fn last_syscall_diag(cpu: usize) -> (u32, &'static str, &'static str) {
    let num = crate::task::scheduler::get_last_syscall(cpu);
    match crate::task::scheduler::get_last_syscall_abi(cpu) {
        1 => (num, "linux", crate::syscall::linux::syscall_name(num)),
        _ => (num, "anyos", crate::syscall::table::syscall_name(num)),
    }
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Enter panic mode: halt other CPUs, switch to blocking serial,
    // force-release serial lock to prevent deadlock.
    crate::drivers::serial::enter_panic_mode();

    crate::serial_println!("=== KERNEL PANIC ===");
    crate::serial_println!("{}", info);

    // ---- CPU & thread context ----
    let cpu = crate::arch::hal::cpu_id();
    let tid = crate::task::scheduler::debug_current_tid();
    let is_user = crate::task::scheduler::debug_is_current_user();
    let (last_sc, sc_abi, sc_name) = last_syscall_diag(cpu);
    let name_buf = crate::task::scheduler::current_thread_name();
    let name_len = name_buf
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(name_buf.len());
    let name = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("<invalid>");

    crate::serial_println!("--- CPU & Thread ---");
    crate::serial_println!("  CPU:    {}", cpu);
    crate::serial_println!("  TID:    {} \"{}\" (user={})", tid, name, is_user);
    crate::serial_println!("  LastSC: {}:{} ({})", sc_abi, last_sc, sc_name);

    // ---- Register snapshot (x86-64) ----
    #[cfg(target_arch = "x86_64")]
    {
        let rsp: u64;
        let rbp: u64;
        let rip: u64;
        let cr3: u64;
        let rflags: u64;
        unsafe {
            core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nomem, nostack));
            core::arch::asm!("mov {}, rbp", out(reg) rbp, options(nomem, nostack));
            core::arch::asm!("lea {}, [rip]", out(reg) rip, options(nomem, nostack));
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack));
            core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem));
        }
        crate::serial_println!("--- Registers ---");
        crate::serial_println!("  RIP:    {:#018x}", rip);
        crate::serial_println!("  RSP:    {:#018x}", rsp);
        crate::serial_println!("  RBP:    {:#018x}", rbp);
        crate::serial_println!("  CR3:    {:#018x}", cr3);
        crate::serial_println!("  RFLAGS: {:#018x}", rflags);
    }

    // ---- Register snapshot (aarch64) ----
    #[cfg(target_arch = "aarch64")]
    {
        let sp: u64;
        let fp: u64; // x29
        let lr: u64; // x30
        let ttbr1: u64;
        unsafe {
            core::arch::asm!("mov {}, sp", out(reg) sp, options(nomem, nostack));
            core::arch::asm!("mov {}, x29", out(reg) fp, options(nomem, nostack));
            core::arch::asm!("mov {}, x30", out(reg) lr, options(nomem, nostack));
            core::arch::asm!("mrs {}, ttbr1_el1", out(reg) ttbr1, options(nomem, nostack));
        }
        crate::serial_println!("--- Registers ---");
        crate::serial_println!("  SP:     {:#018x}", sp);
        crate::serial_println!("  FP:     {:#018x}", fp);
        crate::serial_println!("  LR:     {:#018x}", lr);
        crate::serial_println!("  TTBR1:  {:#018x}", ttbr1);
    }

    // ---- Physical memory ----
    // The physical allocator lock may be held — check before trying to acquire.
    crate::serial_println!("--- Physical Memory ---");
    if crate::memory::physical::is_allocator_locked() {
        crate::serial_println!("  (allocator lock held — skipping)");
    } else {
        let free = crate::memory::physical::free_frame_count();
        let total = crate::memory::physical::total_frames();
        let free_mib = free * crate::memory::FRAME_SIZE / (1024 * 1024);
        let total_mib = total * crate::memory::FRAME_SIZE / (1024 * 1024);
        crate::serial_println!(
            "  Frames: {}/{} free ({}/{} MiB)",
            free,
            total,
            free_mib,
            total_mib
        );
    }

    // ---- Heap ----
    // The heap lock may be held — check before trying to acquire.
    crate::serial_println!("--- Heap ---");
    if crate::memory::heap::is_heap_locked() {
        // At least show the committed watermark (lock-free atomic read).
        let committed =
            crate::memory::heap::HEAP_COMMITTED.load(core::sync::atomic::Ordering::Relaxed);
        crate::serial_println!("  (heap lock held — skipping free-list walk)");
        crate::serial_println!("  Committed: {} KiB", committed / 1024);
    } else {
        let (used, committed) = crate::memory::heap::heap_stats();
        crate::serial_println!(
            "  Used:      {} KiB / {} KiB committed",
            used / 1024,
            committed / 1024
        );
        crate::serial_println!("  Free:      {} KiB", (committed - used) / 1024);
    }

    crate::serial_println!("=== END PANIC ===");

    // Show Red Screen of Death on the framebuffer (x86-only)
    #[cfg(target_arch = "x86_64")]
    crate::drivers::rsod::show_panic(&format_args!("{}", info));

    loop {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            core::arch::asm!("cli; hlt");
        }
        #[cfg(target_arch = "aarch64")]
        unsafe {
            core::arch::asm!("wfi");
        }
    }
}

#[alloc_error_handler]
fn alloc_error(layout: core::alloc::Layout) -> ! {
    // Enter panic mode early so we can print diagnostics even if serial lock
    // is held by the thread that triggered this allocation failure.
    crate::drivers::serial::enter_panic_mode();

    crate::serial_println!("=== ALLOCATION FAILURE ===");
    crate::serial_println!(
        "  Size:  {} bytes (align {})",
        layout.size(),
        layout.align()
    );

    // Show heap stats (lock-free path if heap lock is held — we may be inside
    // the allocator when this fires).
    if crate::memory::heap::is_heap_locked() {
        let committed =
            crate::memory::heap::HEAP_COMMITTED.load(core::sync::atomic::Ordering::Relaxed);
        crate::serial_println!("  Heap:  (lock held) committed={} KiB", committed / 1024);
    } else {
        let (used, committed) = crate::memory::heap::heap_stats();
        crate::serial_println!("  Heap:  {}/{} KiB used", used / 1024, committed / 1024);
    }

    // Show physical memory (same lock-safety check).
    if !crate::memory::physical::is_allocator_locked() {
        let free = crate::memory::physical::free_frame_count();
        crate::serial_println!(
            "  Phys:  {} frames free ({} MiB)",
            free,
            free * crate::memory::FRAME_SIZE / (1024 * 1024)
        );
    }

    panic!(
        "Heap allocation failed: size={} align={}",
        layout.size(),
        layout.align()
    );
}
