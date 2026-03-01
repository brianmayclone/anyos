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
//! use libcorevm_client::{self as vm, VmHandle, ExitReason};
//!
//! vm::init();
//! let vm = VmHandle::new(16).unwrap(); // 16 MiB RAM
//! vm.load_binary(0xF_0000, &bios_rom);
//! vm.set_rip(0xFFF0);
//! vm.setup_standard_devices();
//!
//! loop {
//!     match vm.run(1_000_000) {
//!         ExitReason::Halted => break,
//!         ExitReason::InstructionLimit => continue,
//!         _ => break,
//!     }
//! }
//! ```

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use dynlink::{DlHandle, dl_open, dl_sym};

// ══════════════════════════════════════════════════════════════════════
//  Exit reason and CPU mode enums
// ══════════════════════════════════════════════════════════════════════

/// Reason the VM stopped executing.
///
/// These values match the `u32` codes returned by the `corevm_run` C ABI
/// function in libcorevm.so:
/// - 0 = Halted
/// - 1 = Exception
/// - 2 = InstructionLimit
/// - 3 = Breakpoint
/// - 4 = StopRequested
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ExitReason {
    /// The guest executed a HLT instruction.
    Halted = 0,
    /// An unrecoverable CPU exception occurred.
    Exception = 1,
    /// The maximum instruction count was reached.
    InstructionLimit = 2,
    /// A breakpoint (INT 3) was hit.
    Breakpoint = 3,
    /// An external stop was requested via [`VmHandle::request_stop`].
    StopRequested = 4,
}

impl ExitReason {
    /// Convert a raw `u32` exit code from the C ABI into an `ExitReason`.
    ///
    /// Returns `ExitReason::Exception` for any unrecognized value as a
    /// safe fallback.
    pub fn from_u32(val: u32) -> Self {
        match val {
            0 => ExitReason::Halted,
            1 => ExitReason::Exception,
            2 => ExitReason::InstructionLimit,
            3 => ExitReason::Breakpoint,
            4 => ExitReason::StopRequested,
            _ => ExitReason::Exception,
        }
    }
}

/// CPU execution mode.
///
/// These values match the `u32` codes returned by the `corevm_get_mode` C ABI
/// function in libcorevm.so:
/// - 0 = RealMode (16-bit)
/// - 1 = ProtectedMode (32-bit)
/// - 2 = LongMode (64-bit)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum CpuMode {
    /// 16-bit real mode.
    RealMode = 0,
    /// 32-bit protected mode.
    ProtectedMode = 1,
    /// 64-bit long mode.
    LongMode = 2,
}

impl CpuMode {
    /// Convert a raw `u32` mode code from the C ABI into a `CpuMode`.
    ///
    /// Returns `CpuMode::RealMode` for any unrecognized value as a
    /// safe fallback (real mode is the power-on default).
    pub fn from_u32(val: u32) -> Self {
        match val {
            0 => CpuMode::RealMode,
            1 => CpuMode::ProtectedMode,
            2 => CpuMode::LongMode,
            _ => CpuMode::RealMode,
        }
    }
}

// ══════════════════════════════════════════════════════════════════════
//  Internal: cached function pointers from libcorevm.so
// ══════════════════════════════════════════════════════════════════════

/// Holds all resolved function pointers from libcorevm.so.
///
/// Each field corresponds to a `#[no_mangle] pub extern "C"` function
/// exported by the shared library. The DlHandle is kept alive to
/// prevent the library from being unloaded.
struct CoreVmLib {
    /// Retained handle so the .so stays mapped.
    _handle: DlHandle,

    // ── VM lifecycle ─────────────────────────────────────────────
    /// Create a new VM with `ram_size_mb` megabytes of guest RAM.
    /// Returns an opaque handle (0 on failure).
    create: extern "C" fn(u32) -> u64,
    /// Destroy a VM and free all associated resources.
    destroy: extern "C" fn(u64),
    /// Reset the VM to power-on state (preserves RAM content and I/O
    /// handlers, resets CPU and MMU).
    reset: extern "C" fn(u64),
    /// Execute up to `max_instructions` guest instructions.
    /// Returns an `ExitReason` as a `u32`.
    run: extern "C" fn(u64, u64) -> u32,
    /// Request the VM to stop at the next instruction boundary.
    request_stop: extern "C" fn(u64),

    // ── CPU state: instruction pointer ───────────────────────────
    /// Get the current instruction pointer (RIP/EIP/IP).
    get_rip: extern "C" fn(u64) -> u64,
    /// Set the instruction pointer.
    set_rip: extern "C" fn(u64, u64),

    // ── CPU state: general-purpose registers ─────────────────────
    /// Read a 64-bit GPR by index (0=RAX..15=R15).
    get_gpr: extern "C" fn(u64, u8) -> u64,
    /// Write a 64-bit GPR by index.
    set_gpr: extern "C" fn(u64, u8, u64),

    // ── CPU state: flags ─────────────────────────────────────────
    /// Get RFLAGS.
    get_rflags: extern "C" fn(u64) -> u64,
    /// Set RFLAGS.
    set_rflags: extern "C" fn(u64, u64),

    // ── CPU state: control registers ─────────────────────────────
    /// Read a control register (n = 0, 2, 3, 4, or 8).
    get_cr: extern "C" fn(u64, u8) -> u64,
    /// Write a control register.
    set_cr: extern "C" fn(u64, u8, u64),

    // ── CPU state: mode and privilege ────────────────────────────
    /// Get the current CPU mode as a `u32` (`CpuMode` discriminant).
    get_mode: extern "C" fn(u64) -> u32,
    /// Get the current privilege level (0-3).
    get_cpl: extern "C" fn(u64) -> u8,
    /// Get the total number of instructions executed since last reset.
    get_instruction_count: extern "C" fn(u64) -> u64,

    // ── Memory access ────────────────────────────────────────────
    /// Load raw binary data at a guest physical address.
    /// `data_ptr` / `data_len` define the source buffer.
    /// Returns 1 on success, 0 on failure.
    load_binary: extern "C" fn(u64, u64, *const u8, u32) -> u32,
    /// Read a byte from guest physical memory.
    read_phys_u8: extern "C" fn(u64, u64) -> u8,
    /// Read a 16-bit value from guest physical memory (little-endian).
    read_phys_u16: extern "C" fn(u64, u64) -> u16,
    /// Read a 32-bit value from guest physical memory (little-endian).
    read_phys_u32: extern "C" fn(u64, u64) -> u32,
    /// Write a byte to guest physical memory.
    write_phys_u8: extern "C" fn(u64, u64, u8),
    /// Write a 16-bit value to guest physical memory (little-endian).
    write_phys_u16: extern "C" fn(u64, u64, u16),
    /// Write a 32-bit value to guest physical memory (little-endian).
    write_phys_u32: extern "C" fn(u64, u64, u32),

    // ── Device setup ─────────────────────────────────────────────
    /// Register all standard devices (PIC, PIT, PS/2, CMOS, serial, VGA).
    setup_standard_devices: extern "C" fn(u64),
    /// Register a PCI bus device.
    setup_pci_bus: extern "C" fn(u64),
    /// Register an E1000 NIC with the given MMIO base and MAC address.
    /// `mac_ptr` points to a 6-byte MAC address array.
    setup_e1000: extern "C" fn(u64, u64, *const u8),

    // ── PS/2 keyboard and mouse input ────────────────────────────
    /// Inject a keyboard key press (scancode).
    ps2_key_press: extern "C" fn(u64, u8),
    /// Inject a keyboard key release (scancode).
    ps2_key_release: extern "C" fn(u64, u8),
    /// Inject a mouse movement packet.
    ps2_mouse_move: extern "C" fn(u64, i16, i16, u8),

    // ── VGA framebuffer access ───────────────────────────────────
    /// Get a pointer to the VGA framebuffer pixels.
    /// On success, writes width/height/bpp to the out-pointers and
    /// returns a pointer to the pixel data. Returns null on failure.
    vga_get_framebuffer: extern "C" fn(u64, *mut u32, *mut u32, *mut u8) -> *const u8,
    /// Get a pointer to the VGA text buffer (80x25 u16 cells).
    /// Returns null if VGA is not in text mode.
    vga_get_text_buffer: extern "C" fn(u64, *mut u32) -> *const u16,

    // ── Serial port ──────────────────────────────────────────────
    /// Send input bytes to the guest serial port (COM1).
    serial_send_input: extern "C" fn(u64, *const u8, u32),
    /// Read output bytes from the guest serial port (COM1).
    /// Copies up to `buf_len` bytes into `buf_ptr`.
    /// Returns the number of bytes actually written.
    serial_take_output: extern "C" fn(u64, *mut u8, u32) -> u32,

    // ── E1000 network ────────────────────────────────────────────
    /// Deliver a network packet to the guest E1000 NIC.
    e1000_receive_packet: extern "C" fn(u64, *const u8, u32),
    /// Take transmitted packets from the guest E1000 NIC.
    /// Copies serialized packet data into `buf_ptr` (up to `buf_len` bytes).
    /// Returns the number of bytes written.
    e1000_take_tx_packets: extern "C" fn(u64, *mut u8, u32) -> u32,

    // ── PIT timer ────────────────────────────────────────────────
    /// Advance the PIT by one tick.
    /// Returns 1 if channel 0 fired (IRQ 0 should be raised), 0 otherwise.
    pit_tick: extern "C" fn(u64) -> u32,

    // ── PIC interrupt controller ─────────────────────────────────
    /// Assert an IRQ line on the PIC (0-15).
    pic_raise_irq: extern "C" fn(u64, u8),
    /// Get the next pending interrupt vector from the PIC.
    /// Returns the vector number, or 0xFFFF if no interrupt is pending.
    pic_get_interrupt: extern "C" fn(u64) -> u32,

    // ── IDE/ATA disk controller ─────────────────────────────────
    /// Register an IDE controller on the primary channel.
    setup_ide: extern "C" fn(u64),
    /// Attach a disk image (raw bytes) to the IDE controller.
    ide_attach_disk: extern "C" fn(u64, *const u8, u32),
    /// Detach the disk image from the IDE controller.
    ide_detach_disk: extern "C" fn(u64),
    /// Check if the IDE controller has a pending IRQ (1=yes, 0=no).
    ide_irq_raised: extern "C" fn(u64) -> u32,
    /// Clear the pending IDE IRQ.
    ide_clear_irq: extern "C" fn(u64),
}

/// Singleton holding the loaded library.
static mut LIB: Option<CoreVmLib> = None;

/// Get a reference to the loaded library, panicking if not initialized.
fn lib() -> &'static CoreVmLib {
    unsafe { LIB.as_ref().expect("libcorevm not loaded -- call init() first") }
}

/// Resolve a function pointer from the loaded library, or panic.
///
/// # Safety
///
/// The caller must ensure that the symbol name matches the actual function
/// signature type `T`. The transmute is unchecked.
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
/// `true` on success, `false` if the shared library could not be loaded
/// (e.g., the file is missing or symbol resolution failed).
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
            run: resolve(&handle, "corevm_run"),
            request_stop: resolve(&handle, "corevm_request_stop"),
            // CPU state: instruction pointer
            get_rip: resolve(&handle, "corevm_get_rip"),
            set_rip: resolve(&handle, "corevm_set_rip"),
            // CPU state: GPR
            get_gpr: resolve(&handle, "corevm_get_gpr"),
            set_gpr: resolve(&handle, "corevm_set_gpr"),
            // CPU state: flags
            get_rflags: resolve(&handle, "corevm_get_rflags"),
            set_rflags: resolve(&handle, "corevm_set_rflags"),
            // CPU state: control registers
            get_cr: resolve(&handle, "corevm_get_cr"),
            set_cr: resolve(&handle, "corevm_set_cr"),
            // CPU state: mode and privilege
            get_mode: resolve(&handle, "corevm_get_mode"),
            get_cpl: resolve(&handle, "corevm_get_cpl"),
            get_instruction_count: resolve(&handle, "corevm_get_instruction_count"),
            // Memory
            load_binary: resolve(&handle, "corevm_load_binary"),
            read_phys_u8: resolve(&handle, "corevm_read_phys_u8"),
            read_phys_u16: resolve(&handle, "corevm_read_phys_u16"),
            read_phys_u32: resolve(&handle, "corevm_read_phys_u32"),
            write_phys_u8: resolve(&handle, "corevm_write_phys_u8"),
            write_phys_u16: resolve(&handle, "corevm_write_phys_u16"),
            write_phys_u32: resolve(&handle, "corevm_write_phys_u32"),
            // Device setup
            setup_standard_devices: resolve(&handle, "corevm_setup_standard_devices"),
            setup_pci_bus: resolve(&handle, "corevm_setup_pci_bus"),
            setup_e1000: resolve(&handle, "corevm_setup_e1000"),
            // PS/2
            ps2_key_press: resolve(&handle, "corevm_ps2_key_press"),
            ps2_key_release: resolve(&handle, "corevm_ps2_key_release"),
            ps2_mouse_move: resolve(&handle, "corevm_ps2_mouse_move"),
            // VGA
            vga_get_framebuffer: resolve(&handle, "corevm_vga_get_framebuffer"),
            vga_get_text_buffer: resolve(&handle, "corevm_vga_get_text_buffer"),
            // Serial
            serial_send_input: resolve(&handle, "corevm_serial_send_input"),
            serial_take_output: resolve(&handle, "corevm_serial_take_output"),
            // E1000
            e1000_receive_packet: resolve(&handle, "corevm_e1000_receive_packet"),
            e1000_take_tx_packets: resolve(&handle, "corevm_e1000_take_tx_packets"),
            // PIT
            pit_tick: resolve(&handle, "corevm_pit_tick"),
            // PIC
            pic_raise_irq: resolve(&handle, "corevm_pic_raise_irq"),
            pic_get_interrupt: resolve(&handle, "corevm_pic_get_interrupt"),
            // IDE
            setup_ide: resolve(&handle, "corevm_setup_ide"),
            ide_attach_disk: resolve(&handle, "corevm_ide_attach_disk"),
            ide_detach_disk: resolve(&handle, "corevm_ide_detach_disk"),
            ide_irq_raised: resolve(&handle, "corevm_ide_irq_raised"),
            ide_clear_irq: resolve(&handle, "corevm_ide_clear_irq"),
            // Handle
            _handle: handle,
        };
        LIB = Some(corevm);
    }

    true
}

// ══════════════════════════════════════════════════════════════════════
//  VmHandle: high-level RAII wrapper
// ══════════════════════════════════════════════════════════════════════

/// An active virtual machine instance.
///
/// `VmHandle` wraps an opaque `u64` handle returned by `corevm_create`
/// and provides typed methods for all VM operations. The VM is
/// automatically destroyed when the handle is dropped.
///
/// # Examples
///
/// ```rust
/// let vm = VmHandle::new(16).unwrap(); // 16 MiB guest RAM
/// vm.set_rip(0x7C00);
/// vm.load_binary(0x7C00, &boot_sector);
/// let reason = vm.run(0); // run until exit
/// ```
pub struct VmHandle {
    /// Opaque handle identifying this VM instance in libcorevm.so.
    handle: u64,
}

impl VmHandle {
    // ── Lifecycle ────────────────────────────────────────────────

    /// Create a new virtual machine with the specified guest RAM size.
    ///
    /// # Arguments
    ///
    /// * `ram_size_mb` - Guest RAM size in megabytes. Minimum 1 MiB.
    ///
    /// # Returns
    ///
    /// `Some(VmHandle)` on success, `None` if allocation failed (e.g.,
    /// out of memory for the requested RAM size).
    pub fn new(ram_size_mb: u32) -> Option<Self> {
        let h = (lib().create)(ram_size_mb);
        if h == 0 {
            None
        } else {
            Some(VmHandle { handle: h })
        }
    }

    /// Reset the VM to its power-on state.
    ///
    /// CPU registers, MMU, and interrupt controller are reset. Guest RAM
    /// content and registered I/O handlers are preserved.
    pub fn reset(&self) {
        (lib().reset)(self.handle);
    }

    /// Execute guest instructions.
    ///
    /// Runs the CPU fetch-decode-execute loop for up to `max_instructions`
    /// guest instructions. Pass 0 for unlimited execution (the VM will
    /// run until it halts, hits a breakpoint, encounters an unrecoverable
    /// exception, or an external stop is requested).
    ///
    /// # Returns
    ///
    /// The reason the VM stopped executing.
    pub fn run(&self, max_instructions: u64) -> ExitReason {
        let code = (lib().run)(self.handle, max_instructions);
        ExitReason::from_u32(code)
    }

    /// Request the VM to stop at the next instruction boundary.
    ///
    /// This is safe to call from another thread or a signal handler.
    /// The next call to [`run`](Self::run) will return
    /// [`ExitReason::StopRequested`] promptly.
    pub fn request_stop(&self) {
        (lib().request_stop)(self.handle);
    }

    // ── CPU state: instruction pointer ──────────────────────────

    /// Get the current instruction pointer (RIP in long mode, EIP in
    /// protected mode, IP in real mode).
    pub fn rip(&self) -> u64 {
        (lib().get_rip)(self.handle)
    }

    /// Set the instruction pointer.
    pub fn set_rip(&self, rip: u64) {
        (lib().set_rip)(self.handle, rip);
    }

    // ── CPU state: general-purpose registers ─────────────────────

    /// Read a 64-bit general-purpose register by index.
    ///
    /// Register indices follow the standard x86 encoding:
    /// 0=RAX, 1=RCX, 2=RDX, 3=RBX, 4=RSP, 5=RBP, 6=RSI, 7=RDI,
    /// 8=R8 .. 15=R15.
    ///
    /// # Panics
    ///
    /// The behavior is undefined if `index > 15`.
    pub fn gpr(&self, index: u8) -> u64 {
        (lib().get_gpr)(self.handle, index)
    }

    /// Write a 64-bit general-purpose register by index.
    ///
    /// See [`gpr`](Self::gpr) for the index mapping.
    pub fn set_gpr(&self, index: u8, val: u64) {
        (lib().set_gpr)(self.handle, index, val);
    }

    // ── CPU state: flags ─────────────────────────────────────────

    /// Get the RFLAGS register.
    pub fn rflags(&self) -> u64 {
        (lib().get_rflags)(self.handle)
    }

    /// Set the RFLAGS register.
    pub fn set_rflags(&self, val: u64) {
        (lib().set_rflags)(self.handle, val);
    }

    // ── CPU state: control registers ─────────────────────────────

    /// Read a control register.
    ///
    /// Valid indices are 0 (CR0), 2 (CR2), 3 (CR3), 4 (CR4), and 8 (CR8).
    /// Other indices return 0.
    pub fn cr(&self, n: u8) -> u64 {
        (lib().get_cr)(self.handle, n)
    }

    /// Write a control register.
    ///
    /// Valid indices are 0 (CR0), 2 (CR2), 3 (CR3), 4 (CR4), and 8 (CR8).
    /// Writes to other indices are silently ignored.
    pub fn set_cr(&self, n: u8, val: u64) {
        (lib().set_cr)(self.handle, n, val);
    }

    // ── CPU state: mode and privilege ────────────────────────────

    /// Get the current CPU execution mode.
    pub fn mode(&self) -> CpuMode {
        let code = (lib().get_mode)(self.handle);
        CpuMode::from_u32(code)
    }

    /// Get the current privilege level (0=kernel, 3=user).
    pub fn cpl(&self) -> u8 {
        (lib().get_cpl)(self.handle)
    }

    /// Get the total number of instructions executed since the last
    /// reset.
    pub fn instruction_count(&self) -> u64 {
        (lib().get_instruction_count)(self.handle)
    }

    // ── Memory access ────────────────────────────────────────────

    /// Load raw binary data into guest physical memory.
    ///
    /// Copies `data` into guest RAM starting at guest physical address
    /// `addr`. This bypasses the MMU and writes directly to the flat
    /// memory backing store, making it suitable for loading BIOS ROMs,
    /// kernels, and initial data before the VM starts.
    ///
    /// # Returns
    ///
    /// `true` on success, `false` if the address range is out of bounds.
    pub fn load_binary(&self, addr: u64, data: &[u8]) -> bool {
        (lib().load_binary)(self.handle, addr, data.as_ptr(), data.len() as u32) != 0
    }

    /// Read a byte from guest physical memory.
    pub fn read_phys_u8(&self, addr: u64) -> u8 {
        (lib().read_phys_u8)(self.handle, addr)
    }

    /// Read a 16-bit little-endian value from guest physical memory.
    pub fn read_phys_u16(&self, addr: u64) -> u16 {
        (lib().read_phys_u16)(self.handle, addr)
    }

    /// Read a 32-bit little-endian value from guest physical memory.
    pub fn read_phys_u32(&self, addr: u64) -> u32 {
        (lib().read_phys_u32)(self.handle, addr)
    }

    /// Write a byte to guest physical memory.
    pub fn write_phys_u8(&self, addr: u64, val: u8) {
        (lib().write_phys_u8)(self.handle, addr, val);
    }

    /// Write a 16-bit little-endian value to guest physical memory.
    pub fn write_phys_u16(&self, addr: u64, val: u16) {
        (lib().write_phys_u16)(self.handle, addr, val);
    }

    /// Write a 32-bit little-endian value to guest physical memory.
    pub fn write_phys_u32(&self, addr: u64, val: u32) {
        (lib().write_phys_u32)(self.handle, addr, val);
    }

    // ── Device setup ─────────────────────────────────────────────

    /// Register all standard hardware devices.
    ///
    /// This sets up the full complement of PC-compatible devices:
    /// - Intel 8259A dual PIC (IRQ 0-15)
    /// - Intel 8254 PIT (system timer on IRQ 0)
    /// - PS/2 controller (keyboard + mouse)
    /// - CMOS RTC
    /// - 16550 UART serial port (COM1)
    /// - VGA/SVGA framebuffer
    pub fn setup_standard_devices(&self) {
        (lib().setup_standard_devices)(self.handle);
    }

    /// Register a PCI configuration space bus.
    ///
    /// Must be called before [`setup_e1000`](Self::setup_e1000) to provide
    /// the PCI bus infrastructure that PCI devices attach to.
    pub fn setup_pci_bus(&self) {
        (lib().setup_pci_bus)(self.handle);
    }

    /// Register an Intel E1000 network interface card.
    ///
    /// # Arguments
    ///
    /// * `mmio_base` - MMIO base address for the E1000 register space (128 KB)
    /// * `mac` - 6-byte MAC address for the virtual NIC
    pub fn setup_e1000(&self, mmio_base: u64, mac: &[u8; 6]) {
        (lib().setup_e1000)(self.handle, mmio_base, mac.as_ptr());
    }

    // ── PS/2 keyboard and mouse ──────────────────────────────────

    /// Inject a keyboard key press event.
    ///
    /// The `scancode` is in the format matching the currently active
    /// scancode set (default: set 2).
    pub fn ps2_key_press(&self, scancode: u8) {
        (lib().ps2_key_press)(self.handle, scancode);
    }

    /// Inject a keyboard key release event.
    ///
    /// For scancode set 2, the controller automatically generates the
    /// `0xF0` break prefix. For set 1, it generates `scancode | 0x80`.
    pub fn ps2_key_release(&self, scancode: u8) {
        (lib().ps2_key_release)(self.handle, scancode);
    }

    /// Inject a mouse movement packet.
    ///
    /// # Arguments
    ///
    /// * `dx` - Horizontal displacement (-256..255)
    /// * `dy` - Vertical displacement (-256..255)
    /// * `buttons` - Button state (bit 0=left, bit 1=right, bit 2=middle)
    pub fn ps2_mouse_move(&self, dx: i16, dy: i16, buttons: u8) {
        (lib().ps2_mouse_move)(self.handle, dx, dy, buttons);
    }

    // ── VGA display ──────────────────────────────────────────────

    /// Get a read-only view of the VGA framebuffer.
    ///
    /// Returns `Some((pixels, width, height, bpp))` if the VGA adapter is
    /// in a graphics mode, or `None` if no framebuffer is available (e.g.,
    /// text mode, or VGA not initialized).
    ///
    /// The returned slice is a direct reference into the guest's VGA
    /// framebuffer memory and is only valid until the next mutable VM
    /// operation.
    pub fn vga_framebuffer(&self) -> Option<(&[u8], u32, u32, u8)> {
        let mut width: u32 = 0;
        let mut height: u32 = 0;
        let mut bpp: u8 = 0;
        let ptr = (lib().vga_get_framebuffer)(
            self.handle,
            &mut width as *mut u32,
            &mut height as *mut u32,
            &mut bpp as *mut u8,
        );
        if ptr.is_null() || width == 0 || height == 0 {
            return None;
        }
        let bytes_per_pixel = ((bpp as usize) + 7) / 8;
        let len = (width as usize) * (height as usize) * bytes_per_pixel;
        let slice = unsafe { core::slice::from_raw_parts(ptr, len) };
        Some((slice, width, height, bpp))
    }

    /// Get a read-only view of the VGA text mode buffer.
    ///
    /// Returns `Some(cells)` if the VGA adapter is in 80x25 text mode,
    /// where each `u16` cell is `(attribute << 8) | character`. Returns
    /// `None` if VGA is not in text mode.
    pub fn vga_text_buffer(&self) -> Option<&[u16]> {
        let mut count: u32 = 0;
        let ptr = (lib().vga_get_text_buffer)(self.handle, &mut count as *mut u32);
        if ptr.is_null() || count == 0 {
            return None;
        }
        let slice = unsafe { core::slice::from_raw_parts(ptr, count as usize) };
        Some(slice)
    }

    // ── Serial port (COM1) ───────────────────────────────────────

    /// Send input to the guest serial port.
    ///
    /// The bytes become available for the guest to read from the Receive
    /// Buffer Register (port 0x3F8 with DLAB=0).
    pub fn serial_send_input(&self, data: &[u8]) {
        (lib().serial_send_input)(self.handle, data.as_ptr(), data.len() as u32);
    }

    /// Read output bytes produced by the guest serial port.
    ///
    /// Drains up to `buf.len()` bytes from the serial output buffer into
    /// `buf`. Returns the number of bytes actually read (may be 0 if no
    /// output is available).
    pub fn serial_take_output(&self, buf: &mut [u8]) -> usize {
        let n = (lib().serial_take_output)(self.handle, buf.as_mut_ptr(), buf.len() as u32);
        n as usize
    }

    /// Convenience method: drain all serial output into a new `Vec<u8>`.
    ///
    /// Allocates a temporary buffer and returns only the bytes that were
    /// actually produced. Returns an empty vector if no output is
    /// available.
    pub fn serial_take_output_vec(&self) -> Vec<u8> {
        let mut buf = [0u8; 4096];
        let n = self.serial_take_output(&mut buf);
        let mut v = Vec::with_capacity(n);
        v.extend_from_slice(&buf[..n]);
        v
    }

    // ── E1000 network ────────────────────────────────────────────

    /// Deliver a network packet to the guest E1000 NIC.
    ///
    /// The packet should be a complete Ethernet frame (destination MAC,
    /// source MAC, EtherType, payload). The E1000 will set the RX
    /// interrupt cause bit; call [`pit_tick`](Self::pit_tick) or
    /// [`pic_raise_irq`](Self::pic_raise_irq) to deliver the interrupt.
    pub fn e1000_receive_packet(&self, data: &[u8]) {
        (lib().e1000_receive_packet)(self.handle, data.as_ptr(), data.len() as u32);
    }

    /// Take transmitted packets from the guest E1000 NIC.
    ///
    /// Copies serialized packet data into `buf`. Returns the number of
    /// bytes written. The format is implementation-defined; typically
    /// each packet is length-prefixed.
    pub fn e1000_take_tx_packets(&self, buf: &mut [u8]) -> usize {
        let n = (lib().e1000_take_tx_packets)(self.handle, buf.as_mut_ptr(), buf.len() as u32);
        n as usize
    }

    // ── PIT timer ────────────────────────────────────────────────

    /// Advance the Programmable Interval Timer by one tick.
    ///
    /// Returns `true` if PIT channel 0 fired, indicating that IRQ 0
    /// should be raised on the PIC (call
    /// [`pic_raise_irq(0)`](Self::pic_raise_irq) to deliver it).
    pub fn pit_tick(&self) -> bool {
        (lib().pit_tick)(self.handle) != 0
    }

    // ── PIC interrupt controller ─────────────────────────────────

    /// Assert an IRQ line on the PIC.
    ///
    /// IRQ 0-7 are routed to the master PIC, IRQ 8-15 to the slave.
    /// The interrupt will be delivered to the CPU when the guest has
    /// interrupts enabled (IF=1) and the IRQ is not masked.
    pub fn pic_raise_irq(&self, irq: u8) {
        (lib().pic_raise_irq)(self.handle, irq);
    }

    /// Get the next pending interrupt vector from the PIC.
    ///
    /// Returns `Some(vector)` if an unmasked interrupt is pending, or
    /// `None` if all interrupts are masked or no IRQs are asserted.
    pub fn pic_get_interrupt(&self) -> Option<u8> {
        let val = (lib().pic_get_interrupt)(self.handle);
        if val == 0xFFFF {
            None
        } else {
            Some(val as u8)
        }
    }

    // ── IDE/ATA disk controller ───────────────────────────────────

    /// Register an ATA/IDE disk controller on the primary channel.
    ///
    /// Sets up I/O handlers at ports 0x1F0-0x1F7 (command block) and
    /// 0x3F6-0x3F7 (control block). The controller supports PIO data
    /// transfers used by BIOS INT 13h and early Linux boot.
    pub fn setup_ide(&self) {
        (lib().setup_ide)(self.handle);
    }

    /// Attach a disk image to the IDE controller.
    ///
    /// The raw disk image bytes are copied into the VM. The caller retains
    /// ownership of the source data. Must be called after
    /// [`setup_ide`](Self::setup_ide).
    pub fn ide_attach_disk(&self, data: &[u8]) {
        (lib().ide_attach_disk)(self.handle, data.as_ptr(), data.len() as u32);
    }

    /// Detach the disk image from the IDE controller.
    ///
    /// Frees the in-VM copy of the disk image.
    pub fn ide_detach_disk(&self) {
        (lib().ide_detach_disk)(self.handle);
    }

    /// Check whether the IDE controller has a pending IRQ (IRQ 14).
    ///
    /// Returns `true` if an IRQ is pending and should be raised on the
    /// PIC via [`pic_raise_irq(14)`](Self::pic_raise_irq).
    pub fn ide_irq_raised(&self) -> bool {
        (lib().ide_irq_raised)(self.handle) != 0
    }

    /// Clear the pending IDE IRQ.
    pub fn ide_clear_irq(&self) {
        (lib().ide_clear_irq)(self.handle);
    }
}

impl Drop for VmHandle {
    /// Destroy the VM and free all associated resources.
    fn drop(&mut self) {
        (lib().destroy)(self.handle);
    }
}

// ══════════════════════════════════════════════════════════════════════
//  GPR index constants (convenience)
// ══════════════════════════════════════════════════════════════════════

/// General-purpose register index: RAX.
pub const GPR_RAX: u8 = 0;
/// General-purpose register index: RCX.
pub const GPR_RCX: u8 = 1;
/// General-purpose register index: RDX.
pub const GPR_RDX: u8 = 2;
/// General-purpose register index: RBX.
pub const GPR_RBX: u8 = 3;
/// General-purpose register index: RSP.
pub const GPR_RSP: u8 = 4;
/// General-purpose register index: RBP.
pub const GPR_RBP: u8 = 5;
/// General-purpose register index: RSI.
pub const GPR_RSI: u8 = 6;
/// General-purpose register index: RDI.
pub const GPR_RDI: u8 = 7;
/// General-purpose register index: R8.
pub const GPR_R8: u8 = 8;
/// General-purpose register index: R9.
pub const GPR_R9: u8 = 9;
/// General-purpose register index: R10.
pub const GPR_R10: u8 = 10;
/// General-purpose register index: R11.
pub const GPR_R11: u8 = 11;
/// General-purpose register index: R12.
pub const GPR_R12: u8 = 12;
/// General-purpose register index: R13.
pub const GPR_R13: u8 = 13;
/// General-purpose register index: R14.
pub const GPR_R14: u8 = 14;
/// General-purpose register index: R15.
pub const GPR_R15: u8 = 15;
