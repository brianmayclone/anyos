use crate::memory::{address::PhysAddr, physmap};

use super::{SvmVcpu, Vmcb};

pub(super) fn log_shutdown(npt_root: u64, vcpu: &SvmVcpu, vmcb: &Vmcb) {
    let rip_gpa = vmcb.state.cs.base.wrapping_add(vmcb.state.rip);
    let rip_hpa = super::super::ept::npt_translate(npt_root, rip_gpa).unwrap_or(0);
    let rip_bytes = read_guest_hpa_u64(rip_hpa);
    crate::serial_println!(
        "[svm] shutdown: rip={:#x} gpa={:#x} hpa={:#x} bytes={:#x} cr0={:#x} cr3={:#x} cr4={:#x} efer={:#x} cs={:#x}/{:#x} ds={:#x}/{:#x} ss={:#x}/{:#x} fs={:#x}/{:#x} gs={:#x}/{:#x} gdtr={:#x}:{:#x} idtr={:#x}:{:#x} rsi={:#x} rsp={:#x}",
        vmcb.state.rip,
        rip_gpa,
        rip_hpa,
        rip_bytes,
        vmcb.state.cr0,
        vmcb.state.cr3,
        vmcb.state.cr4,
        vmcb.state.efer,
        vmcb.state.cs.selector,
        vmcb.state.cs.attrib,
        vmcb.state.ds.selector,
        vmcb.state.ds.attrib,
        vmcb.state.ss.selector,
        vmcb.state.ss.attrib,
        vmcb.state.fs.selector,
        vmcb.state.fs.attrib,
        vmcb.state.gs.selector,
        vmcb.state.gs.attrib,
        vmcb.state.gdtr.base,
        vmcb.state.gdtr.limit,
        vmcb.state.idtr.base,
        vmcb.state.idtr.limit,
        vcpu.guest_gprs.rsi,
        vmcb.state.rsp
    );
}

fn read_guest_hpa_u64(hpa: u64) -> u64 {
    if hpa == 0 || (hpa & 0xfff) > 0xff8 {
        return 0;
    }
    physmap::phys_to_virt(PhysAddr::new(hpa))
        .map(|ptr| unsafe { core::ptr::read_unaligned(ptr as *const u64) })
        .unwrap_or(0)
}
