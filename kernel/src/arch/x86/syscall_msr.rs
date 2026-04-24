//! SYSCALL/SYSRET MSR configuration and per-CPU scratch data.
//!
//! Sets up the Model-Specific Registers (MSRs) needed for the fast SYSCALL/SYSRET
//! instruction pair, and manages per-CPU data used by the SYSCALL entry point
//! to perform the user→kernel stack switch via SWAPGS.

use core::arch::asm;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::arch::x86::smp::MAX_CPUS;

// MSR addresses
const MSR_EFER: u32 = 0xC000_0080;
const MSR_STAR: u32 = 0xC000_0081;
const MSR_LSTAR: u32 = 0xC000_0082;
const MSR_SFMASK: u32 = 0xC000_0084;
pub const MSR_GS_BASE: u32 = 0xC000_0101;
pub const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;

/// Expands to a NASM-Intel instruction sequence that prepares the GS state
/// for a transition from CPL 0 to CPL 3.
///
/// Invariant before expansion: `GS.base = current CPU's PERCPU pointer`
/// (the invariant maintained by [`crate::arch::x86::syscall_msr`] and the
/// post-commit syscall entry path).
///
/// Effect after expansion:
///   * `IA32_KERNEL_GS_BASE ← GS.base` (so the user's next `swapgs` on
///     SYSCALL re-loads this CPU's PERCPU).
///   * `IA32_GS_BASE ← 0` (the user starts with a clean GS base; no kernel
///     pointer leak).
///
/// Clobbers `EAX`, `ECX`, `EDX`. Must be expanded inside an `asm!` block
/// with interrupts disabled, immediately before the `iretq` that transitions
/// to ring 3. See the "GS.base / KERNEL_GS_BASE Invariant" section in
/// `CLAUDE.md` for the rationale.
#[macro_export]
macro_rules! prepare_gs_for_ring3_asm {
    () => {
        concat!(
            "mov ecx, 0xC0000101\n",   // IA32_GS_BASE
            "rdmsr\n",                 // EDX:EAX = current GS.base (PERCPU)
            "mov ecx, 0xC0000102\n",   // IA32_KERNEL_GS_BASE
            "wrmsr\n",                 // KERNEL_GS_BASE = PERCPU
            "xor eax, eax\n",
            "xor edx, edx\n",
            "mov ecx, 0xC0000101\n",   // IA32_GS_BASE
            "wrmsr\n",                 // GS.base = 0 (user)
        )
    };
}

/// Debug-only sanity check that GS.base is a kernel higher-half pointer.
///
/// Any ring-3 transition expects `GS.base = current CPU's PERCPU`. If this
/// invariant is broken (for example because a new code path entered with
/// user-GS still loaded), calling [`prepare_gs_for_ring3_asm!`] would copy
/// garbage into `KERNEL_GS_BASE` and bring back the very bug the macro is
/// meant to prevent. Fail loudly at the source in debug builds.
#[cfg(target_arch = "x86_64")]
#[inline(always)]
pub unsafe fn debug_assert_gs_is_kernel() {
    #[cfg(debug_assertions)]
    {
        let gs_base = rdmsr(MSR_GS_BASE);
        if gs_base < 0xFFFF_8000_0000_0000 {
            panic!(
                "ring-3 transition with non-kernel GS.base={:#x} — caller violated the \
                 'GS.base = PERCPU throughout kernel residency' invariant",
                gs_base
            );
        }
    }
}

// EFER bits
const EFER_SCE: u64 = 1 << 0;  // Syscall Enable
const EFER_NXE: u64 = 1 << 11; // No-Execute Enable (must be set for PAGE_NX in PTEs to work)

// SFMASK: bits cleared in RFLAGS on SYSCALL entry
// Clear TF (bit 8), IF (bit 9), DF (bit 10)
const SFMASK_VALUE: u64 = (1 << 8) | (1 << 9) | (1 << 10);

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Per-CPU data accessed via GS segment during SYSCALL entry.
/// Layout must match the offsets used in syscall_fast.asm:
///   [gs:0]  = kernel_rsp
///   [gs:8]  = user_rsp (scratch for SYSCALL entry)
///   [gs:16] = lapic_id (for ownership verification in syscall_fast_entry)
///   [gs:24] = scratch_rax (used by pre-stack-switch LAPIC check)
#[repr(C, align(64))]
struct SyscallPerCpu {
    kernel_rsp: u64,
    user_rsp: u64,
    lapic_id: u8,
    _pad: [u8; 7],
    scratch_rax: u64,
}

static mut PERCPU: [SyscallPerCpu; MAX_CPUS] = {
    const INIT: SyscallPerCpu = SyscallPerCpu {
        kernel_rsp: 0,
        user_rsp: 0,
        lapic_id: 0xFF,
        _pad: [0; 7],
        scratch_rax: 0,
    };
    [INIT; MAX_CPUS]
};

/// Lookup table: LAPIC_ID → PERCPU address.
/// Used by syscall_fast_entry to find the correct PERCPU slot when
/// KERNEL_GS_BASE is corrupted. Indexed by hardware LAPIC ID (0-255).
#[no_mangle]
static mut LAPIC_TO_PERCPU: [u64; 256] = [0u64; 256];

#[inline(always)]
unsafe fn wrmsr(msr: u32, value: u64) {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") lo,
        in("edx") hi,
        options(nostack, preserves_flags),
    );
}

#[inline(always)]
unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") lo,
        out("edx") hi,
        options(nostack, preserves_flags),
    );
    ((hi as u64) << 32) | (lo as u64)
}

// Import the SYSCALL entry point from assembly
extern "C" {
    fn syscall_fast_entry();
}

/// Set up SYSCALL/SYSRET MSRs on the current CPU.
/// `cpu_id` is the logical CPU index (0 = BSP).
fn setup_msrs(cpu_id: usize) {
    let lapic_id = crate::arch::x86::apic::lapic_id();

    unsafe {
        // Populate PERCPU ownership fields for assembly-level verification
        PERCPU[cpu_id].lapic_id = lapic_id;

        // Populate the LAPIC→PERCPU lookup table for the repair path
        LAPIC_TO_PERCPU[lapic_id as usize] =
            &PERCPU[cpu_id] as *const SyscallPerCpu as u64;

        // Enable SYSCALL/SYSRET and NX (No-Execute) in IA32_EFER.
        // EFER.NXE (bit 11) must be set before any PTE with bit 63 set is loaded;
        // without it the CPU treats bit 63 as reserved and raises #GP.
        let efer = rdmsr(MSR_EFER);
        let nx_bit = if crate::arch::x86::cpuid::features().nx { EFER_NXE } else { 0 };
        wrmsr(MSR_EFER, efer | EFER_SCE | nx_bit);

        // STAR: kernel/user segment selectors
        wrmsr(MSR_STAR, crate::arch::x86::gdt::STAR_MSR_VALUE);

        // LSTAR: SYSCALL entry point address
        wrmsr(MSR_LSTAR, syscall_fast_entry as u64);

        // SFMASK: clear TF, IF, DF on SYSCALL entry
        wrmsr(MSR_SFMASK, SFMASK_VALUE);

        // Set MSR_KERNEL_GS_BASE to this CPU's per-CPU data
        let percpu_addr = &PERCPU[cpu_id] as *const SyscallPerCpu as u64;
        wrmsr(MSR_KERNEL_GS_BASE, percpu_addr);
    }

    crate::serial_verbose_println!(
        "[OK] SYSCALL/SYSRET configured on CPU{} (LAPIC_ID={})",
        cpu_id, lapic_id,
    );
}

/// Initialize SYSCALL/SYSRET on the BSP (CPU 0).
/// Must be called after GDT is set up and CPUID has confirmed SYSCALL support.
pub fn init_bsp() {
    setup_msrs(0);
    INITIALIZED.store(true, Ordering::Release);
}

/// Initialize SYSCALL/SYSRET on an AP.
pub fn init_ap(cpu_id: usize) {
    if cpu_id < MAX_CPUS {
        setup_msrs(cpu_id);
    }
}

/// Update the kernel RSP for SYSCALL on the specified CPU.
/// Called by the scheduler on every context switch (alongside TSS RSP0 update).
///
/// Uses direct `PERCPU[cpu_id]` indexing — the data is always written to the
/// correct slot regardless of KERNEL_GS_BASE state.
///
/// NOTE: Does NOT read or write the KERNEL_GS_BASE MSR. The MSR is maintained
/// separately by `refresh_kernel_gs_base()` which runs on every timer tick.
/// This separation prevents a feedback loop where a transiently wrong cpu_id
/// would cause the "repair" code to actively corrupt the MSR.
pub fn set_kernel_rsp(cpu_id: usize, rsp: u64) {
    if !INITIALIZED.load(Ordering::Acquire) || cpu_id >= MAX_CPUS {
        return;
    }
    // Guard: kernel RSP must be in higher-half (same check as TSS.RSP0).
    // Without this, a corrupt kernel_stack_top() would be stored here while
    // set_kernel_stack_for_cpu() rejects it — leaving PERCPU.kernel_rsp corrupt
    // but TSS.RSP0 valid. The SYSCALL path loads RSP from [gs:0] (PERCPU),
    // so corrupt PERCPU.kernel_rsp → RSP=garbage → Double Fault.
    if rsp == 0 || rsp < 0xFFFF_FFFF_8000_0000 {
        unsafe {
            use crate::arch::x86::port::{inb, outb};
            let msg = b"\r\n!!! BUG: set_kernel_rsp bad rsp cpu=";
            for &c in msg { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
            outb(0x3F8, b'0' + cpu_id as u8);
            let msg2 = b" rsp=";
            for &c in msg2 { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
            // Print hex value of rsp
            let hex = b"0123456789abcdef";
            for i in (0..16).rev() {
                let nibble = ((rsp >> (i * 4)) & 0xF) as usize;
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, hex[nibble]);
            }
            let msg3 = b"\r\n";
            for &c in msg3 { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
        }
        return; // Keep previous valid kernel_rsp
    }
    unsafe {
        PERCPU[cpu_id].kernel_rsp = rsp;
    }
}

/// DEPRECATED — do not call.
///
/// Under the pre-241b1475 invariant (Phase 3 `swapgs` back to user GS inside
/// the SYSCALL handler) this function safely re-asserted
/// `KERNEL_GS_BASE = PERCPU` during kernel residency, which was also the
/// expected state back then.
///
/// Post-241b1475 the invariant inverted: during kernel residency
/// `GS.base = PERCPU` and `KERNEL_GS_BASE = user_gs` (usually 0). Overwriting
/// `KERNEL_GS_BASE` with PERCPU from an IRQ handler now *breaks* the
/// invariant: the final Phase 6 `swapgs` degenerates into a no-op and user
/// space inherits `GS.base = PERCPU`, leaking the per-CPU pointer into
/// ring 3.
///
/// MSR-leak recovery lives in the Phase 1b LAPIC-ID check inside
/// `syscall_fast.asm`, which inspects hardware state rather than a software
/// shadow and is therefore immune to this ambiguity. Do not re-introduce a
/// timer-tick MSR refresh without rethinking the invariant end-to-end.
#[deprecated(
    note = "Breaks the post-241b1475 GS invariant; see syscall_fast.asm \
            Phase 1b for the correct leak-recovery mechanism."
)]
#[allow(dead_code)]
pub fn refresh_kernel_gs_base() {}

/// Read the kernel RSP for SYSCALL on the specified CPU (diagnostic use).
/// Reads directly from `PERCPU[cpu_id]` for consistency with `set_kernel_rsp`.
pub fn get_kernel_rsp(cpu_id: usize) -> u64 {
    if !INITIALIZED.load(Ordering::Acquire) || cpu_id >= MAX_CPUS {
        return 0;
    }
    unsafe { PERCPU[cpu_id].kernel_rsp }
}

/// Read the kernel RSP via KERNEL_GS_BASE MSR (what SYSCALL entry actually uses).
/// This is the value that `[gs:0]` will load after SWAPGS in syscall_fast.asm.
/// Used for diagnostics to detect if KERNEL_GS_BASE points to the wrong slot.
pub fn get_kernel_rsp_via_msr() -> u64 {
    if !INITIALIZED.load(Ordering::Acquire) {
        return 0;
    }
    unsafe {
        let percpu_base = rdmsr(MSR_KERNEL_GS_BASE);
        if percpu_base != 0 {
            (*(percpu_base as *const SyscallPerCpu)).kernel_rsp
        } else {
            0
        }
    }
}
