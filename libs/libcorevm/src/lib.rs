//! libcorevm — Hardware-virtualized x86 virtual machine library for anyOS.
//!
//! Provides hardware-backed x86 virtualization using:
//! - Intel VT-x (VMX) / AMD-V (SVM) — direct hardware on anyOS
//! - KVM — on Linux
//! - Windows Hypervisor Platform (WHP) — on Windows
//! - Apple Hypervisor.framework (HVF) — on macOS
//!
//! All instruction execution is performed by the hardware — no software
//! emulation or JIT.
//!
//! # Architecture
//!
//! The library is organized into these layers:
//! - **Hypervisor** (`hypervisor/`) — abstraction over VT-x/AMD-V/KVM/WHP/HVF
//! - **CPU** (`cpu.rs`) — hardware-backed vCPU management and VM-exit dispatch
//! - **Memory** (`memory/`) — guest RAM, MMIO dispatch, ROM regions
//! - **Devices** (`devices/`) — emulated hardware (SVGA, PS/2, E1000, etc.)
//! - **I/O** (`io.rs`) — port I/O dispatch table
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
pub mod hypervisor;
pub mod memory;
pub mod cpu;
pub mod interrupts;
pub mod io;
pub mod devices;

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
pub use hypervisor::{VmExit, HvError};

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ptr;
#[cfg(feature = "host_test")]
use core::sync::atomic::{AtomicU32, Ordering};

#[cfg(feature = "host_test")]
static IRQ_TRACE_BUDGET: AtomicU32 = AtomicU32::new(96);

// ── VmEngine ──

/// High-level VM engine — combines CPU, memory, and I/O components.
///
/// The CPU is hardware-backed via the hypervisor abstraction layer.
/// Execution uses VM-exit driven dispatch rather than instruction emulation.
pub struct VmEngine {
    /// Virtual CPU backed by hardware virtualization.
    pub cpu: Cpu,
    /// Guest physical memory (RAM + MMIO regions).
    pub memory: GuestMemory,
    /// Memory management unit (segmentation + paging translation).
    pub mmu: Mmu,
    /// Interrupt controller (pending interrupt tracking).
    pub interrupts: InterruptController,
    /// Port I/O dispatcher (maps port ranges to device handlers).
    pub io: IoDispatch,
    /// Configured logical CPU count exposed to the guest.
    pub vcpu_count: u8,
}

impl VmEngine {
    /// Create a new VM with the specified guest RAM size in bytes.
    pub fn new(ram_size: usize) -> core::result::Result<Self, HvError> {
        Self::new_with_vcpus(ram_size, 1)
    }

    /// Create a new VM with a configured logical CPU count.
    pub fn new_with_vcpus(ram_size: usize, vcpu_count: u8) -> core::result::Result<Self, HvError> {
        let count = vcpu_count.max(1);
        let mut cpu = Cpu::new()?;
        cpu.configure_topology(0, count);
        cpu.init_vm()?;
        let memory = GuestMemory::new(ram_size);

        // Map guest RAM into the hypervisor backend
        let (ram_ptr, ram_len) = memory.ram_ptr();
        cpu.map_memory(0, 0, ram_len as u64, ram_ptr as *mut u8, false)?;

        let mut mmu = Mmu::new();
        mmu.set_ram_ptr(ram_ptr as *mut u8, ram_len);

        Ok(VmEngine {
            cpu,
            memory,
            mmu,
            interrupts: InterruptController::new(),
            io: IoDispatch::new(),
            vcpu_count: count,
        })
    }

    /// Load raw binary data at a guest physical address.
    pub fn load_binary(&mut self, addr: usize, data: &[u8]) {
        self.memory.load_at(addr, data);
    }

    /// Load a firmware ROM into guest memory at a physical address.
    pub fn load_rom(&mut self, base: u64, data: Vec<u8>) {
        let end = base as usize + data.len();
        if end <= self.memory.ram().size() {
            self.memory.load_at(base as usize, &data);
        } else {
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

    /// Get the current CPU mode.
    pub fn mode(&self) -> Mode {
        self.cpu.mode
    }

    /// Request the VM to stop at the next opportunity.
    pub fn request_stop(&mut self) {
        self.cpu.request_stop();
    }

    /// Reset the VM to power-on state.
    pub fn reset(&mut self) {
        let _ = self.cpu.reset();
        self.cpu.configure_topology(0, self.vcpu_count);
        self.mmu = Mmu::new();
        let (ram_p, ram_s) = self.memory.ram_ptr();
        self.mmu.set_ram_ptr(ram_p as *mut u8, ram_s);
        self.interrupts = InterruptController::new();
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
}


// ════════════════════════════════════════════════════════════════════════
// C ABI layer — opaque handle-based interface for dl_sym() consumers.
// ════════════════════════════════════════════════════════════════════════

// ── IoProxy ──

/// Thin proxy that forwards [`IoHandler`] calls through a raw pointer.
struct IoProxy<T: IoHandler> {
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
struct MmioProxy<T: MmioHandler> {
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
struct VmInstance {
    /// The core VM engine (CPU, memory, MMU, interrupt controller, I/O dispatch).
    engine: VmEngine,

    /// Last error that caused the VM to exit, if any.
    last_error: Option<error::VmError>,
    /// RIP at the time of the last error.
    last_error_rip: u64,

    // Raw pointers to heap-allocated devices, registered via proxies.
    pic_ptr: *mut devices::pic::PicPair,
    pit_ptr: *mut devices::pit::Pit,
    cmos_ptr: *mut devices::cmos::Cmos,
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
    ahci_ptr: *mut devices::ahci::Ahci,
    /// True once an external runner starts driving PIT ticks explicitly.
    pit_is_externally_clocked: bool,
    /// TSC frequency in Hz (calibrated at startup, host_test only).
    #[cfg(feature = "host_test")]
    tsc_freq: u64,
    /// Last TSC timestamp for wall-clock timer advancement (host_test only).
    #[cfg(feature = "host_test")]
    timer_tsc_last: u64,
    /// Accumulated TSC ticks already converted to PIT ticks (host_test only).
    #[cfg(feature = "host_test")]
    pit_tsc_accum: u64,
    /// Accumulated TSC ticks already converted to CMOS RTC ticks (host_test only).
    #[cfg(feature = "host_test")]
    cmos_tsc_accum: u64,
}

impl Drop for VmInstance {
    fn drop(&mut self) {
        unsafe {
            if !self.pic_ptr.is_null() { let _ = Box::from_raw(self.pic_ptr); }
            if !self.pit_ptr.is_null() { let _ = Box::from_raw(self.pit_ptr); }
            if !self.cmos_ptr.is_null() { let _ = Box::from_raw(self.cmos_ptr); }
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
            if !self.ahci_ptr.is_null() { let _ = Box::from_raw(self.ahci_ptr); }
        }
    }
}

/// Convert an opaque `u64` handle to a mutable `VmInstance` reference.
#[inline]
unsafe fn vm_from_handle(handle: u64) -> &'static mut VmInstance {
    &mut *(handle as *mut VmInstance)
}

#[inline]
fn drain_lapic_eois_raw(
    lapic_ptr: *mut devices::lapic::Lapic,
    ioapic_ptr: *mut devices::ioapic::IoApic,
) {
    if lapic_ptr.is_null() || ioapic_ptr.is_null() {
        return;
    }
    let lapic = unsafe { &mut *lapic_ptr };
    let ioapic = unsafe { &mut *ioapic_ptr };
    while let Some(vector) = lapic.take_eoi_vector() {
        ioapic.eoi_vector(vector);
    }
    drain_ioapic_service(lapic_ptr, ioapic_ptr);
}

/// Forward IOAPIC service output to the LAPIC.
fn drain_ioapic_service(
    lapic_ptr: *mut devices::lapic::Lapic,
    ioapic_ptr: *mut devices::ioapic::IoApic,
) {
    if lapic_ptr.is_null() || ioapic_ptr.is_null() {
        return;
    }
    let ioapic = unsafe { &mut *ioapic_ptr };
    let lapic = unsafe { &mut *lapic_ptr };
    let out = ioapic.take_service_output();
    for &(vector, level_triggered) in out {
        lapic.raise_vector(vector, level_triggered);
    }
    ioapic.clear_service_output();
}

#[inline]
fn drain_lapic_eois(vm: &mut VmInstance) {
    drain_lapic_eois_raw(vm.lapic_ptr, vm.ioapic_ptr);
}


// ════════════════════════════════════════════════════════════════════════
// VM Lifecycle
// ════════════════════════════════════════════════════════════════════════

/// Create a new VM instance with the specified guest RAM size in megabytes.
///
/// Returns an opaque handle (non-zero on success, 0 on failure).
#[no_mangle]
pub extern "C" fn corevm_create(ram_size_mb: u32) -> u64 {
    corevm_create_ex(ram_size_mb, 1)
}

/// Create a new VM instance with RAM size and logical CPU count.
#[no_mangle]
pub extern "C" fn corevm_create_ex(ram_size_mb: u32, vcpu_count: u32) -> u64 {
    let count = (vcpu_count.clamp(1, 255)) as u8;
    vm_log!(
        "creating VM with {} MiB RAM (vcpus={})",
        ram_size_mb,
        count
    );
    let ram_bytes = (ram_size_mb as usize) * 1024 * 1024;
    let engine = match VmEngine::new_with_vcpus(ram_bytes, count) {
        Ok(e) => e,
        Err(e) => {
            vm_log!("failed to create VM engine: {}", e);
            #[cfg(feature = "host_test")]
            eprintln!("[corevm] failed to create VM engine: {}", e);
            return 0;
        }
    };
    let instance = Box::new(VmInstance {
        engine,
        last_error: None,
        last_error_rip: 0,
        pic_ptr: ptr::null_mut(),
        pit_ptr: ptr::null_mut(),
        cmos_ptr: ptr::null_mut(),
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
        ahci_ptr: ptr::null_mut(),
        pit_is_externally_clocked: false,
        #[cfg(feature = "host_test")]
        tsc_freq: calibrate_tsc_freq(),
        #[cfg(feature = "host_test")]
        timer_tsc_last: 0,
        #[cfg(feature = "host_test")]
        pit_tsc_accum: 0,
        #[cfg(feature = "host_test")]
        cmos_tsc_accum: 0,
    });
    let h = Box::into_raw(instance) as u64;
    vm_log!("VM created (handle=0x{:X})", h);
    h
}

/// Read the CPU timestamp counter.
#[cfg(feature = "host_test")]
#[inline(always)]
pub(crate) fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    unsafe { core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, nomem)); }
    (hi as u64) << 32 | lo as u64
}

/// Calibrate TSC frequency by measuring rdtsc over a short sleep.
#[cfg(feature = "host_test")]
fn calibrate_tsc_freq() -> u64 {
    let t0 = rdtsc();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let t1 = rdtsc();
    let elapsed = t1.wrapping_sub(t0);
    let freq = elapsed * 100;
    vm_log!("TSC freq calibrated: {} Hz ({:.2} GHz)", freq, freq as f64 / 1e9);
    freq
}

/// Destroy a VM instance and free all associated resources.
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
#[no_mangle]
pub extern "C" fn corevm_reset(handle: u64) {
    vm_log!("resetting VM");
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.reset();
    vm.last_error = None;
    vm.last_error_rip = 0;
    vm.pit_is_externally_clocked = false;
    #[cfg(feature = "host_test")]
    {
        vm.timer_tsc_last = 0;
        vm.pit_tsc_accum = 0;
        vm.cmos_tsc_accum = 0;
    }
}


// ════════════════════════════════════════════════════════════════════════
// CPU State — Register Access via Hypervisor Backend
// ════════════════════════════════════════════════════════════════════════

/// Get the current instruction pointer (RIP).
#[no_mangle]
pub extern "C" fn corevm_get_rip(handle: u64) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.last_rip
}

/// Set the instruction pointer (RIP).
#[no_mangle]
pub extern "C" fn corevm_set_rip(handle: u64, rip: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if let Ok(mut regs) = vm.engine.cpu.get_regs() {
        regs.rip = rip;
        let _ = vm.engine.cpu.set_regs(&regs);
    }
}

/// Read a general-purpose register by index (0=RAX, 1=RCX, 2=RDX, 3=RBX,
/// 4=RSP, 5=RBP, 6=RSI, 7=RDI, 8-15=R8-R15).
#[no_mangle]
pub extern "C" fn corevm_get_gpr(handle: u64, index: u8) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    let regs = match vm.engine.cpu.get_regs() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    match index {
        0 => regs.rax, 1 => regs.rcx, 2 => regs.rdx, 3 => regs.rbx,
        4 => regs.rsp, 5 => regs.rbp, 6 => regs.rsi, 7 => regs.rdi,
        8 => regs.r8, 9 => regs.r9, 10 => regs.r10, 11 => regs.r11,
        12 => regs.r12, 13 => regs.r13, 14 => regs.r14, 15 => regs.r15,
        _ => 0,
    }
}

/// Write a general-purpose register by index.
#[no_mangle]
pub extern "C" fn corevm_set_gpr(handle: u64, index: u8, val: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if let Ok(mut regs) = vm.engine.cpu.get_regs() {
        match index {
            0 => regs.rax = val, 1 => regs.rcx = val,
            2 => regs.rdx = val, 3 => regs.rbx = val,
            4 => regs.rsp = val, 5 => regs.rbp = val,
            6 => regs.rsi = val, 7 => regs.rdi = val,
            8 => regs.r8 = val, 9 => regs.r9 = val,
            10 => regs.r10 = val, 11 => regs.r11 = val,
            12 => regs.r12 = val, 13 => regs.r13 = val,
            14 => regs.r14 = val, 15 => regs.r15 = val,
            _ => return,
        }
        let _ = vm.engine.cpu.set_regs(&regs);
    }
}

/// Get the RFLAGS register.
#[no_mangle]
pub extern "C" fn corevm_get_rflags(handle: u64) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.get_regs().map(|r| r.rflags).unwrap_or(0)
}

/// Set the RFLAGS register.
#[no_mangle]
pub extern "C" fn corevm_set_rflags(handle: u64, val: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if let Ok(mut regs) = vm.engine.cpu.get_regs() {
        regs.rflags = val;
        let _ = vm.engine.cpu.set_regs(&regs);
    }
}

/// Read a control register (CR0, CR2, CR3, CR4, CR8).
#[no_mangle]
pub extern "C" fn corevm_get_cr(handle: u64, n: u8) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    let regs = match vm.engine.cpu.get_regs() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    match n {
        0 => regs.cr0, 2 => regs.cr2, 3 => regs.cr3,
        4 => regs.cr4, 8 => regs.cr8,
        _ => 0,
    }
}

/// Write a control register.
#[no_mangle]
pub extern "C" fn corevm_set_cr(handle: u64, n: u8, val: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if let Ok(mut regs) = vm.engine.cpu.get_regs() {
        match n {
            0 => regs.cr0 = val,
            2 => regs.cr2 = val,
            3 => regs.cr3 = val,
            4 => regs.cr4 = val,
            8 => regs.cr8 = val,
            _ => return,
        }
        let _ = vm.engine.cpu.set_regs(&regs);
    }
}

/// Get the segment selector for a segment register.
/// `seg`: 0=ES, 1=CS, 2=SS, 3=DS, 4=FS, 5=GS.
#[no_mangle]
pub extern "C" fn corevm_get_segment_selector(handle: u64, seg: u8) -> u16 {
    let vm = unsafe { vm_from_handle(handle) };
    let regs = match vm.engine.cpu.get_regs() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    match seg {
        0 => regs.es.selector, 1 => regs.cs.selector,
        2 => regs.ss.selector, 3 => regs.ds.selector,
        4 => regs.fs.selector, 5 => regs.gs.selector,
        _ => 0,
    }
}

/// Get the cached base address of a segment register.
#[no_mangle]
pub extern "C" fn corevm_get_segment_base(handle: u64, seg: u8) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    let regs = match vm.engine.cpu.get_regs() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    match seg {
        0 => regs.es.base, 1 => regs.cs.base,
        2 => regs.ss.base, 3 => regs.ds.base,
        4 => regs.fs.base, 5 => regs.gs.base,
        _ => 0,
    }
}

/// Get the current CPU execution mode.
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
    let regs = match vm.engine.cpu.get_regs() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    // CPL is the RPL of CS
    (regs.cs.selector & 3) as u8
}

/// Get configured logical CPU count for this VM.
#[no_mangle]
pub extern "C" fn corevm_get_vcpu_count(handle: u64) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.vcpu_count as u32
}

/// Get one model-specific register value.
#[no_mangle]
pub extern "C" fn corevm_get_msr(handle: u64, idx: u32) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    let regs = match vm.engine.cpu.get_regs() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    match idx {
        0xC000_0080 => regs.efer,    // IA32_EFER
        0x1B => regs.apic_base,       // IA32_APIC_BASE
        0x277 => regs.pat,            // IA32_PAT
        0x174 => regs.sysenter_cs,    // IA32_SYSENTER_CS
        0x175 => regs.sysenter_esp,   // IA32_SYSENTER_ESP
        0x176 => regs.sysenter_eip,   // IA32_SYSENTER_EIP
        0xC000_0081 => regs.star,     // IA32_STAR
        0xC000_0082 => regs.lstar,    // IA32_LSTAR
        0xC000_0083 => regs.cstar,    // IA32_CSTAR
        0xC000_0084 => regs.sfmask,   // IA32_FMASK
        0xC000_0101 => regs.kernel_gs_base, // IA32_KERNEL_GS_BASE
        0xC000_0102 => regs.tsc_aux,  // IA32_TSC_AUX
        _ => 0,
    }
}


// ════════════════════════════════════════════════════════════════════════
// Execution — VM-Exit Driven
// ════════════════════════════════════════════════════════════════════════

/// Handle a CPUID exit from the guest.
fn handle_cpuid(vm: &mut VmInstance, leaf: u32, subleaf: u32) {
    let (mut eax, mut ebx, mut ecx, mut edx) = (0u32, 0u32, 0u32, 0u32);

    match leaf {
        0x0000_0000 => {
            // Vendor: "GenuineIntel"
            eax = 0x16; // max basic leaf
            ebx = 0x756E_6547; // "Genu"
            edx = 0x4965_6E69; // "ineI"
            ecx = 0x6C65_746E; // "ntel"
        }
        0x0000_0001 => {
            // Family 6 Model 60 (Haswell)
            eax = 0x0003_06C3;
            // APIC ID in bits 31:24, logical CPUs in bits 23:16
            ebx = ((vm.engine.cpu.apic_id as u32) << 24)
                | ((vm.engine.cpu.logical_cpu_count as u32) << 16)
                | 0x0800; // CLFLUSH size = 8
            // Feature flags (ECX)
            ecx = (1 << 0)  // SSE3
                | (1 << 9)  // SSSE3
                | (1 << 13) // CMPXCHG16B
                | (1 << 19) // SSE4.1
                | (1 << 20) // SSE4.2
                | (1 << 21) // x2APIC
                | (1 << 25) // AESNI
                | (1 << 31); // hypervisor
            // Feature flags (EDX)
            edx = (1 << 0)  // FPU
                | (1 << 3)  // PSE
                | (1 << 4)  // TSC
                | (1 << 5)  // MSR
                | (1 << 6)  // PAE
                | (1 << 8)  // CMPXCHG8B
                | (1 << 9)  // APIC
                | (1 << 11) // SEP (SYSENTER/SYSEXIT)
                | (1 << 13) // PGE
                | (1 << 15) // CMOV
                | (1 << 16) // PAT
                | (1 << 17) // PSE-36
                | (1 << 19) // CLFLUSH
                | (1 << 23) // MMX
                | (1 << 24) // FXSR
                | (1 << 25) // SSE
                | (1 << 26); // SSE2
        }
        0x4000_0000 => {
            // Hypervisor vendor: "COREVMCOREV"
            eax = 0x4000_0001;
            ebx = 0x4552_4F43; // "CORE"
            ecx = 0x4F43_4D56; // "VMCO"
            edx = 0x0056_4552; // "REV\0"
        }
        0x8000_0000 => {
            eax = 0x8000_0008; // max extended leaf
        }
        0x8000_0001 => {
            ecx = (1 << 0); // LAHF/SAHF in 64-bit
            edx = (1 << 11) // SYSCALL/SYSRET
                | (1 << 20) // NX
                | (1 << 27) // RDTSCP
                | (1 << 29); // Long Mode
        }
        0x8000_0008 => {
            // Address sizes: 48-bit virtual, 39-bit physical
            eax = 0x0000_3027;
        }
        _ => {}
    }

    let _ = vm.engine.cpu.complete_cpuid(eax, ebx, ecx, edx);
}

/// Handle an MSR read exit from the guest.
fn handle_msr_read(vm: &mut VmInstance, index: u32) {
    let value = match index {
        0xC000_0080 => { // IA32_EFER
            vm.engine.cpu.get_regs().map(|r| r.efer).unwrap_or(0)
        }
        0x1B => { // IA32_APIC_BASE
            vm.engine.cpu.get_regs().map(|r| r.apic_base).unwrap_or(0xFEE0_0900)
        }
        0x277 => 0x0007_0406_0007_0406, // IA32_PAT default
        0xCE => 20u64 << 8, // MSR_PLATFORM_INFO: ratio=20 → 2 GHz
        0x174 => vm.engine.cpu.get_regs().map(|r| r.sysenter_cs).unwrap_or(0),
        0x175 => vm.engine.cpu.get_regs().map(|r| r.sysenter_esp).unwrap_or(0),
        0x176 => vm.engine.cpu.get_regs().map(|r| r.sysenter_eip).unwrap_or(0),
        0xC000_0081 => vm.engine.cpu.get_regs().map(|r| r.star).unwrap_or(0),
        0xC000_0082 => vm.engine.cpu.get_regs().map(|r| r.lstar).unwrap_or(0),
        0xC000_0083 => vm.engine.cpu.get_regs().map(|r| r.cstar).unwrap_or(0),
        0xC000_0084 => vm.engine.cpu.get_regs().map(|r| r.sfmask).unwrap_or(0),
        0xC000_0100 => vm.engine.cpu.get_regs().map(|r| r.fs.base).unwrap_or(0), // FS_BASE
        0xC000_0101 => vm.engine.cpu.get_regs().map(|r| r.kernel_gs_base).unwrap_or(0),
        0xC000_0102 => vm.engine.cpu.get_regs().map(|r| r.tsc_aux).unwrap_or(0),
        0x10 => { // IA32_TIME_STAMP_COUNTER
            #[cfg(feature = "host_test")]
            { rdtsc() }
            #[cfg(not(feature = "host_test"))]
            { 0 }
        }
        _ => 0, // Unknown MSRs return 0
    };
    let _ = vm.engine.cpu.complete_msr_read(value);
}

/// Handle an MSR write exit from the guest.
fn handle_msr_write(vm: &mut VmInstance, index: u32, value: u64) {
    match index {
        0xC000_0080 => { // IA32_EFER
            if let Ok(mut regs) = vm.engine.cpu.get_regs() {
                regs.efer = value;
                let _ = vm.engine.cpu.set_regs(&regs);
            }
        }
        0x1B => { // IA32_APIC_BASE
            if let Ok(mut regs) = vm.engine.cpu.get_regs() {
                regs.apic_base = value;
                let _ = vm.engine.cpu.set_regs(&regs);
            }
        }
        0x174 | 0x175 | 0x176 => { // SYSENTER MSRs
            if let Ok(mut regs) = vm.engine.cpu.get_regs() {
                match index {
                    0x174 => regs.sysenter_cs = value,
                    0x175 => regs.sysenter_esp = value,
                    0x176 => regs.sysenter_eip = value,
                    _ => {}
                }
                let _ = vm.engine.cpu.set_regs(&regs);
            }
        }
        0xC000_0081 | 0xC000_0082 | 0xC000_0083 | 0xC000_0084 => { // SYSCALL MSRs
            if let Ok(mut regs) = vm.engine.cpu.get_regs() {
                match index {
                    0xC000_0081 => regs.star = value,
                    0xC000_0082 => regs.lstar = value,
                    0xC000_0083 => regs.cstar = value,
                    0xC000_0084 => regs.sfmask = value,
                    _ => {}
                }
                let _ = vm.engine.cpu.set_regs(&regs);
            }
        }
        0xC000_0100 => { // FS_BASE
            if let Ok(mut regs) = vm.engine.cpu.get_regs() {
                regs.fs.base = value;
                let _ = vm.engine.cpu.set_regs(&regs);
            }
        }
        0xC000_0101 => { // KERNEL_GS_BASE
            if let Ok(mut regs) = vm.engine.cpu.get_regs() {
                regs.kernel_gs_base = value;
                let _ = vm.engine.cpu.set_regs(&regs);
            }
        }
        0xC000_0102 => { // TSC_AUX
            if let Ok(mut regs) = vm.engine.cpu.get_regs() {
                regs.tsc_aux = value;
                let _ = vm.engine.cpu.set_regs(&regs);
            }
        }
        _ => {} // Ignore unknown MSR writes
    }
    let _ = vm.engine.cpu.advance_rip(2); // WRMSR is 2 bytes (0F 30)
}

/// Run the VM — VM-exit driven execution loop.
///
/// Returns an exit reason code:
/// - 0 = halted (HLT executed)
/// - 1 = unhandled exception / shutdown
/// - 2 = max iterations reached
/// - 3 = breakpoint (INT 3)
/// - 4 = stop requested via [`corevm_request_stop`]
/// - 5 = PS/2 system reset
#[no_mangle]
pub extern "C" fn corevm_run(handle: u64, max_exits: u64) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };

    // Initialize wall-clock timer reference on first call.
    #[cfg(feature = "host_test")]
    if vm.timer_tsc_last == 0 {
        vm.timer_tsc_last = rdtsc();
    }

    let limit = if max_exits == 0 { u64::MAX } else { max_exits };
    let mut exits_done: u64 = 0;

    loop {
        // Drain EOIs and deliver pending interrupts before each VM-entry.
        drain_lapic_eois(vm);
        try_inject_interrupt(vm);

        // Deliver IDE IRQ 14 before running.
        if \!vm.ide_ptr.is_null() {
            let ide = unsafe { &mut *vm.ide_ptr };
            if ide.irq_raised() {
                inject_irq_line(vm, 14);
            }
        }

        // Deliver AHCI IRQ 11.
        if \!vm.ahci_ptr.is_null() {
            let ahci = unsafe { &mut *vm.ahci_ptr };
            if ahci.irq_raised() {
                inject_irq_line(vm, 11);
            }
        }

        // Execute one VM-entry/exit cycle.
        let exit = match vm.engine.cpu.run_once() {
            Ok(e) => e,
            Err(e) => {
                vm_log\!("VM run error: {}", e);
                #[cfg(feature = "host_test")]
                eprintln\!("[corevm] VM run error: {}", e);
                vm.last_error = Some(VmError::GeneralProtection(0));
                vm.last_error_rip = vm.engine.cpu.last_rip;
                return 1;
            }
        };

        exits_done += 1;

        // Handle the VM-exit.
        match exit {
            VmExit::IoIn { port, size } => {
                let data = vm.engine.io.port_in(port, size).unwrap_or(0xFFFFFFFF);
                let _ = vm.engine.cpu.complete_io_in(data, size);
            }
            VmExit::IoOut { port, size, data } => {
                let _ = vm.engine.io.port_out(port, size, data);
            }
            VmExit::MmioRead { address, size } => {
                use memory::MemoryBus;
                let data = match size {
                    1 => vm.engine.memory.read_u8(address).unwrap_or(0xFF) as u64,
                    2 => vm.engine.memory.read_u16(address).unwrap_or(0xFFFF) as u64,
                    4 => vm.engine.memory.read_u32(address).unwrap_or(0xFFFF_FFFF) as u64,
                    8 => vm.engine.memory.read_u64(address).unwrap_or(0xFFFF_FFFF_FFFF_FFFF),
                    _ => 0xFF,
                };
                let _ = vm.engine.cpu.complete_mmio_read(data, size);
            }
            VmExit::MmioWrite { address, size, data } => {
                use memory::MemoryBus;
                match size {
                    1 => { let _ = vm.engine.memory.write_u8(address, data as u8); }
                    2 => { let _ = vm.engine.memory.write_u16(address, data as u16); }
                    4 => { let _ = vm.engine.memory.write_u32(address, data as u32); }
                    8 => { let _ = vm.engine.memory.write_u64(address, data); }
                    _ => {}
                }
            }
            VmExit::Cpuid { eax, ecx } => {
                handle_cpuid(vm, eax, ecx);
            }
            VmExit::MsrRead { index } => {
                handle_msr_read(vm, index);
            }
            VmExit::MsrWrite { index, value } => {
                handle_msr_write(vm, index, value);
            }
            VmExit::Hlt => {
                // Advance timers before returning HLT so they stay current.
                advance_timers(vm);
                return 0;
            }
            VmExit::Shutdown => {
                vm_log\!("VM shutdown (triple fault)");
                vm.last_error = Some(VmError::Shutdown);
                vm.last_error_rip = vm.engine.cpu.last_rip;
                return 1;
            }
            VmExit::StopRequested => {
                if \!vm.ps2_ptr.is_null() && unsafe { (*vm.ps2_ptr).reset_requested } {
                    unsafe { (*vm.ps2_ptr).reset_requested = false };
                    vm_log\!("PS/2 system reset requested");
                    return 5;
                }
                vm_log\!("VM stop requested");
                return 4;
            }
            VmExit::InterruptWindow => {
                // An interrupt window opened — try to inject pending IRQs.
                try_inject_interrupt(vm);
            }
            VmExit::EptViolation { guest_phys, is_write } => {
                // EPT violation for an unmapped address — treat as MMIO.
                use memory::MemoryBus;
                if is_write {
                    // We cannot easily decode the write value from the instruction
                    // without guest memory access. For now, ignore.
                } else {
                    let _ = vm.engine.cpu.complete_mmio_read(0xFFFF_FFFF, 4);
                }
            }
            VmExit::Debug => {
                vm_log\!("VM breakpoint at RIP=0x{:X}", vm.engine.cpu.last_rip);
                return 3;
            }
            VmExit::Unknown(code) => {
                vm_log\!("unknown VM exit: {}", code);
                #[cfg(feature = "host_test")]
                eprintln\!("[corevm] unknown VM exit: {}", code);
            }
        }

        // Advance timers after each VM-exit.
        advance_timers(vm);

        // Check for PS/2 system reset.
        if \!vm.ps2_ptr.is_null() {
            let ps2 = unsafe { &mut *vm.ps2_ptr };
            if ps2.reset_requested {
                return 5;
            }
        }

        // Check exit limit.
        if exits_done >= limit {
            return 2;
        }
    }
}

/// Try to inject a pending interrupt into the guest.
fn try_inject_interrupt(vm: &mut VmInstance) {
    // First, try LAPIC.
    if \!vm.lapic_ptr.is_null() {
        let lapic = unsafe { &mut *vm.lapic_ptr };
        if vm.engine.cpu.interrupts_enabled() {
            if let Some(vector) = lapic.next_deliverable_vector() {
                lapic.accept_vector(vector);
                let _ = vm.engine.cpu.inject_interrupt(vector);
                return;
            }
        }
    }

    // Then, try PIC.
    if \!vm.pic_ptr.is_null() {
        let pic = unsafe { &mut *vm.pic_ptr };
        let pic_accepted = if \!vm.lapic_ptr.is_null() {
            unsafe { (*vm.lapic_ptr).accepts_pic_intr() }
        } else {
            true
        };
        if pic_accepted && vm.engine.cpu.interrupts_enabled() {
            if let Some(vector) = pic.get_interrupt_vector() {
                let ack_irq = pic.irq_for_vector(vector).unwrap_or(0);
                pic.acknowledge(ack_irq);
                let _ = vm.engine.cpu.inject_interrupt(vector);
                return;
            }
        }
    }

    // If there are pending interrupts but IF=0, request an interrupt window.
    let has_pending = (\!vm.lapic_ptr.is_null() && unsafe { (*vm.lapic_ptr).has_pending_vector() })
        || (\!vm.pic_ptr.is_null() && pic_has_latched_irq(vm));
    if has_pending && \!vm.engine.cpu.interrupts_enabled() {
        let _ = vm.engine.cpu.request_interrupt_window();
    }
}

/// Advance all wall-clock-based timers.
fn advance_timers(vm: &mut VmInstance) {
    // LAPIC timer — always sync from TSC.
    if \!vm.lapic_ptr.is_null() {
        let lapic = unsafe { &mut *vm.lapic_ptr };
        #[cfg(feature = "host_test")]
        {
            lapic.sync_timer_from_tsc();
            if lapic.take_timer_irq() {
                let vec = lapic.timer_vector();
                lapic.raise_vector(vec, false);
            }
        }
        #[cfg(not(feature = "host_test"))]
        {
            // Approximate: advance by a fixed amount per VM-exit.
            if let Some(vector) = lapic.advance(64) {
                lapic.raise_vector(vector, false);
            }
        }
    }

    drain_lapic_eois(vm);

    // PIT timer.
    if \!vm.pit_ptr.is_null() && \!vm.pit_is_externally_clocked {
        #[cfg(feature = "host_test")]
        {
            let now = rdtsc();
            let elapsed_tsc = now.wrapping_sub(vm.timer_tsc_last);
            let tsc_freq = vm.tsc_freq;
            if tsc_freq > 0 && elapsed_tsc > 0 {
                let total_tsc = vm.pit_tsc_accum + elapsed_tsc;
                let pit_ticks = (total_tsc as u128 * 1_193_182 / tsc_freq as u128) as u32;
                if pit_ticks > 0 {
                    vm.pit_tsc_accum = total_tsc - (pit_ticks as u128 * tsc_freq as u128 / 1_193_182) as u64;
                    let fires = unsafe { (*vm.pit_ptr).advance(pit_ticks) };
                    if fires > 0 {
                        inject_irq_line(vm, 0);
                    }
                } else {
                    vm.pit_tsc_accum = total_tsc;
                }
            }
        }
        #[cfg(not(feature = "host_test"))]
        {
            // Approximate: 1 PIT tick per VM-exit.
            let fires = unsafe { (*vm.pit_ptr).advance(1) };
            if fires > 0 {
                inject_irq_line(vm, 0);
            }
        }
    }

    // CMOS RTC.
    if \!vm.cmos_ptr.is_null() {
        #[cfg(feature = "host_test")]
        {
            let now = rdtsc();
            let elapsed_tsc = now.wrapping_sub(vm.timer_tsc_last);
            let tsc_freq = vm.tsc_freq;
            if tsc_freq > 0 && elapsed_tsc > 0 {
                let total_tsc = vm.cmos_tsc_accum + elapsed_tsc;
                let rtc_ticks = (total_tsc as u128 * 32_768u128 / tsc_freq as u128) as u64;
                if rtc_ticks > 0 {
                    vm.cmos_tsc_accum = total_tsc - (rtc_ticks as u128 * tsc_freq as u128 / 32_768u128) as u64;
                    let fired = unsafe { (*vm.cmos_ptr).advance(rtc_ticks) };
                    if fired {
                        inject_irq_line(vm, 8);
                    }
                } else {
                    vm.cmos_tsc_accum = total_tsc;
                }
            }
        }
        #[cfg(not(feature = "host_test"))]
        {
            let fired = unsafe { (*vm.cmos_ptr).advance(1) };
            if fired {
                inject_irq_line(vm, 8);
            }
        }
    }

    // Update wall-clock reference point.
    #[cfg(feature = "host_test")]
    { vm.timer_tsc_last = rdtsc(); }

    // ACPI PM timer.
    if \!vm.acpi_pm_ptr.is_null() {
        unsafe { (*vm.acpi_pm_ptr).advance(1) };
    }
}

/// Request the VM to stop at the next opportunity.
#[no_mangle]
pub extern "C" fn corevm_request_stop(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.request_stop();
}

/// Get the total number of VM-exit cycles executed since the last reset.
#[no_mangle]
pub extern "C" fn corevm_get_instruction_count(handle: u64) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.cpu.run_cycles
}

/// Get the RIP at the time of the last error.
#[no_mangle]
pub extern "C" fn corevm_get_last_error_rip(handle: u64) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.last_error_rip
}

/// Dump the exception ring buffer — stub for compatibility.
#[no_mangle]
pub extern "C" fn corevm_dump_exception_ring(_handle: u64) {
    // No exception ring in hardware virtualization mode.
}

/// Write a human-readable description of the last error into the provided buffer.
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
    use core::fmt::Write;
    let mut tmp = StackWriter::new();
    let _ = write\!(tmp, "{}", err);
    let msg = tmp.as_bytes();
    let copy_len = msg.len().min((buf_len - 1) as usize);
    unsafe {
        ptr::copy_nonoverlapping(msg.as_ptr(), buf, copy_len);
        *buf.add(copy_len) = 0;
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
#[no_mangle]
pub extern "C" fn corevm_read_linear_u8(handle: u64, linear: u64) -> u8 {
    let vm = unsafe { vm_from_handle(handle) };
    use memory::{AccessType, MemoryBus};
    let regs = match vm.engine.cpu.get_regs() {
        Ok(r) => r,
        Err(_) => return 0,
    };
    let cpl = (regs.cs.selector & 3) as u8;
    let cr3 = regs.cr3;
    match vm.engine.mmu.translate_linear(linear, cr3, AccessType::Read, cpl, &vm.engine.memory) {
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

/// Generate minimal ACPI tables (RSDP, RSDT, FADT, DSDT, MADT) and the
/// `etc/table-loader` linker script so SeaBIOS can expose them to the guest.
fn generate_acpi_tables(fw_cfg: &mut devices::fw_cfg::FwCfg) {
    use alloc::vec;

    fn acpi_checksum(data: &[u8]) -> u8 {
        let sum: u8 = data.iter().fold(0u8, |a, &b| a.wrapping_add(b));
        (!sum).wrapping_add(1)
    }

    let dsdt = {
        fn pkg_len(len: usize) -> Vec<u8> {
            if len < 63 {
                alloc::vec![len as u8]
            } else if len < 4096 {
                alloc::vec![0x40 | (len & 0xF) as u8, ((len >> 4) & 0xFF) as u8]
            } else {
                alloc::vec![0x80 | (len & 0xF) as u8, ((len >> 4) & 0xFF) as u8, ((len >> 12) & 0xFF) as u8]
            }
        }
        fn wrap_pkg(content: &[u8]) -> Vec<u8> {
            for extra in 1..=3usize {
                let full = content.len() + extra;
                let enc = pkg_len(full);
                if enc.len() == extra {
                    let mut out = enc;
                    out.extend_from_slice(content);
                    return out;
                }
            }
            unreachable!()
        }

        let mut aml = Vec::new();
        // Name(PICF, Zero)
        aml.extend_from_slice(&[0x08]);
        aml.extend_from_slice(b"PICF");
        aml.push(0x00);
        // Method(\_PIC, 1) { Store(Arg0, PICF) }
        {
            let mut body = Vec::new();
            body.extend_from_slice(&[0x5C]);
            body.extend_from_slice(b"_PIC");
            body.push(0x01);
            body.extend_from_slice(&[0x70]);
            body.push(0x68);
            body.extend_from_slice(b"PICF");
            aml.push(0x14);
            aml.extend_from_slice(&wrap_pkg(&body));
        }
        // Scope(\_SB_) { Device(PCI0) { _HID, _PRT } }
        {
            let mut pci0_body = Vec::new();
            pci0_body.push(0x08);
            pci0_body.extend_from_slice(b"_HID");
            pci0_body.push(0x0C);
            pci0_body.extend_from_slice(&0x030AD041u32.to_le_bytes());
            let prt_entries: &[(u32, u8, u8)] = &[
                (0x0001_FFFF, 0, 14),
                (0x0002_FFFF, 0, 10),
                (0x0004_FFFF, 0, 11),
            ];
            let mut prt_inner_pkgs = Vec::new();
            for &(addr, pin, gsi) in prt_entries {
                let mut elem = Vec::new();
                elem.push(0x0C);
                elem.extend_from_slice(&addr.to_le_bytes());
                if pin == 0 { elem.push(0x00); } else { elem.push(0x0A); elem.push(pin); }
                elem.push(0x00);
                elem.push(0x0A);
                elem.push(gsi);
                let mut inner_pkg = alloc::vec![0x12];
                let mut inner_content = alloc::vec![4u8];
                inner_content.extend_from_slice(&elem);
                inner_pkg.extend_from_slice(&wrap_pkg(&inner_content));
                prt_inner_pkgs.extend_from_slice(&inner_pkg);
            }
            let mut prt_pkg_content = alloc::vec![prt_entries.len() as u8];
            prt_pkg_content.extend_from_slice(&prt_inner_pkgs);
            let mut prt_pkg = alloc::vec![0x12];
            prt_pkg.extend_from_slice(&wrap_pkg(&prt_pkg_content));
            pci0_body.push(0x08);
            pci0_body.extend_from_slice(b"_PRT");
            pci0_body.extend_from_slice(&prt_pkg);
            let mut dev_body = Vec::new();
            dev_body.extend_from_slice(b"PCI0");
            dev_body.extend_from_slice(&pci0_body);
            let mut device = alloc::vec![0x5Bu8, 0x82];
            device.extend_from_slice(&wrap_pkg(&dev_body));
            let mut scope_body = Vec::new();
            scope_body.extend_from_slice(&[0x5C, b'_', b'S', b'B', b'_']);
            scope_body.extend_from_slice(&device);
            aml.push(0x10);
            aml.extend_from_slice(&wrap_pkg(&scope_body));
        }
        let dsdt_len = 36 + aml.len();
        let mut dsdt = alloc::vec![0u8; dsdt_len];
        dsdt[0..4].copy_from_slice(b"DSDT");
        dsdt[4..8].copy_from_slice(&(dsdt_len as u32).to_le_bytes());
        dsdt[8] = 1;
        dsdt[10..16].copy_from_slice(b"COREVM");
        dsdt[16..24].copy_from_slice(b"COREVMDT");
        dsdt[24..28].copy_from_slice(&1u32.to_le_bytes());
        dsdt[28..32].copy_from_slice(b"CRVM");
        dsdt[32..36].copy_from_slice(&1u32.to_le_bytes());
        dsdt[36..].copy_from_slice(&aml);
        dsdt[9] = acpi_checksum(&dsdt);
        dsdt
    };

    let fadt_len = 116u32;
    let mut fadt = vec![0u8; fadt_len as usize];
    fadt[0..4].copy_from_slice(b"FACP");
    fadt[4..8].copy_from_slice(&fadt_len.to_le_bytes());
    fadt[8] = 1;
    fadt[10..16].copy_from_slice(b"COREVM");
    fadt[16..24].copy_from_slice(b"COREVMFC");
    fadt[24..28].copy_from_slice(&1u32.to_le_bytes());
    fadt[28..32].copy_from_slice(b"CRVM");
    fadt[32..36].copy_from_slice(&1u32.to_le_bytes());
    fadt[40..44].copy_from_slice(&0u32.to_le_bytes());
    fadt[46..48].copy_from_slice(&9u16.to_le_bytes());
    fadt[48..52].copy_from_slice(&0u32.to_le_bytes());
    fadt[56..60].copy_from_slice(&0x600u32.to_le_bytes());
    fadt[64..68].copy_from_slice(&0x604u32.to_le_bytes());
    fadt[76..80].copy_from_slice(&0x608u32.to_le_bytes());
    fadt[88] = 4;
    fadt[89] = 2;
    fadt[91] = 4;
    fadt[108..112].copy_from_slice(&((1u32 << 8) | 1).to_le_bytes());
    fadt[9] = acpi_checksum(&fadt);

    let num_isos = 5;
    let madt_entry_size = 8 + 12 + 10 * num_isos + 6;
    let madt_len = 44 + madt_entry_size;
    let mut madt = vec![0u8; madt_len];
    madt[0..4].copy_from_slice(b"APIC");
    madt[4..8].copy_from_slice(&(madt_len as u32).to_le_bytes());
    madt[8] = 1;
    madt[10..16].copy_from_slice(b"COREVM");
    madt[16..24].copy_from_slice(b"COREVMMA");
    madt[24..28].copy_from_slice(&1u32.to_le_bytes());
    madt[28..32].copy_from_slice(b"CRVM");
    madt[32..36].copy_from_slice(&1u32.to_le_bytes());
    madt[36..40].copy_from_slice(&0xFEE00000u32.to_le_bytes());
    madt[40..44].copy_from_slice(&1u32.to_le_bytes());
    let mut off = 44;
    madt[off] = 0; madt[off+1] = 8;
    madt[off+2] = 0; madt[off+3] = 0;
    madt[off+4..off+8].copy_from_slice(&1u32.to_le_bytes());
    off += 8;
    madt[off] = 1; madt[off+1] = 12;
    madt[off+2] = 0; madt[off+3] = 0;
    madt[off+4..off+8].copy_from_slice(&0xFEC00000u32.to_le_bytes());
    madt[off+8..off+12].copy_from_slice(&0u32.to_le_bytes());
    off += 12;
    let mut add_iso = |off: &mut usize, src_irq: u8, gsi: u32, flags: u16| {
        madt[*off] = 2; madt[*off+1] = 10;
        madt[*off+2] = 0;
        madt[*off+3] = src_irq;
        madt[*off+4..*off+8].copy_from_slice(&gsi.to_le_bytes());
        madt[*off+8..*off+10].copy_from_slice(&flags.to_le_bytes());
        *off += 10;
    };
    add_iso(&mut off, 0, 2, 0x0000);
    add_iso(&mut off, 5, 5, 0x000D);
    add_iso(&mut off, 9, 9, 0x000D);
    add_iso(&mut off, 10, 10, 0x000D);
    add_iso(&mut off, 11, 11, 0x000D);
    madt[off] = 4; madt[off+1] = 6;
    madt[off+2] = 0xFF;
    madt[off+3..off+5].copy_from_slice(&0u16.to_le_bytes());
    madt[off+5] = 1;
    off += 6;
    let _ = off;
    madt[9] = acpi_checksum(&madt);

    let rsdt_len = 36 + 2 * 4;
    let mut rsdt = vec![0u8; rsdt_len];
    rsdt[0..4].copy_from_slice(b"RSDT");
    rsdt[4..8].copy_from_slice(&(rsdt_len as u32).to_le_bytes());
    rsdt[8] = 1;
    rsdt[10..16].copy_from_slice(b"COREVM");
    rsdt[16..24].copy_from_slice(b"COREVMRS");
    rsdt[24..28].copy_from_slice(&1u32.to_le_bytes());
    rsdt[28..32].copy_from_slice(b"CRVM");
    rsdt[32..36].copy_from_slice(&1u32.to_le_bytes());
    rsdt[9] = acpi_checksum(&rsdt);

    let mut rsdp = vec![0u8; 20];
    rsdp[0..8].copy_from_slice(b"RSD PTR ");
    rsdp[8] = 0;
    rsdp[9..15].copy_from_slice(b"COREVM");
    rsdp[15] = 0;
    rsdp[8] = acpi_checksum(&rsdp);

    let dsdt_off = 0u32;
    let fadt_off = dsdt.len() as u32;
    let madt_off = fadt_off + fadt.len() as u32;
    let rsdt_off = madt_off + madt.len() as u32;

    let mut tables = Vec::with_capacity(dsdt.len() + fadt.len() + madt.len() + rsdt.len());
    tables.extend_from_slice(&dsdt);
    tables.extend_from_slice(&fadt);
    tables.extend_from_slice(&madt);
    tables.extend_from_slice(&rsdt);

    let mut linker = Vec::new();

    let mut cmd = |tag: u32, build: &dyn Fn(&mut [u8; 128])| {
        let mut entry = [0u8; 128];
        entry[0..4].copy_from_slice(&tag.to_le_bytes());
        build(&mut entry);
        linker.extend_from_slice(&entry);
    };

    let tables_name = b"etc/acpi/tables\0";
    let rsdp_name = b"etc/acpi/rsdp\0";

    cmd(1, &|e| {
        e[4..4+tables_name.len()].copy_from_slice(tables_name);
        e[60..64].copy_from_slice(&4096u32.to_le_bytes());
        e[64] = 2;
    });
    cmd(1, &|e| {
        e[4..4+rsdp_name.len()].copy_from_slice(rsdp_name);
        e[60..64].copy_from_slice(&16u32.to_le_bytes());
        e[64] = 2;
    });
    cmd(2, &|e| {
        e[4..4+tables_name.len()].copy_from_slice(tables_name);
        e[60..60+tables_name.len()].copy_from_slice(tables_name);
        e[116..120].copy_from_slice(&(fadt_off + 40).to_le_bytes());
        e[120] = 4;
    });
    cmd(2, &|e| {
        e[4..4+tables_name.len()].copy_from_slice(tables_name);
        e[60..60+tables_name.len()].copy_from_slice(tables_name);
        e[116..120].copy_from_slice(&(rsdt_off + 36).to_le_bytes());
        e[120] = 4;
    });
    cmd(2, &|e| {
        e[4..4+tables_name.len()].copy_from_slice(tables_name);
        e[60..60+tables_name.len()].copy_from_slice(tables_name);
        e[116..120].copy_from_slice(&(rsdt_off + 40).to_le_bytes());
        e[120] = 4;
    });
    cmd(2, &|e| {
        e[4..4+rsdp_name.len()].copy_from_slice(rsdp_name);
        e[60..60+tables_name.len()].copy_from_slice(tables_name);
        e[116..120].copy_from_slice(&16u32.to_le_bytes());
        e[120] = 4;
    });

    tables[(fadt_off + 40) as usize..(fadt_off + 44) as usize]
        .copy_from_slice(&dsdt_off.to_le_bytes());
    tables[(rsdt_off + 36) as usize..(rsdt_off + 40) as usize]
        .copy_from_slice(&fadt_off.to_le_bytes());
    tables[(rsdt_off + 40) as usize..(rsdt_off + 44) as usize]
        .copy_from_slice(&madt_off.to_le_bytes());
    rsdp[16..20].copy_from_slice(&rsdt_off.to_le_bytes());

    cmd(3, &|e| {
        e[4..4+tables_name.len()].copy_from_slice(tables_name);
        e[60..64].copy_from_slice(&(dsdt_off + 9).to_le_bytes());
        e[64..68].copy_from_slice(&dsdt_off.to_le_bytes());
        e[68..72].copy_from_slice(&(dsdt.len() as u32).to_le_bytes());
    });
    cmd(3, &|e| {
        e[4..4+tables_name.len()].copy_from_slice(tables_name);
        e[60..64].copy_from_slice(&(fadt_off + 9).to_le_bytes());
        e[64..68].copy_from_slice(&fadt_off.to_le_bytes());
        e[68..72].copy_from_slice(&(fadt.len() as u32).to_le_bytes());
    });
    cmd(3, &|e| {
        e[4..4+tables_name.len()].copy_from_slice(tables_name);
        e[60..64].copy_from_slice(&(madt_off + 9).to_le_bytes());
        e[64..68].copy_from_slice(&madt_off.to_le_bytes());
        e[68..72].copy_from_slice(&(madt.len() as u32).to_le_bytes());
    });
    cmd(3, &|e| {
        e[4..4+tables_name.len()].copy_from_slice(tables_name);
        e[60..64].copy_from_slice(&(rsdt_off + 9).to_le_bytes());
        e[64..68].copy_from_slice(&rsdt_off.to_le_bytes());
        e[68..72].copy_from_slice(&(rsdt.len() as u32).to_le_bytes());
    });
    cmd(3, &|e| {
        e[4..4+rsdp_name.len()].copy_from_slice(rsdp_name);
        e[60..64].copy_from_slice(&8u32.to_le_bytes());
        e[64..68].copy_from_slice(&0u32.to_le_bytes());
        e[68..72].copy_from_slice(&20u32.to_le_bytes());
    });

    fw_cfg.add_file("etc/acpi/tables", tables);
    fw_cfg.add_file("etc/acpi/rsdp", rsdp);
    fw_cfg.add_file("etc/table-loader", linker);

    vm_log!("ACPI tables generated (RSDP+RSDT+FADT+DSDT+MADT) via fw_cfg");
}


/// Register standard PC devices: PIC, PIT, CMOS, PS/2, Serial, VGA (800x600).
#[no_mangle]
pub extern "C" fn corevm_setup_standard_devices(handle: u64) {
    vm_log!("setting up standard devices (PIC, PIT, CMOS, PS/2, serial, VGA)");
    let vm = unsafe { vm_from_handle(handle) };

    // PIC
    let pic = Box::into_raw(Box::new(devices::pic::PicPair::new()));
    vm.pic_ptr = pic;
    vm.engine.io.register(0x20, 2, Box::new(IoProxy { ptr: pic }));
    vm.engine.io.register(0xA0, 2, Box::new(IoProxy { ptr: pic }));

    // PIT
    let pit = Box::into_raw(Box::new(devices::pit::Pit::new()));
    vm.pit_ptr = pit;
    vm.engine.io.register(0x40, 4, Box::new(IoProxy { ptr: pit }));
    let port61 = Box::new(devices::port61::Port61::new(pit));
    vm.engine.io.register(0x61, 1, port61);

    // CMOS
    let ram_bytes = vm.engine.memory.ram().size();
    let cmos = Box::into_raw(Box::new(devices::cmos::Cmos::new(ram_bytes)));
    vm.cmos_ptr = cmos;
    vm.engine.io.register(0x70, 2, Box::new(IoProxy { ptr: cmos }));

    // APM
    let apm = Box::new(devices::apm::ApmControl::new());
    vm.engine.io.register(0xB2, 2, apm);

    // PS/2
    let ps2 = Box::into_raw(Box::new(devices::ps2::Ps2Controller::new()));
    vm.ps2_ptr = ps2;
    vm.engine.io.register(0x60, 1, Box::new(IoProxy { ptr: ps2 }));
    vm.engine.io.register(0x64, 1, Box::new(IoProxy { ptr: ps2 }));

    // Serial (COM1)
    let serial = Box::into_raw(Box::new(devices::serial::Serial::new()));
    vm.serial_ptr = serial;
    vm.engine.io.register(0x3F8, 8, Box::new(IoProxy { ptr: serial }));

    // VGA/SVGA
    let svga = Box::into_raw(Box::new(devices::svga::Svga::new(800, 600)));
    vm.svga_ptr = svga;
    vm.engine.io.register(0x3C0, 0x1B, Box::new(IoProxy { ptr: svga }));
    vm.engine.io.register(0x1CE, 2, Box::new(IoProxy { ptr: svga }));
    vm.engine.memory.add_mmio(0xA0000, 0x20000, Box::new(MmioProxy { ptr: svga }));
    vm.engine.memory.add_mmio(0xE0000000, 0x01000000, Box::new(MmioProxy { ptr: svga }));
    vm.engine.memory.add_mmio(0xFD000000, 0x01000000, Box::new(MmioProxy { ptr: svga }));

    // PCI bus with Q35 (MCH + ICH9)
    let mut bus = devices::bus::PciBus::new();

    // Q35 MCH at 0:0.0
    let mut host_bridge = devices::bus::PciDevice::new(0x8086, 0x29C0, 0x06, 0x00, 0x00);
    host_bridge.bus = 0;
    host_bridge.device = 0;
    host_bridge.function = 0;
    host_bridge.config_space[0x90] = 0x30;
    for i in 0x91..=0x96 {
        host_bridge.config_space[i] = 0x33;
    }
    host_bridge.config_space[0x60] = 0x01;
    host_bridge.config_space[0x61] = 0x00;
    host_bridge.config_space[0x62] = 0x00;
    host_bridge.config_space[0x63] = 0xB0;
    host_bridge.config_space[0x64] = 0x00;
    host_bridge.config_space[0x65] = 0x00;
    host_bridge.config_space[0x66] = 0x00;
    host_bridge.config_space[0x67] = 0x00;
    host_bridge.set_subsystem(0x1AF4, 0x1100);
    bus.add_device(host_bridge);

    // ICH9 LPC at 0:1F.0
    let mut lpc_bridge = devices::bus::PciDevice::new(0x8086, 0x2918, 0x06, 0x01, 0x02);
    lpc_bridge.bus = 0;
    lpc_bridge.device = 31;
    lpc_bridge.function = 0;
    lpc_bridge.config_space[0x0E] = 0x00;
    lpc_bridge.set_subsystem(0x1AF4, 0x1100);
    bus.add_device(lpc_bridge);

    // IDE controller at 0:1.0
    let mut ide_pci = devices::bus::PciDevice::new(0x8086, 0x7010, 0x01, 0x01, 0x80);
    ide_pci.bus = 0;
    ide_pci.device = 1;
    ide_pci.function = 0;
    ide_pci.set_interrupt(14, 1);
    ide_pci.set_subsystem(0x1AF4, 0x1100);
    bus.add_device(ide_pci);

    // VGA at 0:2.0
    let mut vga_pci = devices::bus::PciDevice::new(0x1234, 0x1111, 0x03, 0x00, 0x00);
    vga_pci.bus = 0;
    vga_pci.device = 2;
    vga_pci.function = 0;
    vga_pci.set_bar(0, 0xE0000000, 0x01000000, true);
    vga_pci.set_bar(2, 0xFEBE0000, 0x1000, true);
    vga_pci.config_space[0x30] = 0x01;
    vga_pci.config_space[0x31] = 0x00;
    vga_pci.config_space[0x32] = 0x0C;
    vga_pci.config_space[0x33] = 0x00;
    vga_pci.set_subsystem(0x1AF4, 0x1100);
    bus.add_device(vga_pci);

    let bus_ptr = Box::into_raw(Box::new(bus));
    vm.bus_ptr = bus_ptr;
    vm.engine.io.register(0xCF8, 8, Box::new(IoProxy { ptr: bus_ptr }));

    // MMCONFIG
    let mmcfg = devices::bus::PciMmcfgHandler::new(bus_ptr);
    vm.engine.memory.add_mmio(0xB0000000, 0x10000000, Box::new(mmcfg));

    // IO-APIC
    let ioapic = Box::into_raw(Box::new(devices::ioapic::IoApic::new()));
    vm.ioapic_ptr = ioapic;
    vm.engine.memory.add_mmio(0xFEC00000, 0x1000, Box::new(MmioProxy { ptr: ioapic }));

    // Local APIC
    let mut lapic_obj = devices::lapic::Lapic::new();
    #[cfg(feature = "host_test")]
    lapic_obj.set_host_tsc_freq(vm.tsc_freq);
    #[cfg(not(feature = "host_test"))]
    lapic_obj.set_host_tsc_freq(2_000_000_000);
    let lapic = Box::into_raw(Box::new(lapic_obj));
    vm.lapic_ptr = lapic;
    vm.engine.memory.add_mmio(0xFEE00000, 0x1000, Box::new(MmioProxy { ptr: lapic }));

    // ACPI PM
    let acpi_pm = Box::into_raw(Box::new(devices::acpi::AcpiPm::new()));
    vm.acpi_pm_ptr = acpi_pm;
    vm.engine.io.register(0x600, 0x40, Box::new(IoProxy { ptr: acpi_pm }));
    vm.engine.io.register(0xB000, 0x40, Box::new(IoProxy { ptr: acpi_pm }));

    // fw_cfg
    let fw_cfg = Box::into_raw(Box::new(
        devices::fw_cfg::FwCfg::new(ram_bytes as u64),
    ));
    vm.fw_cfg_ptr = fw_cfg;
    vm.engine.io.register(0x510, 2, Box::new(IoProxy { ptr: fw_cfg }));

    generate_acpi_tables(unsafe { &mut *fw_cfg });

    // Debug port
    let debug_port = Box::into_raw(Box::new(devices::debug_port::DebugPort::new()));
    vm.debug_port_ptr = debug_port;
    vm.engine.io.register(0x402, 1, Box::new(IoProxy { ptr: debug_port }));

    let count = vm.engine.memory.mmio_region_count();
    let (lo, hi) = vm.engine.memory.mmio_bounds();
    vm_log!("MMIO setup: {} regions, bounds=[0x{:X}, 0x{:X})", count, lo, hi);
    vm_log!("PCI bus: 4 devices (Q35 MCH 0:0.0, ICH9 LPC 0:1F.0, IDE 0:1.0, VGA 0:2.0)");
}

/// Register a PCI bus at the standard configuration ports.
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

/// Register an Intel E1000 network card.
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
    vm.engine.memory.add_mmio(mmio_base, 0x20000, Box::new(MmioProxy { ptr: e1000 }));
}


// ════════════════════════════════════════════════════════════════════════
// Device Interaction — PS/2
// ════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub extern "C" fn corevm_ps2_key_press(handle: u64, scancode: u8) {
    let vm = unsafe { vm_from_handle(handle) };
    if !vm.ps2_ptr.is_null() {
        unsafe { (*vm.ps2_ptr).key_press(scancode) };
    }
    inject_irq_line(vm, 1);
}

#[no_mangle]
pub extern "C" fn corevm_ps2_key_release(handle: u64, scancode: u8) {
    let vm = unsafe { vm_from_handle(handle) };
    if !vm.ps2_ptr.is_null() {
        unsafe { (*vm.ps2_ptr).key_release(scancode) };
    }
    inject_irq_line(vm, 1);
}

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
            if !width.is_null() { unsafe { *width = svga.width }; }
            if !height.is_null() { unsafe { *height = svga.height }; }
            if !bpp.is_null() { unsafe { *bpp = svga.bpp }; }
            svga.framebuffer.as_ptr()
        }
    }
}

#[no_mangle]
pub extern "C" fn corevm_vga_get_text_buffer(handle: u64, count: *mut u32) -> *const u16 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.svga_ptr.is_null() {
        if !count.is_null() { unsafe { *count = 0 }; }
        return ptr::null();
    }
    let svga = unsafe { &*vm.svga_ptr };
    match svga.mode {
        devices::svga::VgaMode::Text80x25 => {
            if !count.is_null() { unsafe { *count = svga.text_buffer.len() as u32 }; }
            svga.text_buffer.as_ptr()
        }
        _ => {
            if !count.is_null() { unsafe { *count = 0 }; }
            ptr::null()
        }
    }
}

#[no_mangle]
pub extern "C" fn corevm_vga_debug_counters(
    handle: u64,
    total_writes: *mut u64,
    text_writes: *mut u64,
) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.svga_ptr.is_null() { return; }
    let svga = unsafe { &*vm.svga_ptr };
    if !total_writes.is_null() { unsafe { *total_writes = svga.mmio_write_count }; }
    if !text_writes.is_null() { unsafe { *text_writes = svga.mmio_text_write_count }; }
}

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
    if !region_count.is_null() { unsafe { *region_count = count as u32 }; }
    if !min_base.is_null() { unsafe { *min_base = lo }; }
    if !max_end.is_null() { unsafe { *max_end = hi }; }
    if !ram_b8000.is_null() {
        use memory::MemoryBus;
        let val = vm.engine.memory.ram().read_u32(0xB8000).unwrap_or(0);
        unsafe { *ram_b8000 = val };
    }
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — Serial
// ════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub extern "C" fn corevm_serial_send_input(handle: u64, data: *const u8, len: u32) {
    if data.is_null() || len == 0 { return; }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.serial_ptr.is_null() { return; }
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    unsafe { (*vm.serial_ptr).send_input(slice) };
}

#[no_mangle]
pub extern "C" fn corevm_serial_take_output(
    handle: u64,
    buf: *mut u8,
    buf_len: u32,
) -> u32 {
    if buf.is_null() || buf_len == 0 { return 0; }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.serial_ptr.is_null() { return 0; }
    let output = unsafe { (*vm.serial_ptr).take_output() };
    let copy_len = (output.len() as u32).min(buf_len) as usize;
    if copy_len > 0 {
        unsafe { ptr::copy_nonoverlapping(output.as_ptr(), buf, copy_len); }
    }
    copy_len as u32
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — Debug Port
// ════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub extern "C" fn corevm_debug_take_output(
    handle: u64,
    buf: *mut u8,
    buf_len: u32,
) -> u32 {
    if buf.is_null() || buf_len == 0 { return 0; }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.debug_port_ptr.is_null() { return 0; }
    let output = unsafe { (*vm.debug_port_ptr).take_output() };
    let copy_len = (output.len() as u32).min(buf_len) as usize;
    if copy_len > 0 {
        unsafe { ptr::copy_nonoverlapping(output.as_ptr(), buf, copy_len); }
    }
    copy_len as u32
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — E1000
// ════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub extern "C" fn corevm_e1000_receive_packet(handle: u64, data: *const u8, len: u32) {
    if data.is_null() || len == 0 { return; }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.e1000_ptr.is_null() { return; }
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    unsafe { (*vm.e1000_ptr).receive_packet(slice) };
}

#[no_mangle]
pub extern "C" fn corevm_e1000_take_tx_packets(
    handle: u64,
    buf: *mut u8,
    buf_len: u32,
) -> u32 {
    if buf.is_null() || buf_len == 0 { return 0; }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.e1000_ptr.is_null() { return 0; }
    let packets = unsafe { (*vm.e1000_ptr).take_tx_packets() };
    let mut offset: u32 = 0;
    for pkt in &packets {
        let header_size = 4u32;
        let pkt_len = pkt.len() as u32;
        let needed = header_size + pkt_len;
        if offset + needed > buf_len { break; }
        unsafe {
            let len_bytes = pkt_len.to_le_bytes();
            ptr::copy_nonoverlapping(len_bytes.as_ptr(), buf.add(offset as usize), 4);
            offset += header_size;
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

#[no_mangle]
pub extern "C" fn corevm_pit_tick(handle: u64) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.pit_ptr.is_null() { return 0; }
    vm.pit_is_externally_clocked = true;
    let fired = unsafe { (*vm.pit_ptr).tick() };
    if fired { 1 } else { 0 }
}

#[no_mangle]
pub extern "C" fn corevm_pit_advance(handle: u64, n: u32) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.pit_ptr.is_null() { return 0; }
    vm.pit_is_externally_clocked = true;
    unsafe { (*vm.pit_ptr).advance(n) }
}

// ════════════════════════════════════════════════════════════════════════
// Device Interaction — PIC / IRQ Routing
// ════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub extern "C" fn corevm_pic_raise_irq(handle: u64, irq: u8) {
    let vm = unsafe { vm_from_handle(handle) };
    inject_irq_line(vm, irq);
}

/// Route one external IRQ line through available interrupt controllers.
fn inject_irq_line(vm: &mut VmInstance, irq: u8) {
    // IOAPIC path
    if !vm.ioapic_ptr.is_null() {
        let ioapic = unsafe { &mut *vm.ioapic_ptr };
        let pin = if irq == 0 { 2 } else { irq };
        let route_result = ioapic.route_irq(pin)
            .or_else(|| if pin != irq { ioapic.route_irq(irq) } else { None });
        if let Some((vector, level_triggered)) = route_result {
            if !vm.lapic_ptr.is_null() {
                unsafe { (*vm.lapic_ptr).raise_vector(vector, level_triggered) };
            }
        }
    }

    // PIC path
    if !vm.pic_ptr.is_null() {
        let pic = unsafe { &mut *vm.pic_ptr };
        pic.raise_irq(irq);
    }
}

fn pic_has_latched_irq(vm: &VmInstance) -> bool {
    if vm.pic_ptr.is_null() { return false; }
    let pic = unsafe { &*vm.pic_ptr };
    (pic.master.irr & !pic.master.imr) != 0 || (pic.slave.irr & !pic.slave.imr) != 0
}

#[no_mangle]
pub extern "C" fn corevm_pic_get_interrupt(handle: u64) -> i32 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.pic_ptr.is_null() { return -1; }
    match unsafe { (*vm.pic_ptr).get_interrupt_vector() } {
        Some(vec) => vec as i32,
        None => -1,
    }
}

#[no_mangle]
pub extern "C" fn corevm_pic_diag_state(handle: u64) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.pic_ptr.is_null() { return 0; }
    let pic = unsafe { &*vm.pic_ptr };
    (pic.master.irr as u64)
        | ((pic.master.isr as u64) << 8)
        | ((pic.master.imr as u64) << 16)
        | ((pic.slave.irr as u64) << 24)
        | ((pic.slave.isr as u64) << 32)
        | ((pic.slave.imr as u64) << 40)
}

#[no_mangle]
pub extern "C" fn corevm_lapic_diag_state(
    handle: u64,
    init_count_out: *mut u32,
    cur_count_out: *mut u32,
    timer_divide_out: *mut u32,
) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.lapic_ptr.is_null() {
        if !init_count_out.is_null() { unsafe { *init_count_out = 0; } }
        if !cur_count_out.is_null() { unsafe { *cur_count_out = 0; } }
        if !timer_divide_out.is_null() { unsafe { *timer_divide_out = 0; } }
        return 0;
    }
    let lapic = unsafe { &mut *vm.lapic_ptr };
    let (svr, lvt_timer, init, cur, div) = lapic.diag_state();
    if !init_count_out.is_null() { unsafe { *init_count_out = init; } }
    if !cur_count_out.is_null() { unsafe { *cur_count_out = cur; } }
    if !timer_divide_out.is_null() { unsafe { *timer_divide_out = div; } }
    (svr as u64) | ((lvt_timer as u64) << 32)
}

#[no_mangle]
pub extern "C" fn corevm_irq_pending_word(handle: u64, idx: u32) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    vm.engine.interrupts.pending_word(idx as usize)
}

#[no_mangle]
pub extern "C" fn corevm_ioapic_redir_entry(handle: u64, irq: u8) -> u64 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ioapic_ptr.is_null() { return 0; }
    let ioapic = unsafe { &*vm.ioapic_ptr };
    ioapic.redir_entry(irq)
}


// ════════════════════════════════════════════════════════════════════════
// Device Setup — IDE/ATA Disk Controller
// ════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub extern "C" fn corevm_setup_ide(handle: u64) {
    vm_log!("setting up IDE controller (ports 0x1F0-0x1F7, 0x3F6-0x3F7)");
    let vm = unsafe { vm_from_handle(handle) };
    let ide = Box::into_raw(Box::new(devices::ide::Ide::new()));
    vm.ide_ptr = ide;
    vm.engine.io.register(0x1F0, 8, Box::new(IoProxy { ptr: ide }));
    vm.engine.io.register(0x3F6, 2, Box::new(IoProxy { ptr: ide }));
}

#[no_mangle]
pub extern "C" fn corevm_ide_attach_disk(handle: u64, data: *const u8, len: u32) {
    if data.is_null() || len == 0 { return; }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() { return; }
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    vm_log!("attaching IDE master disk image ({} bytes)", len);
    let mut image = alloc::vec::Vec::with_capacity(len as usize);
    image.extend_from_slice(slice);
    unsafe { (*vm.ide_ptr).attach_disk(image) };
}

#[no_mangle]
pub extern "C" fn corevm_ide_attach_slave(handle: u64, data: *const u8, len: u32) {
    if data.is_null() || len == 0 { return; }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() { return; }
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    vm_log!("attaching IDE slave disk image ({} bytes)", len);
    let mut image = alloc::vec::Vec::with_capacity(len as usize);
    image.extend_from_slice(slice);
    unsafe { (*vm.ide_ptr).attach_slave(image) };
}

#[no_mangle]
pub extern "C" fn corevm_ide_attach_disk_fd(handle: u64, fd: u32, size: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() { return; }
    vm_log!("attaching IDE master disk via fd {} ({} bytes)", fd, size);
    unsafe { (*vm.ide_ptr).attach_disk_fd(fd as i32, size) };
}

#[no_mangle]
pub extern "C" fn corevm_ide_attach_slave_fd(handle: u64, fd: u32, size: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() { return; }
    vm_log!("attaching IDE slave disk via fd {} ({} bytes)", fd, size);
    unsafe { (*vm.ide_ptr).attach_slave_fd(fd as i32, size) };
}

#[no_mangle]
pub extern "C" fn corevm_ide_detach_disk(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() { return; }
    unsafe { (*vm.ide_ptr).detach_disk() };
}

#[no_mangle]
pub extern "C" fn corevm_ide_detach_slave(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() { return; }
    unsafe { (*vm.ide_ptr).detach_slave() };
}

#[no_mangle]
pub extern "C" fn corevm_ide_irq_raised(handle: u64) -> u32 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() { return 0; }
    if unsafe { (*vm.ide_ptr).irq_raised() } { 1 } else { 0 }
}

#[no_mangle]
pub extern "C" fn corevm_ide_clear_irq(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ide_ptr.is_null() { return; }
    unsafe { (*vm.ide_ptr).clear_irq() };
}

// ════════════════════════════════════════════════════════════════════════
// Device Setup — AHCI
// ════════════════════════════════════════════════════════════════════════

#[no_mangle]
pub extern "C" fn corevm_setup_ahci(handle: u64, mmio_base: u64, num_ports: u8) {
    vm_log!("setting up AHCI controller at MMIO 0x{:X} ({} ports)", mmio_base, num_ports);
    let vm = unsafe { vm_from_handle(handle) };
    let ahci = Box::into_raw(Box::new(devices::ahci::Ahci::new(num_ports)));
    let (mem_ptr, mem_len) = vm.engine.memory.ram_mut_ptr();
    unsafe { (*ahci).set_guest_memory(mem_ptr, mem_len) };
    vm.ahci_ptr = ahci;
    vm.engine.memory.add_mmio(
        mmio_base,
        devices::ahci::AHCI_MMIO_SIZE,
        Box::new(MmioProxy { ptr: ahci }),
    );
    if !vm.bus_ptr.is_null() {
        let mut pci_dev = devices::ahci::create_ahci_pci_device(mmio_base as u32);
        pci_dev.bus = 0;
        pci_dev.device = 4;
        pci_dev.function = 0;
        unsafe { (*vm.bus_ptr).add_device(pci_dev) };
    }
}

#[no_mangle]
pub extern "C" fn corevm_ahci_attach_disk(handle: u64, port: u8, data: *const u8, len: u64) {
    if data.is_null() || len == 0 { return; }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ahci_ptr.is_null() { return; }
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    vm_log!("attaching AHCI port {} disk image ({} bytes)", port, len);
    let mut image = alloc::vec::Vec::with_capacity(len as usize);
    image.extend_from_slice(slice);
    unsafe { (*vm.ahci_ptr).attach_disk(port as usize, image, devices::ahci::AhciDriveKind::AtaDisk) };
}

#[no_mangle]
pub extern "C" fn corevm_ahci_attach_cdrom(handle: u64, port: u8, data: *const u8, len: u64) {
    if data.is_null() || len == 0 { return; }
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ahci_ptr.is_null() { return; }
    let slice = unsafe { core::slice::from_raw_parts(data, len as usize) };
    vm_log!("attaching AHCI port {} CDROM image ({} bytes)", port, len);
    let mut image = alloc::vec::Vec::with_capacity(len as usize);
    image.extend_from_slice(slice);
    unsafe { (*vm.ahci_ptr).attach_disk(port as usize, image, devices::ahci::AhciDriveKind::AtapiCdrom) };
}

#[no_mangle]
pub extern "C" fn corevm_ahci_attach_disk_fd(handle: u64, port: u8, fd: u64, size: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ahci_ptr.is_null() { return; }
    vm_log!("attaching AHCI port {} disk fd={} size={}", port, fd, size);
    unsafe { (*vm.ahci_ptr).attach_disk_fd(port as usize, fd as i32, size, devices::ahci::AhciDriveKind::AtaDisk) };
}

#[no_mangle]
pub extern "C" fn corevm_ahci_attach_cdrom_fd(handle: u64, port: u8, fd: u64, size: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ahci_ptr.is_null() { return; }
    vm_log!("attaching AHCI port {} CDROM fd={} size={}", port, fd, size);
    unsafe { (*vm.ahci_ptr).attach_disk_fd(port as usize, fd as i32, size, devices::ahci::AhciDriveKind::AtapiCdrom) };
}

#[no_mangle]
pub extern "C" fn corevm_ahci_irq_raised(handle: u64) -> u8 {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ahci_ptr.is_null() { return 0; }
    if unsafe { (*vm.ahci_ptr).irq_raised() } { 1 } else { 0 }
}

#[no_mangle]
pub extern "C" fn corevm_ahci_clear_irq(handle: u64) {
    let vm = unsafe { vm_from_handle(handle) };
    if vm.ahci_ptr.is_null() { return; }
    unsafe { (*vm.ahci_ptr).clear_irq() };
}

// ════════════════════════════════════════════════════════════════════════
// JIT / Decode Cache — Compatibility Stubs
// ════════════════════════════════════════════════════════════════════════
// These functions are kept as no-ops for C ABI compatibility.

#[no_mangle]
pub extern "C" fn corevm_jit_cache_stats(
    _handle: u64,
    cached_blocks: *mut u32,
    hits: *mut u64,
    misses: *mut u64,
) {
    if !cached_blocks.is_null() { unsafe { *cached_blocks = 0 }; }
    if !hits.is_null() { unsafe { *hits = 0 }; }
    if !misses.is_null() { unsafe { *misses = 0 }; }
}

#[no_mangle]
pub extern "C" fn corevm_jit_flush_cache(_handle: u64) {}

#[no_mangle]
pub extern "C" fn corevm_jit_enable(_handle: u64, _enable: u32) {
    // Hardware virtualization — JIT not applicable.
}

#[no_mangle]
pub extern "C" fn corevm_jit_stats(
    _handle: u64,
    blocks_compiled: *mut u64,
    native_count: *mut u64,
    fallback_count: *mut u64,
    code_buffer_used: *mut u32,
) {
    if !blocks_compiled.is_null() { unsafe { *blocks_compiled = 0 }; }
    if !native_count.is_null() { unsafe { *native_count = 0 }; }
    if !fallback_count.is_null() { unsafe { *fallback_count = 0 }; }
    if !code_buffer_used.is_null() { unsafe { *code_buffer_used = 0 }; }
}

#[no_mangle]
pub extern "C" fn corevm_jit_helper_top(
    _handle: u64,
    _buf: *mut u8,
    _buf_len: u32,
) -> u32 {
    0
}

#[no_mangle]
pub extern "C" fn corevm_cache_stats(
    _handle: u64,
    hits: *mut u64,
    misses: *mut u64,
    entries: *mut u64,
) {
    if !hits.is_null() { unsafe { *hits = 0 }; }
    if !misses.is_null() { unsafe { *misses = 0 }; }
    if !entries.is_null() { unsafe { *entries = 0 }; }
}

// ── Debugger FFI (stubs) ─────────────────────────────────────────────

#[cfg(feature = "host_test")]
#[no_mangle]
pub extern "C" fn corevm_debugger_enable() {
    // Debugger not yet adapted for hardware virtualization.
}

#[cfg(feature = "host_test")]
#[no_mangle]
pub extern "C" fn corevm_debugger_break() {
    // Debugger not yet adapted for hardware virtualization.
}
