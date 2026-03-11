//! C FFI layer for libcorevm.
//!
//! All `extern "C"` functions that form the public API consumed by the VM
//! daemon (vmd) and other C/C++ callers. A global VM registry maps opaque
//! `u64` handles to [`Vm`] instances.

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
/// 6=Cpuid, 7=Halted, 8=InterruptWindow, 9=Shutdown, 10=Debug, 11=Error
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
    pub _reserved: u32,
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
        VmExitReason::MmioRead { addr, size } => {
            e.reason = 2; e.addr = addr; e.size = size;
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
        Err(_) => 0,
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
        Some(vm) => if vm.create_vcpu(vcpu_id).is_ok() { 0 } else { -1 },
        None => -1,
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
        Err(_) => -1,
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
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if sregs.is_null() { return -1; }
    if vm.set_vcpu_sregs(vcpu_id, unsafe { &*sregs }).is_ok() { 0 } else { -1 }
}

// ── Interrupt injection ─────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn corevm_inject_interrupt(handle: u64, vcpu_id: u32, vector: u8) -> i32 {
    match get_vm(handle) {
        Some(vm) => if vm.inject_interrupt(vcpu_id, vector).is_ok() { 0 } else { -1 },
        None => -1,
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
    #[cfg(not(feature = "linux"))]
    { 0 }
}

// ── I/O and MMIO exit dispatch ──────────────────────────────────────────────

/// Dispatch a port I/O exit to the registered device handler.
///
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
    0
}

/// Dispatch an MMIO exit to the registered device handler.
///
/// For reads (`is_write`=0), `data` is filled with the result.
/// For writes (`is_write`=1), `data` contains the guest-written value.
#[no_mangle]
pub extern "C" fn corevm_handle_mmio_exit(
    handle: u64, addr: u64, is_write: u8, size: u8, data: *mut u8,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if data.is_null() { return -1; }
    let buf = unsafe { core::slice::from_raw_parts_mut(data, size as usize) };
    vm.handle_mmio(addr, is_write != 0, size, buf);
    0
}

// ── Standard device setup ───────────────────────────────────────────────────

/// Register all standard chipset devices into the VM.
#[no_mangle]
pub extern "C" fn corevm_setup_standard_devices(handle: u64) -> i32 {
    match get_vm(handle) {
        Some(vm) => { vm.setup_standard_devices(); 0 }
        None => -1,
    }
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

/// Set up the AHCI SATA controller with the given number of ports.
#[no_mangle]
pub extern "C" fn corevm_setup_ahci(handle: u64, num_ports: u8) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    let ahci = Box::new(crate::devices::ahci::Ahci::new(num_ports));
    vm.ahci_ptr = &*ahci as *const crate::devices::ahci::Ahci as *mut crate::devices::ahci::Ahci;
    vm.memory.add_mmio(0xFEBF_0000, 0x1000, ahci);
    0
}

/// Attach a disk image to an AHCI port.
#[no_mangle]
pub extern "C" fn corevm_ahci_attach_disk(handle: u64, port: u32, fd: i32, size: u64) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    match vm.ahci() {
        Some(ahci) => {
            ahci.attach_disk_fd(port as usize, fd, size, crate::devices::ahci::AhciDriveKind::Disk);
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
            ahci.attach_disk_fd(port as usize, fd, size, crate::devices::ahci::AhciDriveKind::Cdrom);
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
#[no_mangle]
pub extern "C" fn corevm_vga_get_framebuffer(
    handle: u64, out_ptr: *mut *const u8, out_len: *mut u32,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if out_ptr.is_null() || out_len.is_null() { return -1; }
    match vm.svga() {
        Some(svga) => {
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

/// Get a pointer to the VGA text buffer (array of u16: char+attr pairs).
/// Sets `*out_ptr` and `*out_len` (number of u16 entries). Returns 0 on success, -1 on error.
#[no_mangle]
pub extern "C" fn corevm_vga_get_text_buffer(
    handle: u64, out_ptr: *mut *const u16, out_len: *mut u32,
) -> i32 {
    let vm = match get_vm(handle) { Some(v) => v, None => return -1 };
    if out_ptr.is_null() || out_len.is_null() { return -1; }
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
