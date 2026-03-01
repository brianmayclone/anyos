//! Hardware Abstraction Layer — platform-agnostic API.
//!
//! Free functions with `cfg`-gated forwarding to the active architecture.
//! All cross-platform kernel code should use `arch::hal::*` instead of
//! directly referencing `arch::x86::*` or `arch::arm64::*`.

// =============================================================================
// Constants
// =============================================================================

/// Maximum number of CPUs supported by the kernel.
#[cfg(target_arch = "x86_64")]
#[allow(unused_imports)]
pub use crate::arch::x86::smp::MAX_CPUS;

#[cfg(target_arch = "aarch64")]
pub const MAX_CPUS: usize = 16;

// =============================================================================
// CPU Operations
// =============================================================================

/// Get the current CPU's ID (always accurate, even after migration).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn cpu_id() -> usize {
    crate::arch::x86::smp::current_cpu_id() as usize
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn cpu_id() -> usize {
    let mpidr: u64;
    unsafe { core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr, options(nomem, nostack)); }
    (mpidr & 0xFF) as usize
}

/// Number of online CPUs (at least 1).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn cpu_count() -> usize {
    let n = crate::arch::x86::smp::cpu_count() as usize;
    if n == 0 { 1 } else { n }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn cpu_count() -> usize {
    // ARM64: will be set during SMP init via PSCI/DTB
    1 // TODO: implement for ARM64
}

/// Enable interrupts on the current CPU.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn enable_interrupts() {
    unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn enable_interrupts() {
    unsafe { core::arch::asm!("msr daifclr, #0xf", options(nomem, nostack)); }
}

/// Disable interrupts on the current CPU.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn disable_interrupts() {
    unsafe { core::arch::asm!("cli", options(nomem, nostack)); }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn disable_interrupts() {
    unsafe { core::arch::asm!("msr daifset, #0xf", options(nomem, nostack)); }
}

/// Check if interrupts are enabled on the current CPU.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn interrupts_enabled() -> bool {
    let rflags: u64;
    unsafe { core::arch::asm!("pushfq; pop {}", out(reg) rflags, options(nomem)); }
    rflags & 0x200 != 0
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn interrupts_enabled() -> bool {
    let daif: u64;
    unsafe { core::arch::asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack)); }
    daif & 0x3C0 == 0 // All DAIF bits clear = interrupts enabled
}

/// Halt the CPU (low-power wait for interrupt).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn halt() {
    unsafe { core::arch::asm!("hlt", options(nomem, nostack)); }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn halt() {
    unsafe { core::arch::asm!("wfi", options(nomem, nostack)); }
}

// =============================================================================
// Interrupt State Save/Restore
// =============================================================================

/// Save the current interrupt state and disable interrupts.
/// Returns an opaque value that must be passed to `restore_interrupt_state`.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn save_and_disable_interrupts() -> u64 {
    let flags: u64;
    unsafe {
        core::arch::asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags));
        core::arch::asm!("cli", options(nomem, nostack, preserves_flags));
    }
    flags
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn save_and_disable_interrupts() -> u64 {
    let daif: u64;
    unsafe {
        core::arch::asm!("mrs {}, daif", out(reg) daif, options(nomem, nostack));
        core::arch::asm!("msr daifset, #0xf", options(nomem, nostack));
    }
    daif
}

/// Restore interrupt state from a value returned by `save_and_disable_interrupts`.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn restore_interrupt_state(saved: u64) {
    if saved & 0x200 != 0 {
        unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn restore_interrupt_state(saved: u64) {
    unsafe { core::arch::asm!("msr daif, {}", in(reg) saved, options(nomem, nostack)); }
}

// =============================================================================
// Timer
// =============================================================================

/// Get the current timer tick count (monotonic, ~1000 Hz on all platforms).
///
/// On x86_64: PIT IRQ counter (TICK_HZ = 1000).
/// On AArch64: hardware counter normalized to millisecond granularity.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn timer_current_ticks() -> u32 {
    crate::arch::x86::pit::get_ticks()
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn timer_current_ticks() -> u32 {
    let cnt: u64;
    unsafe { core::arch::asm!("mrs {}, cntpct_el0", out(reg) cnt, options(nomem, nostack)); }
    let freq: u64;
    unsafe { core::arch::asm!("mrs {}, cntfrq_el0", out(reg) freq, options(nomem, nostack)); }
    // Normalize to ~1000 Hz (millisecond ticks) for consistency with x86 PIT
    if freq == 0 { return 0; }
    let divisor = freq / 1000;
    if divisor == 0 { return cnt as u32; }
    (cnt / divisor) as u32
}

/// Get the timer tick frequency in Hz (~1000 on all platforms).
///
/// This is the effective frequency of `timer_current_ticks()`.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn timer_frequency_hz() -> u64 {
    crate::arch::x86::pit::TICK_HZ as u64
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn timer_frequency_hz() -> u64 {
    1000 // Ticks are normalized to 1000 Hz (millisecond granularity)
}

// =============================================================================
// Per-CPU Kernel Stack Setup
// =============================================================================

/// Set the kernel stack for a CPU (used by syscall entry and IRQ handlers).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn set_kernel_stack_for_cpu(cpu: usize, stack_top: u64) {
    crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu, stack_top);
    crate::arch::x86::syscall_msr::set_kernel_rsp(cpu, stack_top);
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn set_kernel_stack_for_cpu(_cpu: usize, _stack_top: u64) {
    // ARM64: kernel stack is managed via SP_EL1; set during context switch
    // TODO: implement for ARM64
}

/// Get the kernel stack pointer for a CPU (e.g., TSS.RSP0 on x86).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn get_kernel_stack_for_cpu(cpu: usize) -> u64 {
    crate::arch::x86::tss::get_kernel_stack_for_cpu(cpu)
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn get_kernel_stack_for_cpu(_cpu: usize) -> u64 {
    // ARM64: SP_EL1 per-CPU; TODO: implement
    0
}

// =============================================================================
// CPU Feature Detection
// =============================================================================

/// Check if XSAVE/XRSTOR is supported (x86 FPU/SSE/AVX state management).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn has_xsave() -> bool {
    crate::arch::x86::cpuid::HAS_XSAVE.load(core::sync::atomic::Ordering::Relaxed)
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn has_xsave() -> bool {
    false // ARM64 does not use XSAVE
}

/// Check if MONITOR/MWAIT is supported (x86 idle optimization).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn has_mwait() -> bool {
    crate::arch::x86::cpuid::HAS_MWAIT.load(core::sync::atomic::Ordering::Relaxed)
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn has_mwait() -> bool {
    false // ARM64 uses WFE/WFI instead
}

// =============================================================================
// Interrupt Controller
// =============================================================================

/// Send an End-Of-Interrupt signal to the interrupt controller.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn irq_eoi() {
    crate::arch::x86::apic::eoi();
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn irq_eoi() {
    // ARM64: write ICC_EOIR1_EL1
    // TODO: implement for ARM64 GICv3
}

/// Send an Inter-Processor Interrupt to the specified CPU.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn send_ipi(cpu: usize, vector: u8) {
    crate::arch::x86::apic::send_ipi(cpu as u8, vector);
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn send_ipi(_cpu: usize, _vector: u8) {
    // ARM64: ICC_SGI1R_EL1 for Software Generated Interrupt
    // TODO: implement for ARM64 GICv3
}

/// Halt all other CPUs (for panic/fatal error handling).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn halt_other_cpus() {
    crate::arch::x86::smp::halt_other_cpus();
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn halt_other_cpus() {
    // ARM64: TODO — send SGI to halt other CPUs
}

// =============================================================================
// Page Tables
// =============================================================================

/// Read the current page table base register (CR3 on x86, TTBR0_EL1 on ARM64).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn current_page_table() -> u64 {
    crate::memory::virtual_mem::current_cr3()
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn current_page_table() -> u64 {
    let ttbr0: u64;
    unsafe { core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr0, options(nomem, nostack)); }
    ttbr0
}

/// Switch to a different page table.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn switch_page_table(addr: u64) {
    unsafe { core::arch::asm!("mov cr3, {}", in(reg) addr, options(nostack)); }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn switch_page_table(addr: u64) {
    unsafe {
        core::arch::asm!(
            "msr ttbr0_el1, {}",
            "isb",
            in(reg) addr,
            options(nostack),
        );
    }
}

/// Flush TLB for a single virtual address.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn flush_tlb(vaddr: u64) {
    unsafe { core::arch::asm!("invlpg [{}]", in(reg) vaddr, options(nostack)); }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn flush_tlb(vaddr: u64) {
    unsafe {
        core::arch::asm!(
            "tlbi vale1is, {}",
            "dsb ish",
            "isb",
            in(reg) vaddr >> 12,
            options(nostack),
        );
    }
}

/// Flush the entire TLB.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn flush_tlb_all() {
    unsafe {
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn flush_tlb_all() {
    unsafe {
        core::arch::asm!(
            "tlbi vmalle1is",
            "dsb ish",
            "isb",
            options(nostack),
        );
    }
}

// =============================================================================
// FPU State Management
// =============================================================================

/// Set the "task switched" flag to trigger lazy FPU restore on next FPU use.
/// On x86, this sets CR0.TS. On ARM64, FPU trapping is controlled via CPACR_EL1.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn fpu_set_trap() {
    unsafe {
        let cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nostack, nomem, preserves_flags));
        core::arch::asm!("mov cr0, {}", in(reg) cr0 | 8, options(nostack, nomem, preserves_flags));
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn fpu_set_trap() {
    // ARM64: trap FP/NEON via CPACR_EL1.FPEN = 0b00
    unsafe {
        core::arch::asm!(
            "mrs {tmp}, cpacr_el1",
            "bic {tmp}, {tmp}, #(3 << 20)",
            "msr cpacr_el1, {tmp}",
            "isb",
            tmp = out(reg) _,
            options(nostack),
        );
    }
}

/// Clear the FPU trap flag (allow FPU/SIMD instructions).
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn fpu_clear_trap() {
    unsafe { core::arch::asm!("clts", options(nostack, preserves_flags)); }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn fpu_clear_trap() {
    // ARM64: enable FP/NEON via CPACR_EL1.FPEN = 0b11
    unsafe {
        core::arch::asm!(
            "mrs {tmp}, cpacr_el1",
            "orr {tmp}, {tmp}, #(3 << 20)",
            "msr cpacr_el1, {tmp}",
            "isb",
            tmp = out(reg) _,
            options(nostack),
        );
    }
}

/// Save FPU/SIMD state to a buffer.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn fpu_save(buf: *mut u8) {
    unsafe {
        if has_xsave() {
            core::arch::asm!(
                "xsave [{}]",
                in(reg) buf,
                in("eax") 0xFFFF_FFFFu32,
                in("edx") 0xFFFF_FFFFu32,
                options(nostack, preserves_flags),
            );
        } else {
            core::arch::asm!("fxsave [{}]", in(reg) buf, options(nostack, preserves_flags));
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn fpu_save(_buf: *mut u8) {
    // ARM64: save Q0-Q31 + FPCR + FPSR
    // TODO: implement for ARM64
}

/// Restore FPU/SIMD state from a buffer.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn fpu_restore(buf: *const u8) {
    unsafe {
        if has_xsave() {
            core::arch::asm!(
                "xrstor [{}]",
                in(reg) buf,
                in("eax") 0xFFFF_FFFFu32,
                in("edx") 0xFFFF_FFFFu32,
                options(nostack, preserves_flags),
            );
        } else {
            core::arch::asm!("fxrstor [{}]", in(reg) buf, options(nostack, preserves_flags));
        }
    }
}

#[cfg(target_arch = "aarch64")]
#[inline]
pub fn fpu_restore(_buf: *const u8) {
    // ARM64: restore Q0-Q31 + FPCR + FPSR
    // TODO: implement for ARM64
}

// =============================================================================
// Context Switch (re-exported from architecture)
// =============================================================================

/// Architecture-specific CPU context — re-exported for cross-platform use.
#[cfg(target_arch = "x86_64")]
#[allow(unused_imports)]
pub use crate::task::context::CpuContext;

// ARM64 CpuContext will be defined in arch::arm64::context

/// Architecture-specific context switch — re-exported.
#[cfg(target_arch = "x86_64")]
#[allow(unused_imports)]
pub use crate::task::context::context_switch;
