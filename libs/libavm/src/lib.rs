//! libavm -- KVM-style userspace API for anyOS virtualization.
//!
//! AVM intentionally looks like KVM at the control-flow level:
//! a system object creates VM handles, VM handles create vCPU handles, and
//! operations are issued through ioctl-like request codes.

#![no_std]

pub const AVM_SYSTEM_HANDLE: u64 = 0;
pub const AVM_API_VERSION: u32 = 1;

pub const AVM_EXT_DIRTY_LOG: u32 = 1;
pub const AVM_EXT_MP_STATE: u32 = 2;
pub const AVM_EXT_GVA_TRANSLATE: u32 = 3;
pub const AVM_EXT_FPU_STATE: u32 = 4;

pub const AVMIO_GET_API_VERSION: u32 = 0xAE00;
pub const AVMIO_CHECK_EXTENSION: u32 = 0xAE01;
pub const AVMIO_GET_BACKEND_INFO: u32 = 0xAE02;
pub const AVMIO_CREATE_VM: u32 = 0xAE03;

pub const AVMIO_SET_USER_MEMORY_REGION: u32 = 0xAE40;
pub const AVMIO_CREATE_VCPU: u32 = 0xAE41;
pub const AVMIO_SET_CPUID2: u32 = 0xAE42;
pub const AVMIO_GET_DIRTY_LOG: u32 = 0xAE43;
pub const AVMIO_DESTROY_VM: u32 = 0xAE44;

pub const AVMIO_RUN: u32 = 0xAE80;
pub const AVMIO_GET_REGS: u32 = 0xAE81;
pub const AVMIO_SET_REGS: u32 = 0xAE82;
pub const AVMIO_GET_SREGS: u32 = 0xAE83;
pub const AVMIO_SET_SREGS: u32 = 0xAE84;
pub const AVMIO_PAUSE: u32 = 0xAE85;
pub const AVMIO_RESUME: u32 = 0xAE86;
pub const AVMIO_GET_FPU: u32 = 0xAE87;
pub const AVMIO_SET_FPU: u32 = 0xAE88;
pub const AVMIO_GET_MP_STATE: u32 = 0xAE89;
pub const AVMIO_SET_MP_STATE: u32 = 0xAE8A;
pub const AVMIO_TRANSLATE: u32 = 0xAE8B;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AvmError {
    UnsupportedOnHost,
    ApiMismatch,
    KernelError,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AvmBackendInfo {
    pub api_version: u32,
    pub backend_kind: u32,
    pub feature_bits: u64,
    pub max_vcpus: u32,
    pub exit_info_size: u32,
    pub regs_size: u32,
    pub sregs_size: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AvmUserspaceMemoryRegion {
    pub slot: u32,
    pub flags: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AvmExitInfo {
    pub reason: u32,
    pub hw_reason: u32,
    pub qualification: u64,
    pub guest_phys_addr: u64,
    pub guest_virt_addr: u64,
    pub instruction_len: u32,
    pub io_port: u16,
    pub access_size: u8,
    pub is_read: u8,
    pub io_data: u64,
    pub io_data2: u64,
    pub msr_index: u32,
    pub cpuid_function: u32,
    pub cpuid_index: u32,
    pub cr_number: u8,
    pub cr_is_read: u8,
    pub dr_number: u8,
    pub dr_is_read: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AvmRegs {
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
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AvmSregs {
    pub cs_selector: u16,
    pub cs_base: u64,
    pub cs_limit: u32,
    pub cs_ar: u32,
    pub ds_selector: u16,
    pub ds_base: u64,
    pub ds_limit: u32,
    pub ds_ar: u32,
    pub es_selector: u16,
    pub es_base: u64,
    pub es_limit: u32,
    pub es_ar: u32,
    pub fs_selector: u16,
    pub fs_base: u64,
    pub fs_limit: u32,
    pub fs_ar: u32,
    pub gs_selector: u16,
    pub gs_base: u64,
    pub gs_limit: u32,
    pub gs_ar: u32,
    pub ss_selector: u16,
    pub ss_base: u64,
    pub ss_limit: u32,
    pub ss_ar: u32,
    pub tr_selector: u16,
    pub tr_base: u64,
    pub tr_limit: u32,
    pub tr_ar: u32,
    pub ldtr_selector: u16,
    pub ldtr_base: u64,
    pub ldtr_limit: u32,
    pub ldtr_ar: u32,
    pub gdtr_base: u64,
    pub gdtr_limit: u32,
    pub idtr_base: u64,
    pub idtr_limit: u32,
    pub cr0: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AvmCpuidEntry {
    pub function: u32,
    pub index: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvmFpuState {
    pub fxsave: [u8; 512],
}

impl Default for AvmFpuState {
    fn default() -> Self {
        Self { fxsave: [0; 512] }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AvmTranslate {
    pub gva: u64,
    pub out_gpa: u64,
    pub out_valid: u32,
    pub _pad: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Avm {
    handle: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvmVm {
    handle: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AvmVcpu {
    handle: u64,
}

impl Avm {
    pub const fn new() -> Self {
        Self {
            handle: AVM_SYSTEM_HANDLE,
        }
    }

    pub fn api_version(&self) -> Result<u32, AvmError> {
        let version = avm_ioctl(self.handle, AVMIO_GET_API_VERSION, 0, 0)?;
        Ok(version as u32)
    }

    pub fn require_api_version(&self) -> Result<(), AvmError> {
        if self.api_version()? == AVM_API_VERSION {
            Ok(())
        } else {
            Err(AvmError::ApiMismatch)
        }
    }

    pub fn check_extension(&self, extension: u32) -> Result<bool, AvmError> {
        Ok(avm_ioctl(self.handle, AVMIO_CHECK_EXTENSION, extension as u64, 0)? != 0)
    }

    pub fn backend_info(&self) -> Result<AvmBackendInfo, AvmError> {
        let mut info = AvmBackendInfo::default();
        ioctl_ptr(self.handle, AVMIO_GET_BACKEND_INFO, &mut info)?;
        Ok(info)
    }

    pub fn create_vm(&self) -> Result<AvmVm, AvmError> {
        let handle = avm_ioctl(self.handle, AVMIO_CREATE_VM, 0, 0)?;
        Ok(AvmVm { handle })
    }
}

impl Default for Avm {
    fn default() -> Self {
        Self::new()
    }
}

impl AvmVm {
    pub const fn from_raw_handle(handle: u64) -> Self {
        Self { handle }
    }

    pub const fn raw_handle(&self) -> u64 {
        self.handle
    }

    pub fn set_user_memory_region(&self, region: &AvmUserspaceMemoryRegion) -> Result<(), AvmError> {
        ioctl_ptr_const(self.handle, AVMIO_SET_USER_MEMORY_REGION, region)
    }

    pub fn create_vcpu(&self, vcpu_id: u32) -> Result<AvmVcpu, AvmError> {
        let handle = avm_ioctl(self.handle, AVMIO_CREATE_VCPU, vcpu_id as u64, 0)?;
        Ok(AvmVcpu { handle })
    }

    pub fn set_cpuid(&self, entries: &[AvmCpuidEntry]) -> Result<(), AvmError> {
        if entries.is_empty() {
            return Err(AvmError::KernelError);
        }
        avm_ioctl(
            self.handle,
            AVMIO_SET_CPUID2,
            entries.as_ptr() as u64,
            entries.len() as u64,
        )?;
        Ok(())
    }

    pub fn destroy(&self) -> Result<(), AvmError> {
        avm_ioctl(self.handle, AVMIO_DESTROY_VM, 0, 0)?;
        Ok(())
    }
}

impl AvmVcpu {
    pub const fn from_raw_handle(handle: u64) -> Self {
        Self { handle }
    }

    pub const fn raw_handle(&self) -> u64 {
        self.handle
    }

    pub fn run(&self, exit: &mut AvmExitInfo) -> Result<(), AvmError> {
        ioctl_ptr(self.handle, AVMIO_RUN, exit)
    }

    pub fn regs(&self) -> Result<AvmRegs, AvmError> {
        let mut regs = AvmRegs::default();
        ioctl_ptr(self.handle, AVMIO_GET_REGS, &mut regs)?;
        Ok(regs)
    }

    pub fn set_regs(&self, regs: &AvmRegs) -> Result<(), AvmError> {
        ioctl_ptr_const(self.handle, AVMIO_SET_REGS, regs)
    }

    pub fn sregs(&self) -> Result<AvmSregs, AvmError> {
        let mut sregs = AvmSregs::default();
        ioctl_ptr(self.handle, AVMIO_GET_SREGS, &mut sregs)?;
        Ok(sregs)
    }

    pub fn set_sregs(&self, sregs: &AvmSregs) -> Result<(), AvmError> {
        ioctl_ptr_const(self.handle, AVMIO_SET_SREGS, sregs)
    }

    pub fn pause(&self) -> Result<(), AvmError> {
        avm_ioctl(self.handle, AVMIO_PAUSE, 0, 0)?;
        Ok(())
    }

    pub fn resume(&self) -> Result<(), AvmError> {
        avm_ioctl(self.handle, AVMIO_RESUME, 0, 0)?;
        Ok(())
    }

    pub fn mp_state(&self) -> Result<u32, AvmError> {
        Ok(avm_ioctl(self.handle, AVMIO_GET_MP_STATE, 0, 0)? as u32)
    }

    pub fn set_mp_state(&self, state: u32) -> Result<(), AvmError> {
        avm_ioctl(self.handle, AVMIO_SET_MP_STATE, state as u64, 0)?;
        Ok(())
    }

    pub fn translate(&self, gva: u64) -> Result<Option<u64>, AvmError> {
        let mut req = AvmTranslate {
            gva,
            ..AvmTranslate::default()
        };
        ioctl_ptr(self.handle, AVMIO_TRANSLATE, &mut req)?;
        Ok(if req.out_valid != 0 { Some(req.out_gpa) } else { None })
    }
}

fn ioctl_ptr<T>(handle: u64, request: u32, value: &mut T) -> Result<(), AvmError> {
    avm_ioctl(handle, request, value as *mut T as u64, 0)?;
    Ok(())
}

fn ioctl_ptr_const<T>(handle: u64, request: u32, value: &T) -> Result<(), AvmError> {
    avm_ioctl(handle, request, value as *const T as u64, 0)?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn avm_ioctl(handle: u64, request: u32, arg: u64, flags: u64) -> Result<u64, AvmError> {
    let rc = libsyscall::syscall4(
        libsyscall::SYS_AVM_IOCTL,
        handle,
        request as u64,
        arg,
        flags,
    );
    if rc == u64::MAX {
        Err(AvmError::KernelError)
    } else {
        Ok(rc)
    }
}

#[cfg(target_os = "linux")]
fn avm_ioctl(_handle: u64, _request: u32, _arg: u64, _flags: u64) -> Result<u64, AvmError> {
    Err(AvmError::UnsupportedOnHost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avm_api_uses_kvm_style_handles_and_ioctls() {
        assert_eq!(AVM_SYSTEM_HANDLE, 0);
        assert!(AVMIO_GET_API_VERSION < AVMIO_SET_USER_MEMORY_REGION);
        assert!(AVMIO_SET_USER_MEMORY_REGION < AVMIO_RUN);
    }

    #[test]
    fn memory_region_is_kvm_shaped() {
        let region = AvmUserspaceMemoryRegion {
            slot: 0,
            flags: 0,
            guest_phys_addr: 0x1000,
            memory_size: 0x2000,
            userspace_addr: 0x4000,
        };
        assert_eq!(region.guest_phys_addr, 0x1000);
        assert_eq!(region.userspace_addr, 0x4000);
    }
}
