//! Top-level VM struct that ties backend, memory, I/O dispatch, and devices together.

use alloc::boxed::Box;
use crate::backend::{VmBackend, VmExitReason, VmError};
use crate::backend::types::*;
use crate::io::IoDispatch;
use crate::memory::GuestMemory;

use crate::devices::serial::Serial;
use crate::devices::ps2::Ps2Controller;
use crate::devices::svga::Svga;
use crate::devices::ahci::Ahci;

/// The virtual machine instance.
///
/// Owns the hardware backend, guest memory, and I/O dispatch tables.
/// Device models are registered into `io` (port I/O) and `memory` (MMIO)
/// during setup. Typed device pointers are retained for device-specific
/// FFI functions that need direct access (e.g., serial input/output,
/// PS/2 key injection, VGA framebuffer).
pub struct Vm {
    #[cfg(feature = "linux")]
    backend: crate::backend::kvm::KvmBackend,
    #[cfg(feature = "anyos")]
    backend: crate::backend::anyos::AnyOsBackend,
    #[cfg(feature = "windows")]
    backend: crate::backend::whp::WhpBackend,

    pub memory: GuestMemory,
    pub io: IoDispatch,

    // Typed device pointers (set during setup_standard_devices / setup_ahci).
    // These point into the Box<dyn IoHandler/MmioHandler> owned by IoDispatch
    // or GuestMemory. Valid for the lifetime of the Vm.
    pub serial_ptr: *mut Serial,
    pub ps2_ptr: *mut Ps2Controller,
    pub svga_ptr: *mut Svga,
    pub ahci_ptr: *mut Ahci,
    pub pit_ptr: *mut crate::devices::pit::Pit,
    pub pic_ptr: *mut crate::devices::pic::PicPair,
    pub debug_port_ptr: *mut crate::devices::debug_port::DebugPort,
    pub pci_bus_ptr: *mut crate::devices::bus::PciBus,
    pub fw_cfg_ptr: *mut crate::devices::fw_cfg::FwCfg,
}

impl Vm {
    /// Create a new VM with `ram_size_mb` megabytes of guest RAM.
    pub fn new(ram_size_mb: u32) -> Result<Self, VmError> {
        let ram_bytes = (ram_size_mb as usize) * 1024 * 1024;

        #[cfg(feature = "linux")]
        let mut backend = crate::backend::kvm::KvmBackend::new()?;
        #[cfg(feature = "anyos")]
        let mut backend = crate::backend::anyos::AnyOsBackend::new(ram_bytes)?;
        #[cfg(feature = "windows")]
        let mut backend = crate::backend::whp::WhpBackend::new(ram_bytes)?;

        let mut memory = GuestMemory::new(ram_bytes);
        let io = IoDispatch::new();

        // Register guest RAM with the backend.
        let (ptr, size) = memory.ram_mut_ptr();
        backend.set_memory_region(0, 0, size as u64, ptr)?;

        Ok(Self {
            backend,
            memory,
            io,
            serial_ptr: core::ptr::null_mut(),
            ps2_ptr: core::ptr::null_mut(),
            svga_ptr: core::ptr::null_mut(),
            ahci_ptr: core::ptr::null_mut(),
            pit_ptr: core::ptr::null_mut(),
            pic_ptr: core::ptr::null_mut(),
            debug_port_ptr: core::ptr::null_mut(),
            pci_bus_ptr: core::ptr::null_mut(),
            fw_cfg_ptr: core::ptr::null_mut(),
        })
    }

    /// Register all standard chipset devices into I/O and MMIO dispatch.
    pub fn setup_standard_devices(&mut self) {
        use crate::devices::pic::PicPair;
        use crate::devices::pit::Pit;
        use crate::devices::cmos::Cmos;
        use crate::devices::debug_port::DebugPort;
        use crate::devices::acpi::AcpiPm;
        use crate::devices::apm::ApmControl;
        use crate::devices::fw_cfg::FwCfg;
        use crate::devices::bus::{PciBus, PciDevice, PciMmcfgHandler};
        use crate::devices::ioapic::IoApic;
        use crate::devices::lapic::Lapic;
        use crate::devices::port61::Port61;
        use crate::devices::ide::Ide;

        let ram_size = self.memory.ram_size();

        // PIC is registered at the end with a wide range (0x20, count=0x82)
        // covering both master (0x20-0x21) and slave (0xA0-0xA1).
        // All specific devices below are registered first for priority.

        // PIT (0x40-0x43)
        let pit = Box::new(Pit::new());
        let pit_ptr = &*pit as *const Pit as *mut Pit;
        self.io.register(0x40, 4, pit);
        self.pit_ptr = pit_ptr;

        // Port 61 (speaker gate, needs PIT pointer)
        self.io.register(0x61, 1, Box::new(Port61::new(pit_ptr)));

        // CMOS (0x70-0x71)
        self.io.register(0x70, 2, Box::new(Cmos::new(ram_size)));

        // PS/2 controller (0x60 and 0x64 — register as 0x60 count=5)
        let ps2 = Box::new(Ps2Controller::new());
        self.ps2_ptr = &*ps2 as *const Ps2Controller as *mut Ps2Controller;
        self.io.register(0x60, 5, ps2);

        // Serial COM1 (0x3F8-0x3FF)
        let serial = Box::new(Serial::new());
        self.serial_ptr = &*serial as *const Serial as *mut Serial;
        self.io.register(0x3F8, 8, serial);

        // VGA I/O ports (0x3C0-0x3DA) and MMIO framebuffer
        let svga = Box::new(Svga::new(1024, 768));
        self.svga_ptr = &*svga as *const Svga as *mut Svga;
        self.io.register(0x3C0, 0x1B, svga);
        // Bochs VBE ports (0x1CE-0x1CF) — same Svga device, accessed via svga_ptr
        self.io.register(0x1CE, 2, Box::new(SvgaVbePortProxy(self.svga_ptr)));
        // VGA legacy framebuffer MMIO at 0xA0000 (128KB)
        self.memory.add_mmio(0xA0000, 0x20000, Box::new(SvgaMmioProxy(self.svga_ptr)));

        // Debug port (0x402)
        let dbg = Box::new(DebugPort::new());
        self.debug_port_ptr = &*dbg as *const DebugPort as *mut DebugPort;
        self.io.register(0x402, 1, dbg);

        // ACPI PM (0x600-0x607)
        self.io.register(0x600, 8, Box::new(AcpiPm::new()));

        // APM (0xB2-0xB3)
        self.io.register(0xB2, 2, Box::new(ApmControl::new()));

        // fw_cfg (0x510-0x511)
        let fw_cfg = Box::new(FwCfg::new(ram_size as u64));
        self.fw_cfg_ptr = &*fw_cfg as *const FwCfg as *mut FwCfg;
        self.io.register(0x510, 2, fw_cfg);

        // PCI bus (0xCF8-0xCFF)
        // Add standard PCI devices before registering the bus with I/O dispatcher.
        let mut pci_bus = Box::new(PciBus::new());

        // PCI Host Bridge (i440FX) at 00:00.0 — required by SeaBIOS
        {
            let mut host = PciDevice::new(0x8086, 0x1237, 0x06, 0x00, 0x00);
            host.device = 0;
            pci_bus.add_device(host);
        }

        // ISA Bridge (PIIX3) at 00:01.0 — SeaBIOS uses this for PCI IRQ routing
        {
            let mut isa = PciDevice::new(0x8086, 0x7000, 0x06, 0x01, 0x00);
            isa.device = 1;
            // Header type 0x80 = multi-function device (SeaBIOS expects this)
            isa.config_space[0x0E] = 0x80;
            // PIRQ routing registers (offsets 0x60-0x63): map PIRQA-D to IRQs 10,10,11,11
            isa.config_space[0x60] = 10;
            isa.config_space[0x61] = 10;
            isa.config_space[0x62] = 11;
            isa.config_space[0x63] = 11;
            pci_bus.add_device(isa);
        }

        // VGA (QEMU stdvga) at 00:02.0 — SeaBIOS needs a PCI VGA for option ROM
        {
            let mut vga = PciDevice::new(0x1234, 0x1111, 0x03, 0x00, 0x00);
            vga.device = 2;
            // BAR0: linear framebuffer (16MB)
            vga.set_bar(0, 0xFD00_0000, 0x100_0000, true);
            // BAR2: Bochs dispi MMIO registers (4KB)
            vga.set_bar(2, 0xFEBE_0000, 0x1000, true);
            // Expansion ROM BAR (0xC0000 VGA BIOS area)
            vga.config_space[0x30] = 0x00; // ROM base (will be set by SeaBIOS)
            vga.config_space[0x31] = 0x00;
            vga.config_space[0x32] = 0x00;
            vga.config_space[0x33] = 0x00;
            pci_bus.add_device(vga);
        }

        let pci_bus_ptr = &*pci_bus as *const PciBus as *mut PciBus;
        self.pci_bus_ptr = pci_bus_ptr;
        self.io.register(0xCF8, 8, pci_bus);

        // PCI MMCONFIG MMIO (0xB0000000, 256MB)
        self.memory.add_mmio(0xB000_0000, 0x1000_0000, Box::new(PciMmcfgHandler::new(pci_bus_ptr)));

        // IDE (0x1F0-0x1F7, 0x3F6, 0x170-0x177, 0x376)
        self.io.register(0x1F0, 8, Box::new(Ide::new()));

        // I/O APIC MMIO (0xFEC00000, 4KB)
        self.memory.add_mmio(0xFEC0_0000, 0x1000, Box::new(IoApic::new()));

        // Local APIC MMIO (0xFEE00000, 4KB)
        self.memory.add_mmio(0xFEE0_0000, 0x1000, Box::new(Lapic::new()));

        // Now register PIC pair covering both master and slave port ranges.
        // Registered AFTER all other port-I/O devices so it has lowest priority.
        // Ports 0x20-0x21 and 0xA0-0xA1 are the only ones PicPair responds to;
        // all intermediate ports that already have handlers get matched first.
        let pic = Box::new(PicPair::new());
        self.pic_ptr = &*pic as *const PicPair as *mut PicPair;
        self.io.register(0x20, 0x82, pic);
    }

    // ── VmBackend delegations ──

    pub fn create_vcpu(&mut self, id: u32) -> Result<(), VmError> {
        self.backend.create_vcpu(id)
    }

    pub fn destroy_vcpu(&mut self, id: u32) -> Result<(), VmError> {
        self.backend.destroy_vcpu(id)
    }

    pub fn run_vcpu(&mut self, id: u32) -> Result<VmExitReason, VmError> {
        self.backend.run_vcpu(id)
    }

    pub fn get_vcpu_regs(&self, id: u32) -> Result<VcpuRegs, VmError> {
        self.backend.get_vcpu_regs(id)
    }

    pub fn set_vcpu_regs(&mut self, id: u32, regs: &VcpuRegs) -> Result<(), VmError> {
        self.backend.set_vcpu_regs(id, regs)
    }

    /// Store a pending MMIO read response (WHP only).
    #[cfg(feature = "windows")]
    pub fn set_pending_mmio_read(&mut self, value: u64, dest_reg: u8) {
        self.backend.set_pending_mmio_read(value, dest_reg);
    }

    pub fn get_vcpu_sregs(&self, id: u32) -> Result<VcpuSregs, VmError> {
        self.backend.get_vcpu_sregs(id)
    }

    pub fn set_vcpu_sregs(&mut self, id: u32, sregs: &VcpuSregs) -> Result<(), VmError> {
        self.backend.set_vcpu_sregs(id, sregs)
    }

    pub fn inject_interrupt(&mut self, id: u32, vector: u8) -> Result<(), VmError> {
        self.backend.inject_interrupt(id, vector)
    }

    pub fn inject_exception(&mut self, id: u32, vector: u8, error_code: Option<u32>) -> Result<(), VmError> {
        self.backend.inject_exception(id, vector, error_code)
    }

    pub fn inject_nmi(&mut self, id: u32) -> Result<(), VmError> {
        self.backend.inject_nmi(id)
    }

    pub fn request_interrupt_window(&mut self, id: u32, enable: bool) -> Result<(), VmError> {
        self.backend.request_interrupt_window(id, enable)
    }

    pub fn set_cpuid(&mut self, entries: &[CpuidEntry]) -> Result<(), VmError> {
        self.backend.set_cpuid(entries)
    }

    pub fn set_memory_region(&mut self, slot: u32, guest_phys: u64, size: u64, host_ptr: *mut u8) -> Result<(), VmError> {
        self.backend.set_memory_region(slot, guest_phys, size, host_ptr)
    }

    pub fn reset(&mut self) -> Result<(), VmError> {
        self.backend.reset()
    }

    pub fn destroy_backend(&mut self) {
        self.backend.destroy();
    }

    // ── I/O exit dispatch ──

    /// Route a port I/O exit to the registered device handler.
    pub fn handle_io(&mut self, port: u16, is_write: bool, size: u8, data: &mut [u8]) {
        if is_write {
            let val = match size {
                1 => data[0] as u32,
                2 => u16::from_le_bytes([data[0], data[1]]) as u32,
                4 => u32::from_le_bytes([data[0], data[1], data[2], data[3]]),
                _ => 0,
            };
            let _ = self.io.port_out(port, size, val);
        } else {
            let val = self.io.port_in(port, size).unwrap_or(0xFFFF_FFFF);
            let bytes = val.to_le_bytes();
            for i in 0..size as usize {
                if i < data.len() {
                    data[i] = bytes[i];
                }
            }
        }
    }

    /// Handle a bulk string I/O transfer (REP INSB/OUTSB).
    ///
    /// Performs `count` byte-sized port I/O operations, reading from or writing
    /// to guest physical memory starting at `gpa`, advancing by `step` each time.
    /// Updates guest registers (RCX, RDI/RSI, RIP) after completion.
    pub fn handle_string_io(
        &mut self, port: u16, is_write: bool, count: u64, gpa: u64,
        step: i64, instr_len: u64, addr_size: u8,
    ) {
        let mut current_gpa = gpa;
        for _ in 0..count {
            if is_write {
                // OUTSB: read byte from guest memory, write to port
                let byte = self.memory.read_u8(current_gpa).unwrap_or(0);
                let _ = self.io.port_out(port, 1, byte as u32);
            } else {
                // INSB: read byte from port, write to guest memory
                let val = self.io.port_in(port, 1).unwrap_or(0xFF);
                let _ = self.memory.write_u8(current_gpa, val as u8);
            }
            current_gpa = (current_gpa as i64 + step) as u64;
        }

        // Update guest registers
        if let Ok(mut regs) = self.get_vcpu_regs(0) {
            // Update pointer register
            let total_delta = step * count as i64;
            if is_write {
                regs.rsi = match addr_size {
                    2 => { let v = (regs.rsi as u16).wrapping_add(total_delta as u16); (regs.rsi & !0xFFFF) | v as u64 }
                    4 => (regs.rsi as u32).wrapping_add(total_delta as u32) as u64,
                    _ => regs.rsi.wrapping_add(total_delta as u64),
                };
            } else {
                regs.rdi = match addr_size {
                    2 => { let v = (regs.rdi as u16).wrapping_add(total_delta as u16); (regs.rdi & !0xFFFF) | v as u64 }
                    4 => (regs.rdi as u32).wrapping_add(total_delta as u32) as u64,
                    _ => regs.rdi.wrapping_add(total_delta as u64),
                };
            }
            // Update counter
            regs.rcx = match addr_size {
                2 => { let v = (regs.rcx as u16).wrapping_sub(count as u16); (regs.rcx & !0xFFFF) | v as u64 }
                4 => (regs.rcx as u32).wrapping_sub(count as u32) as u64,
                _ => regs.rcx.wrapping_sub(count),
            };
            // Advance RIP
            regs.rip += instr_len;
            let _ = self.set_vcpu_regs(0, &regs);
        }
    }

    /// Route an MMIO exit to the registered device handler.
    pub fn handle_mmio(&mut self, addr: u64, is_write: bool, size: u8, data: &mut [u8]) {
        if is_write {
            let val = match size {
                1 => data[0] as u64,
                2 => u16::from_le_bytes([data[0], data[1]]) as u64,
                4 => u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as u64,
                8 => u64::from_le_bytes([
                    data[0], data[1], data[2], data[3],
                    data[4], data[5], data[6], data[7],
                ]),
                _ => 0,
            };
            self.memory.dispatch_mmio_write(addr, size, val);
        } else {
            let val = self.memory.dispatch_mmio_read(addr, size).unwrap_or(0xFFFF_FFFF_FFFF_FFFF);
            let bytes = val.to_le_bytes();
            for i in 0..size as usize {
                if i < data.len() {
                    data[i] = bytes[i];
                }
            }
        }
    }

    // ── KVM-specific response writing ──

    #[cfg(feature = "linux")]
    pub fn set_io_response(&mut self, vcpu_id: u32, data: &[u8]) {
        self.backend.set_io_response(vcpu_id, data);
    }

    #[cfg(feature = "linux")]
    pub fn set_mmio_response(&mut self, vcpu_id: u32, data: &[u8]) {
        self.backend.set_mmio_response(vcpu_id, data);
    }

    // ── Physical memory access ──

    pub fn read_phys(&self, addr: u64, buf: &mut [u8]) -> Result<(), VmError> {
        self.backend.read_phys(addr, buf)
    }

    pub fn write_phys(&mut self, addr: u64, buf: &[u8]) -> Result<(), VmError> {
        self.backend.write_phys(addr, buf)
    }

    pub fn load_binary(&mut self, guest_phys: u64, data: &[u8]) -> Result<(), VmError> {
        self.backend.write_phys(guest_phys, data)
    }

    // ── Typed device access ──

    /// Get a mutable reference to the serial port, if set up.
    ///
    /// # Safety
    /// The pointer is valid for the lifetime of the Vm (points into a Box
    /// owned by IoDispatch). Single-threaded access only.
    pub fn serial(&mut self) -> Option<&mut Serial> {
        if self.serial_ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.serial_ptr })
        }
    }

    /// Get a mutable reference to the PS/2 controller, if set up.
    pub fn ps2(&mut self) -> Option<&mut Ps2Controller> {
        if self.ps2_ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.ps2_ptr })
        }
    }

    /// Get a reference to the VGA adapter, if set up.
    pub fn svga(&self) -> Option<&Svga> {
        if self.svga_ptr.is_null() {
            None
        } else {
            Some(unsafe { &*self.svga_ptr })
        }
    }

    /// Get a mutable reference to the VGA adapter, if set up.
    pub fn svga_mut(&mut self) -> Option<&mut Svga> {
        if self.svga_ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.svga_ptr })
        }
    }

    /// Get a mutable reference to the AHCI controller, if set up.
    pub fn ahci(&mut self) -> Option<&mut Ahci> {
        if self.ahci_ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.ahci_ptr })
        }
    }

    /// Get a mutable reference to the PIT timer, if set up.
    pub fn pit_mut(&mut self) -> Option<&mut crate::devices::pit::Pit> {
        if self.pit_ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.pit_ptr })
        }
    }
}

// ── Proxy wrappers for VGA device ──
// The Svga instance is owned by IoDispatch (ports 0x3C0-0x3DA).
// These proxies delegate to the same instance via raw pointer for
// additional port ranges and MMIO regions.

/// Proxy for Bochs VBE I/O ports 0x1CE-0x1CF.
struct SvgaVbePortProxy(*mut Svga);
unsafe impl Send for SvgaVbePortProxy {}

impl crate::io::IoHandler for SvgaVbePortProxy {
    fn read(&mut self, port: u16, size: u8) -> crate::error::Result<u32> {
        unsafe { &mut *self.0 }.read(port, size)
    }
    fn write(&mut self, port: u16, size: u8, val: u32) -> crate::error::Result<()> {
        unsafe { &mut *self.0 }.write(port, size, val)
    }
}

/// Proxy for VGA legacy framebuffer MMIO at 0xA0000 (128KB).
struct SvgaMmioProxy(*mut Svga);
unsafe impl Send for SvgaMmioProxy {}

impl crate::memory::mmio::MmioHandler for SvgaMmioProxy {
    fn read(&mut self, offset: u64, size: u8) -> crate::error::Result<u64> {
        unsafe { &mut *self.0 }.read(offset, size)
    }
    fn write(&mut self, offset: u64, size: u8, val: u64) -> crate::error::Result<()> {
        unsafe { &mut *self.0 }.write(offset, size, val)
    }
}
