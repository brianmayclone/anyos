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

#![no_std]
#![no_main]

extern crate alloc;
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

/// Syscall wrappers for the allocator, panic handler, and debug output.
mod syscall {
    pub use libsyscall::{sbrk, mmap, munmap, exit, serial_print, write_bytes};
}

/// Print a formatted line to the serial console (stdout fd=1).
macro_rules! vm_log {
    ($($arg:tt)*) => {{
        libsyscall::serial_print(format_args!("[corevm] "));
        libsyscall::serial_print(format_args!($($arg)*));
        libsyscall::write_bytes(b"\n");
    }};
}

libheap::dll_allocator!(crate::syscall::sbrk, crate::syscall::mmap, crate::syscall::munmap);

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
}

impl VmEngine {
    /// Create a new VM with the specified guest RAM size in bytes.
    ///
    /// The CPU starts in real mode at the standard reset vector (CS:IP = F000:FFF0).
    pub fn new(ram_size: usize) -> Self {
        VmEngine {
            cpu: Cpu::new(),
            memory: GuestMemory::new(ram_size),
            mmu: Mmu::new(),
            interrupts: InterruptController::new(),
            io: IoDispatch::new(),
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
    ps2_ptr: *mut devices::ps2::Ps2Controller,
    serial_ptr: *mut devices::serial::Serial,
    svga_ptr: *mut devices::svga::Svga,
    e1000_ptr: *mut devices::e1000::E1000,
    bus_ptr: *mut devices::bus::PciBus,
    ide_ptr: *mut devices::ide::Ide,
}

impl Drop for VmInstance {
    fn drop(&mut self) {
        // Free all heap-allocated devices. The proxies hold dangling pointers
        // after this, but they are destroyed together with the engine.
        unsafe {
            if !self.pic_ptr.is_null() { let _ = Box::from_raw(self.pic_ptr); }
            if !self.pit_ptr.is_null() { let _ = Box::from_raw(self.pit_ptr); }
            if !self.ps2_ptr.is_null() { let _ = Box::from_raw(self.ps2_ptr); }
            if !self.serial_ptr.is_null() { let _ = Box::from_raw(self.serial_ptr); }
            if !self.svga_ptr.is_null() { let _ = Box::from_raw(self.svga_ptr); }
            if !self.e1000_ptr.is_null() { let _ = Box::from_raw(self.e1000_ptr); }
            if !self.bus_ptr.is_null() { let _ = Box::from_raw(self.bus_ptr); }
            if !self.ide_ptr.is_null() { let _ = Box::from_raw(self.ide_ptr); }
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
    vm_log!("creating VM with {} MiB RAM", ram_size_mb);
    let ram_bytes = (ram_size_mb as usize) * 1024 * 1024;
    let instance = Box::new(VmInstance {
        engine: VmEngine::new(ram_bytes),
        last_error: None,
        last_error_rip: 0,
        pic_ptr: ptr::null_mut(),
        pit_ptr: ptr::null_mut(),
        ps2_ptr: ptr::null_mut(),
        serial_ptr: ptr::null_mut(),
        svga_ptr: ptr::null_mut(),
        e1000_ptr: ptr::null_mut(),
        bus_ptr: ptr::null_mut(),
        ide_ptr: ptr::null_mut(),
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
    let exit = vm.engine.run(max_instructions);
    match exit {
        ExitReason::Halted => {
            vm_log!("VM halted after {} instructions", vm.engine.instruction_count());
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

    // CMOS — RTC and NVRAM. Ram size derived from engine memory.
    // We pass a representative size; the CMOS constructor populates memory fields.
    let cmos = Box::new(devices::cmos::Cmos::new(16 * 1024 * 1024));
    vm.engine.io.register(0x70, 2, cmos);

    // PS/2 — keyboard and mouse controller.
    let ps2 = Box::into_raw(Box::new(devices::ps2::Ps2Controller::new()));
    vm.ps2_ptr = ps2;
    vm.engine.io.register(0x60, 1, Box::new(IoProxy { ptr: ps2 }));
    vm.engine.io.register(0x64, 1, Box::new(IoProxy { ptr: ps2 }));

    // Serial (COM1) — 16550 UART.
    let serial = Box::into_raw(Box::new(devices::serial::Serial::new()));
    vm.serial_ptr = serial;
    vm.engine.io.register(0x3F8, 8, Box::new(IoProxy { ptr: serial }));

    // VGA/SVGA — standard VGA ports + legacy framebuffer MMIO.
    let svga = Box::into_raw(Box::new(devices::svga::Svga::new(800, 600)));
    vm.svga_ptr = svga;
    vm.engine.io.register(0x3C0, 0x1B, Box::new(IoProxy { ptr: svga }));
    vm.engine.memory.add_mmio(0xA0000, 0x20000, Box::new(MmioProxy { ptr: svga }));
}

/// Register a PCI bus at the standard configuration ports (0xCF8-0xCFF).
///
/// Must only be called once per VM instance.
#[no_mangle]
pub extern "C" fn corevm_setup_pci_bus(handle: u64) {
    vm_log!("setting up PCI bus (ports 0xCF8-0xCFF)");
    let vm = unsafe { vm_from_handle(handle) };

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

/// Get a pointer to the VGA text-mode buffer (80x25 cells, `u16` per cell).
///
/// Each cell: low byte = ASCII character, high byte = color attribute.
/// If `count` is non-null, `*count` is set to the number of `u16` cells (2000).
/// Returns null if the VGA device has not been set up.
#[no_mangle]
pub extern "C" fn corevm_vga_get_text_buffer(handle: u64, count: *mut u32) -> *const u16 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.svga_ptr.is_null() {
        return ptr::null();
    }
    let svga = unsafe { &*vm.svga_ptr };
    if !count.is_null() {
        unsafe { *count = svga.text_buffer.len() as u32 };
    }
    svga.text_buffer.as_ptr()
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
    let fired = unsafe { (*vm.pit_ptr).tick() };
    if fired { 1 } else { 0 }
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — PIC
// ════════════════════════════════════════════════════════════════════════

/// Assert an IRQ line on the PIC (edge-triggered).
///
/// IRQ 0-7 go to the master PIC, 8-15 to the slave. No-op if PIC has not
/// been set up.
#[no_mangle]
pub extern "C" fn corevm_pic_raise_irq(handle: u64, irq: u8) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.pic_ptr.is_null() {
        return;
    }
    unsafe { (*vm.pic_ptr).raise_irq(irq) };
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

/// Attach a disk image to the IDE controller.
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
    vm_log!("attaching IDE disk image ({} bytes)", len);
    let mut image = alloc::vec::Vec::with_capacity(len as usize);
    image.extend_from_slice(slice);
    unsafe { (*vm.ide_ptr).attach_disk(image) };
}

/// Detach the disk image from the IDE controller.
///
/// The image data is freed. No-op if IDE has not been set up or no disk
/// is attached.
#[no_mangle]
pub extern "C" fn corevm_ide_detach_disk(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() {
        return;
    }
    unsafe { (*vm.ide_ptr).detach_disk() };
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
