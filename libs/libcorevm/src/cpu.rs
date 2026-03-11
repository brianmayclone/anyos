//! CPU virtualization core — hardware-backed vCPU management.
//!
//! The `Cpu` struct wraps a hardware virtualization backend (VT-x/AMD-V
//! via the hypervisor abstraction layer) and manages the guest vCPU state.
//! Execution is performed by the hardware — no software emulation or JIT.

use alloc::boxed::Box;
use crate::hypervisor::{self, HypervisorBackend, HvError, VmExit, VcpuRegs, SegmentState, DtableState, MemoryRegion};
use crate::error::VmError;

/// CPU execution mode (derived from guest register state after VM-exit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    /// 16-bit real mode.
    RealMode,
    /// 32-bit protected mode.
    ProtectedMode,
    /// 64-bit long mode.
    LongMode,
}

/// Reason the CPU stopped executing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    /// HLT instruction executed.
    Halted,
    /// Unhandled exception (triple fault or fatal error).
    Exception(VmError),
    /// Maximum run-loop iterations reached.
    InstructionLimit,
    /// Breakpoint (INT 3 or debug trap).
    Breakpoint,
    /// External stop request via `request_stop()`.
    StopRequested,
}

/// Guest I/O exit that the VMM must handle.
#[derive(Debug, Clone, Copy)]
pub enum IoExit {
    /// Guest performed IN instruction — VMM must provide data.
    In { port: u16, size: u8 },
    /// Guest performed OUT instruction.
    Out { port: u16, size: u8, data: u32 },
}

/// Guest MMIO exit that the VMM must handle.
#[derive(Debug, Clone, Copy)]
pub enum MmioExit {
    /// Guest read from MMIO — VMM must provide data.
    Read { address: u64, size: u8 },
    /// Guest wrote to MMIO.
    Write { address: u64, size: u8, data: u64 },
}

/// Virtual CPU backed by hardware virtualization.
pub struct Cpu {
    /// The hypervisor backend (VT-x, AMD-V, KVM, WHV, or HVF).
    backend: Box<dyn HypervisorBackend>,
    /// Current CPU mode (derived after each VM-exit).
    pub mode: Mode,
    /// Number of VM-exit cycles executed since creation.
    pub run_cycles: u64,
    /// If true, stop at the next opportunity.
    pub stop_requested: bool,
    /// Whether the backend VM has been created.
    vm_created: bool,
    /// Whether vCPU 0 has been created.
    vcpu_created: bool,
    /// Cached guest RIP (updated on each VM-exit).
    pub last_rip: u64,
    /// Cached guest CS selector.
    pub last_cs: u16,
    /// Local APIC/x2APIC ID for topology reporting.
    pub apic_id: u32,
    /// Configured number of logical CPUs.
    pub logical_cpu_count: u8,
}

impl Cpu {
    /// Create a new hardware-virtualized CPU.
    ///
    /// Detects the available virtualization technology and creates
    /// the appropriate backend.
    pub fn new() -> Result<Self, HvError> {
        let backend = hypervisor::create_backend()?;
        Ok(Cpu {
            backend,
            mode: Mode::RealMode,
            run_cycles: 0,
            stop_requested: false,
            vm_created: false,
            vcpu_created: false,
            last_rip: 0xFFF0,
            last_cs: 0xF000,
            apic_id: 0,
            logical_cpu_count: 1,
        })
    }

    /// Initialize the VM and vCPU.
    pub fn init_vm(&mut self) -> Result<(), HvError> {
        self.backend.create_vm()?;
        self.vm_created = true;
        self.backend.create_vcpu(0)?;
        self.vcpu_created = true;
        // Set initial real-mode state (x86 reset vector: CS:IP = F000:FFF0)
        self.set_reset_state()?;
        Ok(())
    }

    /// Map a region of host memory into the guest physical address space.
    pub fn map_memory(&mut self, slot: u32, guest_phys: u64, size: u64, host_ptr: *mut u8, readonly: bool) -> Result<(), HvError> {
        self.backend.map_memory(&MemoryRegion {
            slot,
            guest_phys_addr: guest_phys,
            memory_size: size,
            userspace_addr: host_ptr,
            readonly,
        })
    }

    /// Unmap a previously mapped memory region.
    pub fn unmap_memory(&mut self, slot: u32) -> Result<(), HvError> {
        self.backend.unmap_memory(slot)
    }

    /// Set the CPU to x86 power-on reset state.
    fn set_reset_state(&mut self) -> Result<(), HvError> {
        let mut regs = VcpuRegs::default();

        // Reset vector: CS:IP = F000:FFF0 → linear 0xFFFF0
        regs.rip = 0xFFF0;
        regs.rflags = 0x0000_0002; // Bit 1 is always set

        // Real-mode CS: selector=0xF000, base=0xF0000 (NOT 0xFFFF0000 like post-INIT)
        // Actually for hardware reset, CS base = 0xFFFF0000 on modern CPUs
        // so the first instruction fetches from 0xFFFFFFF0. But with BIOS
        // loaded at 0xF0000, we use the standard BIOS mapping.
        regs.cs = SegmentState {
            selector: 0xF000,
            base: 0xFFFF_0000, // Hardware reset value
            limit: 0xFFFF,
            access_rights: 0x009B, // present, DPL=0, code, readable, accessed
        };

        // Data segments: base=0, limit=0xFFFF
        let data_seg = SegmentState {
            selector: 0x0000,
            base: 0,
            limit: 0xFFFF,
            access_rights: 0x0093, // present, DPL=0, data, writable, accessed
        };
        regs.ds = data_seg;
        regs.es = data_seg;
        regs.fs = data_seg;
        regs.gs = data_seg;
        regs.ss = data_seg;

        // TR and LDTR (required by VMX)
        regs.tr = SegmentState {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            access_rights: 0x008B, // present, 32-bit busy TSS
        };
        regs.ldtr = SegmentState {
            selector: 0,
            base: 0,
            limit: 0xFFFF,
            access_rights: 0x0082, // present, LDT
        };

        // GDT and IDT
        regs.gdtr = DtableState { base: 0, limit: 0xFFFF };
        regs.idtr = DtableState { base: 0, limit: 0xFFFF };

        // Control registers — real mode defaults
        // CR0: PE=0, ET=1, CD=1, NW=1 (caches disabled at reset)
        regs.cr0 = 0x6000_0010; // ET=1, CD=1, NW=1
        regs.cr2 = 0;
        regs.cr3 = 0;
        regs.cr4 = 0;
        regs.cr8 = 0;

        // EFER = 0 (no long mode, no NXE, no SCE)
        regs.efer = 0;

        // PAT: default value
        regs.pat = 0x0007_0406_0007_0406;

        // APIC base: default enabled at 0xFEE00000
        regs.apic_base = 0xFEE0_0900; // enabled, BSP

        // GPRs: all zero except EDX = processor signature
        regs.rdx = 0x0000_0600; // P6 family

        self.backend.set_regs(0, &regs)?;
        Ok(())
    }

    /// Reset the CPU to power-on state.
    pub fn reset(&mut self) -> Result<(), HvError> {
        self.set_reset_state()?;
        self.mode = Mode::RealMode;
        self.run_cycles = 0;
        self.stop_requested = false;
        self.last_rip = 0xFFF0;
        self.last_cs = 0xF000;
        Ok(())
    }

    /// Configure the CPU topology values for CPUID reporting.
    pub fn configure_topology(&mut self, apic_id: u32, logical_cpu_count: u8) {
        self.apic_id = apic_id;
        self.logical_cpu_count = logical_cpu_count.max(1);
    }

    /// Get the current guest register state.
    pub fn get_regs(&self) -> Result<VcpuRegs, HvError> {
        self.backend.get_regs(0)
    }

    /// Set the guest register state.
    pub fn set_regs(&mut self, regs: &VcpuRegs) -> Result<(), HvError> {
        self.backend.set_regs(0, regs)
    }

    /// Request the CPU to stop at the next opportunity.
    pub fn request_stop(&mut self) {
        self.stop_requested = true;
        let _ = self.backend.request_stop(0);
    }

    /// Check if the guest has interrupts enabled.
    pub fn interrupts_enabled(&self) -> bool {
        self.backend.interrupts_enabled(0).unwrap_or(false)
    }

    /// Inject an external interrupt into the guest.
    pub fn inject_interrupt(&mut self, vector: u8) -> Result<(), HvError> {
        self.backend.inject_interrupt(0, vector)
    }

    /// Request an interrupt window exit so the VMM can inject an IRQ
    /// when the guest enables interrupts.
    pub fn request_interrupt_window(&mut self) -> Result<(), HvError> {
        self.backend.request_interrupt_window(0)
    }

    /// Execute one VM-entry/exit cycle.
    ///
    /// Returns the VM-exit reason. The caller (VMM) is responsible for
    /// handling I/O, MMIO, CPUID, MSR, and other exits before calling
    /// `run_once()` again.
    pub fn run_once(&mut self) -> Result<VmExit, HvError> {
        if self.stop_requested {
            self.stop_requested = false;
            return Ok(VmExit::StopRequested);
        }
        let exit = self.backend.run(0)?;
        self.run_cycles += 1;
        // Update cached state
        if let Ok(regs) = self.backend.get_regs(0) {
            self.last_rip = regs.rip;
            self.last_cs = regs.cs.selector;
            self.mode = Self::derive_mode(regs.cr0, regs.efer, &regs.cs);
        }
        Ok(exit)
    }

    /// Provide the response data for a guest IN instruction.
    pub fn complete_io_in(&mut self, data: u32, size: u8) -> Result<(), HvError> {
        self.backend.set_io_in_data(0, data, size)
    }

    /// Provide the response data for a guest MMIO read.
    pub fn complete_mmio_read(&mut self, data: u64, size: u8) -> Result<(), HvError> {
        self.backend.set_mmio_read_data(0, data, size)
    }

    /// Provide CPUID response values.
    pub fn complete_cpuid(&mut self, eax: u32, ebx: u32, ecx: u32, edx: u32) -> Result<(), HvError> {
        self.backend.set_cpuid_response(0, eax, ebx, ecx, edx)
    }

    /// Provide MSR read response.
    pub fn complete_msr_read(&mut self, value: u64) -> Result<(), HvError> {
        self.backend.set_msr_read_data(0, value)
    }

    /// Advance RIP past the current instruction.
    pub fn advance_rip(&mut self, len: u8) -> Result<(), HvError> {
        self.backend.advance_rip(0, len)
    }

    /// Derive the CPU mode from control register state and CS descriptor.
    fn derive_mode(cr0: u64, efer: u64, cs: &SegmentState) -> Mode {
        let pe = cr0 & 1 != 0; // CR0.PE
        let pg = cr0 & (1 << 31) != 0; // CR0.PG
        let lma = efer & (1 << 10) != 0; // EFER.LMA
        let cs_l = (cs.access_rights >> 13) & 1 != 0; // CS.L bit

        if pe && pg && lma && cs_l {
            Mode::LongMode
        } else if pe {
            Mode::ProtectedMode
        } else {
            Mode::RealMode
        }
    }

    /// Destroy the backend and release all hardware resources.
    pub fn destroy(&mut self) {
        self.backend.destroy();
        self.vm_created = false;
        self.vcpu_created = false;
    }
}

impl Drop for Cpu {
    fn drop(&mut self) {
        self.destroy();
    }
}
