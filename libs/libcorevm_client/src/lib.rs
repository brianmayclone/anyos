//! libcorevm_client -- Client-side wrapper for libcorevm.so.
//!
//! Provides a safe, typed Rust API for user programs to create and control
//! virtual machine instances hosted in libcorevm.so. The shared library is
//! loaded at runtime via `dl_open`/`dl_sym`, and all communication happens
//! through C ABI function pointers resolved during [`init`].
//!
//! # Architecture
//!
//! The client library mirrors the pattern used by `libanyui_client`:
//! - A `CoreVmLib` struct holds cached function pointers resolved from the .so
//! - A `static mut` singleton stores the loaded library state
//! - `VmHandle` provides a high-level RAII wrapper that automatically destroys
//!   the VM on drop
//!
//! # Usage
//!
//! ```rust
//! use libcorevm_client::{self as vm, VmHandle};
//!
//! vm::init();
//! let vm = VmHandle::new(256).unwrap(); // 256 MiB RAM
//! vm.create_vcpu(0);
//! vm.load_binary(0xF_0000, &bios_rom);
//! vm.setup_standard_devices();
//!
//! loop {
//!     match vm.run_vcpu(0) {
//!         VmExitReason::Halted => break,
//!         VmExitReason::IoOut { port, size, data } => { /* handle */ }
//!         _ => break,
//!     }
//! }
//! ```

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use dynlink::{dl_open, dl_sym, DlHandle};

// ══════════════════════════════════════════════════════════════════════
//  C-compatible types (mirror backend/types.rs)
// ══════════════════════════════════════════════════════════════════════

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VcpuRegs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SegmentReg {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DescriptorTable {
    pub base: u64,
    pub limit: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VcpuSregs {
    pub cs: SegmentReg,
    pub ds: SegmentReg,
    pub es: SegmentReg,
    pub fs: SegmentReg,
    pub gs: SegmentReg,
    pub ss: SegmentReg,
    pub tr: SegmentReg,
    pub ldt: SegmentReg,
    pub gdt: DescriptorTable,
    pub idt: DescriptorTable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuidEntry {
    pub function: u32,
    pub index: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// C-compatible tagged struct for VM exit reasons (matches ffi.rs CExitReason).
#[repr(C)]
#[derive(Default)]
struct CExitReason {
    reason: u32,
    port: u16,
    size: u8,
    _pad: u8,
    data_u32: u32,
    _pad2: u32,
    addr: u64,
    data_u64: u64,
    msr_index: u32,
    cpuid_fn: u32,
    cpuid_idx: u32,
    _reserved: u32,
}

// ══════════════════════════════════════════════════════════════════════
//  VM exit reason enum (Rust-friendly)
// ══════════════════════════════════════════════════════════════════════

/// Reason the VM stopped executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmExitReason {
    IoIn {
        port: u16,
        size: u8,
    },
    IoOut {
        port: u16,
        size: u8,
        data: u32,
    },
    MmioRead {
        addr: u64,
        size: u8,
    },
    MmioWrite {
        addr: u64,
        size: u8,
        data: u64,
    },
    MsrRead {
        index: u32,
    },
    MsrWrite {
        index: u32,
        value: u64,
    },
    CpuidExit {
        function: u32,
        index: u32,
    },
    Halted,
    InterruptWindow,
    Shutdown,
    Debug,
    StringIo {
        port: u16,
        size: u8,
        is_out: bool,
        count: u32,
    },
    Error,
}

impl VmExitReason {
    fn from_c(c: &CExitReason) -> Self {
        match c.reason {
            0 => VmExitReason::IoIn {
                port: c.port,
                size: c.size,
            },
            1 => VmExitReason::IoOut {
                port: c.port,
                size: c.size,
                data: c.data_u32,
            },
            2 => VmExitReason::MmioRead {
                addr: c.addr,
                size: c.size,
            },
            3 => VmExitReason::MmioWrite {
                addr: c.addr,
                size: c.size,
                data: c.data_u64,
            },
            4 => VmExitReason::MsrRead { index: c.msr_index },
            5 => VmExitReason::MsrWrite {
                index: c.msr_index,
                value: c.data_u64,
            },
            6 => VmExitReason::CpuidExit {
                function: c.cpuid_fn,
                index: c.cpuid_idx,
            },
            7 => VmExitReason::Halted,
            8 => VmExitReason::InterruptWindow,
            9 => VmExitReason::Shutdown,
            10 => VmExitReason::Debug,
            12 => VmExitReason::StringIo {
                port: c.port,
                size: c.size,
                is_out: c.data_u32 != 0,
                count: c.addr as u32,
            },
            _ => VmExitReason::Error,
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Internal: cached function pointers from libcorevm.so
// ══════════════════════════════════════════════════════════════════════

/// Holds all resolved function pointers from libcorevm.so.
struct CoreVmLib {
    _handle: DlHandle,

    // VM lifecycle
    create: extern "C" fn(u32) -> u64,
    destroy: extern "C" fn(u64),
    reset: extern "C" fn(u64) -> i32,

    // vCPU management
    create_vcpu: extern "C" fn(u64, u32) -> i32,
    destroy_vcpu: extern "C" fn(u64, u32) -> i32,
    run_vcpu: extern "C" fn(u64, u32, *mut CExitReason) -> i32,

    // Register access
    get_vcpu_regs: extern "C" fn(u64, u32, *mut VcpuRegs) -> i32,
    set_vcpu_regs: extern "C" fn(u64, u32, *const VcpuRegs) -> i32,
    get_vcpu_sregs: extern "C" fn(u64, u32, *mut VcpuSregs) -> i32,
    set_vcpu_sregs: extern "C" fn(u64, u32, *const VcpuSregs) -> i32,

    // Interrupt injection
    inject_interrupt: extern "C" fn(u64, u32, u8) -> i32,
    inject_exception: extern "C" fn(u64, u32, u8, i64) -> i32,
    inject_nmi: extern "C" fn(u64, u32) -> i32,
    request_interrupt_window: extern "C" fn(u64, u32, u8) -> i32,

    // CPUID
    set_cpuid: extern "C" fn(u64, *const CpuidEntry, u32) -> i32,

    // Memory
    set_memory_region: extern "C" fn(u64, u32, u64, u64, *mut u8) -> i32,
    read_phys: extern "C" fn(u64, u64, *mut u8, u32) -> i32,
    write_phys: extern "C" fn(u64, u64, *const u8, u32) -> i32,
    load_binary: extern "C" fn(u64, u64, *const u8, u32) -> i32,

    // Error reporting
    last_error: extern "C" fn() -> *const u8,
    last_error_len: extern "C" fn() -> u32,

    // Hardware support
    has_hw_support: extern "C" fn() -> i32,

    // I/O and MMIO exit dispatch
    handle_io_exit: extern "C" fn(u64, u16, u8, u8, *mut u8) -> i32,
    handle_mmio_exit: extern "C" fn(u64, u64, u8, u8, *mut u8, u8, u8) -> i32,
    handle_string_io_exit: extern "C" fn(u64, u16, u8, u8, *mut u8, u32) -> i32,
    drain_coalesced_mmio: extern "C" fn(u64) -> u32,

    // Device setup
    setup_standard_devices: extern "C" fn(u64) -> i32,
    setup_e1000: extern "C" fn(u64, *const u8) -> i32,
    setup_ahci: extern "C" fn(u64, u8) -> i32,
    setup_acpi_tables: extern "C" fn(u64) -> i32,
    setup_acpi_tables_with_hpet: extern "C" fn(u64) -> i32,
    fw_cfg_add_file: extern "C" fn(u64, *const u8, u32, *const u8, u32) -> i32,
    ahci_attach_disk: extern "C" fn(u64, u32, i32, u64) -> i32,
    ahci_attach_cdrom: extern "C" fn(u64, u32, i32, u64) -> i32,

    // CMOS boot order
    cmos_set_boot_order: extern "C" fn(u64, u8, u8) -> i32,

    // Timer & IRQ polling
    pit_advance: extern "C" fn(u64, u32) -> u32,
    cmos_advance: extern "C" fn(u64, u64) -> u32,
    poll_irqs: extern "C" fn(u64) -> u32,
    lapic_timer_advance: extern "C" fn(u64, u64) -> u32,

    // PS/2 input
    ps2_key_press: extern "C" fn(u64, u8) -> i32,
    ps2_key_release: extern "C" fn(u64, u8) -> i32,
    ps2_mouse_move: extern "C" fn(u64, i16, i16, u8) -> i32,

    // Serial
    serial_send_input: extern "C" fn(u64, *const u8, u32) -> i32,
    serial_take_output: extern "C" fn(u64, *mut u8, u32) -> i32,

    // VGA
    vga_get_framebuffer: extern "C" fn(u64, *mut *const u8, *mut u32) -> i32,
    vga_get_text_buffer: extern "C" fn(u64, *mut *const u16, *mut u32) -> i32,
    vga_get_mode: extern "C" fn(u64, *mut u32, *mut u32, *mut u32) -> i32,
    vga_get_lfb_addr: extern "C" fn(u64) -> u64,
    // Debug
    debug_port_take_output: extern "C" fn(u64, *mut u8, u32) -> i32,
}

/// Singleton holding the loaded library.
static mut LIB: Option<CoreVmLib> = None;

/// Get a reference to the loaded library, panicking if not initialized.
fn lib() -> &'static CoreVmLib {
    unsafe {
        LIB.as_ref()
            .expect("libcorevm not loaded -- call init() first")
    }
}

/// Resolve a function pointer from the loaded library, or panic.
unsafe fn resolve<T: Copy>(handle: &DlHandle, name: &str) -> T {
    let ptr = match dl_sym(handle, name) {
        Some(p) => p,
        None => panic!("symbol '{}' not found in libcorevm.so", name),
    };
    core::mem::transmute_copy::<*const (), T>(&ptr)
}

// ══════════════════════════════════════════════════════════════════════
//  Public API: init
// ══════════════════════════════════════════════════════════════════════

/// Load and initialize libcorevm.so.
///
/// Must be called once before any other function in this crate. Returns
/// `true` on success, `false` if the shared library could not be loaded.
pub fn init() -> bool {
    let handle = match dl_open("/Libraries/libcorevm.so") {
        Some(h) => h,
        None => return false,
    };

    unsafe {
        let corevm = CoreVmLib {
            // VM lifecycle
            create: resolve(&handle, "corevm_create"),
            destroy: resolve(&handle, "corevm_destroy"),
            reset: resolve(&handle, "corevm_reset"),
            // vCPU management
            create_vcpu: resolve(&handle, "corevm_create_vcpu"),
            destroy_vcpu: resolve(&handle, "corevm_destroy_vcpu"),
            run_vcpu: resolve(&handle, "corevm_run_vcpu"),
            // Register access
            get_vcpu_regs: resolve(&handle, "corevm_get_vcpu_regs"),
            set_vcpu_regs: resolve(&handle, "corevm_set_vcpu_regs"),
            get_vcpu_sregs: resolve(&handle, "corevm_get_vcpu_sregs"),
            set_vcpu_sregs: resolve(&handle, "corevm_set_vcpu_sregs"),
            // Interrupt injection
            inject_interrupt: resolve(&handle, "corevm_inject_interrupt"),
            inject_exception: resolve(&handle, "corevm_inject_exception"),
            inject_nmi: resolve(&handle, "corevm_inject_nmi"),
            request_interrupt_window: resolve(&handle, "corevm_request_interrupt_window"),
            // CPUID
            set_cpuid: resolve(&handle, "corevm_set_cpuid"),
            // Memory
            set_memory_region: resolve(&handle, "corevm_set_memory_region"),
            read_phys: resolve(&handle, "corevm_read_phys"),
            write_phys: resolve(&handle, "corevm_write_phys"),
            load_binary: resolve(&handle, "corevm_load_binary"),
            // Error reporting
            last_error: resolve(&handle, "corevm_last_error"),
            last_error_len: resolve(&handle, "corevm_last_error_len"),
            // Hardware support
            has_hw_support: resolve(&handle, "corevm_has_hw_support"),
            // I/O and MMIO exit dispatch
            handle_io_exit: resolve(&handle, "corevm_handle_io_exit"),
            handle_mmio_exit: resolve(&handle, "corevm_handle_mmio_exit"),
            handle_string_io_exit: resolve(&handle, "corevm_handle_string_io_exit"),
            drain_coalesced_mmio: resolve(&handle, "corevm_drain_coalesced_mmio"),
            // Device setup
            setup_standard_devices: resolve(&handle, "corevm_setup_standard_devices"),
            setup_e1000: resolve(&handle, "corevm_setup_e1000"),
            setup_ahci: resolve(&handle, "corevm_setup_ahci"),
            setup_acpi_tables: resolve(&handle, "corevm_setup_acpi_tables"),
            setup_acpi_tables_with_hpet: resolve(&handle, "corevm_setup_acpi_tables_with_hpet"),
            fw_cfg_add_file: resolve(&handle, "corevm_fw_cfg_add_file"),
            ahci_attach_disk: resolve(&handle, "corevm_ahci_attach_disk"),
            ahci_attach_cdrom: resolve(&handle, "corevm_ahci_attach_cdrom"),
            // CMOS boot order
            cmos_set_boot_order: resolve(&handle, "corevm_cmos_set_boot_order"),
            // Timer & IRQ polling
            pit_advance: resolve(&handle, "corevm_pit_advance"),
            cmos_advance: resolve(&handle, "corevm_cmos_advance"),
            poll_irqs: resolve(&handle, "corevm_poll_irqs"),
            lapic_timer_advance: resolve(&handle, "corevm_lapic_timer_advance"),
            // PS/2 input
            ps2_key_press: resolve(&handle, "corevm_ps2_key_press"),
            ps2_key_release: resolve(&handle, "corevm_ps2_key_release"),
            ps2_mouse_move: resolve(&handle, "corevm_ps2_mouse_move"),
            // Serial
            serial_send_input: resolve(&handle, "corevm_serial_send_input"),
            serial_take_output: resolve(&handle, "corevm_serial_take_output"),
            // VGA
            vga_get_framebuffer: resolve(&handle, "corevm_vga_get_framebuffer"),
            vga_get_text_buffer: resolve(&handle, "corevm_vga_get_text_buffer"),
            vga_get_mode: resolve(&handle, "corevm_vga_get_mode"),
            vga_get_lfb_addr: resolve(&handle, "corevm_vga_get_lfb_addr"),
            debug_port_take_output: resolve(&handle, "corevm_debug_port_take_output"),
            // Handle
            _handle: handle,
        };
        LIB = Some(corevm);
    }

    true
}

// ══════════════════════════════════════════════════════════════════════
//  Error reporting
// ══════════════════════════════════════════════════════════════════════

/// Get the last error message from libcorevm, if any.
pub fn last_error() -> Option<alloc::string::String> {
    let l = lib();
    let len = (l.last_error_len)() as usize;
    if len == 0 {
        return None;
    }
    let ptr = (l.last_error)();
    if ptr.is_null() {
        return None;
    }
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    Some(alloc::string::String::from_utf8_lossy(bytes).into_owned())
}

// ══════════════════════════════════════════════════════════════════════
//  VmHandle: high-level RAII wrapper
// ══════════════════════════════════════════════════════════════════════

/// An active virtual machine instance.
///
/// Wraps an opaque `u64` handle returned by `corevm_create` and provides
/// typed methods for all VM operations. The VM is automatically destroyed
/// when the handle is dropped.
pub struct VmHandle {
    handle: u64,
}

impl VmHandle {
    // ── Lifecycle ────────────────────────────────────────────────

    /// Create a new virtual machine with the specified guest RAM size.
    ///
    /// Returns `Ok(VmHandle)` on success, `Err(String)` with a descriptive
    /// error message on failure (e.g., missing KVM/WHP support).
    pub fn new(ram_mb: u32) -> Result<Self, alloc::string::String> {
        let h = (lib().create)(ram_mb);
        if h == 0 {
            Err(last_error().unwrap_or_else(|| "Unknown error creating VM".into()))
        } else {
            Ok(VmHandle { handle: h })
        }
    }

    /// Reset the VM. Returns 0 on success, -1 on error.
    pub fn reset(&self) -> i32 {
        (lib().reset)(self.handle)
    }

    // ── vCPU management ──────────────────────────────────────────

    /// Create a vCPU with the given ID. Returns 0 on success.
    pub fn create_vcpu(&self, id: u32) -> i32 {
        (lib().create_vcpu)(self.handle, id)
    }

    /// Destroy a vCPU. Returns 0 on success.
    pub fn destroy_vcpu(&self, id: u32) -> i32 {
        (lib().destroy_vcpu)(self.handle, id)
    }

    /// Run a vCPU until it exits. Returns the exit reason.
    pub fn run_vcpu(&self, id: u32) -> VmExitReason {
        let mut exit = CExitReason::default();
        let rc = (lib().run_vcpu)(self.handle, id, &mut exit);
        if rc != 0 {
            return VmExitReason::Error;
        }
        VmExitReason::from_c(&exit)
    }

    // ── Register access ──────────────────────────────────────────

    /// Get general-purpose registers for a vCPU.
    pub fn get_vcpu_regs(&self, id: u32) -> VcpuRegs {
        let mut regs = VcpuRegs::default();
        (lib().get_vcpu_regs)(self.handle, id, &mut regs);
        regs
    }

    /// Set general-purpose registers for a vCPU. Returns 0 on success.
    pub fn set_vcpu_regs(&self, id: u32, regs: &VcpuRegs) -> i32 {
        (lib().set_vcpu_regs)(self.handle, id, regs)
    }

    /// Get system registers for a vCPU.
    pub fn get_vcpu_sregs(&self, id: u32) -> VcpuSregs {
        let mut sregs = VcpuSregs::default();
        (lib().get_vcpu_sregs)(self.handle, id, &mut sregs);
        sregs
    }

    /// Set system registers for a vCPU. Returns 0 on success.
    pub fn set_vcpu_sregs(&self, id: u32, sregs: &VcpuSregs) -> i32 {
        (lib().set_vcpu_sregs)(self.handle, id, sregs)
    }

    // ── Interrupt injection ──────────────────────────────────────

    /// Inject an external interrupt into a vCPU.
    pub fn inject_interrupt(&self, id: u32, vector: u8) -> i32 {
        (lib().inject_interrupt)(self.handle, id, vector)
    }

    /// Inject an exception into a vCPU.
    pub fn inject_exception(
        &self,
        id: u32,
        vector: u8,
        has_error_code: bool,
        error_code: u32,
    ) -> i32 {
        let ec: i64 = if has_error_code {
            error_code as i64
        } else {
            -1
        };
        (lib().inject_exception)(self.handle, id, vector, ec)
    }

    /// Inject an NMI into a vCPU.
    pub fn inject_nmi(&self, id: u32) -> i32 {
        (lib().inject_nmi)(self.handle, id)
    }

    /// Request or cancel interrupt window notification for a vCPU.
    pub fn request_interrupt_window(&self, id: u32, enable: bool) -> i32 {
        (lib().request_interrupt_window)(self.handle, id, enable as u8)
    }

    // ── CPUID ────────────────────────────────────────────────────

    /// Set CPUID entries for the VM.
    pub fn set_cpuid(&self, entries: &[CpuidEntry]) -> i32 {
        (lib().set_cpuid)(self.handle, entries.as_ptr(), entries.len() as u32)
    }

    // ── Memory ───────────────────────────────────────────────────

    /// Map a memory region into the guest physical address space.
    pub fn set_memory_region(
        &self,
        slot: u32,
        guest_phys: u64,
        size: u64,
        host_ptr: *mut u8,
    ) -> i32 {
        (lib().set_memory_region)(self.handle, slot, guest_phys, size, host_ptr)
    }

    /// Read from guest physical memory.
    pub fn read_phys(&self, addr: u64, buf: &mut [u8]) -> i32 {
        (lib().read_phys)(self.handle, addr, buf.as_mut_ptr(), buf.len() as u32)
    }

    /// Write to guest physical memory.
    pub fn write_phys(&self, addr: u64, buf: &[u8]) -> i32 {
        (lib().write_phys)(self.handle, addr, buf.as_ptr(), buf.len() as u32)
    }

    /// Load binary data at a guest physical address.
    pub fn load_binary(&self, addr: u64, data: &[u8]) -> i32 {
        (lib().load_binary)(self.handle, addr, data.as_ptr(), data.len() as u32)
    }

    // ── Hardware support ─────────────────────────────────────────

    /// Returns true if hardware virtualization is available.
    pub fn has_hw_support() -> bool {
        (lib().has_hw_support)() != 0
    }

    /// Returns the hardware virtualization type as a string:
    /// "VT-x", "AMD-V", or "none".
    pub fn hw_type() -> &'static str {
        match (lib().has_hw_support)() {
            1 => "Intel VT-x",
            2 => "AMD-V",
            _ => "none",
        }
    }

    // ── I/O and MMIO exit dispatch ───────────────────────────────

    /// Dispatch a port I/O exit to registered device handlers.
    pub fn handle_io_exit(&self, port: u16, direction: u8, size: u8, data: &mut [u8]) -> i32 {
        (lib().handle_io_exit)(self.handle, port, direction, size, data.as_mut_ptr())
    }

    /// Dispatch an MMIO exit to registered device handlers.
    pub fn handle_mmio_exit(
        &self,
        addr: u64,
        direction: u8,
        size: u8,
        data: &mut [u8],
        dest_reg: u8,
        instr_len: u8,
    ) -> i32 {
        (lib().handle_mmio_exit)(
            self.handle,
            addr,
            direction,
            size,
            data.as_mut_ptr(),
            dest_reg,
            instr_len,
        )
    }

    /// Drain coalesced MMIO writes batched by KVM during the last run_vcpu.
    /// Must be called after run_vcpu returns and before dispatching MMIO exits,
    /// so that writes preceding a read are applied before the read is processed.
    pub fn drain_coalesced_mmio(&self) -> u32 {
        (lib().drain_coalesced_mmio)(self.handle)
    }

    /// Dispatch a string I/O (REP INS/OUTS) exit to device handlers.
    pub fn handle_string_io_exit(
        &self,
        port: u16,
        direction: u8,
        size: u8,
        data: &mut [u8],
        count: u32,
    ) -> i32 {
        (lib().handle_string_io_exit)(self.handle, port, direction, size, data.as_mut_ptr(), count)
    }

    // ── Device setup ─────────────────────────────────────────────

    /// Register all standard chipset devices (PIC, PIT, PS/2, CMOS, serial, VGA).
    pub fn setup_standard_devices(&self) -> i32 {
        (lib().setup_standard_devices)(self.handle)
    }

    /// Set up an E1000 NIC with the given MAC address.
    pub fn setup_e1000(&self, mac: &[u8; 6]) -> i32 {
        (lib().setup_e1000)(self.handle, mac.as_ptr())
    }

    /// Set up the AHCI SATA controller. Returns 0 on success.
    pub fn setup_ahci(&self, num_ports: u8) -> i32 {
        (lib().setup_ahci)(self.handle, num_ports)
    }

    /// Set up ACPI tables (RSDP, RSDT, FADT, MADT, DSDT) and register them via fw_cfg.
    pub fn setup_acpi_tables(&self) -> i32 {
        (lib().setup_acpi_tables)(self.handle)
    }

    /// Set up ACPI tables with HPET table included (for Windows guests).
    pub fn setup_acpi_tables_with_hpet(&self) -> i32 {
        (lib().setup_acpi_tables_with_hpet)(self.handle)
    }

    /// Add a file to the fw_cfg device (used by SeaBIOS to find ROMs and tables).
    pub fn fw_cfg_add_file(&self, name: &str, data: &[u8]) -> i32 {
        (lib().fw_cfg_add_file)(
            self.handle,
            name.as_ptr(),
            name.len() as u32,
            data.as_ptr(),
            data.len() as u32,
        )
    }

    /// Attach a disk image to an AHCI port (fd-backed).
    pub fn ahci_attach_disk(&self, port: u32, fd: i32, size: u64) -> i32 {
        (lib().ahci_attach_disk)(self.handle, port, fd, size)
    }

    /// Attach a CD-ROM image to an AHCI port (fd-backed).
    pub fn ahci_attach_cdrom(&self, port: u32, fd: i32, size: u64) -> i32 {
        (lib().ahci_attach_cdrom)(self.handle, port, fd, size)
    }

    // ── CMOS boot order ────────────────────────────────────────

    /// Set CMOS boot device priority.
    /// `first` and `second`: 0=none, 1=floppy, 2=HDD, 3=CD-ROM, 4=BEV/network.
    /// Call after `setup_standard_devices()` and before starting the VM.
    pub fn cmos_set_boot_order(&self, first: u8, second: u8) -> i32 {
        (lib().cmos_set_boot_order)(self.handle, first, second)
    }

    // ── Timer & IRQ polling ─────────────────────────────────────

    /// Advance the PIT by the given number of ticks and deliver any pending IRQ 0.
    pub fn pit_advance(&self, ticks: u32) -> u32 {
        (lib().pit_advance)(self.handle, ticks)
    }

    /// Advance the CMOS RTC periodic timer. `ticks_32768` is in units of
    /// the 32.768 kHz base clock. Returns 1 if IRQ 8 fired.
    pub fn cmos_advance(&self, ticks_32768: u64) -> u32 {
        (lib().cmos_advance)(self.handle, ticks_32768)
    }

    /// Poll all device IRQs (PS/2, AHCI, etc.) and route pending interrupts
    /// through the PIC/IOAPIC. Returns a bitmask of pending vectors.
    pub fn poll_irqs(&self) -> u32 {
        (lib().poll_irqs)(self.handle)
    }

    /// Advance the LAPIC timer by the given number of TSC ticks.
    pub fn lapic_timer_advance(&self, ticks: u64) -> u32 {
        (lib().lapic_timer_advance)(self.handle, ticks)
    }

    // ── PS/2 input ───────────────────────────────────────────────

    /// Send a PS/2 key press scancode.
    pub fn ps2_key_press(&self, scancode: u8) {
        (lib().ps2_key_press)(self.handle, scancode);
    }

    /// Send a PS/2 key release scancode.
    pub fn ps2_key_release(&self, scancode: u8) {
        (lib().ps2_key_release)(self.handle, scancode);
    }

    /// Send a PS/2 mouse movement.
    pub fn ps2_mouse_move(&self, dx: i16, dy: i16, buttons: u8) {
        (lib().ps2_mouse_move)(self.handle, dx, dy, buttons);
    }

    // ── Serial port ──────────────────────────────────────────────

    /// Send input bytes to the guest serial port (COM1).
    pub fn serial_send_input(&self, data: &[u8]) {
        (lib().serial_send_input)(self.handle, data.as_ptr(), data.len() as u32);
    }

    /// Take output bytes from the guest serial port (COM1).
    pub fn serial_take_output(&self) -> Vec<u8> {
        let mut buf = [0u8; 4096];
        let n = (lib().serial_take_output)(self.handle, buf.as_mut_ptr(), buf.len() as u32);
        if n <= 0 {
            return Vec::new();
        }
        let mut v = Vec::with_capacity(n as usize);
        v.extend_from_slice(&buf[..n as usize]);
        v
    }

    // ── VGA ──────────────────────────────────────────────────────

    /// Get the VGA framebuffer contents.
    /// Returns `(pixels, width, height, bpp)` or `None` if unavailable.
    pub fn vga_framebuffer(&self) -> Option<(Vec<u8>, u32, u32, u8)> {
        let mut ptr: *const u8 = core::ptr::null();
        let mut len: u32 = 0;
        let rc = (lib().vga_get_framebuffer)(self.handle, &mut ptr, &mut len);
        if rc != 0 || ptr.is_null() || len == 0 {
            return None;
        }
        let slice = unsafe { core::slice::from_raw_parts(ptr, len as usize) };
        let mut v = Vec::with_capacity(len as usize);
        v.extend_from_slice(slice);
        // The raw FFI only gives us ptr+len; width/height/bpp are not
        // returned by the current C API, so we return sensible defaults.
        // Callers needing dimensions should query VGA mode separately.
        Some((v, 0, 0, 0))
    }

    /// Get the VGA text buffer contents (array of u16 char+attr pairs).
    pub fn vga_text_buffer(&self) -> Option<Vec<u16>> {
        let mut ptr: *const u16 = core::ptr::null();
        let mut len: u32 = 0;
        let rc = (lib().vga_get_text_buffer)(self.handle, &mut ptr, &mut len);
        if rc != 0 || ptr.is_null() || len == 0 {
            return None;
        }
        let slice = unsafe { core::slice::from_raw_parts(ptr, len as usize) };
        let mut v = Vec::with_capacity(len as usize);
        v.extend_from_slice(slice);
        Some(v)
    }

    /// Get current VGA mode info: (width, height, bpp).
    pub fn vga_get_mode(&self) -> (u32, u32, u32) {
        let (mut w, mut h, mut bpp) = (0u32, 0u32, 0u32);
        (lib().vga_get_mode)(self.handle, &mut w, &mut h, &mut bpp);
        (w, h, bpp)
    }

    /// Get the VGA linear framebuffer physical address.
    pub fn vga_get_lfb_addr(&self) -> u64 {
        (lib().vga_get_lfb_addr)(self.handle)
    }

    /// Take debug port (0xE9) output bytes.
    pub fn debug_port_take_output(&self) -> Vec<u8> {
        let mut buf = [0u8; 4096];
        let n = (lib().debug_port_take_output)(self.handle, buf.as_mut_ptr(), buf.len() as u32);
        if n <= 0 {
            return Vec::new();
        }
        let mut v = Vec::with_capacity(n as usize);
        v.extend_from_slice(&buf[..n as usize]);
        v
    }
}

impl Drop for VmHandle {
    fn drop(&mut self) {
        (lib().destroy)(self.handle);
    }
}
