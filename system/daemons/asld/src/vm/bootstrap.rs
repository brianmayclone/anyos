use crate::errors::AsldError;
#[cfg(not(target_os = "linux"))]
use crate::model::DistroConfig;

use super::{
    BOOT_CODE_ADDR, BOOT_PDPT_ADDR, BOOT_PD_ADDR, BOOT_PML4_ADDR, BOOT_STACK_GUARD, PAGE_SIZE,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BootstrapLayout {
    pml4_addr: usize,
    pdpt_addr: usize,
    pd_addr: usize,
    code_addr: usize,
    stack_top: usize,
}

impl BootstrapLayout {
    pub(super) fn new(guest_memory_size: usize) -> Result<Self, AsldError> {
        let min_required = BOOT_CODE_ADDR + PAGE_SIZE;
        if guest_memory_size < min_required + BOOT_STACK_GUARD {
            return Err(AsldError::InvalidState(
                "guest memory too small for bootstrap",
            ));
        }

        let stack_top = guest_memory_size - BOOT_STACK_GUARD;
        Ok(Self {
            pml4_addr: BOOT_PML4_ADDR,
            pdpt_addr: BOOT_PDPT_ADDR,
            pd_addr: BOOT_PD_ADDR,
            code_addr: BOOT_CODE_ADDR,
            stack_top: stack_top & !0xf,
        })
    }
}

type GuestGprs = libavm::AvmRegs;
type GuestSregs = libavm::AvmSregs;

pub(super) fn bootstrap_gprs() -> GuestGprs {
    GuestGprs::default()
}

pub(super) fn direct_linux_gprs(layout: &crate::boot::DirectLinuxLayout) -> GuestGprs {
    let mut regs = GuestGprs::default();
    regs.rsi = layout.boot_params_addr as u64;
    regs.rsp = 0x90000;
    regs
}

pub(super) fn seabios_gprs(_layout: &crate::boot::SeaBiosLayout) -> GuestGprs {
    GuestGprs::default()
}

pub(super) fn seabios_sregs(layout: &crate::boot::SeaBiosLayout) -> GuestSregs {
    const REAL_CODE_AR: u32 = 0x009B;
    const REAL_DATA_AR: u32 = 0x0093;
    const REAL_TSS_AR: u32 = 0x008B;
    const NULL_SEGMENT_AR: u32 = 0x10000;
    const CR0_RESET: u64 = 0x6000_0030;

    GuestSregs {
        cs_selector: 0xf000,
        cs_base: 0xf0000,
        cs_limit: 0xffff,
        cs_ar: REAL_CODE_AR,
        ds_selector: 0,
        ds_base: 0,
        ds_limit: 0xffff,
        ds_ar: REAL_DATA_AR,
        es_selector: 0,
        es_base: 0,
        es_limit: 0xffff,
        es_ar: REAL_DATA_AR,
        fs_selector: 0,
        fs_base: 0,
        fs_limit: 0xffff,
        fs_ar: REAL_DATA_AR,
        gs_selector: 0,
        gs_base: 0,
        gs_limit: 0xffff,
        gs_ar: REAL_DATA_AR,
        ss_selector: 0,
        ss_base: 0,
        ss_limit: 0xffff,
        ss_ar: REAL_DATA_AR,
        tr_selector: 0,
        tr_base: 0,
        tr_limit: 0xffff,
        tr_ar: REAL_TSS_AR,
        ldtr_selector: 0,
        ldtr_base: 0,
        ldtr_limit: 0,
        ldtr_ar: NULL_SEGMENT_AR,
        gdtr_base: 0,
        gdtr_limit: 0xffff,
        idtr_base: 0,
        idtr_limit: 0x03ff,
        cr0: CR0_RESET,
        cr3: 0,
        cr4: 0,
        efer: 0,
        rip: (layout.reset_vector - 0xf0000) as u64,
        rsp: 0,
        rflags: 0x2,
    }
}

pub(super) fn direct_linux_sregs(layout: &crate::boot::DirectLinuxLayout) -> GuestSregs {
    const CODE_SEGMENT_AR: u32 = 0xC09B;
    const DATA_SEGMENT_AR: u32 = 0xC093;
    const TSS_SEGMENT_AR: u32 = 0x808B;
    const NULL_SEGMENT_AR: u32 = 0x10000;
    const CR0_PE: u64 = 1 << 0;
    const CR0_ET: u64 = 1 << 4;
    const CR0_NE: u64 = 1 << 5;
    const SEGMENT_LIMIT: u32 = 0xFFFFF;

    GuestSregs {
        cs_selector: 0x08,
        cs_base: 0,
        cs_limit: SEGMENT_LIMIT,
        cs_ar: CODE_SEGMENT_AR,
        ds_selector: 0x10,
        ds_base: 0,
        ds_limit: SEGMENT_LIMIT,
        ds_ar: DATA_SEGMENT_AR,
        es_selector: 0x10,
        es_base: 0,
        es_limit: SEGMENT_LIMIT,
        es_ar: DATA_SEGMENT_AR,
        fs_selector: 0x10,
        fs_base: 0,
        fs_limit: SEGMENT_LIMIT,
        fs_ar: DATA_SEGMENT_AR,
        gs_selector: 0x10,
        gs_base: 0,
        gs_limit: SEGMENT_LIMIT,
        gs_ar: DATA_SEGMENT_AR,
        ss_selector: 0x10,
        ss_base: 0,
        ss_limit: SEGMENT_LIMIT,
        ss_ar: DATA_SEGMENT_AR,
        tr_selector: 0x18,
        tr_base: 0,
        tr_limit: 0x67,
        tr_ar: TSS_SEGMENT_AR,
        ldtr_selector: 0,
        ldtr_base: 0,
        ldtr_limit: 0,
        ldtr_ar: NULL_SEGMENT_AR,
        gdtr_base: 0,
        gdtr_limit: 0,
        idtr_base: 0,
        idtr_limit: 0,
        cr0: CR0_PE | CR0_ET | CR0_NE,
        cr3: 0,
        cr4: 0,
        efer: 0,
        rip: layout.kernel_entry_addr as u64,
        rsp: 0x90000,
        rflags: 0x2,
    }
}

pub(super) fn bootstrap_sregs(layout: &BootstrapLayout) -> GuestSregs {
    const CODE_SEGMENT_AR: u32 = 0xA09B;
    const DATA_SEGMENT_AR: u32 = 0xC093;
    const TSS_SEGMENT_AR: u32 = 0x808B;
    const NULL_SEGMENT_AR: u32 = 0x10000;
    const CR0_PE: u64 = 1 << 0;
    const CR0_ET: u64 = 1 << 4;
    const CR0_NE: u64 = 1 << 5;
    const CR0_PG: u64 = 1 << 31;
    const CR4_PAE: u64 = 1 << 5;
    const EFER_LME: u64 = 1 << 8;
    const EFER_LMA: u64 = 1 << 10;
    const SEGMENT_LIMIT: u32 = 0xFFFFF;

    GuestSregs {
        cs_selector: 0x08,
        cs_base: 0,
        cs_limit: SEGMENT_LIMIT,
        cs_ar: CODE_SEGMENT_AR,
        ds_selector: 0x10,
        ds_base: 0,
        ds_limit: SEGMENT_LIMIT,
        ds_ar: DATA_SEGMENT_AR,
        es_selector: 0x10,
        es_base: 0,
        es_limit: SEGMENT_LIMIT,
        es_ar: DATA_SEGMENT_AR,
        fs_selector: 0x10,
        fs_base: 0,
        fs_limit: SEGMENT_LIMIT,
        fs_ar: DATA_SEGMENT_AR,
        gs_selector: 0x10,
        gs_base: 0,
        gs_limit: SEGMENT_LIMIT,
        gs_ar: DATA_SEGMENT_AR,
        ss_selector: 0x10,
        ss_base: 0,
        ss_limit: SEGMENT_LIMIT,
        ss_ar: DATA_SEGMENT_AR,
        tr_selector: 0x18,
        tr_base: 0,
        tr_limit: 0x67,
        tr_ar: TSS_SEGMENT_AR,
        ldtr_selector: 0,
        ldtr_base: 0,
        ldtr_limit: 0,
        ldtr_ar: NULL_SEGMENT_AR,
        gdtr_base: 0,
        gdtr_limit: 0,
        idtr_base: 0,
        idtr_limit: 0,
        cr0: CR0_PE | CR0_ET | CR0_NE | CR0_PG,
        cr3: layout.pml4_addr as u64,
        cr4: CR4_PAE,
        efer: EFER_LME | EFER_LMA,
        rip: layout.code_addr as u64,
        rsp: layout.stack_top as u64,
        rflags: 0x2,
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn configure_direct_linux_vcpu(
    config: &DistroConfig,
    vcpu: &libavm::AvmVcpu,
    guest_memory: *mut u8,
    guest_memory_size: usize,
) -> Result<(), AsldError> {
    let layout = crate::boot::prepare_direct_linux_boot(config, guest_memory, guest_memory_size)?;
    let regs = direct_linux_gprs(&layout);
    let sregs = direct_linux_sregs(&layout);

    if vcpu.set_regs(&regs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_regs failed"));
    }
    if vcpu.set_sregs(&sregs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_sregs failed"));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn configure_seabios_vcpu(
    vcpu: &libavm::AvmVcpu,
    guest_memory: *mut u8,
    guest_memory_size: usize,
) -> Result<(), AsldError> {
    let layout = crate::boot::prepare_seabios_boot(guest_memory, guest_memory_size)?;
    let regs = seabios_gprs(&layout);
    let sregs = seabios_sregs(&layout);

    if vcpu.set_regs(&regs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_regs failed"));
    }
    if vcpu.set_sregs(&sregs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_sregs failed"));
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn configure_boot_vcpu(
    vcpu: &libavm::AvmVcpu,
    guest_memory: *mut u8,
    guest_memory_size: usize,
) -> Result<(), AsldError> {
    let bootstrap = BootstrapLayout::new(guest_memory_size)?;
    write_bootstrap_image(guest_memory, guest_memory_size, &bootstrap)?;
    let regs = bootstrap_gprs();
    let sregs = bootstrap_sregs(&bootstrap);

    if vcpu.set_regs(&regs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_regs failed"));
    }

    if vcpu.set_sregs(&sregs).is_err() {
        return Err(AsldError::BackendUnavailable("avm set_sregs failed"));
    }

    Ok(())
}

pub(super) fn write_bootstrap_image(
    guest_memory: *mut u8,
    guest_memory_size: usize,
    layout: &BootstrapLayout,
) -> Result<(), AsldError> {
    let buffer = unsafe { core::slice::from_raw_parts_mut(guest_memory, guest_memory_size) };
    if layout.stack_top <= layout.code_addr + 16 {
        return Err(AsldError::InvalidState("bootstrap layout overlaps stack"));
    }

    {
        let pml4 = page_mut(buffer, layout.pml4_addr)?;
        zero_page(pml4);
        write_u64(pml4, 0, (layout.pdpt_addr as u64) | 0x3);
    }
    {
        let pdpt = page_mut(buffer, layout.pdpt_addr)?;
        zero_page(pdpt);
        write_u64(pdpt, 0, (layout.pd_addr as u64) | 0x3);
    }
    {
        let pd = page_mut(buffer, layout.pd_addr)?;
        zero_page(pd);
        for index in 0..512usize {
            let guest_phys = (index as u64) * 0x20_0000;
            write_u64(pd, index, guest_phys | 0x83);
        }
    }

    let code = slice_at_mut(buffer, layout.code_addr, 4)?;
    code.copy_from_slice(&[0xfa, 0xf4, 0xeb, 0xfd]);
    Ok(())
}

pub(super) fn page_mut(buffer: &mut [u8], addr: usize) -> Result<&mut [u8], AsldError> {
    slice_at_mut(buffer, addr, PAGE_SIZE)
}

fn slice_at_mut(buffer: &mut [u8], addr: usize, len: usize) -> Result<&mut [u8], AsldError> {
    let end = addr
        .checked_add(len)
        .ok_or(AsldError::InvalidState("bootstrap layout overflow"))?;
    buffer
        .get_mut(addr..end)
        .ok_or(AsldError::InvalidState("bootstrap layout out of bounds"))
}

fn zero_page(page: &mut [u8]) {
    page.fill(0);
}

fn write_u64(page: &mut [u8], index: usize, value: u64) {
    let offset = index * core::mem::size_of::<u64>();
    page[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
