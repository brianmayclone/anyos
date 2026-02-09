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
///   [gs:0] = kernel_rsp
///   [gs:8] = user_rsp (scratch for SYSCALL entry)
#[repr(C, align(64))]
struct SyscallPerCpu {
    kernel_rsp: u64,
    user_rsp: u64,
}

static mut PERCPU: [SyscallPerCpu; MAX_CPUS] = {
    const INIT: SyscallPerCpu = SyscallPerCpu {
        kernel_rsp: 0,
        user_rsp: 0,
    };
    [INIT; MAX_CPUS]
};

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
    unsafe {
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
        "[OK] SYSCALL/SYSRET configured on CPU{}",
        cpu_id
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

/// Update the kernel RSP for SYSCALL on the current CPU.
/// Called by the scheduler on every context switch (alongside TSS RSP0 update).
/// Uses MSR_KERNEL_GS_BASE to find the current CPU's per-CPU data.
pub fn set_kernel_rsp(rsp: u64) {
    if !INITIALIZED.load(Ordering::Acquire) {
        return;
    }
    unsafe {
        let percpu_base = rdmsr(MSR_KERNEL_GS_BASE);
        if percpu_base != 0 {
            (*(percpu_base as *mut SyscallPerCpu)).kernel_rsp = rsp;
        }
    }
}
