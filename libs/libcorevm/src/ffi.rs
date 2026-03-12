//! C FFI layer for libcorevm.
//!
//! All `extern "C"` functions that form the public API consumed by the VM
//! daemon (vmd) and other C/C++ callers. A global VM registry maps opaque
//! `u64` handles to [`Vm`] instances.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use crate::vm::Vm;
use crate::backend::types::*;
#[cfg(feature = "linux")]
use crate::backend::VmBackend;
use crate::backend::VmExitReason;

// ── Global VM registry ──────────────────────────────────────────────────────

static mut VMS: Option<Vec<Option<Vm>>> = None;
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);
static mut LAST_ERROR: Option<String> = None;

fn set_last_error(msg: String) {
    unsafe { LAST_ERROR = Some(msg); }
}

fn clear_last_error() {
    unsafe { LAST_ERROR = None; }
}

fn vm_list() -> &'static mut Vec<Option<Vm>> {
    unsafe {
        if VMS.is_none() {
            VMS = Some(Vec::new());
        }
        VMS.as_mut().unwrap()
    }
}

fn get_vm(handle: u64) -> Option<&'static mut Vm> {
    if handle == 0 {
        return None;
    }
    let idx = (handle - 1) as usize;
    vm_list().get_mut(idx).and_then(|slot| slot.as_mut())
}

// ── C-compatible exit reason ────────────────────────────────────────────────

/// C-compatible tagged struct for VM exit reasons.
///
/// The `reason` field selects which union members are valid:
/// 0=IoIn, 1=IoOut, 2=MmioRead, 3=MmioWrite, 4=MsrRead, 5=MsrWrite,
/// 6=Cpuid, 7=Halted, 8=InterruptWindow, 9=Shutdown, 10=Debug, 11=Error,
/// 12=StringIo
#[repr(C)]
#[derive(Default)]
pub struct CExitReason {
    pub reason: u32,
    pub port: u16,
    pub size: u8,
    pub _pad: u8,
    pub data_u32: u32,
    pub _pad2: u32,
    pub addr: u64,
    pub data_u64: u64,
    pub msr_index: u32,
    pub cpuid_fn: u32,
    pub cpuid_idx: u32,
    pub mmio_dest_reg: u8,
    pub mmio_instr_len: u8,
    pub _reserved: [u8; 2],
    // StringIo fields (reason=12)
    pub string_io_count: u64,
    pub string_io_gpa: u64,
    pub string_io_step: i64,
    pub string_io_instr_len: u64,
    pub string_io_is_write: u8,
    pub string_io_addr_size: u8,
    pub _reserved2: [u8; 6],
}

fn fill_exit(e: &mut CExitReason, reason: VmExitReason) {
    *e = CExitReason::default();
    match reason {
        VmExitReason::IoIn { port, size } => {
            e.reason = 0; e.port = port; e.size = size;
        }
        VmExitReason::IoOut { port, size, data } => {
            e.reason = 1; e.port = port; e.size = size; e.data_u32 = data;
        }
        VmExitReason::MmioRead { addr, size, dest_reg, instr_len } => {
            e.reason = 2; e.addr = addr; e.size = size;
            e.mmio_dest_reg = dest_reg; e.mmio_instr_len = instr_len;
        }
        VmExitReason::MmioWrite { addr, size, data } => {
            e.reason = 3; e.addr = addr; e.size = size; e.data_u64 = data;
        }
        VmExitReason::MsrRead { index } => {
            e.reason = 4; e.msr_index = index;
        }
        VmExitReason::MsrWrite { index, value } => {
            e.reason = 5; e.msr_index = index; e.data_u64 = value;
        }
        VmExitReason::CpuidExit { function, index } => {
            e.reason = 6; e.cpuid_fn = function; e.cpuid_idx = index;
        }
        VmExitReason::StringIo { port, is_write, count, gpa, step, instr_len, addr_size, access_size } => {
            e.reason = 12; e.port = port; e.size = access_size;
            e.string_io_count = count;
            e.string_io_gpa = gpa;
            e.string_io_step = step;
            e.string_io_instr_len = instr_len;
            e.string_io_is_write = if is_write { 1 } else { 0 };
            e.string_io_addr_size = addr_size;
        }
        VmExitReason::Halted => e.reason = 7,
        VmExitReason::InterruptWindow => e.reason = 8,
        VmExitReason::Shutdown => e.reason = 9,
        VmExitReason::Debug => e.reason = 10,
        VmExitReason::Error => e.reason = 11,
    }
}

// ── VM lifecycle ────────────────────────────────────────────────────────────

/// Create a new VM with the given RAM size in megabytes.
/// Returns a non-zero handle on success, 0 on failure.
#[no_mangle]
pub extern "C" fn corevm_create(ram_mb: u32) -> u64 {
    clear_last_error();
    match Vm::new(ram_mb) {
        Ok(vm) => {
            let handle = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
            let idx = (handle - 1) as usize;
            let list = vm_list();
            while list.len() <= idx {
                list.push(None);
            }
            list[idx] = Some(vm);
            handle
        }
        Err(e) => {
            set_last_error(format!("{}", e));
            0
        }
    }
}

/// Destroy a VM and release all resources.
#[no_mangle]
pub extern "C" fn corevm_destroy(handle: u64) {
    if handle == 0 { return; }
    let idx = (handle - 1) as usize;
    if let Some(slot) = vm_list().get_mut(idx) {
        if let Some(mut vm) = slot.take() {
            vm.destroy_backend();
        }
    }
}

/// Get the last error message. Returns a pointer to a null-terminated UTF-8
/// string, or null if no error. The pointer is valid until the next FFI call.
#[no_mangle]
pub extern "C" fn corevm_last_error() -> *const u8 {
    unsafe {
        match &LAST_ERROR {
            Some(s) => s.as_ptr(),
            None => core::ptr::null(),
        }
    }
}

/// Get the length of the last error message (excluding null terminator).
/// Returns 0 if no error.
#[no_mangle]
pub extern "C" fn corevm_last_error_len() -> u32 {
    unsafe {
        match &LAST_ERROR {
            Some(s) => s.len() as u32,
            None => 0,
        }
    }
}

/// Reset the VM. Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn corevm_reset(handle: u64) -> i32 {
    match get_vm(handle) {
        Some(vm) => if vm.reset().is_ok() { 0 } else { -1 },
        None => -1,
    }
}

// ── vCPU management ─────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn corevm_create_vcpu(handle: u64, vcpu_id: u32) -> i32 {
    match get_vm(handle) {
        Some(vm) => match vm.create_vcpu(vcpu_id) {
            Ok(()) => 0,
            Err(e) => { set_last_error(format!("{}", e)); -1 }
        },
        None => { set_last_error("no VM handle".into()); -1 },
    }
}

#[no_mangle]
pub extern "C" fn corevm_destroy_vcpu(handle: u64, vcpu_id: u32) -> i32 {
    match get_vm(handle) {
        Some(vm) => if vm.destroy_vcpu(vcpu_id).is_ok() { 0 } else { -1 },
        None => -1,
    }
}

/// Run a vCPU until it exits. Fills `exit` with the exit reason.
#[no_mangle]
pub extern "C" fn corevm_run_vcpu(handle: u64, vcpu_id: u32, exit: *mut CExitReason) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    match vm.run_vcpu(vcpu_id) {
        Ok(reason) => {
            if !exit.is_null() {
                fill_exit(unsafe { &mut *exit }, reason);
            }
            0
        }
        Err(e) => { set_last_error(format!("{}", e)); -1 }
    }
}

// ── Register access ─────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn corevm_get_vcpu_regs(handle: u64, vcpu_id: u32, regs: *mut VcpuRegs) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if regs.is_null() { return -1; }
    match vm.get_vcpu_regs(vcpu_id) {
        Ok(r) => { unsafe { *regs = r; } 0 }
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn corevm_set_vcpu_regs(handle: u64, vcpu_id: u32, regs: *const VcpuRegs) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if regs.is_null() { return -1; }
    if vm.set_vcpu_regs(vcpu_id, unsafe { &*regs }).is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn corevm_get_vcpu_sregs(handle: u64, vcpu_id: u32, sregs: *mut VcpuSregs) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if sregs.is_null() { return -1; }
    match vm.get_vcpu_sregs(vcpu_id) {
        Ok(s) => { unsafe { *sregs = s; } 0 }
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn corevm_set_vcpu_sregs(handle: u64, vcpu_id: u32, sregs: *const VcpuSregs) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => { set_last_error("no VM handle".into()); return -1 } };
    if sregs.is_null() { set_last_error("null sregs".into()); return -1; }
    match vm.set_vcpu_sregs(vcpu_id, unsafe { &*sregs }) {
        Ok(()) => 0,
        Err(e) => { set_last_error(format!("{}", e)); -1 }
    }
}

// ── Interrupt injection ─────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn corevm_inject_interrupt(handle: u64, vcpu_id: u32, vector: u8) -> i32 {
    match get_vm(handle) {
        Some(vm) => if vm.inject_interrupt(vcpu_id, vector).is_ok() { 0 } else { -1 },
        None => -1,
    }
}

/// Cancel a running WHvRunVirtualProcessor call, causing it to return.
/// Safe to call from any thread — uses atomic globals, no VM registry access.
#[no_mangle]
pub extern "C" fn corevm_cancel_vcpu(_handle: u64, vcpu_id: u32) -> i32 {
    #[cfg(feature = "windows")]
    {
        crate::backend::whp::cancel_vcpu_global(vcpu_id)
    }
    #[cfg(not(feature = "windows"))]
    { let _ = (_handle, vcpu_id); 0 }
}

/// Advance the PIT timer by `ticks` clock cycles.
/// If channel 0 fires, raises IRQ0 on the PIC and injects the resulting
/// interrupt vector into vCPU 0. Returns the number of IRQ0 fires.
#[no_mangle]
pub extern "C" fn corevm_pit_advance(handle: u64, ticks: u32) -> u32 {
    match get_vm(handle) {
        Some(vm) => {
            let fires = if let Some(pit) = vm.pit_mut() {
                pit.advance(ticks)
            } else {
                return 0;
            };
            if fires > 0 && !vm.pic_ptr.is_null() {
                let pic = unsafe { &mut *vm.pic_ptr };
                pic.raise_irq(0);
                // Only raise IRR here, don't inject. Injection is centralized
                // in poll_irqs to avoid overwriting PendingInterruption (WHP
                // can only hold one pending interrupt at a time).
            }
            fires
        }
        None => 0,
    }
}

/// Return PIT channel 0 debug info: mode | (enabled << 8) | (output << 9) | (current << 16)
#[no_mangle]
pub extern "C" fn corevm_pit_debug(handle: u64) -> u64 {
    match get_vm(handle) {
        Some(vm) => {
            if let Some(pit) = vm.pit_mut() {
                let ch = &pit.channels[0];
                (ch.mode as u64)
                    | ((ch.enabled as u64) << 8)
                    | ((ch.output as u64) << 9)
                    | ((ch.current as u64) << 16)
                    | ((ch.count as u64) << 32)
            } else { 0 }
        }
        None => 0,
    }
}

/// Poll all device IRQ sources and inject any pending interrupts.
///
/// Checks PS/2 keyboard (IRQ 1) and mouse (IRQ 12). If a device has pending
/// data, raises the corresponding IRQ on the PIC and injects the resulting
/// interrupt vector into vCPU 0. Returns the number of interrupts injected.
#[no_mangle]
pub extern "C" fn corevm_poll_irqs(handle: u64) -> u32 {
    match get_vm(handle) {
        Some(vm) => {
            let mut injected = 0u32;
            if vm.pic_ptr.is_null() {
                return 0;
            }
            // PS/2 keyboard → IRQ 1, mouse → IRQ 12
            // Only raise when new data entered the output buffer (irq_needed),
            // not every time OUTPUT_FULL is set — avoids duplicate deliveries.
            if let Some(ps2) = vm.ps2() {
                if ps2.irq_needed {
                    ps2.irq_needed = false;
                    let is_mouse = (ps2.status & 0x20) != 0;
                    let irq = if is_mouse { 12 } else { 1 };
                    let pic = unsafe { &mut *vm.pic_ptr };
                    pic.raise_irq(irq);
                }
            }
            // Inject ONE pending PIC interrupt (highest priority),
            // but ONLY if the guest has interrupts enabled (RFLAGS.IF=1).
            // PendingInterruption requires IF=1.
            {
                let if_set = vm.get_vcpu_regs(0)
                    .map(|r| r.rflags & 0x200 != 0)
                    .unwrap_or(false);
                if if_set {
                    let pic = unsafe { &mut *vm.pic_ptr };
                    if let Some(vector) = pic.get_interrupt_vector() {
                        if vm.inject_interrupt(0, vector).is_ok() {
                            let irq = pic.irq_for_vector(vector).unwrap_or(0);
                            pic.lower_irq(irq);
                            injected += 1;
                        }
                    }
                }
            }
            injected
        }
        None => 0,
    }
}

/// Debug: return PIC master state as packed u32.
/// bits 0-7: IRR, 8-15: IMR, 16-23: ISR, 24: icw_step>0
#[no_mangle]
pub extern "C" fn corevm_pic_debug(handle: u64) -> u32 {
    match get_vm(handle) {
        Some(vm) => {
            if vm.pic_ptr.is_null() { return 0xDEAD; }
            let pic = unsafe { &*vm.pic_ptr };
            (pic.master.irr as u32)
                | ((pic.master.imr as u32) << 8)
                | ((pic.master.isr as u32) << 16)
                | if pic.master.icw_step > 0 { 1 << 24 } else { 0 }
        }
        None => 0xDEAD,
    }
}

/// Poll LAPIC timer (TSC-based). Injects interrupt if timer fired and IF=1.
/// Returns the vector injected (>0) or 0.
/// NOTE: In XApic mode (WHP), the LAPIC timer is handled internally by WHP.
/// This function is only active for the software LAPIC path.
#[no_mangle]
pub extern "C" fn corevm_lapic_timer_advance(handle: u64, _ticks: u64) -> u32 {
    #[cfg(feature = "windows")]
    { let _ = handle; return 0; } // XApic mode: WHP handles LAPIC timer

    #[cfg(not(feature = "windows"))]
    match get_vm(handle) {
        Some(vm) => {
            vm.backend.lapic.poll_timer();
            if let Some(vector) = vm.backend.lapic.take_timer_irq() {
                if vm.get_vcpu_regs(0)
                    .map(|r| r.rflags & 0x200 != 0)
                    .unwrap_or(false)
                {
                    if vm.inject_interrupt(0, vector).is_ok() {
                        return vector as u32;
                    }
                }
                vm.backend.lapic.timer_irq_pending = true;
                let _ = vm.request_interrupt_window(0, true);
            }
            0
        }
        None => 0,
    }
}

/// Debug: return LAPIC timer state.
/// Returns [armed:1|pending:1|divide:8|mode:2|vec:8|masked:1] in low bits,
/// and writes initial_count and current_count to out pointers.
#[no_mangle]
pub extern "C" fn corevm_lapic_debug(handle: u64, out_initial: *mut u32, out_current: *mut u32, out_lvt: *mut u32) -> u32 {
    match get_vm(handle) {
        Some(vm) => {
            let lapic = &vm.backend.lapic;
            if !out_initial.is_null() { unsafe { *out_initial = lapic.timer_initial; } }
            if !out_current.is_null() { unsafe { *out_current = lapic.current_count(); } }
            let lvt = lapic.regs[0x32]; // LVT Timer
            if !out_lvt.is_null() { unsafe { *out_lvt = lvt; } }
            let armed = lapic.timer_armed as u32;
            let pending = lapic.timer_irq_pending as u32;
            armed | (pending << 1) | (lapic.timer_divide << 2)
        }
        None => 0xDEAD,
    }
}

/// Inject an exception. Pass `error_code` < 0 for no error code.
#[no_mangle]
pub extern "C" fn corevm_inject_exception(handle: u64, vcpu_id: u32, vector: u8, error_code: i64) -> i32 {
    let ec = if error_code < 0 { None } else { Some(error_code as u32) };
    match get_vm(handle) {
        Some(vm) => if vm.inject_exception(vcpu_id, vector, ec).is_ok() { 0 } else { -1 },
        None => -1,
    }
}

#[no_mangle]
pub extern "C" fn corevm_inject_nmi(handle: u64, vcpu_id: u32) -> i32 {
    match get_vm(handle) {
        Some(vm) => if vm.inject_nmi(vcpu_id).is_ok() { 0 } else { -1 },
        None => -1,
    }
}

#[no_mangle]
pub extern "C" fn corevm_request_interrupt_window(handle: u64, vcpu_id: u32, enable: u8) -> i32 {
    match get_vm(handle) {
        Some(vm) => if vm.request_interrupt_window(vcpu_id, enable != 0).is_ok() { 0 } else { -1 },
        None => -1,
    }
}

#[no_mangle]
pub extern "C" fn corevm_set_cpuid(handle: u64, entries: *const CpuidEntry, count: u32) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if entries.is_null() && count > 0 { return -1; }
    let slice = if count > 0 {
        unsafe { core::slice::from_raw_parts(entries, count as usize) }
    } else {
        &[]
    };
    if vm.set_cpuid(slice).is_ok() { 0 } else { -1 }
}

// ── Memory ──────────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn corevm_set_memory_region(
    handle: u64, slot: u32, guest_phys: u64, size: u64, host_ptr: *mut u8,
) -> i32 {
    match get_vm(handle) {
        Some(vm) => if vm.set_memory_region(slot, guest_phys, size, host_ptr).is_ok() { 0 } else { -1 },
        None => -1,
    }
}

#[no_mangle]
pub extern "C" fn corevm_read_phys(handle: u64, addr: u64, buf: *mut u8, len: u32) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if buf.is_null() { return -1; }
    let slice = unsafe { core::slice::from_raw_parts_mut(buf, len as usize) };
    if vm.read_phys(addr, slice).is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn corevm_write_phys(handle: u64, addr: u64, buf: *const u8, len: u32) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if buf.is_null() { return -1; }
    let slice = unsafe { core::slice::from_raw_parts(buf, len as usize) };
    if vm.write_phys(addr, slice).is_ok() { 0 } else { -1 }
}

#[no_mangle]
pub extern "C" fn corevm_load_binary(handle: u64, guest_phys: u64, data: *const u8, len: u32) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if data.is_null() && len > 0 { return -1; }
    let slice = if len > 0 {
        unsafe { core::slice::from_raw_parts(data, len as usize) }
    } else {
        &[]
    };
    if vm.load_binary(guest_phys, slice).is_ok() { 0 } else { -1 }
}

// ── Hardware support ────────────────────────────────────────────────────────

/// Returns 1 if hardware virtualization is available, 0 otherwise.
#[no_mangle]
pub extern "C" fn corevm_has_hw_support() -> i32 {
    #[cfg(feature = "linux")]
    {
        match crate::backend::kvm::KvmBackend::new() {
            Ok(mut b) => { b.destroy(); 1 }
            Err(_) => 0,
        }
    }
    #[cfg(feature = "windows")]
    {
        match crate::backend::whp::WhpBackend::new(0) {
            Ok(_) => 1,
            Err(_) => 0,
        }
    }
    #[cfg(feature = "anyos")]
    { 1 }
}

// ── I/O and MMIO exit dispatch ──────────────────────────────────────────────

/// Dispatch a port I/O exit to the registered device handler.
///
/// Handle a bulk string I/O exit (REP INSB/OUTSB).
///
/// Performs the entire transfer in one call: reads/writes guest memory and
/// invokes the I/O handler for each byte. Updates guest registers afterward.
#[no_mangle]
pub extern "C" fn corevm_handle_string_io_exit(
    handle: u64, port: u16, is_write: u8, count: u64, gpa: u64,
    step: i64, instr_len: u64, addr_size: u8, access_size: u8,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    vm.handle_string_io(port, is_write != 0, count, gpa, step, instr_len, addr_size, access_size);
    0
}

/// For reads (`is_write`=0), `data` is filled with the result.
/// For writes (`is_write`=1), `data` contains the guest-written value.
#[no_mangle]
pub extern "C" fn corevm_handle_io_exit(
    handle: u64, port: u16, is_write: u8, size: u8, data: *mut u8,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if data.is_null() { return -1; }
    let buf = unsafe { core::slice::from_raw_parts_mut(data, size as usize) };
    vm.handle_io(port, is_write != 0, size, buf);

    // For IN (read), write response back to the backend.
    // KVM: write into kvm_run shared page.
    // anyOS/WHP: write response value into guest RAX via set_vcpu_regs.
    if is_write == 0 {
        #[cfg(feature = "linux")]
        {
            vm.set_io_response(0, buf);
        }
        #[cfg(not(feature = "linux"))]
        {
            // Write result into guest RAX
            if let Ok(mut regs) = vm.get_vcpu_regs(0) {
                let val = match size {
                    1 => buf[0] as u64,
                    2 => u16::from_le_bytes([buf[0], buf[1]]) as u64,
                    4 => u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as u64,
                    _ => 0,
                };
                regs.rax = (regs.rax & !((1u64 << (size as u64 * 8)) - 1)) | val;
                let _ = vm.set_vcpu_regs(0, &regs);
            }
        }
    }
    0
}

/// Dispatch an MMIO exit to the registered device handler.
///
/// For reads (`is_write`=0), `data` is filled with the result.
/// For writes (`is_write`=1), `data` contains the guest-written value.
/// `dest_reg` indicates which GP register receives the read result (0=RAX..7=RDI).
/// `instr_len` is the instruction length for RIP advancement (WHP reads only).
#[no_mangle]
pub extern "C" fn corevm_handle_mmio_exit(
    handle: u64, addr: u64, is_write: u8, size: u8, data: *mut u8,
    dest_reg: u8, instr_len: u8,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if data.is_null() { return -1; }
    let buf = unsafe { core::slice::from_raw_parts_mut(data, size as usize) };
    vm.handle_mmio(addr, is_write != 0, size, buf);

    // For MMIO reads, write response back to backend.
    if is_write == 0 {
        let val = match size {
            1 => buf[0] as u64,
            2 => u16::from_le_bytes([buf[0], buf[1]]) as u64,
            4 => u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as u64,
            8 => u64::from_le_bytes([buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7]]),
            _ => 0,
        };
        #[cfg(feature = "linux")]
        {
            let _ = val; // used by other paths
            vm.set_mmio_response(0, buf);
        }
        #[cfg(feature = "windows")]
        {
            // Store pending response — applied inside run_vcpu before next VM entry.
            vm.set_pending_mmio_read(val, dest_reg);
        }
        #[cfg(not(any(feature = "linux", feature = "windows")))]
        {
            // anyOS or other: set register directly
            if let Ok(mut regs) = vm.get_vcpu_regs(0) {
                if instr_len > 0 {
                    regs.rip += instr_len as u64;
                }
                match dest_reg {
                    0 => regs.rax = val,
                    1 => regs.rcx = val,
                    2 => regs.rdx = val,
                    3 => regs.rbx = val,
                    4 => regs.rsp = val,
                    5 => regs.rbp = val,
                    6 => regs.rsi = val,
                    7 => regs.rdi = val,
                    _ => regs.rax = val,
                }
                let _ = vm.set_vcpu_regs(0, &regs);
            }
        }
    }
    0
}

// ── Standard device setup ───────────────────────────────────────────────────

/// Add a named file to the fw_cfg device (e.g., "vgaroms/vgabios.bin").
#[no_mangle]
pub extern "C" fn corevm_fw_cfg_add_file(
    handle: u64, name: *const u8, name_len: u32, data: *const u8, data_len: u32,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if name.is_null() || data.is_null() || vm.fw_cfg_ptr.is_null() { return -1; }
    let name_slice = unsafe { core::slice::from_raw_parts(name, name_len as usize) };
    let name_str = match core::str::from_utf8(name_slice) {
        Ok(s) => s,
        Err(_) => return -1,
    };
    let data_slice = unsafe { core::slice::from_raw_parts(data, data_len as usize) };
    let fw_cfg = unsafe { &mut *vm.fw_cfg_ptr };
    fw_cfg.add_file(name_str, data_slice.to_vec());
    0
}

/// Register all standard chipset devices into the VM.
#[no_mangle]
pub extern "C" fn corevm_setup_standard_devices(handle: u64) -> i32 {
    match get_vm(handle) {
        Some(vm) => { vm.setup_standard_devices(); 0 }
        None => -1,
    }
}

/// Generate and register ACPI tables via fw_cfg.
/// Must be called after corevm_setup_standard_devices (needs fw_cfg device).
#[no_mangle]
pub extern "C" fn corevm_setup_acpi_tables(handle: u64) -> i32 {
    fn dbg(msg: &str) {
        #[cfg(feature = "windows")]
        {
            use std::io::Write;
            let path = std::env::var("TEMP")
                .map(|t| std::format!("{}\\acpi_debug.log", t))
                .unwrap_or_else(|_| std::string::String::from("acpi_debug.log"));
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
                let _ = writeln!(f, "{}", msg);
            }
        }
    }
    dbg("corevm_setup_acpi_tables called");
    let vm = match get_vm(handle) {
        Some(v) => v,
        None => { dbg("get_vm returned None"); return -1; }
    };
    if vm.fw_cfg_ptr.is_null() {
        dbg("fw_cfg_ptr is NULL");
        return -1;
    }
    let fw_cfg = unsafe { &mut *vm.fw_cfg_ptr };

    let (rsdp, tables, loader) = crate::devices::acpi_tables::generate_acpi_tables();
    dbg(&std::format!("Generated: rsdp={} tables={} loader={} bytes", rsdp.len(), tables.len(), loader.len()));

    fw_cfg.add_file("etc/acpi/rsdp", rsdp);
    fw_cfg.add_file("etc/acpi/tables", tables);
    fw_cfg.add_file("etc/table-loader", loader);
    dbg("ACPI files registered in fw_cfg");
    0
}

// ── Device-specific FFI ─────────────────────────────────────────────────────

/// Set up the E1000 NIC with the given MAC address (6 bytes).
#[no_mangle]
pub extern "C" fn corevm_setup_e1000(handle: u64, mac: *const u8) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if mac.is_null() { return -1; }
    let m: [u8; 6] = unsafe { [*mac, *mac.add(1), *mac.add(2), *mac.add(3), *mac.add(4), *mac.add(5)] };
    let e1000 = crate::devices::e1000::E1000::new(m);
    vm.memory.add_mmio(0xFEBC_0000, 0x2_0000, Box::new(e1000));
    0
}

/// Base address for the AHCI MMIO catch-all region.
/// Covers 0xFE000000-0xFEBFFFFF (12MB) to catch BAR5 wherever SeaBIOS puts it.
const AHCI_MMIO_BASE: u64 = 0xFE00_0000;
const AHCI_MMIO_SIZE: u64 = 0xC0_0000;

/// MMIO wrapper that forwards accesses to AHCI based on current PCI BAR5 address.
/// Registered over a wide range; only responds when the access falls within BAR5.
struct AhciPciMmioWrapper {
    ahci: *mut crate::devices::ahci::Ahci,
    pci_bus: *mut crate::devices::bus::PciBus,
}

unsafe impl Send for AhciPciMmioWrapper {}

impl AhciPciMmioWrapper {
    fn bar5_base(&self) -> u64 {
        if self.pci_bus.is_null() { return 0; }
        let bus = unsafe { &mut *self.pci_bus };
        // Read BAR5 (offset 0x24) from device 00:03.0
        let val = bus.mmcfg_read(0, 3, 0, 0x24, 4);
        val & 0xFFFFFFF0 // mask type bits
    }
}

impl crate::memory::mmio::MmioHandler for AhciPciMmioWrapper {
    fn read(&mut self, offset: u64, size: u8) -> crate::error::Result<u64> {
        let bar_base = self.bar5_base();
        // offset is relative to our MMIO registration base (0xFEBF_0000)
        let abs_addr = AHCI_MMIO_BASE + offset;
        if abs_addr < bar_base || abs_addr >= bar_base + 0x1000 {
            return Ok(0xFFFFFFFF); // not in BAR range
        }
        let ahci_offset = abs_addr - bar_base;
        let ahci = unsafe { &mut *self.ahci };
        ahci.read(ahci_offset, size)
    }

    fn write(&mut self, offset: u64, size: u8, val: u64) -> crate::error::Result<()> {
        let bar_base = self.bar5_base();
        let abs_addr = AHCI_MMIO_BASE + offset;
        if abs_addr < bar_base || abs_addr >= bar_base + 0x1000 {
            return Ok(()); // not in BAR range
        }
        let ahci_offset = abs_addr - bar_base;
        let ahci = unsafe { &mut *self.ahci };
        ahci.write(ahci_offset, size, val)
    }
}

/// Set up the AHCI SATA controller with the given number of ports.
#[no_mangle]
pub extern "C" fn corevm_setup_ahci(handle: u64, num_ports: u8) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    let ahci = Box::new(crate::devices::ahci::Ahci::new(num_ports));
    vm.ahci_ptr = &*ahci as *const crate::devices::ahci::Ahci as *mut crate::devices::ahci::Ahci;

    // Register wide MMIO range covering PCI allocation area.
    // The wrapper reads current BAR5 from PCI config to route accesses.
    let wrapper = Box::new(AhciPciMmioWrapper {
        ahci: vm.ahci_ptr,
        pci_bus: vm.pci_bus_ptr,
    });
    vm.memory.add_mmio(AHCI_MMIO_BASE, AHCI_MMIO_SIZE, wrapper);

    // Give AHCI access to guest RAM for DMA transfers.
    let (ram_ptr, ram_len) = vm.memory.ram_mut_ptr();
    unsafe { &mut *vm.ahci_ptr }.set_guest_memory(ram_ptr, ram_len);

    // Keep the AHCI Box alive by leaking it (wrapper uses raw pointer)
    core::mem::forget(ahci);

    // Register AHCI as a PCI device so SeaBIOS can discover it
    if !vm.pci_bus_ptr.is_null() {
        let pci_bus = unsafe { &mut *vm.pci_bus_ptr };
        let mut pci_dev = crate::devices::ahci::create_ahci_pci_device(AHCI_MMIO_BASE as u32);
        pci_dev.device = 3; // PCI device 00:03.0 (00:01.0 is ISA bridge)
        pci_bus.add_device(pci_dev);
    }
    0
}

/// Attach a disk image to an AHCI port.
#[no_mangle]
pub extern "C" fn corevm_ahci_attach_disk(handle: u64, port: u32, fd: i32, size: u64) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    match vm.ahci() {
        Some(ahci) => {
            ahci.attach_disk_fd(port as usize, fd, size, crate::devices::ahci::AhciDriveKind::AtaDisk);
            0
        }
        None => -1,
    }
}

/// Attach a CD-ROM image to an AHCI port.
#[no_mangle]
pub extern "C" fn corevm_ahci_attach_cdrom(handle: u64, port: u32, fd: i32, size: u64) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    match vm.ahci() {
        Some(ahci) => {
            ahci.attach_disk_fd(port as usize, fd, size, crate::devices::ahci::AhciDriveKind::AtapiCdrom);
            0
        }
        None => -1,
    }
}

/// Send input bytes to the serial port (COM1).
#[no_mangle]
pub extern "C" fn corevm_serial_send_input(handle: u64, data: *const u8, len: u32) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if data.is_null() && len > 0 { return -1; }
    match vm.serial() {
        Some(serial) => {
            let slice = if len > 0 {
                unsafe { core::slice::from_raw_parts(data, len as usize) }
            } else {
                &[]
            };
            serial.send_input(slice);
            0
        }
        None => -1,
    }
}

/// Take output bytes from the serial port. Returns number of bytes written to `buf`,
/// or -1 on error.
#[no_mangle]
pub extern "C" fn corevm_serial_take_output(handle: u64, buf: *mut u8, max_len: u32) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if buf.is_null() { return -1; }
    match vm.serial() {
        Some(serial) => {
            let output = serial.take_output();
            let copy_len = output.len().min(max_len as usize);
            if copy_len > 0 {
                unsafe {
                    core::ptr::copy_nonoverlapping(output.as_ptr(), buf, copy_len);
                }
            }
            copy_len as i32
        }
        None => -1,
    }
}

/// Drain buffered debug port (0x402) output. Returns number of bytes copied,
/// or -1 on error.
#[no_mangle]
pub extern "C" fn corevm_debug_port_take_output(handle: u64, buf: *mut u8, max_len: u32) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if buf.is_null() || vm.debug_port_ptr.is_null() { return -1; }
    let dbg = unsafe { &mut *vm.debug_port_ptr };
    let output = dbg.take_output();
    let copy_len = output.len().min(max_len as usize);
    if copy_len > 0 {
        unsafe { core::ptr::copy_nonoverlapping(output.as_ptr(), buf, copy_len); }
    }
    copy_len as i32
}

/// Send a PS/2 key press scancode.
#[no_mangle]
pub extern "C" fn corevm_ps2_key_press(handle: u64, scancode: u8) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    match vm.ps2() {
        Some(ps2) => { ps2.key_press(scancode); 0 }
        None => -1,
    }
}

/// Send a PS/2 key release scancode.
#[no_mangle]
pub extern "C" fn corevm_ps2_key_release(handle: u64, scancode: u8) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    match vm.ps2() {
        Some(ps2) => { ps2.key_release(scancode); 0 }
        None => -1,
    }
}

/// Send a PS/2 mouse movement.
#[no_mangle]
pub extern "C" fn corevm_ps2_mouse_move(handle: u64, dx: i16, dy: i16, buttons: u8) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    match vm.ps2() {
        Some(ps2) => { ps2.mouse_move(dx, dy, buttons); 0 }
        None => -1,
    }
}

/// Get a pointer to the VGA framebuffer pixel data.
/// Sets `*out_ptr` and `*out_len`. Returns 0 on success, -1 on error.
/// Returns len=0 when in text mode (caller should use get_text_buffer instead).
#[no_mangle]
pub extern "C" fn corevm_vga_get_framebuffer(
    handle: u64, out_ptr: *mut *const u8, out_len: *mut u32,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if out_ptr.is_null() || out_len.is_null() { return -1; }
    match vm.svga() {
        Some(svga) => {
            // Only return framebuffer in graphics modes; in text mode return empty
            // so the caller falls through to get_text_buffer.
            if svga.mode == crate::devices::svga::VgaMode::Text80x25 {
                unsafe { *out_ptr = core::ptr::null(); *out_len = 0; }
                return 0;
            }
            let fb = svga.get_framebuffer();
            unsafe {
                *out_ptr = fb.as_ptr();
                *out_len = fb.len() as u32;
            }
            0
        }
        None => -1,
    }
}

/// Get the current VGA display mode dimensions.
/// Returns 0 on success and fills out_width, out_height, out_bpp.
/// Returns 1 if in text mode (out_width=80, out_height=25, out_bpp=0).
/// Returns -1 on error.
#[no_mangle]
pub extern "C" fn corevm_vga_get_mode(
    handle: u64, out_width: *mut u32, out_height: *mut u32, out_bpp: *mut u8,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if out_width.is_null() || out_height.is_null() || out_bpp.is_null() { return -1; }
    match vm.svga() {
        Some(svga) => {
            match &svga.mode {
                crate::devices::svga::VgaMode::Text80x25 => {
                    unsafe { *out_width = 80; *out_height = 25; *out_bpp = 0; }
                    1
                }
                crate::devices::svga::VgaMode::Graphics320x200x256 => {
                    unsafe { *out_width = 320; *out_height = 200; *out_bpp = 8; }
                    0
                }
                crate::devices::svga::VgaMode::Graphics640x480x16 => {
                    unsafe { *out_width = 640; *out_height = 480; *out_bpp = 4; }
                    0
                }
                crate::devices::svga::VgaMode::LinearFramebuffer { width, height, bpp } => {
                    unsafe { *out_width = *width; *out_height = *height; *out_bpp = *bpp; }
                    0
                }
            }
        }
        None => -1,
    }
}

/// Get the current VGA linear framebuffer physical address from PCI BAR0.
/// SeaBIOS may relocate BARs during PCI enumeration, so this can differ
/// from the initial 0xFD000000.  Returns the BAR0 address, or 0 on error.
#[no_mangle]
pub extern "C" fn corevm_vga_get_lfb_addr(handle: u64) -> u64 {
    let vm = match get_vm(handle) { Some(v) => v, None => return 0 };
    if vm.pci_bus_ptr.is_null() { return 0; }
    let pci_bus = unsafe { &*vm.pci_bus_ptr };
    // VGA device is at bus 0, device 2, function 0
    for dev in &pci_bus.devices {
        if dev.bus == 0 && dev.device == 2 && dev.function == 0 {
            // BAR0 at offset 0x10 (32-bit MMIO BAR)
            let bar0 = (dev.config_space[0x10] as u32)
                | ((dev.config_space[0x11] as u32) << 8)
                | ((dev.config_space[0x12] as u32) << 16)
                | ((dev.config_space[0x13] as u32) << 24);
            // Mask off type bits (bits 0-3 for MMIO BAR)
            return (bar0 & 0xFFFF_FFF0) as u64;
        }
    }
    0
}

/// Get a pointer to the VGA text buffer (array of u16: char+attr pairs).
/// Sets `*out_ptr` and `*out_len` (number of u16 entries). Returns 0 on success, -1 on error.
/// In hardware-virt mode, syncs the text buffer from guest RAM first.
#[no_mangle]
pub extern "C" fn corevm_vga_get_text_buffer(
    handle: u64, out_ptr: *mut *const u16, out_len: *mut u32,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if out_ptr.is_null() || out_len.is_null() { return -1; }

    // In hardware-virt mode (KVM/WHP), sync text buffer from guest RAM
    // since VGA memory writes bypass the MMIO handler.
    let (ram_ptr, ram_size) = vm.memory.ram_ptr();
    if ram_size > 0xB8000 + 80 * 25 * 2 {
        if let Some(svga) = vm.svga_mut() {
            unsafe { svga.sync_text_buffer_from_ram(ram_ptr); }
        }
    }

    match vm.svga() {
        Some(svga) => {
            let tb = svga.get_text_buffer();
            unsafe {
                *out_ptr = tb.as_ptr();
                *out_len = tb.len() as u32;
            }
            0
        }
        None => -1,
    }
}
