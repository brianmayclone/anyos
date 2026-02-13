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
const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;

// EFER bits
const EFER_SCE: u64 = 1 << 0; // Syscall Enable

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

        // Enable SYSCALL/SYSRET in IA32_EFER
        let efer = rdmsr(MSR_EFER);
        wrmsr(MSR_EFER, efer | EFER_SCE);

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

    crate::serial_println!(
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
    unsafe {
        PERCPU[cpu_id].kernel_rsp = rsp;
    }
}

/// Unconditionally reset KERNEL_GS_BASE to the correct PERCPU address for the
/// calling CPU.  Uses the hardware LAPIC ID (immune to software cpu_id bugs)
/// to determine the correct slot.
///
/// Called on every timer tick (before scheduling) and during context switch
/// to ensure KERNEL_GS_BASE is always correct. This closes the window where
/// a corrupted MSR could cause SYSCALL to load the wrong kernel RSP.
///
/// IMPORTANT: This function must run with interrupts disabled (inside an IRQ
/// handler or with CLI) to prevent re-entrant corruption.
pub fn refresh_kernel_gs_base() {
    if !INITIALIZED.load(Ordering::Acquire) {
        return;
    }
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    if cpu_id >= MAX_CPUS {
        return;
    }
    unsafe {
        let correct_base = &PERCPU[cpu_id] as *const SyscallPerCpu as u64;
        let actual_base = rdmsr(MSR_KERNEL_GS_BASE);
        if actual_base != correct_base {
            crate::serial_println!(
                "KERNEL_GS_BASE repair: CPU{} had {:#x}, expected {:#x}",
                cpu_id, actual_base, correct_base,
            );
            wrmsr(MSR_KERNEL_GS_BASE, correct_base);
        }
    }
}

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
