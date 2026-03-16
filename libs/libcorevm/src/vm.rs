//! Top-level VM struct that ties backend, memory, I/O dispatch, and devices together.

use alloc::boxed::Box;
use crate::backend::{VmBackend, VmExitReason, VmError};
use crate::backend::types::*;
use crate::io::IoDispatch;
use crate::memory::{GuestMemory, MemoryBus};

use crate::devices::serial::Serial;
use crate::devices::ps2::Ps2Controller;
use crate::devices::svga::Svga;
use crate::devices::ahci::Ahci;
use crate::devices::e1000::E1000;
use crate::devices::ac97::Ac97;
use crate::devices::hpet::Hpet;

/// The virtual machine instance.
///
/// Owns the hardware backend, guest memory, and I/O dispatch tables.
/// Device models are registered into `io` (port I/O) and `memory` (MMIO)
/// during setup. Typed device pointers are retained for device-specific
/// FFI functions that need direct access (e.g., serial input/output,
/// PS/2 key injection, VGA framebuffer).
pub struct Vm {
    #[cfg(feature = "linux")]
    pub backend: crate::backend::kvm::KvmBackend,
    #[cfg(feature = "anyos")]
    pub backend: crate::backend::anyos::AnyOsBackend,
    #[cfg(feature = "windows")]
    pub backend: crate::backend::whp::WhpBackend,

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
    pub cmos_ptr: *mut crate::devices::cmos::Cmos,
    pub hpet_ptr: *mut Hpet,
    pub ide_ptr: *mut crate::devices::ide::Ide,
    pub e1000_ptr: *mut E1000,
    pub ac97_ptr: *mut Ac97,
    pub uhci_ptr: *mut crate::devices::uhci::Uhci,

    /// Pointer to the PCI MMIO router (lives inside the MMIO dispatch regions).
    /// Used by `corevm_setup_e1000()` to add E1000 to the router after AHCI setup.
    pub pci_mmio_router_ptr: *mut crate::ffi::PciMmioRouter,
    pub pci_io_router_ptr: *mut crate::ffi::PciIoRouter,

    /// Tracks whether AHCI IRQ 11 is currently asserted on the in-kernel irqchip.
    /// Used for level-triggered interrupt semantics: only call set_irq_line when
    /// the state changes.
    pub ahci_irq_asserted: bool,

    /// Tracks whether E1000 IRQ 11 is currently asserted (level-triggered).
    pub e1000_irq_asserted: bool,

    /// Set when the guest writes to port 0xCF9 requesting a system reset.
    pub cf9_reset_pending: bool,

    /// VRAM size in MiB (0 = default 16). Set before setup_standard_devices().
    pub vram_mb: u32,

    /// Number of CPU cores. Set before setup_acpi_tables().
    pub cpu_count: u32,

    /// Network backend (user-mode NAT, TAP, etc.). Set via corevm_setup_net().
    #[cfg(feature = "std")]
    pub net_backend: Option<alloc::boxed::Box<dyn crate::devices::net::NetBackend>>,

    /// Thread-safe queue for mouse events from external threads (e.g., UI or
    /// injection threads).  The FFI function `corevm_ps2_mouse_move` pushes
    /// events here instead of calling `ps2.mouse_move()` directly, avoiding
    /// data races on the PS/2 controller.  The VM loop drains this queue in
    /// `corevm_poll_irqs` (single-threaded context).
    #[cfg(feature = "std")]
    pub pending_mouse: std::sync::Mutex<alloc::vec::Vec<(i16, i16, u8)>>,
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
        // PCI devices (VGA LFB, BIOS ROM, AHCI BAR, etc.) occupy the region
        // 0xE0000000–0xFFFFFFFF ("PCI hole"). Guest RAM that would fall into
        // this range must be relocated above 4 GB, just like real hardware
        // and QEMU do.
        const PCI_HOLE_START: u64 = 0xE000_0000; // 3.5 GB
        const PCI_HOLE_END: u64   = 0x1_0000_0000; // 4 GB

        let (ptr, total_size) = memory.ram_mut_ptr();
        let total = total_size as u64;

        if total <= PCI_HOLE_START {
            // RAM fits entirely below the PCI hole — single slot.
            backend.set_memory_region(0, 0, total, ptr)?;
        } else {
            // Split: slot 0 = below PCI hole, slot 3 = above 4 GB.
            backend.set_memory_region(0, 0, PCI_HOLE_START, ptr)?;
            let above_4g = total - PCI_HOLE_START;
            let above_ptr = unsafe { ptr.add(PCI_HOLE_START as usize) };
            backend.set_memory_region(3, PCI_HOLE_END, above_4g, above_ptr)?;
        }

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
            cmos_ptr: core::ptr::null_mut(),
            hpet_ptr: core::ptr::null_mut(),
            ide_ptr: core::ptr::null_mut(),
            e1000_ptr: core::ptr::null_mut(),
            ac97_ptr: core::ptr::null_mut(),
            uhci_ptr: core::ptr::null_mut(),
            pci_mmio_router_ptr: core::ptr::null_mut(),
            pci_io_router_ptr: core::ptr::null_mut(),
            ahci_irq_asserted: false,
            e1000_irq_asserted: false,
            cf9_reset_pending: false,
            vram_mb: 0,
            cpu_count: 1,
            #[cfg(feature = "std")]
            net_backend: None,
            #[cfg(feature = "std")]
            pending_mouse: std::sync::Mutex::new(alloc::vec::Vec::new()),
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
        let mut port61 = Port61::new(pit_ptr);
        // On Linux/KVM, sync PIT channel 2 gate to the in-kernel PIT
        // and read channel 2 output from the in-kernel PIT (not the userspace one).
        #[cfg(feature = "linux")]
        {
            port61.set_gate_sync(crate::backend::kvm::kvm_sync_pit_ch2_gate);
            port61.set_pit_output(crate::backend::kvm::kvm_pit_ch2_output);
        }
        self.io.register(0x61, 1, Box::new(port61));

        // CMOS (0x70-0x71)
        let cmos = Box::new(Cmos::new(ram_size));
        let cmos_ptr = &*cmos as *const Cmos as *mut Cmos;
        self.io.register(0x70, 2, cmos);
        self.cmos_ptr = cmos_ptr;

        // PS/2 controller (0x60 and 0x64 — register as 0x60 count=5)
        let ps2 = Box::new(Ps2Controller::new());
        self.ps2_ptr = &*ps2 as *const Ps2Controller as *mut Ps2Controller;
        self.io.register(0x60, 5, ps2);

        // Serial COM1 (0x3F8-0x3FF)
        let serial = Box::new(Serial::new());
        self.serial_ptr = &*serial as *const Serial as *mut Serial;
        self.io.register(0x3F8, 8, serial);

        // VGA I/O ports (0x3C0-0x3DA) and MMIO framebuffer
        let svga = Box::new(Svga::new_with_vram(1024, 768, self.vram_mb));
        self.svga_ptr = &*svga as *const Svga as *mut Svga;
        self.io.register(0x3C0, 0x1B, svga);
        // Bochs VBE ports (0x1CE-0x1CF) — same Svga device, accessed via svga_ptr
        self.io.register(0x1CE, 2, Box::new(SvgaVbePortProxy(self.svga_ptr)));
        // VGA legacy framebuffer MMIO at 0xA0000 (128KB)
        self.memory.add_mmio(0xA0000, 0x20000, Box::new(SvgaMmioProxy(self.svga_ptr)));
        // VGA linear framebuffer MMIO at Bochs VBE default (0xE0000000)
        // and PCI BAR0 (0xFD000000). SeaVGABIOS uses 0xE0000000 for the LFB.
        // For software emulation, these catch guest LFB writes directly.
        // For hardware-virt (WHP/KVM), these are shadowed by RAM mappings.
        let lfb_size = unsafe { &*self.svga_ptr }.vram_size() as u64;
        self.memory.add_mmio(0xE000_0000, lfb_size, Box::new(SvgaLfbProxy(self.svga_ptr)));
        // BAR0 proxy at 0xFD000000: cap at 16 MB to avoid colliding with
        // PCI Expansion ROMs that start at 0xFE000000.
        let bar0_proxy_size = lfb_size.min(0x0100_0000);
        self.memory.add_mmio(0xFD00_0000, bar0_proxy_size, Box::new(SvgaLfbProxy(self.svga_ptr)));
        // Bochs dispi register MMIO at the PCI BAR2 default (0xFEBE0000).
        // SeaBIOS may remap BAR2 — the PCI MMIO router handles that dynamically.
        // Do NOT register 0xFE002000 here — it collides with E1000 BAR0 offsets
        // when SeaBIOS places the E1000 at 0xFE000000.
        self.memory.add_mmio(0xFEBE_0000, 0x1000, Box::new(SvgaDispiMmioProxy(self.svga_ptr)));

        // Port 0x92: Fast A20 Gate + System Reset
        // Required by Windows 10 bootmgr to enable A20 and check system state.
        self.io.register(0x92, 1, Box::new(Port92::new()));

        // Debug port (0x402)
        let dbg = Box::new(DebugPort::new());
        self.debug_port_ptr = &*dbg as *const DebugPort as *mut DebugPort;
        self.io.register(0x402, 1, dbg);

        // ACPI PM at PMBASE 0xB000 (matches FADT PM1a_EVT_BLK)
        self.io.register(0xB000, 0x40, Box::new(AcpiPm::new()));

        // APM (0xB2-0xB3)
        self.io.register(0xB2, 2, Box::new(ApmControl::new()));

        // fw_cfg (0x510-0x51B: selector, data, DMA ports)
        let mut fw_cfg = Box::new(FwCfg::new(ram_size as u64));
        // Give fw_cfg access to guest RAM for DMA operations.
        let (ram_ptr, ram_len) = self.memory.ram_mut_ptr();
        fw_cfg.set_ram(ram_ptr, ram_len);
        self.fw_cfg_ptr = &*fw_cfg as *const FwCfg as *mut FwCfg;
        self.io.register(0x510, 12, fw_cfg);

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
            // PIRQ routing registers (offsets 0x60-0x63): map PIRQA-D to IRQs.
            // PCI IRQ swizzle: PIRQ = (device_slot + pin - 1) % 4
            //   Dev 2 VGA:   INTA → (2+0)%4=2 → PIRQC → IRQ 11
            //   Dev 3 AHCI:  INTA → (3+0)%4=3 → PIRQD → IRQ 11
            //   Dev 4 E1000: INTA → (4+0)%4=0 → PIRQA → IRQ 10
            //   Dev 5 AC97:  INTA → (5+0)%4=1 → PIRQB → IRQ 5
            //   Dev 6 UHCI:  INTD → (6+3)%4=1 → PIRQB → IRQ 5
            isa.config_space[0x60] = 11;  // PIRQA → IRQ 11 (E1000, shared with AHCI)
            isa.config_space[0x61] = 5;   // PIRQB → IRQ 5  (AC97, UHCI)
            isa.config_space[0x62] = 11;  // PIRQC → IRQ 11 (VGA)
            isa.config_space[0x63] = 11;  // PIRQD → IRQ 11 (AHCI)
            pci_bus.add_device(isa);
        }

        // VGA (QEMU stdvga) at 00:02.0 — SeaBIOS needs a PCI VGA for option ROM
        {
            let mut vga = PciDevice::new(0x1234, 0x1111, 0x03, 0x00, 0x00);
            vga.device = 2;
            // BAR0: linear framebuffer (size matches VRAM)
            let bar0_size = unsafe { &*self.svga_ptr }.vram_size() as u32;
            vga.set_bar(0, 0xFD00_0000, bar0_size, true);
            // BAR2: Bochs dispi MMIO registers (4KB)
            vga.set_bar(2, 0xFEBE_0000, 0x1000, true);
            // Interrupt: INTA → PIRQC → IRQ 11 (via PIIX3 swizzle: (2+0)%4=2)
            vga.set_interrupt(11, 1);
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
        let ide = Box::new(Ide::new());
        self.ide_ptr = &*ide as *const Ide as *mut Ide;
        self.io.register(0x1F0, 8, ide);

        // Note: HPET is optional — call setup_hpet() separately if needed
        // (required for Windows guests).

        // I/O APIC and Local APIC MMIO.
        // On Linux/KVM with KVM_CREATE_IRQCHIP, these are handled in-kernel.
        // Registering userspace handlers would intercept accesses and prevent
        // the in-kernel IRQCHIP from working (no timer interrupts, etc.).
        #[cfg(not(feature = "linux"))]
        {
            self.memory.add_mmio(0xFEC0_0000, 0x1000, Box::new(IoApic::new()));
            self.memory.add_mmio(0xFEE0_0000, 0x1000, Box::new(Lapic::new()));
        }

        // Now register PIC pair covering both master and slave port ranges.
        // Registered AFTER all other port-I/O devices so it has lowest priority.
        // Ports 0x20-0x21 and 0xA0-0xA1 are the only ones PicPair responds to;
        // all intermediate ports that already have handlers get matched first.
        let pic = Box::new(PicPair::new());
        self.pic_ptr = &*pic as *const PicPair as *mut PicPair;
        self.io.register(0x20, 0x82, pic);
    }

    /// Map the VGA linear framebuffer (8 MiB at GPA 0xE0000000) as a
    /// hypervisor memory region for fast direct guest access.
    ///
    /// Without this, every guest write to the VGA LFB generates an MMIO
    /// exit which is extremely slow for framebuffer-intensive operations.
    /// With this mapping, guest writes go directly to the SVGA device's
    /// framebuffer buffer and the display update reads from it.
    ///
    /// Must be called AFTER `setup_standard_devices()` (which creates the
    /// SVGA device).
    #[cfg(feature = "linux")]
    pub fn setup_vga_lfb_mapping(&mut self) -> Result<(), VmError> {
        if self.svga_ptr.is_null() {
            return Ok(());
        }
        let svga = unsafe { &mut *self.svga_ptr };
        let fb_ptr = svga.framebuffer_mut_ptr();
        let fb_size = svga.vram_size() as u64;
        // Slot 2: VGA LFB at 0xE0000000 — VBE dispi default address.
        // SeaVGABIOS always uses 0xE0000000 for the LFB, so this is the
        // primary mapping. The PCI BAR0 region (0xFD000000) is NOT mapped
        // as a separate KVM slot because for VRAM > 8 MB it would collide
        // with other PCI device regions above 0xFE000000. The PCI BAR0
        // address is handled by the MMIO fallback proxy if any guest
        // driver tries to use it instead of the VBE address.
        // (Slot 0 = RAM below PCI hole, slot 1 = reserved for BIOS ROM, slot 3 = RAM above 4GB)
        self.backend.set_memory_region(2, 0xE000_0000, fb_size, fb_ptr)?;

        // Slot 4: VGA LFB at PCI BAR0 address (0xFD000000).
        // Linux bochs-drm uses the PCI BAR0 address, not 0xE0000000.
        // Cap at 16 MB to avoid colliding with PCI Expansion ROMs at 0xFE000000.
        let bar0_size = fb_size.min(0x0100_0000); // max 16 MB
        self.backend.set_memory_region(4, 0xFD00_0000, bar0_size, fb_ptr)
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

    /// Handle a bulk string I/O transfer (REP INS/OUTS).
    ///
    /// Performs `count` port I/O operations of `access_size` bytes each,
    /// reading from or writing to guest physical memory starting at `gpa`,
    /// advancing by `step` (±access_size) each time.
    /// Updates guest registers (RCX, RDI/RSI, RIP) after completion.
    pub fn handle_string_io(
        &mut self, port: u16, is_write: bool, count: u64, gpa: u64,
        step: i64, instr_len: u64, addr_size: u8, access_size: u8,
    ) {
        let mut current_gpa = gpa;
        for _ in 0..count {
            if is_write {
                // OUTS: read from guest memory, write to port
                let val = match access_size {
                    1 => self.memory.read_u8(current_gpa).unwrap_or(0) as u32,
                    2 => self.memory.read_u16(current_gpa).unwrap_or(0) as u32,
                    4 => self.memory.read_u32(current_gpa).unwrap_or(0),
                    _ => 0,
                };
                let _ = self.io.port_out(port, access_size, val);
            } else {
                // INS: read from port, write to guest memory
                let val = self.io.port_in(port, access_size).unwrap_or(0xFFFF_FFFF);
                match access_size {
                    1 => { let _ = self.memory.write_u8(current_gpa, val as u8); }
                    2 => { let _ = self.memory.write_u16(current_gpa, val as u16); }
                    4 => { let _ = self.memory.write_u32(current_gpa, val); }
                    _ => {}
                }
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
    ///
    /// If no registered MMIO region matches, checks whether the address falls
    /// within the VGA BAR2 region (Bochs VBE DISPI registers). SeaBIOS may
    /// remap BAR2 to an address different from our statically registered ones,
    /// so this fallback ensures the bochs-drm driver always reaches the VBE
    /// registers regardless of where the firmware places BAR2.
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
            if !self.memory.dispatch_mmio_write(addr, size, val) {
                // Fallback: check VGA BAR2 for VBE DISPI registers
                if let Some(offset) = self.vga_bar2_offset(addr) {
                    self.svga_dispi_write(offset, size, val);
                }
            }
        } else {
            match self.memory.dispatch_mmio_read(addr, size) {
                Some(val) => {
                    let bytes = val.to_le_bytes();
                    for i in 0..size as usize {
                        if i < data.len() {
                            data[i] = bytes[i];
                        }
                    }
                }
                None => {
                    // Fallback: check VGA BAR2 for VBE DISPI registers
                    let val = if let Some(offset) = self.vga_bar2_offset(addr) {
                        self.svga_dispi_read(offset, size)
                    } else {
                        0xFFFF_FFFF_FFFF_FFFF
                    };
                    let bytes = val.to_le_bytes();
                    for i in 0..size as usize {
                        if i < data.len() {
                            data[i] = bytes[i];
                        }
                    }
                }
            }
        }
    }

    /// Check if `addr` falls within the current VGA BAR2 region (4KB).
    /// Returns the offset within the BAR2 region, or None.
    fn vga_bar2_offset(&self, addr: u64) -> Option<u64> {
        if self.pci_bus_ptr.is_null() { return None; }
        let pci_bus = unsafe { &*self.pci_bus_ptr };
        // VGA is device 2 (00:02.0) — read BAR2 (config offset 0x18)
        if let Some(vga_dev) = pci_bus.devices.iter().find(|d| d.device == 2 && d.function == 0) {
            let bar2 = u32::from_le_bytes([
                vga_dev.config_space[0x18],
                vga_dev.config_space[0x19],
                vga_dev.config_space[0x1A],
                vga_dev.config_space[0x1B],
            ]) & 0xFFFFF000; // Mask out type bits
            if bar2 != 0 {
                let bar2_base = bar2 as u64;
                if addr >= bar2_base && addr < bar2_base + 0x1000 {
                    return Some(addr - bar2_base);
                }
            }
        }
        None
    }

    /// Read from VBE DISPI MMIO registers via the SVGA device.
    fn svga_dispi_read(&mut self, offset: u64, size: u8) -> u64 {
        if self.svga_ptr.is_null() { return 0xFFFF_FFFF; }
        let svga = unsafe { &mut *self.svga_ptr };
        let mut proxy = SvgaDispiMmioProxy(svga as *mut _);
        use crate::memory::mmio::MmioHandler;
        proxy.read(offset, size).unwrap_or(0xFFFF_FFFF)
    }

    /// Write to VBE DISPI MMIO registers via the SVGA device.
    fn svga_dispi_write(&mut self, offset: u64, size: u8, val: u64) {
        if self.svga_ptr.is_null() { return; }
        let svga = unsafe { &mut *self.svga_ptr };
        let mut proxy = SvgaDispiMmioProxy(svga as *mut _);
        use crate::memory::mmio::MmioHandler;
        let _ = proxy.write(offset, size, val);
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

    /// Handle KVM string I/O IN (REP INSB): loop count times, calling port_in
    /// for each iteration and writing results to the kvm_run data buffer.
    #[cfg(feature = "linux")]
    pub fn complete_string_io_in(&mut self, vcpu_id: u32, port: u16, size: u8, count: u32) {
        for i in 0..count {
            let val = self.io.port_in(port, size).unwrap_or(0xFFFF_FFFF);
            let bytes = val.to_le_bytes();
            self.backend.set_io_response_at(vcpu_id, i, &bytes[..size as usize]);
        }
    }

    /// Handle KVM string I/O OUT (REP OUTSB): loop count times, reading data
    /// from the kvm_run data buffer and calling port_out for each iteration.
    #[cfg(feature = "linux")]
    pub fn complete_string_io_out(&mut self, vcpu_id: u32, port: u16, size: u8, count: u32) {
        for i in 0..count {
            let val = self.backend.get_io_data_at(vcpu_id, i);
            let _ = self.io.port_out(port, size, val);
        }
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

    pub fn cmos_mut(&mut self) -> Option<&mut crate::devices::cmos::Cmos> {
        if self.cmos_ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.cmos_ptr })
        }
    }

    /// Enable the HPET (High Precision Event Timer) device.
    /// Required for Windows guests. Call after setup_standard_devices().
    pub fn setup_hpet(&mut self) {
        if !self.hpet_ptr.is_null() {
            return; // already set up
        }
        use crate::devices::hpet::{HPET_BASE, HPET_SIZE};
        let hpet = Box::new(Hpet::new());
        self.hpet_ptr = &*hpet as *const Hpet as *mut Hpet;
        self.memory.add_mmio(HPET_BASE, HPET_SIZE, hpet);
    }

    pub fn hpet_mut(&mut self) -> Option<&mut Hpet> {
        if self.hpet_ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.hpet_ptr })
        }
    }

    /// Get a mutable reference to the E1000 NIC, if set up.
    pub fn e1000(&mut self) -> Option<&mut E1000> {
        if self.e1000_ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.e1000_ptr })
        }
    }

    /// Get a mutable reference to the UHCI USB controller, if set up.
    pub fn uhci(&mut self) -> Option<&mut crate::devices::uhci::Uhci> {
        if self.uhci_ptr.is_null() { None }
        else { Some(unsafe { &mut *self.uhci_ptr }) }
    }

    /// Get a mutable reference to the AC97 audio controller, if set up.
    pub fn ac97(&mut self) -> Option<&mut Ac97> {
        if self.ac97_ptr.is_null() {
            None
        } else {
            Some(unsafe { &mut *self.ac97_ptr })
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

/// Proxy for Bochs VBE dispi MMIO registers (PCI BAR2).
/// The bochs DRM driver reads VBE registers via MMIO at offset = index * 2.
struct SvgaDispiMmioProxy(*mut Svga);
unsafe impl Send for SvgaDispiMmioProxy {}

impl crate::memory::mmio::MmioHandler for SvgaDispiMmioProxy {
    fn read(&mut self, offset: u64, size: u8) -> crate::error::Result<u64> {
        let svga = unsafe { &mut *self.0 };
        // QEMU Bochs VBE MMIO layout in BAR2:
        //   0x000-0x3FF: VGA I/O ports (0x3C0-0x3DF) mapped at offset = port - 0x3C0
        //   0x400-0x4FF: VGA I/O ports aliased
        //   0x500-0x5FF: VBE dispi registers at offset 0x500 + index * 2
        //   0x600+:      QEMU extended registers
        if offset >= 0x500 && offset < 0x600 {
            // VBE dispi registers
            let idx = ((offset - 0x500) / 2) as usize;
            if idx < svga.vbe_regs.len() {
                let val = svga.vbe_regs[idx] as u64;
                return Ok(match size {
                    1 => if offset & 1 == 0 { val & 0xFF } else { (val >> 8) & 0xFF },
                    2 => val,
                    4 => {
                        let hi = if idx + 1 < svga.vbe_regs.len() { svga.vbe_regs[idx + 1] as u64 } else { 0 };
                        val | (hi << 16)
                    }
                    _ => val,
                });
            }
            return Ok(0);
        } else if offset < 0x400 {
            // VGA I/O port emulation via MMIO
            let port = 0x3C0 + offset as u16;
            let result = <Svga as crate::io::IoHandler>::read(svga, port, size);
            return result.map(|v| v as u64);
        }
        Ok(0xFFFF_FFFF)
    }
    fn write(&mut self, offset: u64, _size: u8, val: u64) -> crate::error::Result<()> {
        let svga = unsafe { &mut *self.0 };
        if offset >= 0x500 && offset < 0x600 {
            // VBE dispi registers
            let idx = ((offset - 0x500) / 2) as usize;
            let v = val as u16;
            if idx < svga.vbe_regs.len() {
                svga.vbe_regs[idx] = v;
                // VBE_DISPI_INDEX_ENABLE (4): mode switch.
                if idx == 4 && (v & 0x01) != 0 {
                    let w = svga.vbe_regs[1] as u32;
                    let h = svga.vbe_regs[2] as u32;
                    let bpp = svga.vbe_regs[3] as u8;
                    if w > 0 && h > 0 && bpp > 0 {
                        svga.set_mode(crate::devices::svga::VgaMode::LinearFramebuffer { width: w, height: h, bpp });
                    }
                } else if idx == 4 && (v & 0x01) == 0 {
                    svga.set_mode(crate::devices::svga::VgaMode::Text80x25);
                }
            }
            return Ok(());
        } else if offset < 0x400 {
            // VGA I/O port emulation via MMIO
            let port = 0x3C0 + offset as u16;
            return <Svga as crate::io::IoHandler>::write(svga, port, _size, val as u32);
        }
        Ok(())
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

/// Port 0x92: System Control Port A (Fast A20 Gate).
/// Bit 0: Fast Reset (write 1 = system reset, always reads 0)
/// Bit 1: A20 Gate Enable (1 = enabled, default = enabled in KVM)
/// Bits 2-7: reserved, read as 0
struct Port92 {
    value: u8,
}

impl Port92 {
    fn new() -> Self { Port92 { value: 0x02 } } // A20 enabled by default
}

impl crate::io::IoHandler for Port92 {
    fn read(&mut self, _port: u16, _size: u8) -> crate::error::Result<u32> {
        Ok(self.value as u32)
    }
    fn write(&mut self, _port: u16, _size: u8, val: u32) -> crate::error::Result<()> {
        let v = val as u8;
        // Bit 0: Fast Reset — if set, request system reset
        // (handled by the VM loop checking cf9_reset_pending)
        // Bit 1: A20 Gate — store it (KVM handles A20 internally)
        self.value = v & 0x02; // only store A20 bit, clear reset bit
        Ok(())
    }
}

/// Proxy for VGA linear framebuffer MMIO at PCI BAR0 (0xFD000000, 16MB).
/// Guest LFB writes go directly to the Svga framebuffer.
struct SvgaLfbProxy(*mut Svga);
unsafe impl Send for SvgaLfbProxy {}

impl crate::memory::mmio::MmioHandler for SvgaLfbProxy {
    fn read(&mut self, offset: u64, size: u8) -> crate::error::Result<u64> {
        let svga = unsafe { &mut *self.0 };
        let off = offset as usize;
        if off >= svga.framebuffer.len() {
            return Ok(0);
        }
        let mut val: u64 = 0;
        let end = (off + size as usize).min(svga.framebuffer.len());
        for i in off..end {
            val |= (svga.framebuffer[i] as u64) << ((i - off) * 8);
        }
        Ok(val)
    }
    fn write(&mut self, offset: u64, size: u8, val: u64) -> crate::error::Result<()> {
        let svga = unsafe { &mut *self.0 };
        let off = offset as usize;
        for i in 0..(size as usize) {
            let idx = off + i;
            if idx < svga.framebuffer.len() {
                svga.framebuffer[idx] = ((val >> (i * 8)) & 0xFF) as u8;
            }
        }
        Ok(())
    }
}

