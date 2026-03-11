//! Hardware virtualization backend abstraction.
//!
//! Defines the `VmBackend` trait that platform-specific backends (KVM, WHP, anyOS)
//! implement, along with shared error and exit-reason types.

pub mod types;
pub use types::*;

#[cfg(feature = "linux")]
pub mod kvm;

#[derive(Debug, Clone)]
pub enum VmError {
    NoHardwareSupport,
    VmxInitFailed,
    SvmInitFailed,
    InvalidVcpuId,
    MemoryMapFailed,
    VmEntryFailed(u32),
    BackendError(i32),
}

#[derive(Debug)]
pub enum VmExitReason {
    IoIn { port: u16, size: u8 },
    IoOut { port: u16, size: u8, data: u32 },
    MmioRead { addr: u64, size: u8 },
    MmioWrite { addr: u64, size: u8, data: u64 },
    MsrRead { index: u32 },
    MsrWrite { index: u32, value: u64 },
    CpuidExit { function: u32, index: u32 },
    Halted,
    InterruptWindow,
    Shutdown,
    Debug,
    Error,
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
