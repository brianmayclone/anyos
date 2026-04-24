pub(super) const MSR_IA32_TSC: u32 = 0x10;
pub(super) const MSR_IA32_APIC_BASE: u32 = 0x1b;
pub(super) const MSR_IA32_SYSENTER_CS: u32 = 0x174;
pub(super) const MSR_IA32_SYSENTER_ESP: u32 = 0x175;
pub(super) const MSR_IA32_SYSENTER_EIP: u32 = 0x176;
pub(super) const MSR_IA32_PAT: u32 = 0x277;
pub(super) const MSR_IA32_MTRR_DEF_TYPE: u32 = 0x2ff;
pub(super) const MSR_IA32_EFER: u32 = 0xc000_0080;
pub(super) const MSR_IA32_STAR: u32 = 0xc000_0081;
pub(super) const MSR_IA32_LSTAR: u32 = 0xc000_0082;
pub(super) const MSR_IA32_CSTAR: u32 = 0xc000_0083;
pub(super) const MSR_IA32_FMASK: u32 = 0xc000_0084;
pub(super) const MSR_IA32_FS_BASE: u32 = 0xc000_0100;
pub(super) const MSR_IA32_GS_BASE: u32 = 0xc000_0101;
pub(super) const MSR_IA32_KERNEL_GS_BASE: u32 = 0xc000_0102;
pub(super) const MSR_IA32_TSC_AUX: u32 = 0xc000_0103;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct MsrState {
    pub(super) apic_base: u64,
    sysenter_cs: u64,
    sysenter_esp: u64,
    sysenter_eip: u64,
    pat: u64,
    mtrr_def_type: u64,
    pub(super) efer: u64,
    star: u64,
    lstar: u64,
    cstar: u64,
    fmask: u64,
    pub(super) fs_base: u64,
    pub(super) gs_base: u64,
    kernel_gs_base: u64,
    tsc_aux: u64,
    pub(super) xcr0: u64,
}

impl Default for MsrState {
    fn default() -> Self {
        Self {
            apic_base: 0xfee0_0800,
            sysenter_cs: 0,
            sysenter_esp: 0,
            sysenter_eip: 0,
            pat: 0x0007_0406_0007_0406,
            mtrr_def_type: 0,
            efer: 0,
            star: 0,
            lstar: 0,
            cstar: 0,
            fmask: 0,
            fs_base: 0,
            gs_base: 0,
            kernel_gs_base: 0,
            tsc_aux: 0,
            xcr0: 1,
        }
    }
}

pub(super) fn msr_read(state: &MsrState, msr: u32) -> u64 {
    match msr {
        MSR_IA32_TSC => 0,
        MSR_IA32_APIC_BASE => state.apic_base,
        MSR_IA32_SYSENTER_CS => state.sysenter_cs,
        MSR_IA32_SYSENTER_ESP => state.sysenter_esp,
        MSR_IA32_SYSENTER_EIP => state.sysenter_eip,
        MSR_IA32_PAT => state.pat,
        MSR_IA32_MTRR_DEF_TYPE => state.mtrr_def_type,
        MSR_IA32_EFER => state.efer,
        MSR_IA32_STAR => state.star,
        MSR_IA32_LSTAR => state.lstar,
        MSR_IA32_CSTAR => state.cstar,
        MSR_IA32_FMASK => state.fmask,
        MSR_IA32_FS_BASE => state.fs_base,
        MSR_IA32_GS_BASE => state.gs_base,
        MSR_IA32_KERNEL_GS_BASE => state.kernel_gs_base,
        MSR_IA32_TSC_AUX => state.tsc_aux,
        _ => 0,
    }
}

pub(super) fn msr_write(state: &mut MsrState, msr: u32, value: u64) {
    match msr {
        MSR_IA32_TSC => {}
        MSR_IA32_APIC_BASE => state.apic_base = value,
        MSR_IA32_SYSENTER_CS => state.sysenter_cs = value,
        MSR_IA32_SYSENTER_ESP => state.sysenter_esp = value,
        MSR_IA32_SYSENTER_EIP => state.sysenter_eip = value,
        MSR_IA32_PAT => state.pat = value,
        MSR_IA32_MTRR_DEF_TYPE => state.mtrr_def_type = value,
        MSR_IA32_EFER => state.efer = value,
        MSR_IA32_STAR => state.star = value,
        MSR_IA32_LSTAR => state.lstar = value,
        MSR_IA32_CSTAR => state.cstar = value,
        MSR_IA32_FMASK => state.fmask = value,
        MSR_IA32_FS_BASE => state.fs_base = value,
        MSR_IA32_GS_BASE => state.gs_base = value,
        MSR_IA32_KERNEL_GS_BASE => state.kernel_gs_base = value,
        MSR_IA32_TSC_AUX => state.tsc_aux = value,
        _ => {}
    }
}
