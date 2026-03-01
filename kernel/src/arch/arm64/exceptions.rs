//! ARM64 exception handling — VBAR_EL1 setup and exception dispatch.
//!
//! The exception vector table is defined in `asm_arm64/exceptions.S`.
//! This module provides the Rust-side dispatch functions called from assembly.

use core::sync::atomic::{AtomicU64, Ordering};

/// Exception Syndrome Register (ESR_EL1) exception class values.
pub const EC_SVC_AARCH64: u32 = 0x15; // SVC in AArch64 state
pub const EC_DATA_ABORT_LOWER: u32 = 0x24; // Data abort from EL0
pub const EC_DATA_ABORT_SAME: u32 = 0x25; // Data abort from EL1
pub const EC_INST_ABORT_LOWER: u32 = 0x20; // Instruction abort from EL0
pub const EC_INST_ABORT_SAME: u32 = 0x21; // Instruction abort from EL1
pub const EC_FP_TRAP: u32 = 0x07; // FP/SIMD trap (CPACR_EL1)
pub const EC_BREAKPOINT_LOWER: u32 = 0x30; // Breakpoint from EL0
pub const EC_SS_LOWER: u32 = 0x32; // Software Step from EL0

/// IRQ handler function pointer table (indexed by interrupt ID).
static mut IRQ_HANDLERS: [Option<fn()>; 1024] = [None; 1024];

/// Fault counter for diagnostics.
static FAULT_COUNT: AtomicU64 = AtomicU64::new(0);

/// Initialize the exception vector table.
///
/// Sets VBAR_EL1 to point to the exception vector table defined in exceptions.S.
pub fn init() {
    extern "C" {
        static _exception_vector_table: u8;
    }
    unsafe {
        let vbar = &_exception_vector_table as *const u8 as u64;
        core::arch::asm!(
            "msr vbar_el1, {}",
            "isb",
            in(reg) vbar,
            options(nostack),
        );
    }
    crate::serial_println!("[OK] VBAR_EL1 set (exception vector table installed)");
}

/// Register an IRQ handler for a specific interrupt ID.
pub fn register_irq(irq: u32, handler: fn()) {
    if (irq as usize) < 1024 {
        unsafe { IRQ_HANDLERS[irq as usize] = Some(handler); }
    }
}

/// IRQ dispatch — called from exceptions.S for IRQ exceptions.
///
/// Acknowledges the interrupt via GIC, dispatches to the registered handler,
/// then sends EOI.
#[no_mangle]
pub extern "C" fn arm64_irq_handler() {
    static IRQ_DBG: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    let intid = super::gic::acknowledge();
    let n = IRQ_DBG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    if n < 3 {
        crate::serial_println!("  [IRQ] #{} intid={}", n, intid);
    }
    if intid < 1020 { // Not spurious
        let handler = unsafe { IRQ_HANDLERS[intid as usize] };
        if let Some(h) = handler {
            h();
        }
        super::gic::eoi(intid);
    }
}

/// Synchronous exception dispatch — called from exceptions.S.
///
/// Reads ESR_EL1 to determine the exception class and dispatches accordingly.
#[no_mangle]
pub extern "C" fn arm64_sync_handler(esr: u64, far: u64, elr: u64) {
    let ec = ((esr >> 26) & 0x3F) as u32;
    let iss = (esr & 0x01FF_FFFF) as u32;

    match ec {
        EC_SVC_AARCH64 => {
            // Syscall — handled by syscall.rs
            // The immediate value (imm16) is in ISS bits [15:0]
            // Actual dispatch happens in exceptions.S before calling here
        }
        EC_DATA_ABORT_LOWER | EC_DATA_ABORT_SAME => {
            FAULT_COUNT.fetch_add(1, Ordering::Relaxed);
            crate::serial_println!(
                "DATA ABORT: FAR={:#018x} ELR={:#018x} ESR={:#018x} ISS={:#010x}",
                far, elr, esr, iss,
            );
            // TODO: page fault handling (demand paging, CoW)
            handle_fault(ec, far, elr);
        }
        EC_INST_ABORT_LOWER | EC_INST_ABORT_SAME => {
            FAULT_COUNT.fetch_add(1, Ordering::Relaxed);
            crate::serial_println!(
                "INSTRUCTION ABORT: FAR={:#018x} ELR={:#018x} ESR={:#018x}",
                far, elr, esr,
            );
            handle_fault(ec, far, elr);
        }
        EC_FP_TRAP => {
            // FP/SIMD trap — lazy FPU restore
            crate::task::scheduler::handle_device_not_available();
        }
        EC_BREAKPOINT_LOWER => {
            crate::serial_println!("BREAKPOINT at ELR={:#018x}", elr);
            // TODO: debug trap handling
        }
        EC_SS_LOWER => {
            crate::serial_println!("SINGLE STEP at ELR={:#018x}", elr);
            // TODO: single-step handling for debugger
        }
        _ => {
            crate::serial_println!(
                "UNHANDLED EXCEPTION: EC={:#04x} ISS={:#010x} FAR={:#018x} ELR={:#018x}",
                ec, iss, far, elr,
            );
            handle_fault(ec, far, elr);
        }
    }
}

/// Handle a fatal fault — kill the current thread or panic.
fn handle_fault(ec: u32, far: u64, elr: u64) {
    // If from EL0 (user mode), kill the thread
    let is_user = ec == EC_DATA_ABORT_LOWER || ec == EC_INST_ABORT_LOWER;
    if is_user {
        crate::serial_println!("  Killing user thread due to fault");
        if !crate::task::scheduler::try_exit_current(139) {
            crate::task::scheduler::fault_kill_and_idle(139);
        }
    } else {
        // Kernel fault — this is fatal
        panic!(
            "Kernel fault: EC={:#04x} FAR={:#018x} ELR={:#018x}",
            ec, far, elr,
        );
    }
}

/// Get the total fault count (for diagnostics).
pub fn fault_count() -> u64 {
    FAULT_COUNT.load(Ordering::Relaxed)
}
