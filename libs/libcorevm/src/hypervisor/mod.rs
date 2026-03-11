//! Hardware virtualization abstraction layer.
//!
//! Provides a unified trait [`HypervisorBackend`] that abstracts over:
//! - Intel VT-x (VMX) — direct hardware on anyOS, via KVM on Linux
//! - AMD-V (SVM) — direct hardware on anyOS, via KVM on Linux
//! - Windows Hypervisor Platform (WHP) — on Windows
//! - Apple Hypervisor.framework (HVF) — on macOS
//!
//! The VMM (vmd) uses only the trait interface, making it completely
//! independent of the underlying hardware virtualization technology.

pub mod vmx;
pub mod svm;

#[cfg(not(feature = "host_test"))]
pub mod bare;

#[cfg(all(feature = "host_test", target_os = "linux"))]
pub mod kvm;

#[cfg(all(feature = "host_test", target_os = "windows"))]
pub mod whp;

#[cfg(all(feature = "host_test", target_os = "macos"))]
pub mod hvf;

use alloc::boxed::Box;

/// Reason the vCPU exited back to the VMM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmExit {
    /// Guest executed an IN instruction.
    IoIn { port: u16, size: u8 },
    /// Guest executed an OUT instruction.
    IoOut { port: u16, size: u8, data: u32 },
    /// Guest read from a memory-mapped I/O region.
    MmioRead { address: u64, size: u8 },
    /// Guest wrote to a memory-mapped I/O region.
    MmioWrite { address: u64, size: u8, data: u64 },
    /// Guest executed HLT.
    Hlt,
    /// Guest executed CPUID — VMM must emulate.
    Cpuid { eax: u32, ecx: u32 },
    /// Guest accessed an MSR (RDMSR/WRMSR).
    MsrRead { index: u32 },
    /// Guest wrote an MSR.
    MsrWrite { index: u32, value: u64 },
    /// Guest triple-faulted or encountered an unrecoverable error.
    Shutdown,
    /// External stop request acknowledged.
    StopRequested,
    /// An interrupt window opened — VMM can inject pending IRQs.
    InterruptWindow,
    /// EPT violation / nested page fault — unhandled guest physical access.
    EptViolation { guest_phys: u64, is_write: bool },
    /// Debug breakpoint or single-step trap.
    Debug,
    /// Unknown or unhandled exit reason.
    Unknown(u32),
}

/// x86 segment register state for guest/host transfer.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct SegmentState {
    pub selector: u16,
    pub base: u64,
    pub limit: u32,
    pub access_rights: u32,
}

/// Descriptor table register state (GDTR, IDTR).
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct DtableState {
    pub base: u64,
    pub limit: u16,
}

/// Complete vCPU register state for get/set operations.
#[derive(Debug, Clone, Default)]
#[repr(C)]
pub struct VcpuRegs {
    // General-purpose registers
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rbx: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,

    // Instruction pointer and flags
    pub rip: u64,
    pub rflags: u64,

    // Segment registers
    pub cs: SegmentState,
    pub ds: SegmentState,
    pub es: SegmentState,
    pub fs: SegmentState,
    pub gs: SegmentState,
    pub ss: SegmentState,
    pub tr: SegmentState,
    pub ldtr: SegmentState,

    // Descriptor table registers
    pub gdtr: DtableState,
    pub idtr: DtableState,

    // Control registers
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,

    // Model-specific registers
    pub efer: u64,
    pub apic_base: u64,
    pub pat: u64,

    // Sysenter/syscall MSRs
    pub sysenter_cs: u64,
    pub sysenter_esp: u64,
    pub sysenter_eip: u64,
    pub star: u64,
    pub lstar: u64,
    pub cstar: u64,
    pub sfmask: u64,
    pub kernel_gs_base: u64,
    pub tsc_aux: u64,
}

/// Memory region descriptor for mapping guest physical memory.
#[derive(Debug, Clone)]
pub struct MemoryRegion {
    /// Slot identifier for updates/removal.
    pub slot: u32,
    /// Guest physical address (page-aligned).
    pub guest_phys_addr: u64,
    /// Size in bytes (page-aligned).
    pub memory_size: u64,
    /// Host virtual address of the backing memory.
    pub userspace_addr: *mut u8,
    /// Read-only flag.
    pub readonly: bool,
}

/// Trait abstracting hardware virtualization backends.
///
/// Each platform implements this trait to provide VM creation, vCPU
/// management, memory mapping, and execution. The VMM code uses only
/// this interface, never platform-specific APIs directly.
pub trait HypervisorBackend: Send {
    /// Initialize the hypervisor and create a VM.
    fn create_vm(&mut self) -> Result<(), HvError>;

    /// Create a virtual CPU within the VM.
    fn create_vcpu(&mut self, vcpu_id: u32) -> Result<(), HvError>;

    /// Map a region of host memory into the guest physical address space.
    fn map_memory(&mut self, region: &MemoryRegion) -> Result<(), HvError>;

    /// Unmap a previously mapped memory region.
    fn unmap_memory(&mut self, slot: u32) -> Result<(), HvError>;

    /// Get the full register state of a vCPU.
    fn get_regs(&self, vcpu_id: u32) -> Result<VcpuRegs, HvError>;

    /// Set the full register state of a vCPU.
    fn set_regs(&mut self, vcpu_id: u32, regs: &VcpuRegs) -> Result<(), HvError>;

    /// Run the vCPU until a VM-exit occurs.
    fn run(&mut self, vcpu_id: u32) -> Result<VmExit, HvError>;

    /// Request an interrupt window exit so the VMM can inject an IRQ.
    fn request_interrupt_window(&mut self, vcpu_id: u32) -> Result<(), HvError>;

    /// Inject an external interrupt into the guest.
    fn inject_interrupt(&mut self, vcpu_id: u32, vector: u8) -> Result<(), HvError>;

    /// Check if the guest has interrupts enabled (IF flag set, no shadow).
    fn interrupts_enabled(&self, vcpu_id: u32) -> Result<bool, HvError>;

    /// Set the data returned for an IO-in exit (guest IN instruction).
    fn set_io_in_data(&mut self, vcpu_id: u32, data: u32, size: u8) -> Result<(), HvError>;

    /// Set the data returned for an MMIO read exit.
    fn set_mmio_read_data(&mut self, vcpu_id: u32, data: u64, size: u8) -> Result<(), HvError>;

    /// Set CPUID response values after a CPUID exit.
    fn set_cpuid_response(&mut self, vcpu_id: u32, eax: u32, ebx: u32, ecx: u32, edx: u32) -> Result<(), HvError>;

    /// Set MSR read response after an MSR read exit.
    fn set_msr_read_data(&mut self, vcpu_id: u32, value: u64) -> Result<(), HvError>;

    /// Advance RIP past the instruction that caused the exit.
    fn advance_rip(&mut self, vcpu_id: u32, len: u8) -> Result<(), HvError>;

    /// Request the vCPU to stop at the next opportunity.
    fn request_stop(&mut self, vcpu_id: u32) -> Result<(), HvError>;

    /// Destroy the VM and release all resources.
    fn destroy(&mut self);
}

/// Hypervisor error type.
#[derive(Debug, Clone)]
pub enum HvError {
    /// The hardware does not support virtualization.
    NotSupported,
    /// VMX/SVM operation failed.
    HardwareError(u32),
    /// Memory mapping failed.
    MemoryError,
    /// Invalid vCPU ID.
    InvalidVcpu,
    /// Invalid register access.
    InvalidRegister,
    /// System call or ioctl failed.
    SystemError(i32),
    /// Generic error with description.
    Other(&'static str),
}

impl core::fmt::Display for HvError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HvError::NotSupported => write!(f, "hardware virtualization not supported"),
            HvError::HardwareError(code) => write!(f, "hardware error: {}", code),
            HvError::MemoryError => write!(f, "memory mapping error"),
            HvError::InvalidVcpu => write!(f, "invalid vCPU ID"),
            HvError::InvalidRegister => write!(f, "invalid register access"),
            HvError::SystemError(errno) => write!(f, "system error: {}", errno),
            HvError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

/// Create the appropriate hypervisor backend for the current platform.
///
/// On anyOS (bare metal), this returns the direct VMX/SVM backend.
/// On Linux, it returns the KVM backend.
/// On Windows, it returns the WHP backend.
/// On macOS, it returns the HVF backend.
pub fn create_backend() -> Result<Box<dyn HypervisorBackend>, HvError> {
    #[cfg(not(feature = "host_test"))]
    {
        Ok(Box::new(bare::BareMetalBackend::new()?))
    }

    #[cfg(all(feature = "host_test", target_os = "linux"))]
    {
        Ok(Box::new(kvm::KvmBackend::new()?))
    }

    #[cfg(all(feature = "host_test", target_os = "windows"))]
    {
        Ok(Box::new(whp::WhpBackend::new()?))
    }

    #[cfg(all(feature = "host_test", target_os = "macos"))]
    {
        Ok(Box::new(hvf::HvfBackend::new()?))
    }
}
