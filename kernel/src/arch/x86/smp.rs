/// SMP (Symmetric Multi-Processing) — AP startup and per-CPU management.
///
/// Starts Application Processors (APs) using the INIT-SIPI-SIPI sequence.
/// Each AP gets its own stack, GDT, TSS, and enters the scheduler loop.

use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};
use crate::arch::x86::acpi::ProcessorInfo;

/// Maximum number of CPUs supported
pub const MAX_CPUS: usize = 16;

/// Per-CPU metadata: CPU index, LAPIC ID, BSP flag, and initialization state.
#[repr(C)]
pub struct PerCpu {
    /// Logical CPU index (0 = BSP, 1+ = APs).
    pub cpu_id: u8,
    /// Hardware LAPIC ID for this CPU.
    pub lapic_id: u8,
    /// `true` for the Bootstrap Processor.
    pub is_bsp: bool,
    /// `true` once this CPU has completed initialization.
    pub initialized: bool,
}

/// Global SMP state
static mut CPU_DATA: [PerCpu; MAX_CPUS] = {
    const INIT: PerCpu = PerCpu {
        cpu_id: 0,
        lapic_id: 0,
        is_bsp: false,
        initialized: false,
    };
    [INIT; MAX_CPUS]
};

static CPU_COUNT: AtomicU8 = AtomicU8::new(0);
static AP_STARTED: AtomicU32 = AtomicU32::new(0);
static BSP_LAPIC_ID: AtomicU8 = AtomicU8::new(0);

/// Physical address of the AP trampoline code (must be < 1MB, page-aligned)
const AP_TRAMPOLINE_PHYS: u32 = 0x8000;

/// Communication area between BSP and AP (below trampoline)
/// Layout at 0x7F00 (64-bit):
///   0x7F00: u64 — stack pointer for the AP (virtual address)
///   0x7F08: u64 — CR3 (PML4 physical address)
///   0x7F10: u64 — entry point (Rust function pointer, virtual address)
///   0x7F18: u8  — AP ready flag (set by AP when initialized)
///   0x7F1C: u32 — cpu_id assigned to the AP
const AP_COMM_BASE: u64 = 0x7F00;
const AP_COMM_STACK: u64 = AP_COMM_BASE;
const AP_COMM_CR3: u64   = AP_COMM_BASE + 8;
const AP_COMM_ENTRY: u64 = AP_COMM_BASE + 16;
const AP_COMM_READY: u64 = AP_COMM_BASE + 24;
const AP_COMM_CPUID: u64 = AP_COMM_BASE + 28;

/// Initialize BSP's per-CPU data.
pub fn init_bsp() {
    let bsp_id = crate::arch::x86::apic::lapic_id();
    BSP_LAPIC_ID.store(bsp_id, Ordering::SeqCst);

    unsafe {
        CPU_DATA[0] = PerCpu {
            cpu_id: 0,
            lapic_id: bsp_id,
            is_bsp: true,
            initialized: true,
        };
    }
    CPU_COUNT.store(1, Ordering::SeqCst);
}

/// Start all Application Processors.
pub fn start_aps(processors: &[ProcessorInfo]) {
    let bsp_id = BSP_LAPIC_ID.load(Ordering::SeqCst);

    // Copy AP trampoline to physical address 0x8000
    install_trampoline();

    let cr3 = crate::memory::virtual_mem::kernel_cr3();

    let mut cpu_id: u8 = 1; // BSP is 0

    for proc_info in processors {
        if !proc_info.enabled {
            continue;
        }
        if proc_info.apic_id == bsp_id {
            continue; // Skip BSP
        }

        crate::serial_println!("  SMP: Starting AP (APIC_ID={})...", proc_info.apic_id);

        // Allocate stack for this AP (16 KiB) — returns virtual stack top
        let stack_top = alloc_ap_stack_top();
        if stack_top == 0 {
            crate::serial_println!("  SMP: Failed to allocate AP stack");
            continue;
        }

        // Set up communication area (64-bit values)
        unsafe {
            core::ptr::write_volatile(AP_COMM_STACK as *mut u64, stack_top);
            core::ptr::write_volatile(AP_COMM_CR3 as *mut u64, cr3);
            core::ptr::write_volatile(AP_COMM_ENTRY as *mut u64, ap_entry as u64);
            core::ptr::write_volatile(AP_COMM_READY as *mut u8, 0);
            core::ptr::write_volatile(AP_COMM_CPUID as *mut u32, cpu_id as u32);
        }

        crate::serial_println!("  SMP: stack_top={:#018x}, CR3={:#018x}", stack_top, cr3);

        // Send INIT IPI
        crate::arch::x86::apic::send_init(proc_info.apic_id);

        // Wait 10ms
        delay_ms(10);

        // Send SIPI (twice, as per Intel spec)
        let vector_page = (AP_TRAMPOLINE_PHYS >> 12) as u8;
        crate::serial_println!("  SMP: Sending SIPI (vector_page={:#x})", vector_page);
        crate::arch::x86::apic::send_sipi(proc_info.apic_id, vector_page);
        delay_ms(1);
        crate::arch::x86::apic::send_sipi(proc_info.apic_id, vector_page);

        // Wait for AP to signal ready (up to 500ms)
        let start = crate::arch::x86::pit::get_ticks();
        let ready = loop {
            let flag = unsafe { core::ptr::read_volatile(AP_COMM_READY as *const u8) };
            if flag != 0 { break true; }
            let elapsed = crate::arch::x86::pit::get_ticks().wrapping_sub(start);
            if elapsed > 50 {
                crate::serial_println!("  SMP: Timeout after {} ticks waiting for AP", elapsed);
                break false;
            }
            core::hint::spin_loop();
        };

        if ready {
            // CPU_DATA[cpu_id] was already written by the AP itself in ap_entry()
            // (before signaling ready and enabling interrupts). No redundant BSP
            // write here — doing so would race with the AP's LAPIC timer which
            // may already be calling current_cpu_id() → reading CPU_DATA.
            AP_STARTED.fetch_add(1, Ordering::SeqCst);
            CPU_COUNT.store(cpu_id + 1, Ordering::SeqCst);
            crate::serial_println!("  SMP: AP (APIC_ID={}) started as CPU#{}", proc_info.apic_id, cpu_id);
            cpu_id += 1;
        } else {
            crate::serial_println!("  SMP: AP (APIC_ID={}) failed to start", proc_info.apic_id);
        }
    }

    crate::serial_println!("  SMP: {} CPU(s) online ({} APs)",
        cpu_count(), AP_STARTED.load(Ordering::SeqCst));
}

/// AP entry point — called by trampoline after switching to long mode.
/// Runs on the AP's own stack. Must never return.
extern "C" fn ap_entry() -> ! {
    // Read CPU ID first (trampoline wrote it before jumping here)
    let cpu_id = unsafe { core::ptr::read_volatile(AP_COMM_CPUID as *const u32) } as usize;
    crate::debug_println!("  [SMP] AP#{}: ap_entry start", cpu_id);

    // Load the kernel's GDT (replace trampoline's minimal GDT)
    crate::arch::x86::gdt::reload();
    crate::debug_println!("  [SMP] AP#{}: GDT reloaded", cpu_id);

    // Load the kernel's IDT (AP starts with no valid IDT)
    crate::arch::x86::idt::reload();
    crate::debug_println!("  [SMP] AP#{}: IDT reloaded", cpu_id);

    // Program PAT MSR (must match BSP — all CPUs need identical PAT config)
    crate::arch::x86::pat::init();
    crate::debug_println!("  [SMP] AP#{}: PAT initialized", cpu_id);

    // Initialize per-CPU TSS (each AP gets its own TSS for correct RSP0)
    crate::arch::x86::tss::init_for_cpu(cpu_id);
    crate::debug_println!("  [SMP] AP#{}: TSS initialized", cpu_id);

    // Initialize this AP's LAPIC (starts periodic timer for scheduling)
    crate::arch::x86::apic::init_ap();
    crate::debug_println!("  [SMP] AP#{}: LAPIC initialized", cpu_id);

    crate::serial_println!("  SMP: AP#{} entry point reached, LAPIC+TSS initialized", cpu_id);

    // Configure SYSCALL/SYSRET MSRs for this AP
    crate::arch::x86::syscall_msr::init_ap(cpu_id);
    crate::debug_println!("  [SMP] AP#{}: SYSCALL MSRs configured", cpu_id);

    // Enable SMEP on this AP (CPUID already detected by BSP; features() is global)
    crate::arch::x86::cpuid::enable_smep();

    // Initialize per-AP power management (HWP / P-state)
    crate::arch::x86::power::init_ap();

    // Register ourselves in CPU_DATA BEFORE signaling ready and enabling
    // interrupts.  This prevents a race where the LAPIC timer fires and
    // schedule_inner → current_cpu_id() can't find our LAPIC ID in
    // CPU_DATA (BSP hasn't written it yet), causing the fallback to
    // return 0 and making us act as CPU 0 (wrong per-CPU data, TSS, etc.).
    let lapic_id = crate::arch::x86::apic::lapic_id();
    crate::debug_println!("  [SMP] AP#{}: registering in CPU_DATA (lapic_id={})", cpu_id, lapic_id);
    unsafe {
        CPU_DATA[cpu_id] = PerCpu {
            cpu_id: cpu_id as u8,
            lapic_id,
            is_bsp: false,
            initialized: true,
        };
    }
    crate::debug_println!("  [SMP] AP#{}: CPU_DATA set", cpu_id);

    // Register this CPU's idle thread in the scheduler
    crate::debug_println!("  [SMP] AP#{}: calling register_ap_idle", cpu_id);
    crate::task::scheduler::register_ap_idle(cpu_id);
    crate::debug_println!("  [SMP] AP#{}: register_ap_idle done", cpu_id);

    // Signal BSP that we're ready
    crate::debug_println!("  [SMP] AP#{}: signaling BSP ready", cpu_id);
    unsafe {
        core::ptr::write_volatile(AP_COMM_READY as *mut u8, 1);
    }

    // Switch from the small 16 KiB AP boot stack to the idle thread's
    // 512 KiB kernel stack. All interrupt handling (scheduler, serial output,
    // panic handler) runs on this stack, so it needs adequate headroom.
    // The 16 KiB boot stack is sufficient for init but marginal for nested
    // interrupt + scheduler + serial formatting under load.
    let idle_kstack = crate::task::scheduler::idle_stack_top(cpu_id);
    if idle_kstack >= 0xFFFF_FFFF_8000_0000 {
        crate::debug_println!("  [SMP] AP#{}: switching to idle kstack {:#018x}", cpu_id, idle_kstack);
        unsafe { core::arch::asm!("mov rsp, {}", in(reg) idle_kstack); }
    }

    // Enter idle loop — the LAPIC timer will trigger scheduling
    crate::debug_println!("  [SMP] AP#{}: entering idle loop (sti + hlt)", cpu_id);
    unsafe { core::arch::asm!("sti"); }
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

/// Copy the AP trampoline code to physical address 0x8000.
fn install_trampoline() {
    // Include the pre-assembled trampoline binary (NASM flat binary)
    let trampoline: &[u8] = include_bytes!(env!("ANYOS_AP_TRAMPOLINE"));

    crate::serial_println!("  SMP: Trampoline size = {} bytes", trampoline.len());

    // Copy to physical address 0x8000 (identity-mapped)
    let dest = AP_TRAMPOLINE_PHYS as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(trampoline.as_ptr(), dest, trampoline.len());
    }
}

/// Allocate a 16 KiB stack for an AP. Returns the virtual address of the stack TOP.
/// Uses the kernel heap so the returned address is a valid higher-half virtual address,
/// accessible in any context that uses the kernel page directory.
fn alloc_ap_stack_top() -> u64 {
    let stack = alloc::vec![0u8; 16 * 1024];
    let top = stack.as_ptr() as u64 + stack.len() as u64;
    core::mem::forget(stack); // intentional leak — AP stack lives forever
    top
}

/// Get the number of online CPUs.
pub fn cpu_count() -> u8 {
    CPU_COUNT.load(Ordering::SeqCst)
}

/// Get the current CPU's index (0 = BSP).
///
/// Scans ALL `MAX_CPUS` entries (not just `cpu_count()`) because an AP may
/// have registered itself in `CPU_DATA` before the BSP incremented the count.
pub fn current_cpu_id() -> u8 {
    if !crate::arch::x86::apic::is_initialized() {
        return 0; // Before APIC init, always BSP
    }
    let lapic_id = crate::arch::x86::apic::lapic_id();
    for i in 0..MAX_CPUS {
        if unsafe { CPU_DATA[i].initialized && CPU_DATA[i].lapic_id == lapic_id } {
            return i as u8;
        }
    }
    0 // fallback — should only happen on BSP before init_bsp
}

/// Check if the current CPU is the BSP.
pub fn is_bsp() -> bool {
    current_cpu_id() == 0
}

/// Virtual address to invalidate in the TLB shootdown IPI handler.
/// `u64::MAX` means "full TLB flush" (invpcid/CR3 reload).
static TLB_FLUSH_VA: AtomicU64 = AtomicU64::new(u64::MAX);

/// Number of CPUs that still need to acknowledge the TLB shootdown.
static TLB_ACK_COUNT: AtomicU32 = AtomicU32::new(0);

/// Register the TLB shootdown IPI handler (IRQ 20 = INT 52).
/// Must be called after IDT is initialized (same time as halt IPI).
pub fn register_tlb_shootdown_ipi() {
    crate::arch::x86::irq::register_irq(20, tlb_shootdown_ipi_handler);
}

/// IRQ 20 handler: invalidate the TLB entry for `TLB_FLUSH_VA` and acknowledge.
fn tlb_shootdown_ipi_handler(_irq: u8) {
    let va = TLB_FLUSH_VA.load(Ordering::Acquire);
    unsafe {
        if va == u64::MAX {
            // Full TLB flush via CR3 reload (flushes all non-global entries)
            let cr3: u64;
            core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
            core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack, nomem));
        } else {
            core::arch::asm!("invlpg [{}]", in(reg) va, options(nostack, preserves_flags));
        }
    }
    TLB_ACK_COUNT.fetch_sub(1, Ordering::Release);
}

/// Send a TLB shootdown IPI to all other online CPUs and wait for acknowledgment.
///
/// `va` is the virtual address to invalidate.  Pass `u64::MAX` to request a
/// full TLB flush on each remote CPU.  The caller must have already performed
/// its own `invlpg` (or CR3 reload) for the same address.
///
/// **Must not be called with IF=0 when other CPUs also have IF=0**, as the
/// IPI delivery is held pending until the receiver enables interrupts — which
/// would cause a deadlock.  All callers in this kernel (Thread::new/drop,
/// unmap_page) run with IF=1 in kernel thread context.
pub fn tlb_shootdown(va: u64) {
    if !crate::arch::x86::apic::is_initialized() {
        return; // Single-CPU or APIC not yet up
    }
    let count = cpu_count() as u32;
    if count <= 1 {
        return; // Nothing to shoot down
    }

    let my_cpu = current_cpu_id();
    let others = count - 1;

    TLB_FLUSH_VA.store(va, Ordering::Release);
    TLB_ACK_COUNT.store(others, Ordering::Release);

    // Ensure the stores are visible to other CPUs before IPIs arrive
    core::sync::atomic::fence(Ordering::SeqCst);

    // Send IPI to every other online CPU
    for i in 0..count as usize {
        if i as u8 == my_cpu {
            continue;
        }
        let lapic_id = unsafe { CPU_DATA[i].lapic_id };
        crate::arch::x86::apic::send_ipi(lapic_id, crate::arch::x86::apic::VECTOR_IPI_TLB);
    }

    // Spin until all remote CPUs have acknowledged
    while TLB_ACK_COUNT.load(Ordering::Acquire) > 0 {
        core::hint::spin_loop();
    }
}

/// Register the halt IPI handler (IRQ 21 = INT 53).
/// Must be called after IDT is initialized.
pub fn register_halt_ipi() {
    crate::arch::x86::irq::register_irq(21, halt_ipi_handler);
}

/// IRQ 21 handler: halt this CPU permanently.
/// Triggered by `halt_other_cpus()` via IPI during panic/fatal exception.
fn halt_ipi_handler(_irq: u8) {
    unsafe { core::arch::asm!("cli"); }
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

/// Halt all other CPUs by sending a halt IPI to each one.
/// Used during panic/fatal exception to prevent cascading crashes
/// and serial output interleaving.
pub fn halt_other_cpus() {
    if !crate::arch::x86::apic::is_initialized() {
        return; // Single CPU or APIC not ready
    }

    let my_cpu = current_cpu_id();
    let count = cpu_count();

    for i in 0..count as usize {
        if i as u8 == my_cpu {
            continue;
        }
        let lapic_id = unsafe { CPU_DATA[i].lapic_id };
        crate::arch::x86::apic::send_ipi(lapic_id, crate::arch::x86::apic::VECTOR_IPI_HALT);
    }
}

fn delay_ms(ms: u32) {
    let pit_hz = crate::arch::x86::pit::TICK_HZ;
    let ms_per_tick = 1000 / pit_hz;
    let ticks = ms / ms_per_tick;
    let ticks = if ticks == 0 { 1 } else { ticks };
    let start = crate::arch::x86::pit::get_ticks();
    while crate::arch::x86::pit::get_ticks().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}
