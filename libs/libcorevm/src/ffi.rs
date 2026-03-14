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
    pub io_count: u32,
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
        VmExitReason::IoIn { port, size, count } => {
            e.reason = 0; e.port = port; e.size = size; e.io_count = count;
        }
        VmExitReason::IoOut { port, size, data, count } => {
            e.reason = 1; e.port = port; e.size = size; e.data_u32 = data; e.io_count = count;
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
        VmExitReason::Cancelled => e.reason = 13,
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

/// Read the in-kernel LAPIC register page (1024 bytes).
/// Only available on Linux/KVM. Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn corevm_get_lapic(handle: u64, vcpu_id: u32, buf: *mut u8) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if buf.is_null() { return -1; }
    #[cfg(feature = "linux")]
    {
        match vm.backend.get_lapic(vcpu_id) {
            Ok(data) => { unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf, 1024); } 0 }
            Err(_) => -1,
        }
    }
    #[cfg(not(feature = "linux"))]
    { -1 }
}

/// Read the in-kernel irqchip state (512 bytes).
/// chip_id: 0=PIC master, 1=PIC slave, 2=IOAPIC.
#[no_mangle]
pub extern "C" fn corevm_get_irqchip(handle: u64, chip_id: u32, buf: *mut u8) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if buf.is_null() { return -1; }
    #[cfg(feature = "linux")]
    {
        match vm.backend.get_irqchip(chip_id) {
            Ok(data) => {
                let len = data.len().min(512);
                unsafe { core::ptr::copy_nonoverlapping(data.as_ptr(), buf, len); }
                0
            }
            Err(_) => -1,
        }
    }
    #[cfg(not(feature = "linux"))]
    { -1 }
}

// ── Interrupt injection ─────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn corevm_inject_interrupt(handle: u64, vcpu_id: u32, vector: u8) -> i32 {
    match get_vm(handle) {
        Some(vm) => if vm.inject_interrupt(vcpu_id, vector).is_ok() { 0 } else { -1 },
        None => -1,
    }
}

/// Cancel a running vCPU, causing run_vcpu to return with Cancelled.
/// Safe to call from any thread — uses atomic globals, no VM registry access.
#[no_mangle]
pub extern "C" fn corevm_cancel_vcpu(_handle: u64, vcpu_id: u32) -> i32 {
    #[cfg(feature = "windows")]
    { return crate::backend::whp::cancel_vcpu_global(vcpu_id); }
    #[cfg(feature = "linux")]
    { return crate::backend::kvm::cancel_vcpu_kvm(vcpu_id); }
    #[cfg(feature = "anyos")]
    { return 0; }
    #[allow(unreachable_code)]
    { let _ = (_handle, vcpu_id); 0 }
}

/// Advance the PIT timer by `ticks` clock cycles.
/// If channel 0 fires, raises IRQ0 on the PIC and injects the resulting
/// interrupt vector into vCPU 0. Returns the number of IRQ0 fires.
///
/// On Linux/KVM: the in-kernel PIT handles channel 0 IRQ 0 automatically,
/// but we still need to tick the userspace PIT for channel 2 (used by
/// port 0x61 for BIOS/bootloader delay loops).
#[no_mangle]
pub extern "C" fn corevm_pit_advance(handle: u64, ticks: u32) -> u32 {
    #[cfg(feature = "linux")]
    {
        // Sync channel 2 config from in-kernel PIT, then tick userspace PIT.
        // The in-kernel PIT handles port 0x40-0x43 writes (mode, count config),
        // but port 0x61 reads the userspace PIT's channel 2 output.
        match get_vm(handle) {
            Some(vm) => {
                let (count, _status, mode, gate, ret) = vm.backend.get_pit2_debug();
                if ret >= 0 {
                    if let Some(pit) = vm.pit_mut() {
                        let ch = &mut pit.channels[2];
                        // Sync configuration from in-kernel PIT to userspace PIT
                        // only if config changed (mode or count written by guest)
                        if mode < 6 && (!ch.enabled || ch.mode != mode || ch.count != count as u16) {
                            ch.mode = mode;
                            ch.count = count as u16;
                            ch.current = count as u16;
                            ch.gate = gate != 0;
                            ch.enabled = true;
                            // Modes 2,3 start with output HIGH; mode 0 starts LOW
                            ch.output = matches!(mode, 2 | 3);
                        }
                        ch.gate = gate != 0;
                        // Tick channel 2 only (not channel 0 — in-kernel PIT does that)
                        for _ in 0..ticks {
                            ch.tick();
                        }
                    }
                }
                0
            }
            None => 0,
        }
    }

    #[cfg(not(feature = "linux"))]
    match get_vm(handle) {
        Some(vm) => {
            let fires = if let Some(pit) = vm.pit_mut() {
                pit.advance(ticks)
            } else {
                return 0;
            };
            if fires > 0 {
                if !vm.pic_ptr.is_null() {
                    let pic = unsafe { &mut *vm.pic_ptr };
                    pic.raise_irq(0);
                }
                // Also route through IOAPIC (pin 2 = remapped IRQ 0 per MADT)
                #[cfg(feature = "windows")]
                {
                    vm.backend.ioapic_set_irq(2, true);
                    vm.backend.ioapic_set_irq(2, false);
                }
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

/// Advance the CMOS RTC periodic timer by `ticks_32768` ticks of the
/// 32.768 kHz base clock. Returns 1 if IRQ 8 should fire, 0 otherwise.
/// On KVM, raises IRQ 8 via KVM_IRQ_LINE when the periodic timer fires.
#[no_mangle]
pub extern "C" fn corevm_cmos_advance(handle: u64, ticks_32768: u64) -> u32 {
    match get_vm(handle) {
        Some(vm) => {
            let fired = if let Some(cmos) = vm.cmos_mut() {
                cmos.advance(ticks_32768)
            } else {
                return 0;
            };
            if fired {
                #[cfg(feature = "linux")]
                {
                    let _ = vm.backend.set_irq_line(8, true);
                    let _ = vm.backend.set_irq_line(8, false);
                }
                #[cfg(not(feature = "linux"))]
                {
                    if !vm.pic_ptr.is_null() {
                        let pic = unsafe { &mut *vm.pic_ptr };
                        pic.raise_irq(8);
                    }
                    #[cfg(feature = "windows")]
                    {
                        vm.backend.ioapic_set_irq(8, true);
                        vm.backend.ioapic_set_irq(8, false);
                    }
                }
                1
            } else {
                0
            }
        }
        None => 0,
    }
}

/// Poll all device IRQ sources and inject any pending interrupts.
///
/// Checks PS/2 keyboard (IRQ 1) and mouse (IRQ 12). Proactively drains
/// device buffers and fires IRQs — does NOT rely on the `irq_needed` flag
/// alone, since it may be set from a different thread (UI thread) without
/// memory barriers.
///
/// On Linux/KVM: uses KVM_IRQ_LINE to signal the in-kernel irqchip.
/// The in-kernel PIC/IOAPIC/LAPIC handles vector injection automatically.
#[no_mangle]
pub extern "C" fn corevm_poll_irqs(handle: u64) -> u32 {
    match get_vm(handle) {
        Some(vm) => {
            let mut injected = 0u32;

            // Drain pending mouse events from the thread-safe queue into
            // the PS/2 controller (single-threaded context — no races).
            // We drain into a local vec first to avoid borrow conflicts
            // between pending_mouse (immutable borrow of vm) and ps2() (mutable).
            #[cfg(feature = "std")]
            {
                let events: alloc::vec::Vec<(i16, i16, u8)> = {
                    if let Ok(mut queue) = vm.pending_mouse.lock() {
                        queue.drain(..).collect()
                    } else {
                        alloc::vec::Vec::new()
                    }
                };
                if !events.is_empty() {
                    if let Some(ps2) = vm.ps2() {
                        for (dx, dy, buttons) in events {
                            ps2.mouse_move(dx, dy, buttons);
                        }
                    }
                }
            }

            // PS/2 keyboard → IRQ 1, mouse → IRQ 12
            if let Some(ps2) = vm.ps2() {
                // Proactively try to fill output buffer from device buffers.
                ps2.try_fill_output();

                // Fire IRQ if output buffer has data ready for the guest.
                // Check the actual buffer state rather than relying solely
                // on irq_needed (which may not be visible cross-thread).
                let need_irq = ps2.irq_needed
                    || (ps2.status & 0x01 != 0); // STATUS_OUTPUT_FULL
                if need_irq {
                    ps2.irq_needed = false;
                    // Use IRQ 12 for mouse data, IRQ 1 for keyboard data.
                    // Linux's i8042 driver registers separate handlers for
                    // IRQ 1 (KBD) and IRQ 12 (AUX). During AUX detection,
                    // i8042_check_aux registers a test handler on IRQ 12
                    // and expects IRQ 12 to fire for loopback data.
                    // KVM_IRQ_LINE signals BOTH PIC and IOAPIC, so even if
                    // IOAPIC pin 12 is masked, the PIC still delivers it.
                    let is_mouse_data = ps2.status & 0x20 != 0; // STATUS_MOUSE_DATA
                    let irq: u8 = if is_mouse_data { 12 } else { 1 };
                    #[cfg(feature = "linux")]
                    {
                        let _ = vm.backend.set_irq_line(irq as u32, true);
                        let _ = vm.backend.set_irq_line(irq as u32, false);
                        injected += 1;
                    }
                    #[cfg(not(feature = "linux"))]
                    {
                        if !vm.pic_ptr.is_null() {
                            let pic = unsafe { &mut *vm.pic_ptr };
                            pic.raise_irq(irq);
                        }
                        #[cfg(feature = "windows")]
                        {
                            vm.backend.ioapic_set_irq(irq, true);
                            vm.backend.ioapic_set_irq(irq, false);
                        }
                    }
                }
            }

            // Serial COM1 IRQ → IRQ 4 (edge-triggered)
            // Fire when the serial device has a new interrupt pending (THRE or RXDA).
            if let Some(serial) = vm.serial() {
                if serial.irq_pending {
                    serial.irq_pending = false;
                    #[cfg(feature = "linux")]
                    {
                        let _ = vm.backend.set_irq_line(4, true);
                        let _ = vm.backend.set_irq_line(4, false);
                        injected += 1;
                    }
                    #[cfg(not(feature = "linux"))]
                    {
                        if !vm.pic_ptr.is_null() {
                            let pic = unsafe { &mut *vm.pic_ptr };
                            pic.raise_irq(4);
                        }
                    }
                }
            }

            // AHCI IRQ → IRQ 11 (level-triggered)
            if !vm.ahci_ptr.is_null() {
                let ahci = unsafe { &mut *vm.ahci_ptr };
                let want_asserted = ahci.irq_raised();
                #[cfg(feature = "linux")]
                {
                    if want_asserted && !vm.ahci_irq_asserted {
                        let _ = vm.backend.set_irq_line(11, true);
                        vm.ahci_irq_asserted = true;
                        injected += 1;
                    } else if !want_asserted && vm.ahci_irq_asserted {
                        let _ = vm.backend.set_irq_line(11, false);
                        vm.ahci_irq_asserted = false;
                    }
                }
                #[cfg(not(feature = "linux"))]
                {
                    if want_asserted {
                        ahci.clear_irq();
                        if !vm.pic_ptr.is_null() {
                            let pic = unsafe { &mut *vm.pic_ptr };
                            pic.raise_irq(11);
                        }
                        #[cfg(feature = "windows")]
                        {
                            vm.backend.ioapic_set_irq(11, true);
                            vm.backend.ioapic_set_irq(11, false);
                        }
                    }
                }
            }

            // HPET Timer 0 → IRQ (edge-triggered pulse on KVM)
            if !vm.hpet_ptr.is_null() {
                let hpet = unsafe { &mut *vm.hpet_ptr };
                if hpet.check_timer() {
                    let irq = hpet.timer0_irq();
                    #[cfg(feature = "linux")]
                    {
                        let _ = vm.backend.set_irq_line(irq, true);
                        let _ = vm.backend.set_irq_line(irq, false);
                        injected += 1;
                    }
                    #[cfg(not(feature = "linux"))]
                    {
                        if !vm.pic_ptr.is_null() {
                            let pic = unsafe { &mut *vm.pic_ptr };
                            pic.raise_irq(irq as u8);
                        }
                        #[cfg(feature = "windows")]
                        {
                            vm.backend.ioapic_set_irq(irq as u8, true);
                            vm.backend.ioapic_set_irq(irq as u8, false);
                        }
                    }
                }
            }

            // Software PIC injection (non-Linux only).
            // On KVM/Linux the in-kernel irqchip handles injection automatically.
            #[cfg(not(feature = "linux"))]
            if !vm.pic_ptr.is_null() {
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
                            if pic.get_interrupt_vector().is_some() {
                                let _ = vm.request_interrupt_window(0, true);
                            }
                        }
                    }
                } else {
                    let pic = unsafe { &*vm.pic_ptr };
                    if pic.get_interrupt_vector().is_some() {
                        let _ = vm.request_interrupt_window(0, true);
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
    #[cfg(not(feature = "windows"))]
    { let _ = (handle, _ticks); return 0; }

    #[cfg(feature = "windows")]
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
    #[cfg(not(feature = "windows"))]
    { let _ = (handle, out_initial, out_current, out_lvt); return 0; }

    #[cfg(feature = "windows")]
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

/// Returns the hardware virtualization type:
///   0 = none / not available
///   1 = Intel VT-x (VMX)
///   2 = AMD-V (SVM)
///
/// A return value != 0 means hardware virtualization is available.
#[no_mangle]
pub extern "C" fn corevm_has_hw_support() -> i32 {
    #[cfg(feature = "linux")]
    {
        return match crate::backend::kvm::KvmBackend::new() {
            Ok(mut b) => { b.destroy(); 1 }
            Err(_) => 0,
        };
    }
    #[cfg(feature = "windows")]
    {
        return match crate::backend::whp::WhpBackend::new(0) {
            Ok(_) => 1,
            Err(_) => 0,
        };
    }
    #[cfg(feature = "anyos")]
    {
        // Ask the kernel: 0=none, 1=VMX, 2=SVM.
        return unsafe { crate::backend::anyos::syscall_vm_hw_info() as i32 };
    }
    #[allow(unreachable_code)]
    0
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

/// Handle a KVM string I/O exit (REP INSB/OUTSB) with count > 1.
/// Loops count times, calling the IO port handler for each iteration.
/// Results are written directly to the kvm_run shared page.
#[no_mangle]
pub extern "C" fn corevm_complete_string_io(
    handle: u64, vcpu_id: u32, port: u16, is_write: u8, size: u8, count: u32,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    #[cfg(feature = "linux")]
    {
        if is_write != 0 {
            vm.complete_string_io_out(vcpu_id, port, size, count);
        } else {
            vm.complete_string_io_in(vcpu_id, port, size, count);
        }
    }
    #[cfg(not(feature = "linux"))]
    {
        let _ = (vm, vcpu_id, port, is_write, size, count);
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

/// Set up direct kernel boot via fw_cfg legacy selectors.
/// Parses a Linux bzImage, splits it into setup + kernel, computes addresses.
#[no_mangle]
pub extern "C" fn corevm_fw_cfg_set_kernel(
    handle: u64,
    kernel: *const u8, kernel_len: u32,
    initrd: *const u8, initrd_len: u32,
    cmdline: *const u8, cmdline_len: u32,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if vm.fw_cfg_ptr.is_null() { return -1; }
    let kernel_slice = if kernel.is_null() || kernel_len == 0 { &[] }
        else { unsafe { core::slice::from_raw_parts(kernel, kernel_len as usize) } };
    let initrd_slice = if initrd.is_null() || initrd_len == 0 { &[] }
        else { unsafe { core::slice::from_raw_parts(initrd, initrd_len as usize) } };
    let cmdline_slice = if cmdline.is_null() || cmdline_len == 0 { &[] }
        else { unsafe { core::slice::from_raw_parts(cmdline, cmdline_len as usize) } };
    let fw_cfg = unsafe { &mut *vm.fw_cfg_ptr };
    fw_cfg.set_kernel(kernel_slice, initrd_slice, cmdline_slice);
    0
}

/// Register all standard chipset devices into the VM.
#[no_mangle]
pub extern "C" fn corevm_setup_standard_devices(handle: u64) -> i32 {
    match get_vm(handle) {
        Some(vm) => {
            vm.setup_standard_devices();
            // Map VGA LFB as a hypervisor memory region for fast access.
            #[cfg(feature = "linux")]
            if let Err(_) = vm.setup_vga_lfb_mapping() {
                eprintln!("[corevm] Warning: failed to map VGA LFB as KVM region");
            }
            0
        }
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
    dbg(&alloc::format!("Generated: rsdp={} tables={} loader={} bytes", rsdp.len(), tables.len(), loader.len()));

    fw_cfg.add_file("etc/acpi/rsdp", rsdp);
    fw_cfg.add_file("etc/acpi/tables", tables);
    fw_cfg.add_file("etc/table-loader", loader);
    dbg("ACPI files registered in fw_cfg");
    0
}

// ── Device-specific FFI ─────────────────────────────────────────────────────

/// Set up the E1000 NIC with the given MAC address (6 bytes).
///
/// Registers the E1000 as:
/// - MMIO region at 0xF0000000 (128 KB) for register access
/// - PCI device 00:04.0 (Intel 82540EM, 8086:100E) so the guest can discover it
///
/// The MMIO base must NOT overlap with AHCI catch-all (0xFE000000-0xFEBFFFFF),
/// VGA LFB (0xE0000000-0xE7FFFFFF), IOAPIC (0xFEC00000), or LAPIC (0xFEE00000).
#[no_mangle]
pub extern "C" fn corevm_setup_e1000(handle: u64, mac: *const u8) -> i32 {
    const E1000_MMIO_BASE: u64 = 0xF000_0000;
    const E1000_MMIO_SIZE: u64 = 0x2_0000; // 128 KB

    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if mac.is_null() { return -1; }
    let m: [u8; 6] = unsafe { [*mac, *mac.add(1), *mac.add(2), *mac.add(3), *mac.add(4), *mac.add(5)] };
    let e1000 = crate::devices::e1000::E1000::new(m);
    vm.memory.add_mmio(E1000_MMIO_BASE, E1000_MMIO_SIZE, Box::new(e1000));

    // Register E1000 as a PCI device so the guest can discover it via PCI scan.
    if !vm.pci_bus_ptr.is_null() {
        let pci_bus = unsafe { &mut *vm.pci_bus_ptr };
        // Intel 82540EM: vendor 8086, device 100E, class 02 (Network), subclass 00 (Ethernet)
        let mut pci_dev = crate::devices::bus::PciDevice::new(0x8086, 0x100E, 0x02, 0x00, 0x00);
        pci_dev.device = 4; // PCI slot 00:04.0
        // BAR0: MMIO at 0xF0000000, size 128 KB
        pci_dev.set_bar(0, E1000_MMIO_BASE as u32, E1000_MMIO_SIZE as u32, true);
        // Interrupt: IRQ 11, pin INTA
        pci_dev.set_interrupt(11, 1);
        // Subsystem ID (common for 82540EM)
        pci_dev.set_subsystem(0x8086, 0x001E);
        pci_bus.add_device(pci_dev);
    }
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
        let abs_addr = AHCI_MMIO_BASE + offset;
        if abs_addr < bar_base || abs_addr >= bar_base + 0x1000 {
            return Ok(0xFFFFFFFF);
        }
        let ahci_offset = abs_addr - bar_base;
        let ahci = unsafe { &mut *self.ahci };
        ahci.read(ahci_offset, size)
    }

    fn write(&mut self, offset: u64, size: u8, val: u64) -> crate::error::Result<()> {
        let bar_base = self.bar5_base();
        let abs_addr = AHCI_MMIO_BASE + offset;
        if abs_addr < bar_base || abs_addr >= bar_base + 0x1000 {
            return Ok(());
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
///
/// Thread-safe: pushes the event into a Mutex-protected queue.
/// The VM loop drains it in `corevm_poll_irqs` (single-threaded).
#[no_mangle]
pub extern "C" fn corevm_ps2_mouse_move(handle: u64, dx: i16, dy: i16, buttons: u8) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    #[cfg(feature = "std")]
    {
        if let Ok(mut queue) = vm.pending_mouse.lock() {
            queue.push((dx, dy, buttons));
            return 0;
        }
        return -1;
    }
    #[cfg(not(feature = "std"))]
    {
        match vm.ps2() {
            Some(ps2) => { ps2.mouse_move(dx, dy, buttons); 0 }
            None => -1,
        }
    }
}

/// Debug: query PS/2 mouse state.
/// Returns: bit 0 = mouse_enabled, bits 8..15 = mouse_buffer length, bits 16..23 = keyboard_buffer length.
/// Returns 0xFFFFFFFF if no PS/2 controller.
#[no_mangle]
pub extern "C" fn corevm_ps2_mouse_state(handle: u64) -> u32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return 0xFFFFFFFF };
    match vm.ps2() {
        Some(ps2) => {
            let enabled = if ps2.mouse_enabled { 1u32 } else { 0 };
            let mbuf = (ps2.mouse_buffer.len() as u32 & 0xFF) << 8;
            let kbuf = (ps2.keyboard_buffer.len() as u32 & 0xFF) << 16;
            enabled | mbuf | kbuf
        }
        None => 0xFFFFFFFF,
    }
}

/// Debug: dump KVM in-kernel IOAPIC redirection table entry for a given pin.
/// Returns the 64-bit redirection entry value, or 0xFFFFFFFF_FFFFFFFF on error.
#[no_mangle]
pub extern "C" fn corevm_ioapic_pin_state(handle: u64, pin: u32) -> u64 {
    #[cfg(feature = "linux")]
    {
        let vm = match get_vm(handle) { Some(v) => v, None => return u64::MAX };
        // KVM IOAPIC chip_id = 2
        match vm.backend.get_irqchip(2) {
            Ok(data) => {
                // KVM ioapic state layout:
                // u64 base_address (8 bytes)
                // u32 ioregsel (4 bytes)
                // u32 id (4 bytes)
                // u32 irr (4 bytes)
                // u32 pad (4 bytes)
                // Then 24 entries of: union { u64 bits; struct { u8 vector, ... } } (8 bytes each)
                // = kvm_ioapic_state
                let entry_offset = 24 + (pin as usize) * 8; // 8+4+4+4+4=24 header bytes
                if entry_offset + 8 <= data.len() {
                    let mut bytes = [0u8; 8];
                    bytes.copy_from_slice(&data[entry_offset..entry_offset + 8]);
                    u64::from_le_bytes(bytes)
                } else {
                    u64::MAX
                }
            }
            Err(_) => u64::MAX,
        }
    }
    #[cfg(not(feature = "linux"))]
    {
        let _ = (handle, pin);
        u64::MAX
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
            // Always return the full VRAM buffer. On KVM/WHP it is mapped as
            // a hypervisor memory region and the guest writes directly to it.
            // The caller can check corevm_vga_get_mode() if it needs to know
            // whether the guest is in text or graphics mode.
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

/// Get the byte offset into VRAM where the display framebuffer starts.
///
/// bochs-drm (Linux DRM driver) uses VBE_DISPI_INDEX_X_OFFSET (reg 8) and
/// VBE_DISPI_INDEX_Y_OFFSET (reg 9) to place its framebuffer at an arbitrary
/// offset within VRAM.  The display start is at:
///   `(y_offset * virt_width + x_offset) * bytes_per_pixel`
///
/// Returns the byte offset, or 0 if the offset registers are zero or on error.
#[no_mangle]
pub extern "C" fn corevm_vga_get_fb_offset(handle: u64) -> u64 {
    let vm = match get_vm(handle) { Some(v) => v, None => return 0 };
    match vm.svga() {
        Some(svga) => {
            let x_off = svga.vbe_regs[8] as u64;   // VBE_DISPI_INDEX_X_OFFSET
            let y_off = svga.vbe_regs[9] as u64;   // VBE_DISPI_INDEX_Y_OFFSET
            let virt_w = svga.vbe_regs[6] as u64;  // VBE_DISPI_INDEX_VIRT_WIDTH
            let bpp = svga.vbe_regs[3] as u64;     // VBE_DISPI_INDEX_BPP
            let bytes_pp = (bpp + 7) / 8;
            (y_off * virt_w + x_off) * bytes_pp
        }
        None => 0,
    }
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

// Re-export WHP debug callback setter for use by vmmanager.
#[cfg(feature = "windows")]
pub use crate::backend::whp::corevm_set_whp_debug_callback;
