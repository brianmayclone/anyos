//! libcorevm — Pure userspace x86 virtual machine library for anyOS.
//!
//! Provides a complete software x86 CPU emulator supporting:
//! - **Real Mode** (16-bit) — BIOS, bootloaders
//! - **Protected Mode** (32-bit) — full segmentation, paging, privilege levels
//! - **Long Mode** (64-bit) — 4-level paging, SYSCALL/SYSRET, R8-R15
//!
//! No hardware virtualization extensions (VT-x/AMD-V) are required — all
//! instruction execution is fully emulated in software.
//!
//! # Architecture
//!
//! The library is organized into these layers:
//! - **Decoder** (`decoder.rs`) — variable-length x86 instruction decoding
//! - **Executor** (`executor/`) — instruction execution grouped by category
//! - **Memory** (`memory/`) — guest RAM, segmentation, paging, MMIO
//! - **Devices** (`devices/`) — emulated hardware (SVGA, PS/2, E1000, etc.)
//! - **CPU** (`cpu.rs`) — ties everything together in the fetch-decode-execute loop
//!
//! # C ABI
//!
//! All public functions are `extern "C"` with `#[no_mangle]` for use via `dl_sym()`.
//! The VM handle is an opaque `u64` representing a pointer to a heap-allocated
//! `VmInstance`.

#![cfg_attr(not(feature = "host_test"), no_std)]
#![cfg_attr(not(feature = "host_test"), no_main)]

extern crate alloc;
#[cfg(not(feature = "host_test"))]
extern crate libheap;

pub mod error;
pub mod flags;
pub mod registers;
pub mod instruction;
pub mod decoder;
pub mod memory;
pub mod cpu;
pub mod executor;
pub mod interrupts;
pub mod io;
pub mod fpu_state;
pub mod sse_state;
pub mod devices;
pub mod jit;

/// Syscall wrappers for the allocator, panic handler, debug output, and
/// file I/O (used by the IDE controller for on-demand disk access).
pub(crate) mod syscall {
    pub use libsyscall::{sbrk, mmap, munmap, exit, serial_print, write_bytes};
    pub use libsyscall::{open, read, write, lseek, close};
}

/// Print a formatted line to the serial console (stdout fd=1).
macro_rules! vm_log {
    ($($arg:tt)*) => {{
        #[cfg(not(feature = "host_test"))]
        {
            libsyscall::serial_print(format_args!("[corevm] "));
            libsyscall::serial_print(format_args!($($arg)*));
            libsyscall::write_bytes(b"\n");
        }
    }};
}

#[cfg(not(feature = "host_test"))]
libheap::dll_allocator!(crate::syscall::sbrk, crate::syscall::mmap, crate::syscall::munmap);

#[cfg(not(feature = "host_test"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    syscall::exit(1);
}

// ── Public re-exports ──

pub use error::{VmError, Result};
pub use cpu::{Cpu, Mode, ExitReason};
pub use memory::{GuestMemory, Mmu};
pub use memory::mmio::MmioHandler;
pub use memory::flat::FlatMemory;
pub use io::{IoDispatch, IoHandler};
pub use interrupts::InterruptController;
pub use decoder::CpuMode;
pub use registers::{RegisterFile, SegReg};
pub use flags::OperandSize;

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ptr;

// ── VmEngine (unchanged convenience wrapper) ──

/// High-level VM engine — convenience wrapper combining all VM components.
///
/// For advanced use cases, the individual components (`Cpu`, `GuestMemory`,
/// `Mmu`, `IoDispatch`, `InterruptController`) can be used directly.
pub struct VmEngine {
    /// Virtual CPU state and execution engine.
    pub cpu: Cpu,
    /// Guest physical memory (RAM + MMIO regions).
    pub memory: GuestMemory,
    /// Memory management unit (segmentation + paging translation).
    pub mmu: Mmu,
    /// Interrupt controller (IDT management, pending interrupt tracking).
    pub interrupts: InterruptController,
    /// Port I/O dispatcher (maps port ranges to device handlers).
    pub io: IoDispatch,
    /// Configured logical CPU count exposed to the guest.
    pub vcpu_count: u8,
}

impl VmEngine {
    /// Create a new VM with the specified guest RAM size in bytes.
    ///
    /// The CPU starts in real mode at the standard reset vector (CS:IP = F000:FFF0).
    pub fn new(ram_size: usize) -> Self {
        Self::new_with_vcpus(ram_size, 1)
    }

    /// Create a new VM with a configured logical CPU count.
    pub fn new_with_vcpus(ram_size: usize, vcpu_count: u8) -> Self {
        let count = vcpu_count.max(1);
        let mut cpu = Cpu::new();
        cpu.configure_topology(0, count);
        VmEngine {
            cpu,
            memory: GuestMemory::new(ram_size),
            mmu: Mmu::new(),
            interrupts: InterruptController::new(),
            io: IoDispatch::new(),
            vcpu_count: count,
        }
    }

    /// Load raw binary data at a guest physical address.
    pub fn load_binary(&mut self, addr: usize, data: &[u8]) {
        self.memory.load_at(addr, data);
    }

    /// Set the instruction pointer directly.
    pub fn set_rip(&mut self, rip: u64) {
        self.cpu.regs.rip = rip;
    }

    /// Run the VM for up to `max_instructions` (0 = unlimited).
    ///
    /// Returns the reason the VM stopped executing.
    pub fn run(&mut self, max_instructions: u64) -> ExitReason {
        self.cpu.run(
            &mut self.memory,
            &mut self.mmu,
            &mut self.interrupts,
            &mut self.io,
            max_instructions,
        )
    }

    /// Request the VM to stop at the next instruction boundary.
    ///
    /// This is safe to call from a signal handler or another thread
    /// (the flag is checked at the top of each instruction cycle).
    pub fn request_stop(&mut self) {
        self.cpu.request_stop();
    }

    /// Reset the VM to power-on state.
    pub fn reset(&mut self) {
        self.cpu.reset();
        self.cpu.configure_topology(0, self.vcpu_count);
        self.mmu = Mmu::new();
        self.interrupts = InterruptController::new();
        // Memory and I/O handlers are preserved across reset
    }

    /// Register a port I/O handler for a range of ports.
    pub fn register_io(
        &mut self,
        base: u16,
        count: u16,
        handler: Box<dyn IoHandler>,
    ) {
        self.io.register(base, count, handler);
    }

    /// Load a firmware ROM into guest memory at a physical address.
    ///
    /// The data is copied directly into flat RAM so the firmware can
    /// use the same region for both code and writable runtime variables
    /// (BIOS data tables, El Torito state, IDE geometry, etc.).
    /// If the address falls within the flat RAM allocation the data is
    /// writable; otherwise it is added as a read-only ROM overlay.
    pub fn load_rom(&mut self, base: u64, data: Vec<u8>) {
        let end = base as usize + data.len();
        if end <= self.memory.ram().size() {
            // Within flat RAM — load as read-write so the BIOS can
            // modify its own data segment at runtime.
            self.memory.load_at(base as usize, &data);
        } else {
            // Above flat RAM — use a read-only ROM overlay.
            self.memory.add_rom(base, data);
        }
    }

    /// Register a memory-mapped I/O handler.
    pub fn register_mmio(
        &mut self,
        base: u64,
        size: u64,
        handler: Box<dyn MmioHandler>,
    ) {
        self.memory.add_mmio(base, size, handler);
    }

    /// Get the current instruction count.
    pub fn instruction_count(&self) -> u64 {
        self.cpu.instruction_count
    }

    /// Get the current CPU mode.
    pub fn mode(&self) -> Mode {
        self.cpu.mode
    }
}

// ════════════════════════════════════════════════════════════════════════
// C ABI layer — opaque handle-based interface for dl_sym() consumers.
// ════════════════════════════════════════════════════════════════════════

// ── IoProxy ──

/// Thin proxy that forwards [`IoHandler`] calls through a raw pointer.
///
/// This allows a device to be owned by `VmInstance` (as a raw pointer) while
/// simultaneously being registered in the [`IoDispatch`] table. The proxy
/// borrows the device through the raw pointer, which is valid as long as the
/// `VmInstance` (and therefore the device allocation) is alive.
struct IoProxy<T: IoHandler> {
    /// Raw pointer to the device. Valid for the lifetime of the owning `VmInstance`.
    ptr: *mut T,
}

impl<T: IoHandler> IoHandler for IoProxy<T> {
    fn read(&mut self, port: u16, size: u8) -> Result<u32> {
        unsafe { (*self.ptr).read(port, size) }
    }

    fn write(&mut self, port: u16, size: u8, val: u32) -> Result<()> {
        unsafe { (*self.ptr).write(port, size, val) }
    }
}

/// Thin proxy that forwards [`MmioHandler`] calls through a raw pointer.
///
/// Same ownership pattern as [`IoProxy`] — the device is heap-allocated and
/// owned by `VmInstance`; this proxy merely borrows it through a raw pointer.
struct MmioProxy<T: MmioHandler> {
    /// Raw pointer to the device. Valid for the lifetime of the owning `VmInstance`.
    ptr: *mut T,
}

impl<T: MmioHandler> MmioHandler for MmioProxy<T> {
    fn read(&mut self, offset: u64, size: u8) -> Result<u64> {
        unsafe { (*self.ptr).read(offset, size) }
    }

    fn write(&mut self, offset: u64, size: u8, val: u64) -> Result<()> {
        unsafe { (*self.ptr).write(offset, size, val) }
    }
}

// ── VmInstance ──

/// Opaque VM instance that owns the engine and direct-access device pointers.
///
/// Devices are heap-allocated via `Box::into_raw`. Proxy objects registered in
/// the engine's `IoDispatch` / `GuestMemory` forward calls through raw pointers.
/// On drop, all device raw pointers are freed with `Box::from_raw`.
struct VmInstance {
    /// The core VM engine (CPU, memory, MMU, interrupt controller, I/O dispatch).
    engine: VmEngine,

    /// Last error that caused the VM to exit, if any.
    last_error: Option<error::VmError>,
    /// RIP at the time of the last error.
    last_error_rip: u64,

    // Raw pointers to heap-allocated devices, registered via proxies.
    // Null when the corresponding device has not been set up.
    pic_ptr: *mut devices::pic::PicPair,
    pit_ptr: *mut devices::pit::Pit,
    acpi_pm_ptr: *mut devices::acpi::AcpiPm,
    ps2_ptr: *mut devices::ps2::Ps2Controller,
    serial_ptr: *mut devices::serial::Serial,
    svga_ptr: *mut devices::svga::Svga,
    lapic_ptr: *mut devices::lapic::Lapic,
    ioapic_ptr: *mut devices::ioapic::IoApic,
    e1000_ptr: *mut devices::e1000::E1000,
    bus_ptr: *mut devices::bus::PciBus,
    ide_ptr: *mut devices::ide::Ide,
    fw_cfg_ptr: *mut devices::fw_cfg::FwCfg,
    debug_port_ptr: *mut devices::debug_port::DebugPort,
    /// True once an external runner starts driving PIT ticks explicitly.
    pit_is_externally_clocked: bool,
}

impl Drop for VmInstance {
    fn drop(&mut self) {
        // Free all heap-allocated devices. The proxies hold dangling pointers
        // after this, but they are destroyed together with the engine.
        unsafe {
            if !self.pic_ptr.is_null() { let _ = Box::from_raw(self.pic_ptr); }
            if !self.pit_ptr.is_null() { let _ = Box::from_raw(self.pit_ptr); }
            if !self.acpi_pm_ptr.is_null() { let _ = Box::from_raw(self.acpi_pm_ptr); }
            if !self.ps2_ptr.is_null() { let _ = Box::from_raw(self.ps2_ptr); }
            if !self.serial_ptr.is_null() { let _ = Box::from_raw(self.serial_ptr); }
            if !self.svga_ptr.is_null() { let _ = Box::from_raw(self.svga_ptr); }
            if !self.lapic_ptr.is_null() { let _ = Box::from_raw(self.lapic_ptr); }
            if !self.ioapic_ptr.is_null() { let _ = Box::from_raw(self.ioapic_ptr); }
            if !self.e1000_ptr.is_null() { let _ = Box::from_raw(self.e1000_ptr); }
            if !self.bus_ptr.is_null() { let _ = Box::from_raw(self.bus_ptr); }
            if !self.ide_ptr.is_null() { let _ = Box::from_raw(self.ide_ptr); }
            if !self.fw_cfg_ptr.is_null() { let _ = Box::from_raw(self.fw_cfg_ptr); }
            if !self.debug_port_ptr.is_null() { let _ = Box::from_raw(self.debug_port_ptr); }
        }
    }
}

/// Convert an opaque `u64` handle to a mutable `VmInstance` reference.
///
/// # Safety
///
/// The caller must guarantee that `handle` was returned by [`corevm_create`]
/// and has not been destroyed via [`corevm_destroy`].
#[inline]
unsafe fn vm_from_handle(handle: u64) -> &'static mut VmInstance {
    &mut *(handle as *mut VmInstance)
}

// ════════════════════════════════════════════════════════════════════════
// VM Lifecycle
// ════════════════════════════════════════════════════════════════════════

/// Create a new VM instance with the specified guest RAM size in megabytes.
///
/// Returns an opaque handle (non-zero on success, 0 on failure).
/// The handle must be destroyed with [`corevm_destroy`] when no longer needed.
#[no_mangle]
pub extern "C" fn corevm_create(ram_size_mb: u32) -> u64 {
    corevm_create_ex(ram_size_mb, 1)
}

/// Create a new VM instance with RAM size and logical CPU count.
///
/// `vcpu_count` is clamped to at least 1.
#[no_mangle]
pub extern "C" fn corevm_create_ex(ram_size_mb: u32, vcpu_count: u32) -> u64 {
    let count = (vcpu_count.clamp(1, 255)) as u8;
    vm_log!(
        "creating VM with {} MiB RAM (vcpus={})",
        ram_size_mb,
        count
    );
    let ram_bytes = (ram_size_mb as usize) * 1024 * 1024;
    let instance = Box::new(VmInstance {
        engine: VmEngine::new_with_vcpus(ram_bytes, count),
        last_error: None,
        last_error_rip: 0,
        pic_ptr: ptr::null_mut(),
        pit_ptr: ptr::null_mut(),
        acpi_pm_ptr: ptr::null_mut(),
        ps2_ptr: ptr::null_mut(),
        serial_ptr: ptr::null_mut(),
        svga_ptr: ptr::null_mut(),
        lapic_ptr: ptr::null_mut(),
        ioapic_ptr: ptr::null_mut(),
        e1000_ptr: ptr::null_mut(),
        bus_ptr: ptr::null_mut(),
        ide_ptr: ptr::null_mut(),
        fw_cfg_ptr: ptr::null_mut(),
        debug_port_ptr: ptr::null_mut(),
        pit_is_externally_clocked: false,
    });
    let h = Box::into_raw(instance) as u64;
    vm_log!("VM created (handle=0x{:X})", h);
    h
}

/// Destroy a VM instance and free all associated resources.
///
/// After this call the handle is invalid and must not be used again.
#[no_mangle]
pub extern "C" fn corevm_destroy(handle: u64) {
    if handle == 0 {
        return;
    }
    vm_log!("destroying VM (handle=0x{:X})", handle);
    unsafe {
        let _ = Box::from_raw(handle as *mut VmInstance);
    }
}

/// Reset the VM to power-on state.
///
/// CPU registers are reset, the MMU and interrupt controller are re-initialized.
/// Guest RAM contents, I/O handlers, and MMIO handlers are preserved.
#[no_mangle]
pub extern "C" fn corevm_reset(handle: u64) {
    vm_log!("resetting VM");
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.reset();
    vm.last_error = None;
    vm.last_error_rip = 0;
    vm.pit_is_externally_clocked = false;
}

// ════════════════════════════════════════════════════════════════════════
// CPU State — General-Purpose Registers
// ════════════════════════════════════════════════════════════════════════

/// Get the current instruction pointer (RIP).
#[no_mangle]
pub extern "C" fn corevm_get_rip(handle: u64) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.regs.rip
}

/// Set the instruction pointer (RIP).
#[no_mangle]
pub extern "C" fn corevm_set_rip(handle: u64, rip: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.regs.rip = rip;
}

/// Read a general-purpose register by index (0=RAX .. 15=R15).
///
/// Returns 0 if `index` is out of range.
#[no_mangle]
pub extern "C" fn corevm_get_gpr(handle: u64, index: u8) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    if (index as usize) < vm.engine.cpu.regs.gpr.len() {
        vm.engine.cpu.regs.gpr[index as usize]
    } else {
        0
    }
}


/// Write a general-purpose register by index (0=RAX .. 15=R15).
///
/// Silently ignored if `index` is out of range.
#[no_mangle]
pub extern "C" fn corevm_set_gpr(handle: u64, index: u8, val: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if (index as usize) < vm.engine.cpu.regs.gpr.len() {
        vm.engine.cpu.regs.gpr[index as usize] = val;
    }
}

/// Get the RFLAGS register.
#[no_mangle]
pub extern "C" fn corevm_get_rflags(handle: u64) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.regs.rflags
}

/// Set the RFLAGS register.
#[no_mangle]
pub extern "C" fn corevm_set_rflags(handle: u64, val: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.regs.rflags = val;
}

// ════════════════════════════════════════════════════════════════════════
// CPU State — Control Registers
// ════════════════════════════════════════════════════════════════════════

/// Read a control register (CR0, CR2, CR3, CR4, CR8).
///
/// `n` selects the register: 0=CR0, 2=CR2, 3=CR3, 4=CR4, 8=CR8.
/// Returns 0 for unrecognized register numbers.
#[no_mangle]
pub extern "C" fn corevm_get_cr(handle: u64, n: u8) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    match n {
        0 => vm.engine.cpu.regs.cr0,
        2 => vm.engine.cpu.regs.cr2,
        3 => vm.engine.cpu.regs.cr3,
        4 => vm.engine.cpu.regs.cr4,
        8 => vm.engine.cpu.regs.cr8,
        _ => 0,
    }
}

/// Get one model-specific register value.
#[no_mangle]
pub extern "C" fn corevm_get_msr(handle: u64, idx: u32) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.regs.read_msr(idx)
}

/// Write a control register (CR0, CR2, CR3, CR4, CR8).
///
/// `n` selects the register: 0=CR0, 2=CR2, 3=CR3, 4=CR4, 8=CR8.
/// After writing CR0 or CR4, the CPU mode is automatically updated.
/// Writes to unrecognized register numbers are silently ignored.
#[no_mangle]
pub extern "C" fn corevm_set_cr(handle: u64, n: u8, val: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    match n {
        0 => {
            vm.engine.cpu.regs.cr0 = val;
            vm.engine.cpu.update_mode();
        }
        2 => vm.engine.cpu.regs.cr2 = val,
        3 => vm.engine.cpu.regs.cr3 = val,
        4 => {
            vm.engine.cpu.regs.cr4 = val;
            vm.engine.cpu.update_mode();
        }
        8 => vm.engine.cpu.regs.cr8 = val,
        _ => {}
    }
}

// ════════════════════════════════════════════════════════════════════════
// CPU State — Segment Registers
// ════════════════════════════════════════════════════════════════════════

/// Get the visible selector of a segment register.
///
/// `seg`: 0=ES, 1=CS, 2=SS, 3=DS, 4=FS, 5=GS. Returns 0 for invalid indices.
#[no_mangle]
pub extern "C" fn corevm_get_segment_selector(handle: u64, seg: u8) -> u16 {
    let vm = unsafe { vm_from_handle(handle) };
    if (seg as usize) < vm.engine.cpu.regs.seg.len() {
        vm.engine.cpu.regs.seg[seg as usize].selector
    } else {
        0
    }
}

/// Get the cached base address of a segment register.
///
/// `seg`: 0=ES, 1=CS, 2=SS, 3=DS, 4=FS, 5=GS. Returns 0 for invalid indices.
#[no_mangle]
pub extern "C" fn corevm_get_segment_base(handle: u64, seg: u8) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    if (seg as usize) < vm.engine.cpu.regs.seg.len() {
        vm.engine.cpu.regs.seg[seg as usize].base
    } else {
        0
    }
}

/// Get the current CPU execution mode.
///
/// Returns: 0 = real mode, 1 = protected mode, 2 = long mode.
#[no_mangle]
pub extern "C" fn corevm_get_mode(handle: u64) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    match vm.engine.cpu.mode {
        Mode::RealMode => 0,
        Mode::ProtectedMode => 1,
        Mode::LongMode => 2,
    }
}

/// Get the current privilege level (CPL, 0-3).
#[no_mangle]
pub extern "C" fn corevm_get_cpl(handle: u64) -> u8 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.regs.cpl
}

/// Get configured logical CPU count for this VM.
#[no_mangle]
pub extern "C" fn corevm_get_vcpu_count(handle: u64) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.vcpu_count as u32
}

// ════════════════════════════════════════════════════════════════════════
// Execution
// ════════════════════════════════════════════════════════════════════════

/// Run the VM for up to `max_instructions` (0 = unlimited).
///
/// Returns an exit reason code:
/// - 0 = halted (HLT executed)
/// - 1 = unhandled exception
/// - 2 = instruction limit reached
/// - 3 = breakpoint (INT 3)
/// - 4 = stop requested via [`corevm_request_stop`]
#[no_mangle]
pub extern "C" fn corevm_run(handle: u64, max_instructions: u64) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    const RUN_SLICE_INSTS: u64 = 2048;
    const PIT_INSTS_PER_TICK: u64 = 64;

    let mut exit = ExitReason::InstructionLimit;
    let mut remaining = max_instructions;
    let mut pit_inst_accum: u64 = 0;

    loop {
        let slice = if max_instructions == 0 {
            max_instructions
        } else {
            remaining.min(RUN_SLICE_INSTS)
        };

        let before_ic = vm.engine.instruction_count();
        exit = vm.engine.run(slice);
        let after_ic = vm.engine.instruction_count();
        let ran = after_ic.saturating_sub(before_ic);

        // Advance LAPIC timer using guest instruction progress.
        if ran > 0 && !vm.lapic_ptr.is_null() {
            let lapic = unsafe { &mut *vm.lapic_ptr };
            if let Some(vector) = lapic.advance(ran) {
                vm.engine.interrupts.raise_irq(vector);
            }
        }

        // Advance PIT continuously during run() so guests that poll PIT
        // inside tight loops can observe timer progress within a batch.
        if ran > 0 && !vm.pit_ptr.is_null() && !vm.pit_is_externally_clocked {
            pit_inst_accum = pit_inst_accum.saturating_add(ran);
            let pit_ticks = (pit_inst_accum / PIT_INSTS_PER_TICK) as u32;
            pit_inst_accum %= PIT_INSTS_PER_TICK;
            if pit_ticks > 0 {
                let fires = unsafe { (*vm.pit_ptr).advance(pit_ticks) };
                if fires > 0 {
                    inject_irq_line(vm, 0);
                }
            }
        }

        // The ACPI PM timer must be free-running even when the guest polls it.
        if ran > 0 && !vm.acpi_pm_ptr.is_null() {
            unsafe { (*vm.acpi_pm_ptr).advance(ran) };
        }

        if max_instructions == 0 {
            break;
        }
        match exit {
            ExitReason::InstructionLimit => {
                remaining = remaining.saturating_sub(ran);
                if remaining == 0 || ran == 0 {
                    break;
                }
            }
            _ => break,
        }
    }

    match exit {
        ExitReason::Halted => {
            0
        }
        ExitReason::Exception(ref err) => {
            let rip = vm.engine.cpu.regs.rip;
            let orig_rip = vm.engine.cpu.last_exec_rip;
            let orig_cs = vm.engine.cpu.last_exec_cs;
            let orig_opcode = vm.engine.cpu.last_opcode;
            let orig_phys = vm.engine.cpu.last_fetch_addr;
            vm_log!("VM exception: {}", err);
            vm_log!("  current RIP=0x{:X}, mode={:?}", rip, vm.engine.cpu.mode);
            vm_log!(
                "  last instruction: CS=0x{:04X} IP=0x{:X} phys=0x{:X} opcode=0x{:04X}",
                orig_cs, orig_rip, orig_phys, orig_opcode
            );
            vm_log!(
                "  instructions executed: {}",
                vm.engine.instruction_count()
            );
            vm.last_error = Some(*err);
            vm.last_error_rip = orig_rip;
            1
        }
        ExitReason::InstructionLimit => 2,
        ExitReason::Breakpoint => {
            vm_log!("VM breakpoint at RIP=0x{:X}", vm.engine.cpu.regs.rip);
            3
        }
        ExitReason::StopRequested => {
            vm_log!("VM stop requested");
            4
        }
    }
}

/// Request the VM to stop at the next instruction boundary.
///
/// Safe to call from any context; the flag is checked at the top of each
/// instruction cycle.
#[no_mangle]
pub extern "C" fn corevm_request_stop(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.request_stop();
}

/// Get the total number of instructions executed since the last reset.
#[no_mangle]
pub extern "C" fn corevm_get_instruction_count(handle: u64) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.instruction_count()
}

/// Get the RIP at the time of the last error.
///
/// Returns 0 if no error has occurred since the last reset.
#[no_mangle]
pub extern "C" fn corevm_get_last_error_rip(handle: u64) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.last_error_rip
}

/// Write a human-readable description of the last error into the provided buffer.
///
/// Returns the number of bytes written (not including any NUL terminator).
/// Returns 0 if no error has occurred since the last reset, or if `buf` is null.
/// The output is NUL-terminated if the buffer is large enough.
#[no_mangle]
pub extern "C" fn corevm_get_last_error(handle: u64, buf: *mut u8, buf_len: u32) -> u32 {
    if buf.is_null() || buf_len == 0 {
        return 0;
    }
    let vm = unsafe { vm_from_handle(handle) };
    let err = match &vm.last_error {
        Some(e) => e,
        None => return 0,
    };
    // Format the error using its Display impl into a stack buffer.
    use core::fmt::Write;
    let mut tmp = StackWriter::new();
    let _ = write!(tmp, "{}", err);
    let msg = tmp.as_bytes();
    let copy_len = msg.len().min((buf_len - 1) as usize); // leave room for NUL
    unsafe {
        ptr::copy_nonoverlapping(msg.as_ptr(), buf, copy_len);
        *buf.add(copy_len) = 0; // NUL terminator
    }
    copy_len as u32
}

/// Small stack-allocated writer for formatting error messages.
struct StackWriter {
    buf: [u8; 256],
    pos: usize,
}

impl StackWriter {
    fn new() -> Self {
        StackWriter { buf: [0u8; 256], pos: 0 }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.pos]
    }
}

impl core::fmt::Write for StackWriter {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let remaining = self.buf.len() - self.pos;
        let copy = bytes.len().min(remaining);
        self.buf[self.pos..self.pos + copy].copy_from_slice(&bytes[..copy]);
        self.pos += copy;
        Ok(())
    }
}

// ════════════════════════════════════════════════════════════════════════
// Memory
// ════════════════════════════════════════════════════════════════════════

/// Load binary data into guest physical memory at the specified address.
///
/// Returns 0 on success, -1 on failure (e.g., null pointer or out of range).
#[no_mangle]
pub extern "C" fn corevm_load_binary(
    handle: u64,
    addr: u64,
    data: *const u8,
    len: u32,
) -> i32 {
    if data.is_null() || len == 0 {
        vm_log!("load_binary: null or empty data");
        return -1;
    }
    vm_log!("loading {} bytes at physical 0x{:X}", len, addr);
    let vm = unsafe { vm_from_handle(handle) };
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    vm.engine.load_binary(addr as usize, slice);
    0
}

/// Map a read-only ROM at a guest physical address.
///
/// This creates a ROM overlay that serves reads from the specified address
/// range without requiring the flat RAM allocation to extend that far.
/// Writes to ROM addresses are silently ignored.
///
/// Used for mapping firmware ROMs in a QEMU-compatible layout:
/// - SeaBIOS 256 KB at 0xFFFC0000 (top of 4 GiB)
/// - Shadow copy 128 KB at 0xE0000 (below 1 MiB)
/// - VGA BIOS at 0xC0000
///
/// Returns 0 on success, -1 on invalid arguments.
#[no_mangle]
pub extern "C" fn corevm_load_rom(
    handle: u64,
    addr: u64,
    data: *const u8,
    len: u32,
) -> i32 {
    if data.is_null() || len == 0 {
        vm_log!("load_rom: null or empty data");
        return -1;
    }
    vm_log!("mapping {} byte ROM at physical 0x{:08X}", len, addr);
    let vm = unsafe { vm_from_handle(handle) };
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    vm.engine.load_rom(addr, Vec::from(slice));
    0
}

/// Add a named file to the fw_cfg device.
///
/// `name` is a NUL-terminated C string (e.g., "vgaroms/vgabios.bin").
/// `data` points to `len` bytes of file content.
/// Returns 0 on success, -1 if fw_cfg is not set up.
#[no_mangle]
pub extern "C" fn corevm_fw_cfg_add_file(
    handle: u64,
    name: *const u8,
    data: *const u8,
    len: u32,
) -> i32 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.fw_cfg_ptr.is_null() {
        vm_log!("fw_cfg_add_file: fw_cfg not set up");
        return -1;
    }
    if name.is_null() || data.is_null() {
        return -1;
    }
    // Read NUL-terminated name string.
    let mut name_len = 0;
    unsafe {
        while *name.add(name_len) != 0 && name_len < 55 {
            name_len += 1;
        }
    }
    let name_str = unsafe {
        core::str::from_utf8_unchecked(core::slice::from_raw_parts(name, name_len))
    };
    let file_data = unsafe { core::slice::from_raw_parts(data, len as usize) };
    let fw_cfg = unsafe { &mut *vm.fw_cfg_ptr };
    fw_cfg.add_file(name_str, Vec::from(file_data));
    vm_log!("fw_cfg: added file '{}' ({} bytes)", name_str, len);
    0
}

/// Read a single byte from guest physical memory.
#[no_mangle]
pub extern "C" fn corevm_read_phys_u8(handle: u64, addr: u64) -> u8 {
    let vm = unsafe { vm_from_handle(handle) };
    use memory::MemoryBus;
    vm.engine.memory.read_u8(addr).unwrap_or(0)
}

/// Read a 16-bit little-endian value from guest physical memory.
#[no_mangle]
pub extern "C" fn corevm_read_phys_u16(handle: u64, addr: u64) -> u16 {
    let vm = unsafe { vm_from_handle(handle) };
    use memory::MemoryBus;
    vm.engine.memory.read_u16(addr).unwrap_or(0)
}

/// Read a 32-bit little-endian value from guest physical memory.
#[no_mangle]
pub extern "C" fn corevm_read_phys_u32(handle: u64, addr: u64) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    use memory::MemoryBus;
    vm.engine.memory.read_u32(addr).unwrap_or(0)
}

/// Read one byte from a guest linear address using current paging state.
///
/// Returns 0 on translation/read failure.
#[no_mangle]
pub extern "C" fn corevm_read_linear_u8(handle: u64, linear: u64) -> u8 {
    let vm = unsafe { vm_from_handle(handle) };
    use memory::{AccessType, MemoryBus};
    let cpl = vm.engine.cpu.regs.cpl;
    let cr3 = vm.engine.cpu.regs.cr3;
    match vm
        .engine
        .mmu
        .translate_linear(linear, cr3, AccessType::Read, cpl, &vm.engine.memory)
    {
        Ok(phys) => vm.engine.memory.read_u8(phys).unwrap_or(0),
        Err(_) => 0,
    }
}

/// Write a single byte to guest physical memory.
#[no_mangle]
pub extern "C" fn corevm_write_phys_u8(handle: u64, addr: u64, val: u8) {
    let vm = unsafe { vm_from_handle(handle) };
    use memory::MemoryBus;
    let _ = vm.engine.memory.write_u8(addr, val);
}

/// Write a 16-bit little-endian value to guest physical memory.
#[no_mangle]
pub extern "C" fn corevm_write_phys_u16(handle: u64, addr: u64, val: u16) {
    let vm = unsafe { vm_from_handle(handle) };
    use memory::MemoryBus;
    let _ = vm.engine.memory.write_u16(addr, val);
}

/// Write a 32-bit little-endian value to guest physical memory.
#[no_mangle]
pub extern "C" fn corevm_write_phys_u32(handle: u64, addr: u64, val: u32) {
    let vm = unsafe { vm_from_handle(handle) };
    use memory::MemoryBus;
    let _ = vm.engine.memory.write_u32(addr, val);
}

// ════════════════════════════════════════════════════════════════════════
// Devices — Setup
// ════════════════════════════════════════════════════════════════════════

/// Register standard PC devices: PIC, PIT, CMOS, PS/2, Serial, VGA (800x600).
///
/// This sets up the following I/O and MMIO regions:
/// - PIC: ports 0x20-0x21 (master), 0xA0-0xA1 (slave)
/// - PIT: ports 0x40-0x43
/// - CMOS: ports 0x70-0x71
/// - PS/2: ports 0x60, 0x64
/// - Serial (COM1): ports 0x3F8-0x3FF
/// - VGA: ports 0x3C0-0x3DA, MMIO at 0xA0000 (128 KB)
///
/// Must only be called once per VM instance.
#[no_mangle]
pub extern "C" fn corevm_setup_standard_devices(handle: u64) {
    vm_log!("setting up standard devices (PIC, PIT, CMOS, PS/2, serial, VGA)");
    let vm = unsafe { vm_from_handle(handle) };

    // PIC — dual 8259A at standard ports.
    let pic = Box::into_raw(Box::new(devices::pic::PicPair::new()));
    vm.pic_ptr = pic;
    vm.engine.io.register(0x20, 2, Box::new(IoProxy { ptr: pic }));
    vm.engine.io.register(0xA0, 2, Box::new(IoProxy { ptr: pic }));

    // PIT — Intel 8254 at standard ports.
    let pit = Box::into_raw(Box::new(devices::pit::Pit::new()));
    vm.pit_ptr = pit;
    vm.engine.io.register(0x40, 4, Box::new(IoProxy { ptr: pit }));
    // Port 0x61 — system control/speaker gate tied to PIT channel 2.
    let port61 = Box::new(devices::port61::Port61::new(pit));
    vm.engine.io.register(0x61, 1, port61);

    // CMOS — RTC and NVRAM. Pass actual guest RAM size.
    let ram_bytes = vm.engine.memory.ram().size();
    let cmos = Box::new(devices::cmos::Cmos::new(ram_bytes));
    vm.engine.io.register(0x70, 2, cmos);

    // APM control/status — SeaBIOS uses 0xB2/0xB3 for SMI handshakes.
    let apm = Box::new(devices::apm::ApmControl::new());
    vm.engine.io.register(0xB2, 2, apm);

    // PS/2 — keyboard and mouse controller.
    let ps2 = Box::into_raw(Box::new(devices::ps2::Ps2Controller::new()));
    vm.ps2_ptr = ps2;
    vm.engine.io.register(0x60, 1, Box::new(IoProxy { ptr: ps2 }));
    vm.engine.io.register(0x64, 1, Box::new(IoProxy { ptr: ps2 }));

    // Serial (COM1) — 16550 UART.
    let serial = Box::into_raw(Box::new(devices::serial::Serial::new()));
    vm.serial_ptr = serial;
    vm.engine.io.register(0x3F8, 8, Box::new(IoProxy { ptr: serial }));

    // VGA/SVGA — standard VGA ports + legacy framebuffer MMIO + Bochs VBE.
    let svga = Box::into_raw(Box::new(devices::svga::Svga::new(800, 600)));
    vm.svga_ptr = svga;
    vm.engine.io.register(0x3C0, 0x1B, Box::new(IoProxy { ptr: svga }));
    // Bochs VBE ports (0x1CE index, 0x1CF data) — used by VGA BIOS to detect hardware.
    vm.engine.io.register(0x1CE, 2, Box::new(IoProxy { ptr: svga }));
    vm.engine.memory.add_mmio(0xA0000, 0x20000, Box::new(MmioProxy { ptr: svga }));

    // PCI bus with Q35 (MCH + ICH9) machine devices.
    let mut bus = devices::bus::PciBus::new();

    // Q35 MCH (Memory Controller Hub) at 0:0.0.
    // SeaBIOS recognizes device ID 0x29C0 as Q35 and calls mch_mem_addr_setup().
    let mut host_bridge = devices::bus::PciDevice::new(
        0x8086,  // Vendor ID: Intel
        0x29C0,  // Device ID: Q35 MCH (82G33/G31/P35/P31 Express DRAM Controller)
        0x06,    // Class: Bridge
        0x00,    // Subclass: Host bridge
        0x00,    // Prog IF
    );
    host_bridge.bus = 0;
    host_bridge.device = 0;
    host_bridge.function = 0;
    // PAM registers (0x90-0x96 on Q35, same semantics as i440FX 0x59-0x5F).
    // Make all PAM regions writable so SeaBIOS can shadow ROMs.
    // PAM0 (0x90): covers 0xF0000-0xFFFFF — BIOS area.
    host_bridge.config_space[0x90] = 0x30; // Read/write enabled
    // PAM1-PAM6 (0x91-0x96): cover 0xC0000-0xEFFFF — option ROM area.
    for i in 0x91..=0x96 {
        host_bridge.config_space[i] = 0x33; // Read/write for both halves
    }
    // PCIEXBAR (0x60): MMCONFIG base address — 0xB0000000, 256 MiB, enabled.
    // SeaBIOS reads this to set up MMIO-based PCI config access (mmconfig).
    // Format: bits[63:28]=base, bits[2:1]=size (00=256M), bit[0]=enable.
    // 0xB0000001 in LE: [0x01, 0x00, 0x00, 0xB0, 0x00, 0x00, 0x00, 0x00]
    host_bridge.config_space[0x60] = 0x01; // enable=1, size=256M
    host_bridge.config_space[0x61] = 0x00;
    host_bridge.config_space[0x62] = 0x00;
    host_bridge.config_space[0x63] = 0xB0; // base bits[31:28] = 0xB
    host_bridge.config_space[0x64] = 0x00; // upper 32 bits = 0
    host_bridge.config_space[0x65] = 0x00;
    host_bridge.config_space[0x66] = 0x00;
    host_bridge.config_space[0x67] = 0x00;
    // QEMU subsystem IDs — SeaBIOS checks these to identify the chipset.
    host_bridge.set_subsystem(0x1AF4, 0x1100);
    bus.add_device(host_bridge);

    // ICH9 LPC (Low Pin Count) bridge at 0:1F.0 — SeaBIOS uses this for IRQ routing.
    // On Q35, the ISA/LPC bridge is at device 31, function 0 (not device 1).
    let mut lpc_bridge = devices::bus::PciDevice::new(
        0x8086,  // Vendor ID: Intel
        0x2918,  // Device ID: ICH9 LPC Interface Controller
        0x06,    // Class: Bridge
        0x01,    // Subclass: ISA bridge
        0x02,    // Prog IF: ICH9
    );
    lpc_bridge.bus = 0;
    lpc_bridge.device = 31;
    lpc_bridge.function = 0;
    // Mark as multi-function (header type bit 7) since ICH9 has multiple functions.
    lpc_bridge.config_space[0x0E] = 0x80;
    lpc_bridge.set_subsystem(0x1AF4, 0x1100);
    bus.add_device(lpc_bridge);

    // Legacy IDE controller at 0:1F.1.
    // SeaBIOS only registers ATA/ATAPI boot devices when it sees a PCI IDE
    // controller. We expose the primary channel in compatibility mode so the
    // controller uses the fixed ISA ports backed by `devices::ide::Ide`.
    let mut ide_pci = devices::bus::PciDevice::new(
        0x8086,  // Vendor ID: Intel
        0x2920,  // Device ID: ICH9 SATA/IDE compatibility function
        0x01,    // Class: Mass storage
        0x01,    // Subclass: IDE controller
        0x80,    // Prog IF: bus-master IDE, compatibility mode channels
    );
    ide_pci.bus = 0;
    ide_pci.device = 31;
    ide_pci.function = 1;
    ide_pci.set_bar(4, 0xC000, 0x10, false);
    ide_pci.set_interrupt(14, 1);
    ide_pci.set_subsystem(0x1AF4, 0x1100);
    bus.add_device(ide_pci);

    // VGA device at 0:2.0 — SeaBIOS scans PCI to detect display hardware.
    let mut vga_pci = devices::bus::PciDevice::new(
        0x1234,  // Vendor ID: QEMU standard VGA
        0x1111,  // Device ID: stdvga
        0x03,    // Class: Display controller
        0x00,    // Subclass: VGA compatible
        0x00,    // Prog IF: VGA
    );
    vga_pci.bus = 0;
    vga_pci.device = 2;
    vga_pci.function = 0;
    // BAR0: framebuffer at 0xFD000000 (256 MiB, matches typical VGA).
    vga_pci.set_bar(0, 0xFD000000, 0x01000000, true); // 16 MiB MMIO
    // BAR2: Bochs VBE MMIO (optional, not strictly needed).
    vga_pci.set_bar(2, 0xFEBE0000, 0x1000, true); // 4 KiB
    // Expansion ROM base address (offset 0x30): 0xC0000 with enable bit.
    vga_pci.config_space[0x30] = 0x01; // enabled
    vga_pci.config_space[0x31] = 0x00;
    vga_pci.config_space[0x32] = 0x0C; // 0x000C0001 LE
    vga_pci.config_space[0x33] = 0x00;
    vga_pci.set_subsystem(0x1AF4, 0x1100);
    bus.add_device(vga_pci);

    let bus_ptr = Box::into_raw(Box::new(bus));
    vm.bus_ptr = bus_ptr;
    vm.engine.io.register(0xCF8, 8, Box::new(IoProxy { ptr: bus_ptr }));

    // MMCONFIG — PCI Express Enhanced Configuration via MMIO.
    // 256 MiB region at 0xB0000000, matching PCIEXBAR in the Q35 MCH.
    // SeaBIOS reads PCIEXBAR, discovers this region, then uses MMIO reads
    // for all PCI config space accesses instead of CF8/CFC port I/O.
    let mmcfg = devices::bus::PciMmcfgHandler::new(bus_ptr);
    vm.engine.memory.add_mmio(
        0xB0000000,
        0x10000000, // 256 MiB
        Box::new(mmcfg),
    );

    // IO-APIC at standard MMIO address.
    let ioapic = Box::into_raw(Box::new(devices::ioapic::IoApic::new()));
    vm.ioapic_ptr = ioapic;
    vm.engine.memory.add_mmio(0xFEC00000, 0x1000, Box::new(MmioProxy { ptr: ioapic }));

    // Local APIC at standard MMIO address (0xFEE00000, 4 KB).
    // SeaBIOS probes LAPIC Version (0xFEE00030) to detect APIC support.
    // Without this, SeaBIOS reports "No apic" and may skip APIC-dependent init.
    let lapic = Box::into_raw(Box::new(devices::lapic::Lapic::new()));
    vm.lapic_ptr = lapic;
    vm.engine.memory.add_mmio(0xFEE00000, 0x1000, Box::new(MmioProxy { ptr: lapic }));

    // ACPI PM — Power Management timer and control registers.
    // ICH9 PMBASE = 0xB000; PM Timer at 0xB008 used by SeaBIOS for all delays.
    let acpi_pm = Box::into_raw(Box::new(devices::acpi::AcpiPm::new()));
    vm.acpi_pm_ptr = acpi_pm;
    vm.engine.io.register(0xB000, 0x40, Box::new(IoProxy { ptr: acpi_pm }));

    // fw_cfg — QEMU firmware configuration interface.
    // SeaBIOS uses this to discover platform config and VGA BIOS files.
    let fw_cfg = Box::into_raw(Box::new(
        devices::fw_cfg::FwCfg::new(ram_bytes as u64),
    ));
    vm.fw_cfg_ptr = fw_cfg;
    vm.engine.io.register(0x510, 2, Box::new(IoProxy { ptr: fw_cfg }));

    // Debug port — QEMU debug console at port 0x402.
    // SeaBIOS writes debug output here; reading 0xE9 signals port is active.
    let debug_port = Box::into_raw(Box::new(devices::debug_port::DebugPort::new()));
    vm.debug_port_ptr = debug_port;
    vm.engine.io.register(0x402, 1, Box::new(IoProxy { ptr: debug_port }));

    let count = vm.engine.memory.mmio_region_count();
    let (lo, hi) = vm.engine.memory.mmio_bounds();
    vm_log!("MMIO setup: {} regions, bounds=[0x{:X}, 0x{:X})", count, lo, hi);
    vm_log!("PCI bus: 4 devices (Q35 MCH 0:0.0, ICH9 LPC 0:1F.0, IDE 0:1F.1, VGA 0:2.0)");
}

/// Register a PCI bus at the standard configuration ports (0xCF8-0xCFF).
///
/// Must only be called once per VM instance.
#[no_mangle]
pub extern "C" fn corevm_setup_pci_bus(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if !vm.bus_ptr.is_null() {
        vm_log!("PCI bus already set up, skipping");
        return;
    }
    vm_log!("setting up PCI bus (ports 0xCF8-0xCFF)");

    let bus = Box::into_raw(Box::new(devices::bus::PciBus::new()));
    vm.bus_ptr = bus;
    vm.engine.io.register(0xCF8, 8, Box::new(IoProxy { ptr: bus }));
}

/// Register an Intel E1000 network card at the specified MMIO base address.
///
/// `mac` must point to exactly 6 bytes (the MAC address). If `mac` is null,
/// the default MAC 52:54:00:12:34:56 is used.
///
/// The E1000 uses MMIO (128 KB region), not port I/O.
#[no_mangle]
pub extern "C" fn corevm_setup_e1000(handle: u64, mmio_base: u64, mac: *const u8) {
    vm_log!("setting up E1000 NIC at MMIO 0x{:X}", mmio_base);
    let vm = unsafe { vm_from_handle(handle) };

    let mac_bytes = if mac.is_null() {
        [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]
    } else {
        let slice = unsafe { core::slice::from_raw_parts(mac, 6) };
        [slice[0], slice[1], slice[2], slice[3], slice[4], slice[5]]
    };

    let e1000 = Box::into_raw(Box::new(devices::e1000::E1000::new(mac_bytes)));
    vm.e1000_ptr = e1000;
    vm.engine.memory.add_mmio(
        mmio_base,
        0x20000, // 128 KB register space
        Box::new(MmioProxy { ptr: e1000 }),
    );
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — PS/2
// ════════════════════════════════════════════════════════════════════════

/// Inject a keyboard key-press (make) scancode into the PS/2 controller.
///
/// No-op if standard devices have not been set up.
#[no_mangle]
pub extern "C" fn corevm_ps2_key_press(handle: u64, scancode: u8) {
    let vm = unsafe { vm_from_handle(handle) };
    if !vm.ps2_ptr.is_null() {
        unsafe { (*vm.ps2_ptr).key_press(scancode) };
    }
    // Raise IRQ 1 (keyboard) so the BIOS INT 09h handler fires.
    raise_keyboard_irq(vm);
}

/// Inject a keyboard key-release (break) scancode into the PS/2 controller.
///
/// No-op if standard devices have not been set up.
#[no_mangle]
pub extern "C" fn corevm_ps2_key_release(handle: u64, scancode: u8) {
    let vm = unsafe { vm_from_handle(handle) };
    if !vm.ps2_ptr.is_null() {
        unsafe { (*vm.ps2_ptr).key_release(scancode) };
    }
    // Raise IRQ 1 for the break code as well.
    raise_keyboard_irq(vm);
}

/// Helper: raise IRQ 1 (keyboard) on the PIC and inject the resulting
/// interrupt vector into the CPU.
fn raise_keyboard_irq(vm: &mut VmInstance) {
    inject_irq_line(vm, 1);
}

/// Inject a mouse movement/button event into the PS/2 controller.
///
/// `dx` and `dy` are relative displacement; `buttons` is a bitmask
/// (bit 0=left, bit 1=right, bit 2=middle).
///
/// No-op if standard devices have not been set up.
#[no_mangle]
pub extern "C" fn corevm_ps2_mouse_move(handle: u64, dx: i16, dy: i16, buttons: u8) {
    let vm = unsafe { vm_from_handle(handle) };
    if !vm.ps2_ptr.is_null() {
        unsafe { (*vm.ps2_ptr).mouse_move(dx, dy, buttons) };
    }
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — VGA / SVGA
// ════════════════════════════════════════════════════════════════════════

/// Get a pointer to the VGA framebuffer and fill in the current dimensions.
///
/// On success, `*width`, `*height`, and `*bpp` are set to the current mode's
/// parameters. Returns a pointer to the raw pixel data, or null if the VGA
/// device has not been set up.
///
/// The returned pointer is valid until the next call that modifies VGA state
/// (e.g., a mode switch triggered by VM execution).
#[no_mangle]
pub extern "C" fn corevm_vga_get_framebuffer(
    handle: u64,
    width: *mut u32,
    height: *mut u32,
    bpp: *mut u8,
) -> *const u8 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.svga_ptr.is_null() {
        return ptr::null();
    }
    let svga = unsafe { &*vm.svga_ptr };
    match svga.mode {
        devices::svga::VgaMode::Text80x25 => {
            if !width.is_null() { unsafe { *width = 0 }; }
            if !height.is_null() { unsafe { *height = 0 }; }
            if !bpp.is_null() { unsafe { *bpp = 0 }; }
            ptr::null()
        }
        _ => {
            if !width.is_null() {
                unsafe { *width = svga.width };
            }
            if !height.is_null() {
                unsafe { *height = svga.height };
            }
            if !bpp.is_null() {
                unsafe { *bpp = svga.bpp };
            }
            svga.framebuffer.as_ptr()
        }
    }
}

/// Get a pointer to the VGA text-mode buffer (80x25 cells, `u16` per cell).
///
/// Each cell: low byte = ASCII character, high byte = color attribute.
/// If `count` is non-null, `*count` is set to the number of `u16` cells (2000).
/// Returns null if the VGA device has not been set up.
#[no_mangle]
pub extern "C" fn corevm_vga_get_text_buffer(handle: u64, count: *mut u32) -> *const u16 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.svga_ptr.is_null() {
        if !count.is_null() {
            unsafe { *count = 0 };
        }
        return ptr::null();
    }
    let svga = unsafe { &*vm.svga_ptr };
    match svga.mode {
        devices::svga::VgaMode::Text80x25 => {
            if !count.is_null() {
                unsafe { *count = svga.text_buffer.len() as u32 };
            }
            svga.text_buffer.as_ptr()
        }
        _ => {
            // Not in text mode.
            if !count.is_null() {
                unsafe { *count = 0 };
            }
            ptr::null()
        }
    }
}

/// Get VGA MMIO debug counters.
///
/// Returns the total MMIO write count and the text-region write count
/// through the output pointers. Useful for diagnosing whether writes
/// to the VGA framebuffer are reaching the device handler.
#[no_mangle]
pub extern "C" fn corevm_vga_debug_counters(
    handle: u64,
    total_writes: *mut u64,
    text_writes: *mut u64,
) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.svga_ptr.is_null() {
        return;
    }
    let svga = unsafe { &*vm.svga_ptr };
    if !total_writes.is_null() {
        unsafe { *total_writes = svga.mmio_write_count };
    }
    if !text_writes.is_null() {
        unsafe { *text_writes = svga.mmio_text_write_count };
    }
}

/// Diagnostic: get MMIO region count and bounds, plus raw RAM at 0xB8000.
///
/// Helps diagnose whether MMIO regions are properly registered and
/// whether writes to the VGA text area are hitting RAM instead of MMIO.
///
/// Output:
/// - `region_count`: number of registered MMIO regions
/// - `min_base`: MMIO fast-reject lower bound
/// - `max_end`: MMIO fast-reject upper bound
/// - `ram_b8000`: first 4 bytes of raw RAM at physical 0xB8000
#[no_mangle]
pub extern "C" fn corevm_mmio_diag(
    handle: u64,
    region_count: *mut u32,
    min_base: *mut u64,
    max_end: *mut u64,
    ram_b8000: *mut u32,
) {
    let vm = unsafe { vm_from_handle(handle) };
    let count = vm.engine.memory.mmio_region_count();
    let (lo, hi) = vm.engine.memory.mmio_bounds();
    if !region_count.is_null() {
        unsafe { *region_count = count as u32 };
    }
    if !min_base.is_null() {
        unsafe { *min_base = lo };
    }
    if !max_end.is_null() {
        unsafe { *max_end = hi };
    }
    if !ram_b8000.is_null() {
        // Read directly from flat RAM (bypasses MMIO).
        use memory::MemoryBus;
        let val = vm.engine.memory.ram().read_u32(0xB8000).unwrap_or(0);
        unsafe { *ram_b8000 = val };
    }
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — Serial
// ════════════════════════════════════════════════════════════════════════

/// Push input data into the serial port's receive buffer.
///
/// The guest will see this data when it reads the Receive Buffer Register.
/// No-op if `data` is null, `len` is 0, or serial has not been set up.
#[no_mangle]
pub extern "C" fn corevm_serial_send_input(handle: u64, data: *const u8, len: u32) {
    if data.is_null() || len == 0 {
        return;
    }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.serial_ptr.is_null() {
        return;
    }
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    unsafe { (*vm.serial_ptr).send_input(slice) };
}

/// Drain serial output written by the guest into the provided buffer.
///
/// Returns the number of bytes written to `buf`. If the output is larger
/// than `buf_len`, only `buf_len` bytes are copied (remaining data is lost).
/// Returns 0 if `buf` is null or serial has not been set up.
#[no_mangle]
pub extern "C" fn corevm_serial_take_output(
    handle: u64,
    buf: *mut u8,
    buf_len: u32,
) -> u32 {
    if buf.is_null() || buf_len == 0 {
        return 0;
    }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.serial_ptr.is_null() {
        return 0;
    }
    let output = unsafe { (*vm.serial_ptr).take_output() };
    let copy_len = (output.len() as u32).min(buf_len) as usize;
    if copy_len > 0 {
        unsafe {
            ptr::copy_nonoverlapping(output.as_ptr(), buf, copy_len);
        }
    }
    copy_len as u32
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — Debug Port
// ════════════════════════════════════════════════════════════════════════

/// Drain debug port output written by the guest into the provided buffer.
///
/// SeaBIOS writes debug messages to port 0x402. This function returns the
/// accumulated bytes. Returns 0 if `buf` is null or the debug port has not
/// been set up.
#[no_mangle]
pub extern "C" fn corevm_debug_take_output(
    handle: u64,
    buf: *mut u8,
    buf_len: u32,
) -> u32 {
    if buf.is_null() || buf_len == 0 {
        return 0;
    }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.debug_port_ptr.is_null() {
        return 0;
    }
    let output = unsafe { (*vm.debug_port_ptr).take_output() };
    let copy_len = (output.len() as u32).min(buf_len) as usize;
    if copy_len > 0 {
        unsafe {
            ptr::copy_nonoverlapping(output.as_ptr(), buf, copy_len);
        }
    }
    copy_len as u32
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — E1000
// ════════════════════════════════════════════════════════════════════════

/// Inject a received network packet into the E1000 RX buffer.
///
/// No-op if `data` is null, `len` is 0, or E1000 has not been set up.
#[no_mangle]
pub extern "C" fn corevm_e1000_receive_packet(handle: u64, data: *const u8, len: u32) {
    if data.is_null() || len == 0 {
        return;
    }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.e1000_ptr.is_null() {
        return;
    }
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    unsafe { (*vm.e1000_ptr).receive_packet(slice) };
}

/// Drain transmitted packets from the E1000 TX buffer into a flat buffer.
///
/// Packets are serialized as: `[u32 length][payload bytes]` repeated.
/// Returns the total number of bytes written to `buf`. If the buffer is
/// too small to fit all packets, only complete packets that fit are written.
/// Returns 0 if `buf` is null or E1000 has not been set up.
#[no_mangle]
pub extern "C" fn corevm_e1000_take_tx_packets(
    handle: u64,
    buf: *mut u8,
    buf_len: u32,
) -> u32 {
    if buf.is_null() || buf_len == 0 {
        return 0;
    }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.e1000_ptr.is_null() {
        return 0;
    }
    let packets = unsafe { (*vm.e1000_ptr).take_tx_packets() };
    let mut offset: u32 = 0;
    for pkt in &packets {
        let header_size = 4u32; // u32 length prefix
        let pkt_len = pkt.len() as u32;
        let needed = header_size + pkt_len;
        if offset + needed > buf_len {
            break; // Not enough room for this packet.
        }
        unsafe {
            // Write length prefix (little-endian u32).
            let len_bytes = pkt_len.to_le_bytes();
            ptr::copy_nonoverlapping(len_bytes.as_ptr(), buf.add(offset as usize), 4);
            offset += header_size;
            // Write packet payload.
            if pkt_len > 0 {
                ptr::copy_nonoverlapping(pkt.as_ptr(), buf.add(offset as usize), pkt_len as usize);
            }
            offset += pkt_len;
        }
    }
    offset
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — PIT
// ════════════════════════════════════════════════════════════════════════

/// Advance the PIT by one tick.
///
/// Returns 1 if channel 0 fired (IRQ 0 should be raised), 0 otherwise.
/// Returns 0 if PIT has not been set up.
#[no_mangle]
pub extern "C" fn corevm_pit_tick(handle: u64) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.pit_ptr.is_null() {
        return 0;
    }
    vm.pit_is_externally_clocked = true;
    let fired = unsafe { (*vm.pit_ptr).tick() };
    if fired { 1 } else { 0 }
}

/// Advance the PIT by `n` ticks in bulk.
///
/// Returns the number of times channel 0 fired.
/// For each fire, the caller should raise IRQ 0 on the PIC.
/// Returns 0 if PIT has not been set up.
#[no_mangle]
pub extern "C" fn corevm_pit_advance(handle: u64, n: u32) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.pit_ptr.is_null() {
        return 0;
    }
    vm.pit_is_externally_clocked = true;
    unsafe { (*vm.pit_ptr).advance(n) }
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — PIC
// ════════════════════════════════════════════════════════════════════════

/// Assert an IRQ line on the PIC (edge-triggered) and inject the resulting
/// interrupt vector into the CPU's interrupt controller.
///
/// IRQ 0-7 go to the master PIC, 8-15 to the slave. The PIC is polled for
/// the highest-priority pending vector, which is then queued for delivery
/// at the top of the next CPU instruction cycle (when IF=1).
/// No-op if PIC has not been set up.
#[no_mangle]
pub extern "C" fn corevm_pic_raise_irq(handle: u64, irq: u8) {
    let vm = unsafe { vm_from_handle(handle) };
    inject_irq_line(vm, irq);
}

/// Route one external IRQ line through available interrupt controllers.
///
/// We support both legacy PIC and IO-APIC routing because guests may switch
/// from 8259 PIC to IO-APIC during early boot. Deliverable vectors from either
/// path are queued into the CPU interrupt controller.
fn inject_irq_line(vm: &mut VmInstance, irq: u8) {
    let mut delivered = false;

    // Legacy 8259 PIC path.
    if !vm.pic_ptr.is_null() {
        let pic = unsafe { &mut *vm.pic_ptr };
        pic.raise_irq(irq);
        if let Some(vector) = pic.get_interrupt_vector() {
            let ack_irq = pic.irq_for_vector(vector).unwrap_or(irq);
            pic.acknowledge(ack_irq);
            vm.engine.interrupts.raise_irq(vector);
            delivered = true;
        }
    }

    // IO-APIC path (single-vCPU fixed-delivery subset).
    if !vm.ioapic_ptr.is_null() {
        let ioapic = unsafe { &mut *vm.ioapic_ptr };
        if let Some(vector) = ioapic.route_irq(irq) {
            vm.engine.interrupts.raise_irq(vector);
            delivered = true;
        }
    }

}

/// Get the vector number of the highest-priority pending interrupt.
///
/// Returns the interrupt vector (0-255) or -1 if no interrupt is pending.
/// Returns -1 if PIC has not been set up.
#[no_mangle]
pub extern "C" fn corevm_pic_get_interrupt(handle: u64) -> i32 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.pic_ptr.is_null() {
        return -1;
    }
    match unsafe { (*vm.pic_ptr).get_interrupt_vector() } {
        Some(vec) => vec as i32,
        None => -1,
    }
}

/// Return PIC state packed into a u64 for diagnostics.
///
/// Layout:
/// - bits 0..7:   master IRR
/// - bits 8..15:  master ISR
/// - bits 16..23: master IMR
/// - bits 24..31: slave IRR
/// - bits 32..39: slave ISR
/// - bits 40..47: slave IMR
#[no_mangle]
pub extern "C" fn corevm_pic_diag_state(handle: u64) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.pic_ptr.is_null() {
        return 0;
    }
    let pic = unsafe { &*vm.pic_ptr };
    (pic.master.irr as u64)
        | ((pic.master.isr as u64) << 8)
        | ((pic.master.imr as u64) << 16)
        | ((pic.slave.irr as u64) << 24)
        | ((pic.slave.isr as u64) << 32)
        | ((pic.slave.imr as u64) << 40)
}

/// Return key LAPIC timer state for diagnostics.
///
/// Return value layout:
/// - bits  0..31: SVR
/// - bits 32..63: LVT Timer
///
/// Optional out-pointers receive:
/// - `init_count_out`: Timer Initial Count register
/// - `cur_count_out`: Timer Current Count register
/// - `timer_divide_out`: Timer Divide Configuration register
#[no_mangle]
pub extern "C" fn corevm_lapic_diag_state(
    handle: u64,
    init_count_out: *mut u32,
    cur_count_out: *mut u32,
    timer_divide_out: *mut u32,
) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.lapic_ptr.is_null() {
        if !init_count_out.is_null() {
            unsafe { *init_count_out = 0; }
        }
        if !cur_count_out.is_null() {
            unsafe { *cur_count_out = 0; }
        }
        if !timer_divide_out.is_null() {
            unsafe { *timer_divide_out = 0; }
        }
        return 0;
    }
    let lapic = unsafe { &mut *vm.lapic_ptr };
    use crate::memory::mmio::MmioHandler;
    let svr = lapic.read(0x0F0, 4).unwrap_or(0) as u32;
    let lvt_timer = lapic.read(0x320, 4).unwrap_or(0) as u32;
    let init = lapic.read(0x380, 4).unwrap_or(0) as u32;
    let cur = lapic.read(0x390, 4).unwrap_or(0) as u32;
    let div = lapic.read(0x3E0, 4).unwrap_or(0) as u32;
    if !init_count_out.is_null() {
        unsafe { *init_count_out = init; }
    }
    if !cur_count_out.is_null() {
        unsafe { *cur_count_out = cur; }
    }
    if !timer_divide_out.is_null() {
        unsafe { *timer_divide_out = div; }
    }
    (svr as u64) | ((lvt_timer as u64) << 32)
}

/// Return one pending-interrupt bitmap word from the CPU interrupt controller.
///
/// `idx` selects the 64-bit word:
/// - 0: vectors 0..63
/// - 1: vectors 64..127
/// - 2: vectors 128..191
/// - 3: vectors 192..255
#[no_mangle]
pub extern "C" fn corevm_irq_pending_word(handle: u64, idx: u32) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.interrupts.pending_word(idx as usize)
}

/// Return a raw IOAPIC redirection entry for a given IRQ line.
///
/// Returns 0 when IOAPIC is not present or `irq` is out of range.
#[no_mangle]
pub extern "C" fn corevm_ioapic_redir_entry(handle: u64, irq: u8) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ioapic_ptr.is_null() {
        return 0;
    }
    let ioapic = unsafe { &*vm.ioapic_ptr };
    ioapic.redir_entry(irq)
}

// ════════════════════════════════════════════════════════════════════════
// Device Setup — IDE/ATA Disk Controller
// ════════════════════════════════════════════════════════════════════════

/// Register an ATA/IDE disk controller on the primary channel.
///
/// Registers I/O handlers at ports 0x1F0-0x1F7 (command block) and
/// 0x3F6-0x3F7 (control block). Must only be called once per VM instance.
#[no_mangle]
pub extern "C" fn corevm_setup_ide(handle: u64) {
    vm_log!("setting up IDE controller (ports 0x1F0-0x1F7, 0x3F6-0x3F7)");
    let vm = unsafe { vm_from_handle(handle) };

    let ide = Box::into_raw(Box::new(devices::ide::Ide::new()));
    vm.ide_ptr = ide;
    vm.engine.io.register(0x1F0, 8, Box::new(IoProxy { ptr: ide }));
    vm.engine.io.register(0x3F6, 2, Box::new(IoProxy { ptr: ide }));
}

/// Attach an in-memory disk image to the IDE master drive.
///
/// `data` points to the raw disk image bytes; `len` is the byte count.
/// The data is copied into the VM — the caller retains ownership of the
/// source buffer. No-op if `data` is null or IDE has not been set up.
#[no_mangle]
pub extern "C" fn corevm_ide_attach_disk(handle: u64, data: *const u8, len: u32) {
    if data.is_null() || len == 0 {
        return;
    }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() {
        return;
    }
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    vm_log!("attaching IDE master disk image ({} bytes)", len);
    let mut image = alloc::vec::Vec::with_capacity(len as usize);
    image.extend_from_slice(slice);
    unsafe { (*vm.ide_ptr).attach_disk(image) };
}

/// Attach an in-memory disk image to the IDE slave drive.
///
/// Same semantics as [`corevm_ide_attach_disk`] but for the slave (drive 1).
#[no_mangle]
pub extern "C" fn corevm_ide_attach_slave(handle: u64, data: *const u8, len: u32) {
    if data.is_null() || len == 0 {
        return;
    }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() {
        return;
    }
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    vm_log!("attaching IDE slave disk image ({} bytes)", len);
    let mut image = alloc::vec::Vec::with_capacity(len as usize);
    image.extend_from_slice(slice);
    unsafe { (*vm.ide_ptr).attach_slave(image) };
}

/// Attach a file-descriptor-backed disk to the IDE master drive.
///
/// Instead of copying the entire image into RAM, the IDE controller reads
/// sectors on demand via the given file descriptor. The caller must keep
/// `fd` open for the lifetime of the VM. `size` is the file size in bytes.
#[no_mangle]
pub extern "C" fn corevm_ide_attach_disk_fd(handle: u64, fd: u32, size: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() {
        return;
    }
    vm_log!("attaching IDE master disk via fd {} ({} bytes)", fd, size);
    unsafe { (*vm.ide_ptr).attach_disk_fd(fd as i32, size) };
}

/// Attach a file-descriptor-backed disk to the IDE slave drive.
///
/// Same semantics as [`corevm_ide_attach_disk_fd`] but for the slave.
#[no_mangle]
pub extern "C" fn corevm_ide_attach_slave_fd(handle: u64, fd: u32, size: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() {
        return;
    }
    vm_log!("attaching IDE slave disk via fd {} ({} bytes)", fd, size);
    unsafe { (*vm.ide_ptr).attach_slave_fd(fd as i32, size) };
}

/// Detach the master disk image from the IDE controller.
///
/// The image data is freed (or the FD is closed). No-op if IDE has not
/// been set up or no disk is attached.
#[no_mangle]
pub extern "C" fn corevm_ide_detach_disk(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() {
        return;
    }
    unsafe { (*vm.ide_ptr).detach_disk() };
}

/// Detach the slave disk image from the IDE controller.
#[no_mangle]
pub extern "C" fn corevm_ide_detach_slave(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() {
        return;
    }
    unsafe { (*vm.ide_ptr).detach_slave() };
}

/// Check whether the IDE controller has a pending IRQ (IRQ 14).
///
/// Returns 1 if an IRQ is pending, 0 otherwise.
/// Returns 0 if IDE has not been set up.
#[no_mangle]
pub extern "C" fn corevm_ide_irq_raised(handle: u64) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() {
        return 0;
    }
    if unsafe { (*vm.ide_ptr).irq_raised() } { 1 } else { 0 }
}

/// Clear the pending IDE IRQ.
///
/// No-op if IDE has not been set up.
#[no_mangle]
pub extern "C" fn corevm_ide_clear_irq(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() {
        return;
    }
    unsafe { (*vm.ide_ptr).clear_irq() };
}

// ════════════════════════════════════════════════════════════════════════
// JIT / Decode Cache
// ════════════════════════════════════════════════════════════════════════

/// Query decode cache statistics.
///
/// Writes the number of cached blocks, cache hits, and cache misses to the
/// provided output pointers. Any pointer may be null (skipped).
#[no_mangle]
pub extern "C" fn corevm_jit_cache_stats(
    handle: u64,
    cached_blocks: *mut u32,
    hits: *mut u64,
    misses: *mut u64,
) {
    let vm = unsafe { vm_from_handle(handle) };
    let cache = &vm.engine.cpu.decode_cache;
    if !cached_blocks.is_null() {
        unsafe { *cached_blocks = cache.len() as u32 };
    }
    if !hits.is_null() {
        unsafe { *hits = cache.hits() };
    }
    if !misses.is_null() {
        unsafe { *misses = cache.misses() };
    }
}

/// Flush the decode cache, forcing all basic blocks to be re-decoded.
///
/// Useful after loading new code into guest memory or when self-modifying
/// code is detected.
#[no_mangle]
pub extern "C" fn corevm_jit_flush_cache(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.decode_cache.flush();
    vm.engine.cpu.jit_engine.flush();
}

/// Enable or disable the JIT engine.
///
/// When enabled, the VM compiles hot basic blocks to native x86-64 code
/// for dramatically faster execution. When disabled (default), all
/// instructions are interpreted.
///
/// # Arguments
/// * `handle` — VM instance handle
/// * `enable` — 1 to enable, 0 to disable
#[no_mangle]
pub extern "C" fn corevm_jit_enable(handle: u64, enable: u32) {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.jit_engine.set_enabled(enable != 0);
    if enable != 0 {
        vm_log!("JIT engine enabled");
    } else {
        vm_log!("JIT engine disabled");
    }
}

/// Query JIT engine statistics.
///
/// Writes the number of compiled blocks, natively translated instruction
/// count, interpreter fallback count, and code buffer usage to the
/// provided output pointers. Any pointer may be null (skipped).
#[no_mangle]
pub extern "C" fn corevm_jit_stats(
    handle: u64,
    blocks_compiled: *mut u64,
    native_count: *mut u64,
    fallback_count: *mut u64,
    code_buffer_used: *mut u32,
) {
    let vm = unsafe { vm_from_handle(handle) };
    let jit = &vm.engine.cpu.jit_engine;
    if !blocks_compiled.is_null() {
        unsafe { *blocks_compiled = jit.blocks_compiled() };
    }
    if !native_count.is_null() {
        unsafe { *native_count = jit.native_count() };
    }
    if !fallback_count.is_null() {
        unsafe { *fallback_count = jit.fallback_count() };
    }
    if !code_buffer_used.is_null() {
        unsafe { *code_buffer_used = jit.code_buffer_used() as u32 };
    }
}
