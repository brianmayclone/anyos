//! Hardware virtualization backend abstraction.
//!
//! Defines the `VmBackend` trait that platform-specific backends (KVM, WHP, anyOS)
//! implement, along with shared error and exit-reason types.

pub mod types;
pub use types::*;

#[cfg(feature = "linux")]
pub mod kvm;

#[cfg(feature = "anyos")]
pub mod anyos;

#[cfg(feature = "windows")]
pub mod whp;

#[derive(Debug, Clone)]
pub enum VmError {
    NoHardwareSupport,
    VmxInitFailed,
    SvmInitFailed,
    VmCreateFailed,
    InvalidVcpuId,
    MemoryMapFailed,
    VmEntryFailed(u32),
    BackendError(i32),
    /// Backend error with context: (HRESULT, step description)
    BackendErrorCtx(i32, &'static str),
}

impl core::fmt::Display for VmError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            VmError::NoHardwareSupport => write!(f, "No hardware virtualization support (VT-x/AMD-V not available or not enabled in kernel)"),
            VmError::VmxInitFailed => write!(f, "Intel VT-x initialization failed"),
            VmError::SvmInitFailed => write!(f, "AMD-V (SVM) initialization failed"),
            VmError::VmCreateFailed => write!(f, "VM creation failed (out of VM slots or page allocation error)"),
            VmError::InvalidVcpuId => write!(f, "Invalid vCPU ID"),
            VmError::MemoryMapFailed => write!(f, "Memory mapping failed"),
            VmError::VmEntryFailed(code) => write!(f, "VM entry failed (code {})", code),
            VmError::BackendError(code) => write!(f, "Backend error (HRESULT 0x{:08X})", *code as u32),
            VmError::BackendErrorCtx(code, step) => write!(f, "{} failed (HRESULT 0x{:08X})", step, *code as u32),
        }
    }
}

#[derive(Debug)]
pub enum VmExitReason {
    IoIn { port: u16, size: u8, count: u32 },
    IoOut { port: u16, size: u8, data: u32, count: u32 },
    MmioRead { addr: u64, size: u8, dest_reg: u8, instr_len: u8 },
    MmioWrite { addr: u64, size: u8, data: u64 },
    MsrRead { index: u32 },
    MsrWrite { index: u32, value: u64 },
    CpuidExit { function: u32, index: u32 },
    /// Bulk string I/O (REP INSB/OUTSB). The backend provides all the info
    /// needed for the caller to handle the entire transfer in one call.
    StringIo {
        port: u16,
        is_write: bool,
        count: u64,
        /// Guest physical address of the first byte.
        gpa: u64,
        /// ±access_size (direction flag × data width).
        step: i64,
        /// Instruction length for RIP advancement after completion.
        instr_len: u64,
        /// Address size (2/4/8) for register update masking.
        addr_size: u8,
        /// Port I/O access size (1=byte, 2=word, 4=dword).
        access_size: u8,
    },
    Halted,
    InterruptWindow,
    Shutdown,
    Debug,
    Error,
    /// vCPU was cancelled (immediate_exit on KVM, WHvCancelRunVirtualProcessor on WHP).
    Cancelled,
}

/// Hardware virtualization backend trait.
///
/// Construction is not part of the trait (not object-safe).
/// Each backend provides `BackendType::new(ram_size) -> Result<Self, VmError>`.
pub trait VmBackend {
    fn destroy(&mut self);
    fn reset(&mut self) -> Result<(), VmError>;

    // Memory
    fn set_memory_region(&mut self, slot: u32, guest_phys: u64, size: u64, host_ptr: *mut u8) -> Result<(), VmError>;
    fn read_phys(&self, addr: u64, buf: &mut [u8]) -> Result<(), VmError>;
    fn write_phys(&mut self, addr: u64, buf: &[u8]) -> Result<(), VmError>;

    // vCPU
    fn create_vcpu(&mut self, id: u32) -> Result<(), VmError>;
    fn destroy_vcpu(&mut self, id: u32) -> Result<(), VmError>;
    fn run_vcpu(&mut self, id: u32) -> Result<VmExitReason, VmError>;
    fn get_vcpu_regs(&self, id: u32) -> Result<VcpuRegs, VmError>;
    fn set_vcpu_regs(&mut self, id: u32, regs: &VcpuRegs) -> Result<(), VmError>;
    fn get_vcpu_sregs(&self, id: u32) -> Result<VcpuSregs, VmError>;
    fn set_vcpu_sregs(&mut self, id: u32, sregs: &VcpuSregs) -> Result<(), VmError>;
    fn inject_interrupt(&mut self, id: u32, vector: u8) -> Result<(), VmError>;
    fn inject_exception(&mut self, id: u32, vector: u8, error_code: Option<u32>) -> Result<(), VmError>;
    fn inject_nmi(&mut self, id: u32) -> Result<(), VmError>;
    fn request_interrupt_window(&mut self, id: u32, enable: bool) -> Result<(), VmError>;
    fn set_cpuid(&mut self, entries: &[CpuidEntry]) -> Result<(), VmError>;
}
