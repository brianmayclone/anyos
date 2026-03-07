//! Runtime helper functions callable from JIT-compiled code.
//!
//! These functions provide the bridge between native JIT code and the
//! emulated VM state. JIT-compiled basic blocks call these helpers for
//! operations that cannot be inlined as native instructions:
//!
//! - **Memory access**: Guest virtual → physical translation + MMIO dispatch
//! - **Port I/O**: IN/OUT instruction emulation
//! - **Exceptions**: Delivering x86 exceptions to the guest IDT
//! - **Interpreter fallback**: Executing a single instruction via the
//!   existing interpreter when the JIT doesn't support it
//!
//! All functions use the C ABI (`extern "C"`) so they can be called from
//! generated machine code via `call <absolute_address>`.

use crate::cpu::Cpu;
use crate::error::VmError;
use crate::interrupts::InterruptController;
use crate::io::IoDispatch;
use crate::memory::{AccessType, GuestMemory, MemoryBus, Mmu};

/// Result code from JIT helper functions.
///
/// 0 = success, non-zero = exception vector that needs to be injected.
pub type JitResult = u32;

/// Success — no exception.
pub const JIT_OK: JitResult = 0;
/// Signals that the JIT block should exit and return to the dispatcher.
pub const JIT_EXIT_BLOCK: JitResult = 0xFFFF_FFFF;

// ════════════════════════════════════════════════════════════════════════
// Memory Access Helpers
// ════════════════════════════════════════════════════════════════════════

/// Read 1/2/4/8 bytes from guest memory at a linear address.
///
/// Performs full segmentation + paging translation. On success, returns
/// the value in the low bits of the return value. On page fault or other
/// error, stores the exception info and returns JIT_EXIT_BLOCK.
///
/// # Arguments
/// * `cpu` — Pointer to the Cpu struct
/// * `memory` — Pointer to GuestMemory
/// * `mmu` — Pointer to Mmu
/// * `linear` — Guest linear address
/// * `size` — Access size in bytes (1, 2, 4, or 8)
///
/// # Returns
/// The read value (zero-extended to u64).
#[no_mangle]
pub extern "C" fn jit_mem_read(
    cpu: &mut Cpu,
    memory: &GuestMemory,
    mmu: &Mmu,
    linear: u64,
    size: u8,
) -> u64 {
    let phys = match mmu.translate_linear(
        linear,
        cpu.regs.cr3,
        AccessType::Read,
        cpu.regs.cpl,
        memory,
    ) {
        Ok(p) => p,
        Err(_) => {
            cpu.jit_fault = true;
            return 0;
        }
    };

    match size {
        1 => memory.read_u8(phys).unwrap_or(0xFF) as u64,
        2 => memory.read_u16(phys).unwrap_or(0xFFFF) as u64,
        4 => memory.read_u32(phys).unwrap_or(0xFFFF_FFFF) as u64,
        8 => memory.read_u64(phys).unwrap_or(0xFFFF_FFFF_FFFF_FFFF),
        _ => 0,
    }
}

/// Write 1/2/4/8 bytes to guest memory at a linear address.
///
/// # Returns
/// `JIT_OK` on success, `JIT_EXIT_BLOCK` on page fault.
#[no_mangle]
pub extern "C" fn jit_mem_write(
    cpu: &mut Cpu,
    memory: &mut GuestMemory,
    mmu: &Mmu,
    linear: u64,
    size: u8,
    value: u64,
) -> JitResult {
    let phys = match mmu.translate_linear(
        linear,
        cpu.regs.cr3,
        AccessType::Write,
        cpu.regs.cpl,
        memory,
    ) {
        Ok(p) => p,
        Err(_) => return JIT_EXIT_BLOCK,
    };

    let result = match size {
        1 => memory.write_u8(phys, value as u8),
        2 => memory.write_u16(phys, value as u16),
        4 => memory.write_u32(phys, value as u32),
        8 => memory.write_u64(phys, value),
        _ => Ok(()),
    };

    match result {
        Ok(()) => JIT_OK,
        Err(_) => JIT_EXIT_BLOCK,
    }
}

// ════════════════════════════════════════════════════════════════════════
// Port I/O Helpers
// ════════════════════════════════════════════════════════════════════════

/// Execute an IN instruction (port read).
///
/// # Returns
/// The value read from the port (zero-extended to u32).
#[no_mangle]
pub extern "C" fn jit_port_in(
    io: &mut IoDispatch,
    port: u16,
    size: u8,
) -> u32 {
    io.port_in(port, size).unwrap_or(0xFFFF_FFFF)
}

/// Execute an OUT instruction (port write).
#[no_mangle]
pub extern "C" fn jit_port_out(
    io: &mut IoDispatch,
    port: u16,
    size: u8,
    value: u32,
) {
    let _ = io.port_out(port, size, value);
}

// ════════════════════════════════════════════════════════════════════════
// Register Access Helpers
// ════════════════════════════════════════════════════════════════════════

/// Return a pointer to the base of the GPR array in the CPU's register file.
///
/// Called once in the JIT block prologue. The returned pointer is used
/// with known offsets (from `#[repr(C)]` on `RegisterFile`) to access
/// GPR, RIP, and RFLAGS without needing to know the `Cpu` struct layout.
#[no_mangle]
pub extern "C" fn jit_get_gpr_ptr(cpu: &mut Cpu) -> *mut u64 {
    cpu.regs.gpr.as_mut_ptr()
}

/// Evaluate a condition code against guest RFLAGS.
///
/// Returns 1 if the condition is met, 0 otherwise. Used by JIT-compiled
/// conditional branches (Jcc, SETcc, CMOVcc).
#[no_mangle]
pub extern "C" fn jit_eval_cc(rflags: u64, cc: u8) -> u32 {
    if crate::flags::eval_cc(cc, rflags) { 1 } else { 0 }
}

// ════════════════════════════════════════════════════════════════════════
// Interpreter Fallback
// ════════════════════════════════════════════════════════════════════════

/// Execute a single instruction via the interpreter.
///
/// Used when the JIT encounters an instruction it cannot translate natively
/// (e.g., FPU, SSE, system instructions). Decodes and executes one
/// instruction at the current CS:RIP, then returns.
///
/// # Returns
/// `JIT_OK` on success, or the exception vector on error.
#[no_mangle]
pub extern "C" fn jit_interpret_one(
    cpu: &mut Cpu,
    memory: &mut GuestMemory,
    mmu: &mut Mmu,
    io: &mut IoDispatch,
    interrupts: &mut InterruptController,
) -> JitResult {
    // Compute fetch address
    let cs_base = cpu.regs.seg[crate::registers::SegReg::Cs as usize].base;
    let fetch_addr = cs_base.wrapping_add(cpu.regs.rip);

    // Translate through paging
    let phys_addr = match mmu.translate_linear(
        fetch_addr,
        cpu.regs.cr3,
        AccessType::Execute,
        cpu.regs.cpl,
        memory,
    ) {
        Ok(addr) => addr,
        Err(_) => return JIT_EXIT_BLOCK,
    };

    // Decode
    let inst = match cpu.decoder.decode(&*memory, phys_addr) {
        Ok(inst) => inst,
        Err(_) => return JIT_EXIT_BLOCK,
    };

    // Execute
    match crate::executor::execute(cpu, &inst, memory, mmu, io, interrupts) {
        Ok(()) => {
            cpu.instruction_count += 1;
            // Mirror the interpreter's mask_rip() call so RIP stays within
            // the correct width for the current mode (16-bit real/protected).
            cpu.mask_rip();
            JIT_OK
        }
        Err(VmError::Halted) => {
            cpu.instruction_count += 1;
            cpu.mask_rip();
            JIT_EXIT_BLOCK
        }
        Err(_) => JIT_EXIT_BLOCK,
    }
}

/// Execute a single pre-decoded instruction via the executor.
/// No address translation, no re-decode.
#[no_mangle]
pub extern "C" fn helper_execute_one(
    inst: *const crate::instruction::DecodedInst,
    cpu: &mut Cpu,
    memory: &mut GuestMemory,
    mmu: &mut Mmu,
    io: &mut IoDispatch,
    interrupts: &mut InterruptController,
) -> JitResult {
    let inst = unsafe { &*inst };
    match crate::executor::execute(cpu, inst, memory, mmu, io, interrupts) {
        Ok(()) => {
            cpu.instruction_count += 1;
            cpu.mask_rip();
            JIT_OK
        }
        Err(VmError::Halted) => {
            cpu.instruction_count += 1;
            cpu.mask_rip();
            JIT_EXIT_BLOCK
        }
        Err(_) => JIT_EXIT_BLOCK,
    }
}

// ════════════════════════════════════════════════════════════════════════
// Dispatcher Helpers
// ════════════════════════════════════════════════════════════════════════

/// Translate a linear address to physical via the MMU.
/// Returns the physical address on success, or u64::MAX on fault.
/// Used by the JIT dispatcher to resolve CS:RIP before block lookup.
#[no_mangle]
pub extern "C" fn jit_mmu_translate(
    cpu: &mut Cpu,
    mmu: &Mmu,
    memory: &GuestMemory,
    linear: u64,
) -> u64 {
    match mmu.translate_linear(
        linear,
        cpu.regs.cr3,
        AccessType::Execute,
        cpu.regs.cpl,
        memory,
    ) {
        Ok(phys) => phys,
        Err(_) => {
            cpu.jit_fault = true;
            u64::MAX
        }
    }
}

/// Check if there is a pending interrupt that should be delivered.
/// Returns 1 if yes (dispatcher should exit), 0 if no.
#[no_mangle]
pub extern "C" fn jit_check_interrupt(
    interrupts: &InterruptController,
    rflags: u64,
) -> u32 {
    if interrupts.pending_interrupt(rflags).is_some() {
        1
    } else {
        0
    }
}

/// Return the current CPU decode mode as a u8.
/// 0 = Real16, 1 = Protected32, 2 = Long64.
#[no_mangle]
pub extern "C" fn jit_get_cpu_mode(cpu: &Cpu) -> u8 {
    cpu.decoder.mode() as u8
}
