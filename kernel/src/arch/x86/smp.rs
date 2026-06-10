use crate::arch::x86::acpi::ProcessorInfo;
/// SMP (Symmetric Multi-Processing) — AP startup and per-CPU management.
///
/// Starts Application Processors (APs) using the INIT-SIPI-SIPI sequence.
/// Each AP gets its own stack, GDT, TSS, and enters the scheduler loop.
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};

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

/// Fast LAPIC-ID → CPU-index lookup table (populated during init_bsp / ap_entry).
/// Index = LAPIC ID (max 255), value = cpu_id (0xFF = unmapped).
static mut LAPIC_TO_CPU: [u8; 256] = [0xFF; 256];

/// IA32_TSC_AUX MSR number — written with cpu_id for RDPID.
const IA32_TSC_AUX: u32 = 0xC0000103;

/// Whether RDPID fast path is available.
static HAS_RDPID: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);

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
const AP_COMM_CR3: u64 = AP_COMM_BASE + 8;
const AP_COMM_ENTRY: u64 = AP_COMM_BASE + 16;
const AP_COMM_READY: u64 = AP_COMM_BASE + 24;
const AP_COMM_CPUID: u64 = AP_COMM_BASE + 28;

/// Temporary per-AP boot stack used until `ap_entry()` switches to the
/// scheduler-owned idle thread stack.
///
/// AP initialization runs a fair amount of Rust code before that switch:
/// GDT/IDT/TSS/LAPIC setup, feature init, serial/debug formatting, and finally
/// `register_ap_idle()`. The previous 16 KiB stack was too close to the edge
/// and could underflow into nearby heap allocations during verbose boot paths.
const AP_BOOT_STACK_SIZE: usize = 256 * 1024;

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
        LAPIC_TO_CPU[bsp_id as usize] = 0;
    }
    CPU_COUNT.store(1, Ordering::SeqCst);

    // Enable RDPID fast path: write cpu_id into IA32_TSC_AUX MSR
    let has_rdpid = crate::arch::x86::cpuid::features().rdpid;
    HAS_RDPID.store(has_rdpid, Ordering::Release);
    if has_rdpid {
        unsafe {
            write_tsc_aux(0);
        }
        crate::serial_verbose_println!("  SMP: RDPID available — fast cpu_id path enabled");
    }
}

/// Hardware LAPIC ID of the bootstrap processor.
pub fn bsp_lapic_id() -> u8 {
    BSP_LAPIC_ID.load(Ordering::SeqCst)
}

/// Write `cpu_id` into IA32_TSC_AUX so RDPID returns it directly.
unsafe fn write_tsc_aux(cpu_id: u32) {
    core::arch::asm!(
        "wrmsr",
        in("ecx") IA32_TSC_AUX,
        in("eax") cpu_id,
        in("edx") 0u32,
        options(nostack, preserves_flags),
    );
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

        crate::serial_verbose_println!("  SMP: Starting AP (APIC_ID={})...", proc_info.apic_id);

        // Allocate the temporary AP boot stack — returns virtual stack top.
        let stack_top = alloc_ap_stack_top();
        if stack_top == 0 {
            crate::serial_verbose_println!("  SMP: Failed to allocate AP stack");
            continue;
        }

        // Set up communication area (64-bit values).
        // Write all data fields first, then issue a store fence to ensure
        // they are globally visible before the AP can observe them after SIPI.
        unsafe {
            core::ptr::write_volatile(AP_COMM_STACK as *mut u64, stack_top);
            core::ptr::write_volatile(AP_COMM_CR3 as *mut u64, cr3);
            core::ptr::write_volatile(AP_COMM_ENTRY as *mut u64, ap_entry as u64);
            core::ptr::write_volatile(AP_COMM_CPUID as *mut u32, cpu_id as u32);
            // Release fence: guarantee all data writes above are globally visible
            // before the ready flag is cleared and SIPI is sent.  The AP reads
            // these fields after starting — without this barrier the compiler
            // (or a weakly-ordered future target) could reorder the stores.
            core::sync::atomic::fence(Ordering::SeqCst);
            core::ptr::write_volatile(AP_COMM_READY as *mut u8, 0);
        }

        crate::serial_verbose_println!(
            "  SMP: stack=[{:#018x}..{:#018x}] ({} KiB), CR3={:#018x}",
            stack_top - AP_BOOT_STACK_SIZE as u64,
            stack_top,
            AP_BOOT_STACK_SIZE / 1024,
            cr3
        );

        // Send INIT IPI
        crate::arch::x86::apic::send_init(proc_info.apic_id);

        // Wait 10ms
        delay_ms(10);

        // Send SIPI (twice, as per Intel spec)
        let vector_page = (AP_TRAMPOLINE_PHYS >> 12) as u8;
        crate::serial_verbose_println!("  SMP: Sending SIPI (vector_page={:#x})", vector_page);
        crate::arch::x86::apic::send_sipi(proc_info.apic_id, vector_page);
        delay_ms(1);
        crate::arch::x86::apic::send_sipi(proc_info.apic_id, vector_page);

        // Wait for AP to signal ready (up to 500ms)
        let start = crate::arch::x86::pit::get_ticks();
        let ready = loop {
            let flag = unsafe { core::ptr::read_volatile(AP_COMM_READY as *const u8) };
            if flag != 0 {
                break true;
            }
            let elapsed = crate::arch::x86::pit::get_ticks().wrapping_sub(start);
            if elapsed > 500 {
                crate::serial_verbose_println!(
                    "  SMP: Timeout after {} ticks waiting for AP",
                    elapsed
                );
                break false;
            }
            core::hint::spin_loop();
        };

        if ready {
            // Acquire fence: ensure all AP initialization writes (CPU_DATA,
            // LAPIC_TO_CPU, TSS, idle thread) are visible to the BSP now that
            // we observed the ready flag.  Pairs with the SeqCst fence the AP
            // issues in ap_entry() just before writing AP_COMM_READY = 1.
            core::sync::atomic::fence(Ordering::SeqCst);
            // CPU_DATA[cpu_id] was already written by the AP itself in ap_entry()
            // (before signaling ready and enabling interrupts). No redundant BSP
            // write here — doing so would race with the AP's LAPIC timer which
            // may already be calling current_cpu_id() → reading CPU_DATA.
            AP_STARTED.fetch_add(1, Ordering::SeqCst);
            CPU_COUNT.store(cpu_id + 1, Ordering::SeqCst);
            crate::serial_verbose_println!(
                "  SMP: AP (APIC_ID={}) started as CPU#{}",
                proc_info.apic_id,
                cpu_id
            );
            cpu_id += 1;
        } else {
            crate::serial_verbose_println!(
                "  SMP: AP (APIC_ID={}) failed to start",
                proc_info.apic_id
            );
        }
    }

    crate::serial_verbose_println!(
        "  SMP: {} CPU(s) online ({} APs)",
        cpu_count(),
        AP_STARTED.load(Ordering::SeqCst)
    );
}

/// AP entry point — called by trampoline after switching to long mode.
/// Runs on the AP's own stack. Must never return.
extern "C" fn ap_entry() -> ! {
    // Acquire fence: ensure all communication-area reads below see the
    // data the BSP wrote before sending SIPI (pairs with the SeqCst fence
    // in start_aps() on the BSP side).
    core::sync::atomic::fence(Ordering::SeqCst);

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

    crate::serial_verbose_println!(
        "  SMP: AP#{} entry point reached, LAPIC+TSS initialized",
        cpu_id
    );

    // Configure SYSCALL/SYSRET MSRs for this AP
    crate::arch::x86::syscall_msr::init_ap(cpu_id);
    crate::debug_println!("  [SMP] AP#{}: SYSCALL MSRs configured", cpu_id);

    // Enable SMEP on this AP (CPUID already detected by BSP; features() is global)
    crate::arch::x86::cpuid::enable_smep();
    crate::arch::x86::cpuid::enable_write_protect();
    // TODO: enable_xsave() disabled — see main.rs
    // crate::arch::x86::cpuid::enable_xsave();

    // Enable PCID on this AP (CR4.PCIDE is per-CPU, BSP already set the global state).
    // Without this, context_switch's `mov cr3, rax` with PCID bits would #GP.
    crate::memory::virtual_mem::enable_pcid();

    // Initialize per-AP power management (HWP / P-state)
    crate::arch::x86::power::init_ap();

    // Enable VMX/SVM on this AP core (global init already ran on BSP)
    crate::arch::x86::virt::per_cpu_init();

    // Register ourselves in CPU_DATA BEFORE signaling ready and enabling
    // interrupts.  This prevents a race where the LAPIC timer fires and
    // schedule_inner → current_cpu_id() can't find our LAPIC ID in
    // CPU_DATA (BSP hasn't written it yet), causing the fallback to
    // return 0 and making us act as CPU 0 (wrong per-CPU data, TSS, etc.).
    let lapic_id = crate::arch::x86::apic::lapic_id();
    crate::debug_println!(
        "  [SMP] AP#{}: registering in CPU_DATA (lapic_id={})",
        cpu_id,
        lapic_id
    );
    unsafe {
        CPU_DATA[cpu_id] = PerCpu {
            cpu_id: cpu_id as u8,
            lapic_id,
            is_bsp: false,
            initialized: true,
        };
        LAPIC_TO_CPU[lapic_id as usize] = cpu_id as u8;
    }
    // Write IA32_TSC_AUX for RDPID on this AP
    if HAS_RDPID.load(Ordering::Acquire) {
        unsafe {
            write_tsc_aux(cpu_id as u32);
        }
    }
    crate::debug_println!("  [SMP] AP#{}: CPU_DATA set", cpu_id);

    // Signal BSP that we're alive BEFORE acquiring the scheduler lock.
    // register_ap_idle() calls SCHEDULER.lock() which can block for 100+ ms
    // under contention, causing the BSP's timeout to expire and declaring
    // this AP "failed to start" — even though it's still running.  By
    // signaling first, the BSP knows we're alive and can proceed.
    // Interrupts are still disabled (no sti yet), so no timer can fire on
    // this AP until after register_ap_idle + stack switch + sti.
    // Release fence: CPU_DATA, LAPIC_TO_CPU, TSS, GDT, IDT must be
    // globally visible before the BSP observes the ready flag.
    crate::debug_println!(
        "  [SMP] AP#{}: signaling BSP ready (before idle registration)",
        cpu_id
    );
    core::sync::atomic::fence(Ordering::SeqCst);
    unsafe {
        core::ptr::write_volatile(AP_COMM_READY as *mut u8, 1);
    }

    // Register this CPU's idle thread in the scheduler.
    // This acquires SCHEDULER.lock() which may block under contention,
    // but the BSP has already been notified and won't time out.
    crate::debug_println!("  [SMP] AP#{}: calling register_ap_idle", cpu_id);
    crate::task::scheduler::register_ap_idle(cpu_id);
    crate::debug_println!("  [SMP] AP#{}: register_ap_idle done", cpu_id);

    // Switch from the temporary AP boot stack to the idle thread's kernel
    // stack. All interrupt handling (scheduler, serial output,
    // panic handler) runs on this stack, so it needs adequate headroom.
    // The boot stack is intentionally larger than the architectural minimum
    // because AP init executes Rust code and verbose formatting before this
    // point, and an underflow here corrupts adjacent heap allocations.
    let idle_kstack = crate::task::scheduler::idle_stack_top(cpu_id);
    if idle_kstack >= 0xFFFF_FFFF_8000_0000 {
        crate::debug_println!(
            "  [SMP] AP#{}: switching to idle kstack {:#018x}",
            cpu_id,
            idle_kstack
        );
        unsafe {
            core::arch::asm!("mov rsp, {}", in(reg) idle_kstack);
        }
    }

    // Enter idle loop — the LAPIC timer will trigger scheduling
    crate::debug_println!("  [SMP] AP#{}: entering idle loop (sti + hlt)", cpu_id);
    unsafe {
        core::arch::asm!("sti");
    }
    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
    }
}

/// Copy the AP trampoline code to physical address 0x8000.
fn install_trampoline() {
    // Include the pre-assembled trampoline binary (NASM flat binary)
    let trampoline: &[u8] = include_bytes!(env!("ANYOS_AP_TRAMPOLINE"));

    crate::serial_verbose_println!("  SMP: Trampoline size = {} bytes", trampoline.len());

    // Copy to physical address 0x8000 (identity-mapped)
    let dest = AP_TRAMPOLINE_PHYS as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(trampoline.as_ptr(), dest, trampoline.len());
    }
}

/// Allocate a temporary stack for an AP. Returns the virtual address of the stack TOP.
/// Uses the kernel heap so the returned address is a valid higher-half virtual address,
/// accessible in any context that uses the kernel page directory.
fn alloc_ap_stack_top() -> u64 {
    let stack = alloc::vec![0u8; AP_BOOT_STACK_SIZE];
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
/// Three tiers, fastest first:
/// 1. **RDPID** (1 instruction) — reads IA32_TSC_AUX, set to cpu_id at boot.
/// 2. **LAPIC + lookup table** (1 MMIO read + 1 array access) — O(1).
/// 3. **Legacy linear scan** — fallback during early boot only.
#[inline]
pub fn current_cpu_id() -> u8 {
    // Fast path: RDPID — single instruction, ~1 cycle
    // Encoded as raw bytes because LLVM's assembler doesn't support rdpid in 64-bit mode.
    // F3 48 0F C7 F8 = RDPID RAX (REX.W + opcode 0F C7 /7, ModRM=F8 → rax)
    if HAS_RDPID.load(Ordering::Relaxed) {
        let id: u64;
        unsafe {
            core::arch::asm!(".byte 0xF3, 0x48, 0x0F, 0xC7, 0xF8", out("rax") id, options(nostack, nomem, preserves_flags));
        }
        if (id as usize) < MAX_CPUS {
            return id as u8;
        }
    }

    if !crate::arch::x86::apic::is_initialized() {
        return 0; // Before APIC init, always BSP
    }

    // Medium path: LAPIC read + O(1) lookup table
    let lapic_id = crate::arch::x86::apic::lapic_id() as usize;
    let cpu = unsafe { LAPIC_TO_CPU[lapic_id] };
    if cpu != 0xFF && (cpu as usize) < MAX_CPUS {
        return cpu;
    }

    // Slow fallback: linear scan (only during early AP init)
    for i in 0..MAX_CPUS {
        if unsafe { CPU_DATA[i].initialized && CPU_DATA[i].lapic_id == lapic_id as u8 } {
            return i as u8;
        }
    }
    0
}

/// Check if the current CPU is the BSP.
pub fn is_bsp() -> bool {
    current_cpu_id() == 0
}

/// Virtual address to invalidate in the TLB shootdown IPI handler.
/// `u64::MAX` means "full TLB flush" (invpcid/CR3 reload).
static TLB_FLUSH_VA: AtomicU64 = AtomicU64::new(u64::MAX);

/// PCID whose translations the shootdown targets. 0 = no specific PCID
/// (legacy full flush). With PCID + no-flush context switches, a process's
/// translations stay cached on every CPU it ever ran on — invalidating only
/// the receiving CPU's *current* PCID misses them. A nonzero value here lets
/// the receiver drop exactly the stale address space via INVPCID
/// single-context instead of nuking its whole TLB.
static TLB_FLUSH_PCID: AtomicU64 = AtomicU64::new(0);

/// Bitmask of logical CPUs that still need to acknowledge the TLB shootdown.
/// A mask (instead of a plain count) lets the timeout path re-send the IPI to
/// exactly the CPUs that have not flushed yet — re-sending to an already-acked
/// CPU with a counter would double-decrement and release the waiter while a
/// straggler still holds a stale translation.
static TLB_ACK_PENDING: AtomicU32 = AtomicU32::new(0);

/// Serializes concurrent TLB shootdown requests.
///
/// Without this lock, two CPUs calling `tlb_shootdown()` simultaneously can
/// corrupt `TLB_ACK_PENDING` (the second `store` overwrites the first), leading
/// to either underflow (wrap to u32::MAX → infinite spin) or missed flushes.
static TLB_SHOOTDOWN_LOCK: AtomicBool = AtomicBool::new(false);

/// After this many spin iterations waiting for remote TLB ACKs, emit a
/// lock-free serial diagnostic and give up instead of freezing the machine.
const TLB_SHOOTDOWN_TIMEOUT_SPINS: u32 = 50_000_000;

#[inline(never)]
fn diag_putc(c: u8) {
    unsafe {
        while crate::arch::x86::port::inb(0x3FD) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        crate::arch::x86::port::outb(0x3F8, c);
    }
}

fn diag_puts(s: &[u8]) {
    for &c in s {
        if c == b'\n' {
            diag_putc(b'\r');
        }
        diag_putc(c);
    }
}

fn diag_hex(mut n: u64) {
    diag_puts(b"0x");
    if n == 0 {
        diag_putc(b'0');
        return;
    }
    let mut buf = [0u8; 16];
    let mut i = 0usize;
    while n > 0 {
        let d = (n & 0xF) as u8;
        buf[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        n >>= 4;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        diag_putc(buf[i]);
    }
}

fn diag_dec(mut n: u32) {
    if n == 0 {
        diag_putc(b'0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0usize;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        diag_putc(buf[i]);
    }
}

/// Register the TLB shootdown IPI handler (IRQ 20 = INT 52).
/// Must be called after IDT is initialized (same time as halt IPI).
pub fn register_tlb_shootdown_ipi() {
    crate::arch::x86::irq::register_irq(20, tlb_shootdown_ipi_handler);
}

/// Per-CPU bitmap of PCIDs whose TLB entries must be flushed before this CPU
/// next switches onto them with the no-flush bit set. 4096 PCIDs / 64 = 64
/// words per CPU.
///
/// This is the asynchronous half of PCID-correct invalidation: a CPU that is
/// NOT currently running the modified address space cannot use its stale
/// entries until a context switch brings the PCID back — so instead of a
/// synchronous all-CPU IPI broadcast (which can wedge behind a CPU spinning
/// with interrupts disabled and freeze every munmap-ing process in the
/// system), the modifier just marks the PCID here and the scheduler flushes
/// it lazily right before the switch.
const PCID_PENDING_WORDS: usize = 4096 / 64;
static PCID_FLUSH_PENDING: [[AtomicU64; PCID_PENDING_WORDS]; MAX_CPUS] = {
    const WORD: AtomicU64 = AtomicU64::new(0);
    const ROW: [AtomicU64; PCID_PENDING_WORDS] = [WORD; PCID_PENDING_WORDS];
    [ROW; MAX_CPUS]
};

#[inline]
fn pcid_bit(pcid: u16) -> (usize, u64) {
    ((pcid as usize / 64) % PCID_PENDING_WORDS, 1u64 << (pcid % 64))
}

/// Mark `pcid` as needing a flush on every other CPU (lock-free, no IPI).
fn pcid_mark_flush_pending_all_cpus(pcid: u16) {
    let (word, bit) = pcid_bit(pcid);
    let me = current_cpu_id() as usize;
    let count = (cpu_count() as usize).min(MAX_CPUS);
    for cpu in 0..count {
        if cpu != me {
            PCID_FLUSH_PENDING[cpu][word].fetch_or(bit, Ordering::SeqCst);
        }
    }
}

/// Consume the pending-flush bit for (`cpu`, `pcid`). Returns true when the
/// caller must flush the PCID before running on it. Called by the scheduler
/// with interrupts disabled immediately before the context switch.
pub fn pcid_take_pending_flush(cpu: usize, page_table: u64) -> bool {
    let pcid = (page_table & 0xFFF) as u16;
    // PCID 0 loads always flush in context_switch (no-flush is never used
    // with the fallback tag), so there is nothing to consume.
    if pcid == 0 || cpu >= MAX_CPUS {
        return false;
    }
    let (word, bit) = pcid_bit(pcid);
    PCID_FLUSH_PENDING[cpu][word].fetch_and(!bit, Ordering::AcqRel) & bit != 0
}

/// Flush all non-global TLB entries of `pcid` on the local CPU.
pub fn pcid_flush_local(page_table: u64) {
    let pcid = (page_table & 0xFFF) as u16;
    unsafe {
        if crate::arch::x86::cpuid::features().invpcid {
            invpcid_single_context(pcid);
        } else {
            // No INVPCID: toggle CR4.PGE — flushes every PCID incl. globals.
            let cr4: u64;
            core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, nomem));
            core::arch::asm!("mov cr4, {}", in(reg) cr4 & !(1u64 << 7), options(nostack, nomem));
            core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nostack, nomem));
        }
    }
}

/// INVPCID type 1 (single-context): drop all non-global TLB entries tagged
/// with `pcid`, regardless of the CPU's current PCID.
#[inline]
unsafe fn invpcid_single_context(pcid: u16) {
    let desc: [u64; 2] = [(pcid & 0xFFF) as u64, 0];
    core::arch::asm!(
        "invpcid {ty}, [{desc}]",
        ty = in(reg) 1u64,
        desc = in(reg) desc.as_ptr(),
        options(nostack)
    );
}

/// IRQ 20 handler: invalidate the TLB entry for `TLB_FLUSH_VA` and acknowledge.
fn tlb_shootdown_ipi_handler(_irq: u8) {
    let va = TLB_FLUSH_VA.load(Ordering::Acquire);
    let pcid = TLB_FLUSH_PCID.load(Ordering::Acquire) as u16;
    unsafe {
        if va == u64::MAX {
            if pcid != 0
                && crate::memory::virtual_mem::pcid_enabled()
                && crate::arch::x86::cpuid::features().invpcid
            {
                // Targeted: drop only the modified address space's entries;
                // every other process keeps its warm TLB. The lazy pending
                // bit set by the sender is satisfied by this flush too.
                invpcid_single_context(pcid);
                let me = current_cpu_id() as usize;
                if me < MAX_CPUS {
                    let (word, bit) = pcid_bit(pcid);
                    PCID_FLUSH_PENDING[me][word].fetch_and(!bit, Ordering::AcqRel);
                }
            } else if crate::memory::virtual_mem::pcid_enabled() {
                // With PCID + no-flush context switches, translations of
                // processes NOT currently running on this CPU stay cached
                // under their PCID tags. A plain CR3 reload only drops the
                // current PCID, so a CoW fork's write-protect would not reach
                // them — the parent could later be rescheduled here with a
                // stale writable entry and corrupt the child. Toggle CR4.PGE:
                // architecturally guaranteed to flush ALL TLB entries for
                // every PCID, including globals.
                let cr4: u64;
                core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, nomem));
                core::arch::asm!("mov cr4, {}", in(reg) cr4 & !(1u64 << 7), options(nostack, nomem));
                core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nostack, nomem));
            } else {
                // Full TLB flush via CR3 reload (flushes all non-global entries)
                let cr3: u64;
                core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
                core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack, nomem));
            }
        } else {
            core::arch::asm!("invlpg [{}]", in(reg) va, options(nostack, preserves_flags));
        }
    }
    let my_bit = 1u32 << current_cpu_id().min(31);
    TLB_ACK_PENDING.fetch_and(!my_bit, Ordering::AcqRel);
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
    let count = cpu_count() as u32;
    let mut mask = 0u32;
    for cpu in 0..count.min(32) {
        mask |= 1u32 << cpu;
    }
    tlb_shootdown_mask(va, mask);
}

/// Invalidate every TLB entry of `pcid` on all CPUs — without a synchronous
/// all-CPU broadcast.
///
/// Under PCID with no-flush context switches a process's translations stay
/// alive on every CPU it ever ran on. Correctness needs all of them gone,
/// but only the CPUs *currently running* the address space can use the stale
/// entries right now — every other CPU must pass through a context switch
/// first. So: mark the PCID flush-pending on all CPUs (lock-free, consumed
/// by the scheduler right before the switch via `pcid_take_pending_flush`),
/// and send the synchronous IPI only to the active CPUs. A synchronous
/// broadcast to ALL CPUs is exactly the "most fragile SMP path" the old
/// munmap code warned about: one CPU spinning with interrupts disabled
/// stalls the shootdown lock and freezes every munmap-ing process behind it.
pub fn tlb_shootdown_pcid(pcid: u16) {
    if pcid == 0 {
        // Fallback tag: switch-in always flushes PCID 0, only currently
        // running CPUs matter.
        let mask = crate::task::scheduler::current_pd_active_cpu_mask();
        tlb_shootdown_mask_pcid(u64::MAX, mask, 0);
        return;
    }
    pcid_mark_flush_pending_all_cpus(pcid);
    core::sync::atomic::fence(Ordering::SeqCst);
    // Mask snapshot AFTER marking: a CPU switching onto this PCID
    // concurrently either shows up in the mask (gets the IPI) or performs
    // its switch after the mark (consumes the pending bit). Either way no
    // CPU runs the address space with stale entries.
    let mask = crate::task::scheduler::current_pd_active_cpu_mask();
    tlb_shootdown_mask_pcid(u64::MAX, mask, pcid);
}

/// Send a TLB shootdown IPI to a selected set of CPUs and wait for
/// acknowledgment. `cpu_mask` uses logical CPU IDs; the current CPU is skipped
/// because callers have already flushed locally.
pub fn tlb_shootdown_mask(va: u64, cpu_mask: u32) {
    tlb_shootdown_mask_pcid(va, cpu_mask, 0);
}

fn tlb_shootdown_mask_pcid(va: u64, cpu_mask: u32, pcid: u16) {
    if !crate::arch::x86::apic::is_initialized() {
        return; // Single-CPU or APIC not yet up
    }
    let count = cpu_count() as u32;
    if count <= 1 {
        return; // Nothing to shoot down
    }

    let my_cpu = current_cpu_id();
    let online_mask = if count >= 32 {
        u32::MAX
    } else {
        (1u32 << count) - 1
    };
    let target_mask = cpu_mask & online_mask & !(1u32 << my_cpu);
    if target_mask == 0 {
        return;
    }

    // Serialize concurrent shootdowns.  Without this, two CPUs can corrupt
    // TLB_ACK_PENDING (store/store race → lost bits → missed flushes).
    while TLB_SHOOTDOWN_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }

    TLB_FLUSH_VA.store(va, Ordering::Release);
    TLB_FLUSH_PCID.store(pcid as u64, Ordering::Release);
    TLB_ACK_PENDING.store(target_mask, Ordering::Release);

    // Ensure the stores are visible to other CPUs before IPIs arrive
    core::sync::atomic::fence(Ordering::SeqCst);

    let send_ipis = |mask: u32| {
        for i in 0..count as usize {
            if (mask & (1u32 << i)) == 0 {
                continue;
            }
            let lapic_id = unsafe { CPU_DATA[i].lapic_id };
            crate::arch::x86::apic::send_ipi(lapic_id, crate::arch::x86::apic::VECTOR_IPI_TLB);
        }
    };
    send_ipis(target_mask);

    // Spin until all remote CPUs have acknowledged. On timeout, re-send the
    // IPI to exactly the CPUs that have not acked (the original IPI may have
    // been lost). Continuing with a stale remote TLB entry is NOT an option:
    // the page may be reused by another process, and the straggler CPU would
    // silently read/write foreign memory. If the stragglers stay silent
    // through all retries, the machine state is no longer trustworthy — halt
    // loudly instead of corrupting memory quietly.
    const TLB_SHOOTDOWN_MAX_RETRIES: u32 = 4;
    let mut retries = 0u32;
    let mut spin_count = 0u32;
    while TLB_ACK_PENDING.load(Ordering::Acquire) != 0 {
        core::hint::spin_loop();
        spin_count = spin_count.saturating_add(1);
        if spin_count >= TLB_SHOOTDOWN_TIMEOUT_SPINS {
            let pending = TLB_ACK_PENDING.load(Ordering::Acquire);
            diag_puts(b"\n!!! TLB SHOOTDOWN TIMEOUT cpu=");
            diag_dec(my_cpu as u32);
            diag_puts(b" pending_mask=");
            diag_hex(pending as u64);
            diag_puts(b" va=");
            diag_hex(va);
            diag_puts(b" retry=");
            diag_dec(retries);
            diag_putc(b'\n');
            retries += 1;
            if retries > TLB_SHOOTDOWN_MAX_RETRIES {
                panic!(
                    "TLB shootdown: CPUs {:#x} unresponsive after {} retries (va={:#x}) — \
                     cannot guarantee TLB coherence, stale translations would corrupt memory",
                    pending, TLB_SHOOTDOWN_MAX_RETRIES, va
                );
            }
            send_ipis(pending);
            spin_count = 0;
        }
    }

    TLB_SHOOTDOWN_LOCK.store(false, Ordering::Release);
}

/// Batch TLB shootdown: invalidates ALL TLB entries on remote CPUs.
/// More efficient than N individual shootdowns when unmapping many pages.
/// Use `tlb_shootdown(u64::MAX)` which triggers a full CR3 reload on receivers.
#[inline]
pub fn tlb_shootdown_full() {
    tlb_shootdown(u64::MAX);
}

/// Register the reschedule IPI handler (IRQ 22 = INT 54).
/// Must be called after IDT is initialized.
pub fn register_resched_ipi() {
    crate::arch::x86::irq::register_irq(22, resched_ipi_handler);
}

/// IRQ 22 handler: force an immediate scheduler pass on this CPU.
///
/// This is used to wake idle remote CPUs promptly when new work is queued for
/// their run queue, instead of waiting for the next local timer tick.
fn resched_ipi_handler(_irq: u8) {
    crate::task::scheduler::schedule_tick();
}

/// Register the halt IPI handler (IRQ 21 = INT 53).
/// Must be called after IDT is initialized.
pub fn register_halt_ipi() {
    crate::arch::x86::irq::register_irq(21, halt_ipi_handler);
}

/// IRQ 21 handler: halt this CPU permanently.
/// Triggered by `halt_other_cpus()` via IPI during panic/fatal exception.
fn halt_ipi_handler(_irq: u8) {
    unsafe {
        core::arch::asm!("cli");
    }
    loop {
        unsafe {
            core::arch::asm!("hlt");
        }
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

/// Request a remote CPU to run the scheduler immediately.
///
/// Used when a thread becomes runnable on another CPU and we don't want that
/// CPU to sleep in `hlt` until its next local timer interrupt.
pub fn resched_cpu(cpu_id: usize) {
    let count = cpu_count() as usize;
    if cpu_id >= count {
        return;
    }
    let my_cpu = current_cpu_id() as usize;
    if cpu_id == my_cpu {
        return;
    }
    let lapic_id = unsafe { CPU_DATA[cpu_id].lapic_id };
    crate::arch::x86::apic::send_ipi(lapic_id, crate::arch::x86::apic::VECTOR_IPI_RESCHED);
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
