//! ARM64 exception handling — VBAR_EL1 setup and exception dispatch.
//!
//! The exception vector table is defined in `asm_arm64/exceptions.S`.
//! This module provides the Rust-side dispatch functions called from assembly.

use core::sync::atomic::{AtomicU64, Ordering};

/// Saved register frame built by `exceptions.S` on entry from an exception.
#[repr(C)]
pub struct ExceptionFrame {
    pub x: [u64; 31],
    pub sp_el0: u64,
    pub elr_el1: u64,
    pub spsr_el1: u64,
}

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
    crate::serial_verbose_println!("[OK] VBAR_EL1 set (exception vector table installed)");
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
/// Counter for diagnostic: only print first IRQ to avoid flooding serial.
static IRQ_DIAG_COUNT: AtomicU64 = AtomicU64::new(0);

#[no_mangle]
pub extern "C" fn arm64_irq_handler() {
    let intid = super::gic::acknowledge();
    let count = IRQ_DIAG_COUNT.fetch_add(1, Ordering::Relaxed);
    if count < 3 {
        crate::serial_verbose_println!("  [IRQ] intid={} count={}", intid, count);
    }
    if intid < 1020 { // Not spurious
        // EOI FIRST: the handler may context-switch (schedule_tick), which would
        // never return here. Without early EOI the interrupt stays active and
        // blocks all further interrupts of equal/lower priority.
        super::gic::eoi(intid);
        let handler = unsafe { IRQ_HANDLERS[intid as usize] };
        if let Some(h) = handler {
            h();
        } else if count < 3 {
            crate::serial_verbose_println!("  [IRQ] no handler for intid={}", intid);
        }
    } else if count < 3 {
        crate::serial_verbose_println!("  [IRQ] spurious intid={}", intid);
    }
}

/// IRQ dispatch for interrupts taken while a user thread was running in EL0.
///
/// The timer IRQ can preempt the current thread, so we pass the saved
/// exception frame to the scheduler instead of trying to reuse the generic
/// EL1 context-switch path.
#[no_mangle]
pub extern "C" fn arm64_user_irq_handler(frame: *mut ExceptionFrame) {
    let intid = super::gic::acknowledge();
    let count = IRQ_DIAG_COUNT.fetch_add(1, Ordering::Relaxed);
    if count < 3 {
        crate::serial_verbose_println!("  [IRQ] intid={} count={}", intid, count);
    }
    if intid < 1020 {
        super::gic::eoi(intid);
        if intid == 30 {
            super::generic_timer::irq_handler();
            crate::task::scheduler::schedule_tick_from_user_irq(frame);
        } else {
            let handler = unsafe { IRQ_HANDLERS[intid as usize] };
            if let Some(h) = handler {
                h();
            } else if count < 3 {
                crate::serial_verbose_println!("  [IRQ] no handler for intid={}", intid);
            }
        }
    } else if count < 3 {
        crate::serial_verbose_println!("  [IRQ] spurious intid={}", intid);
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
            // ISS bits [5:0] = DFSC (Data Fault Status Code)
            // ISS bit  [6]   = WnR (1 = write fault, 0 = read fault)
            // ISS bit  [9]   = EA  (External abort)
            let dfsc = iss & 0x3F;
            let write_fault = (iss >> 6) & 1 != 0;
            let fault_type = match dfsc & 0x3C {
                0x04 => "translation fault",
                0x08 => "access flag fault",
                0x0C => "permission fault",
                0x10 => "synchronous external abort",
                0x20 => "alignment fault",
                _    => "unknown",
            };
            crate::serial_verbose_println!(
                "DATA ABORT ({}{}): FAR={:#018x} ELR={:#018x} ESR={:#018x}",
                if write_fault { "write " } else { "read " },
                fault_type, far, elr, esr,
            );
            handle_fault(ec, far, elr);
        }
        EC_INST_ABORT_LOWER | EC_INST_ABORT_SAME => {
            FAULT_COUNT.fetch_add(1, Ordering::Relaxed);
            // ISS bits [5:0] = IFSC (Instruction Fault Status Code)
            let ifsc = iss & 0x3F;
            let fault_type = match ifsc & 0x3C {
                0x04 => "translation fault",
                0x08 => "access flag fault",
                0x0C => "permission fault",
                0x10 => "synchronous external abort",
                _    => "unknown",
            };
            crate::serial_verbose_println!(
                "INSTRUCTION ABORT ({}): FAR={:#018x} ELR={:#018x} ESR={:#018x}",
                fault_type, far, elr, esr,
            );
            handle_fault(ec, far, elr);
        }
        EC_FP_TRAP => {
            // FP/SIMD trap — lazy FPU restore
            crate::task::scheduler::handle_device_not_available();
        }
        EC_BREAKPOINT_LOWER => {
            crate::serial_verbose_println!("BREAKPOINT at ELR={:#018x} — killing thread", elr);
            // No debugger attached; kill the user thread (SIGTRAP = 5)
            if !crate::task::scheduler::try_exit_current(5) {
                crate::task::scheduler::fault_kill_and_idle(5);
            }
        }
        EC_SS_LOWER => {
            // Single-step from user space: no debugger, just resume (clear MDSCR SS bit)
            // For now, kill the thread to avoid an infinite loop.
            crate::serial_verbose_println!("SINGLE STEP at ELR={:#018x} — killing thread", elr);
            if !crate::task::scheduler::try_exit_current(5) {
                crate::task::scheduler::fault_kill_and_idle(5);
            }
        }
        _ => {
            crate::serial_verbose_println!(
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
        crate::serial_verbose_println!("  Killing user thread due to fault");
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
