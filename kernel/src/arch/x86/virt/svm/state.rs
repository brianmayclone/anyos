use super::super::{GuestFpuState, GuestGprs, GuestSregs, VcpuMpState};
use super::{
    avm_efer_to_svm, avm_segment_attr_to_svm, find_vm, find_vm_mut, svm_efer_to_avm,
    svm_segment_attr_to_avm, vcpu_index, Vmcb, VmcbSegment, DIRTY_LOG_SLOTS,
    DIRTY_LOG_WORDS_PER_SLOT, VMS,
};

pub fn get_regs(vm_id: u32, vcpu_id: u32) -> Option<GuestGprs> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_index(vcpu_id)?].as_ref()?;
    let mut gprs = vcpu.guest_gprs;
    unsafe {
        let vmcb = &*(super::super::phys_to_virt(vcpu.vmcb_phys) as *const Vmcb);
        gprs.rax = vmcb.state.rax;
        gprs.rsp = vmcb.state.rsp;
    }
    Some(gprs)
}

pub fn set_regs(vm_id: u32, vcpu_id: u32, gprs: &GuestGprs) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let idx = match vcpu_index(vcpu_id) {
        Some(idx) => idx,
        None => return false,
    };
    let vcpu = match vm.vcpus[idx].as_mut() {
        Some(v) => v,
        None => return false,
    };
    vcpu.guest_gprs = *gprs;
    unsafe {
        let vmcb = &mut *(super::super::phys_to_virt(vcpu.vmcb_phys) as *mut Vmcb);
        vmcb.state.rax = gprs.rax;
        vmcb.state.rsp = gprs.rsp;
    }
    true
}

pub fn get_sregs(vm_id: u32, vcpu_id: u32) -> Option<GuestSregs> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_index(vcpu_id)?].as_ref()?;

    unsafe {
        let vmcb = &*(super::super::phys_to_virt(vcpu.vmcb_phys) as *const Vmcb);
        Some(GuestSregs {
            cs_selector: vmcb.state.cs.selector,
            cs_base: vmcb.state.cs.base,
            cs_limit: vmcb.state.cs.limit,
            cs_ar: svm_segment_attr_to_avm(vmcb.state.cs.attrib),
            ds_selector: vmcb.state.ds.selector,
            ds_base: vmcb.state.ds.base,
            ds_limit: vmcb.state.ds.limit,
            ds_ar: svm_segment_attr_to_avm(vmcb.state.ds.attrib),
            es_selector: vmcb.state.es.selector,
            es_base: vmcb.state.es.base,
            es_limit: vmcb.state.es.limit,
            es_ar: svm_segment_attr_to_avm(vmcb.state.es.attrib),
            fs_selector: vmcb.state.fs.selector,
            fs_base: vmcb.state.fs.base,
            fs_limit: vmcb.state.fs.limit,
            fs_ar: svm_segment_attr_to_avm(vmcb.state.fs.attrib),
            gs_selector: vmcb.state.gs.selector,
            gs_base: vmcb.state.gs.base,
            gs_limit: vmcb.state.gs.limit,
            gs_ar: svm_segment_attr_to_avm(vmcb.state.gs.attrib),
            ss_selector: vmcb.state.ss.selector,
            ss_base: vmcb.state.ss.base,
            ss_limit: vmcb.state.ss.limit,
            ss_ar: svm_segment_attr_to_avm(vmcb.state.ss.attrib),
            tr_selector: vmcb.state.tr.selector,
            tr_base: vmcb.state.tr.base,
            tr_limit: vmcb.state.tr.limit,
            tr_ar: svm_segment_attr_to_avm(vmcb.state.tr.attrib),
            ldtr_selector: vmcb.state.ldtr.selector,
            ldtr_base: vmcb.state.ldtr.base,
            ldtr_limit: vmcb.state.ldtr.limit,
            ldtr_ar: svm_segment_attr_to_avm(vmcb.state.ldtr.attrib),
            gdtr_base: vmcb.state.gdtr.base,
            gdtr_limit: vmcb.state.gdtr.limit,
            idtr_base: vmcb.state.idtr.base,
            idtr_limit: vmcb.state.idtr.limit,
            cr0: vmcb.state.cr0,
            cr3: vmcb.state.cr3,
            cr4: vmcb.state.cr4,
            efer: svm_efer_to_avm(vmcb.state.efer),
            rip: vmcb.state.rip,
            rsp: vmcb.state.rsp,
            rflags: vmcb.state.rflags,
        })
    }
}

pub fn set_sregs(vm_id: u32, vcpu_id: u32, sregs: &GuestSregs) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let idx = match vcpu_index(vcpu_id) {
        Some(idx) => idx,
        None => return false,
    };
    let vcpu = match vm.vcpus[idx].as_mut() {
        Some(v) => v,
        None => return false,
    };

    unsafe {
        let vmcb = &mut *(super::super::phys_to_virt(vcpu.vmcb_phys) as *mut Vmcb);
        vmcb.state.cs = VmcbSegment {
            selector: sregs.cs_selector,
            base: sregs.cs_base,
            limit: sregs.cs_limit,
            attrib: avm_segment_attr_to_svm(sregs.cs_ar),
        };
        vmcb.state.ds = VmcbSegment {
            selector: sregs.ds_selector,
            base: sregs.ds_base,
            limit: sregs.ds_limit,
            attrib: avm_segment_attr_to_svm(sregs.ds_ar),
        };
        vmcb.state.es = VmcbSegment {
            selector: sregs.es_selector,
            base: sregs.es_base,
            limit: sregs.es_limit,
            attrib: avm_segment_attr_to_svm(sregs.es_ar),
        };
        vmcb.state.fs = VmcbSegment {
            selector: sregs.fs_selector,
            base: sregs.fs_base,
            limit: sregs.fs_limit,
            attrib: avm_segment_attr_to_svm(sregs.fs_ar),
        };
        vmcb.state.gs = VmcbSegment {
            selector: sregs.gs_selector,
            base: sregs.gs_base,
            limit: sregs.gs_limit,
            attrib: avm_segment_attr_to_svm(sregs.gs_ar),
        };
        vmcb.state.ss = VmcbSegment {
            selector: sregs.ss_selector,
            base: sregs.ss_base,
            limit: sregs.ss_limit,
            attrib: avm_segment_attr_to_svm(sregs.ss_ar),
        };
        vmcb.state.tr = VmcbSegment {
            selector: sregs.tr_selector,
            base: sregs.tr_base,
            limit: sregs.tr_limit,
            attrib: avm_segment_attr_to_svm(sregs.tr_ar),
        };
        vmcb.state.ldtr = VmcbSegment {
            selector: sregs.ldtr_selector,
            base: sregs.ldtr_base,
            limit: sregs.ldtr_limit,
            attrib: avm_segment_attr_to_svm(sregs.ldtr_ar),
        };
        vmcb.state.gdtr.base = sregs.gdtr_base;
        vmcb.state.gdtr.limit = sregs.gdtr_limit;
        vmcb.state.idtr.base = sregs.idtr_base;
        vmcb.state.idtr.limit = sregs.idtr_limit;
        vmcb.state.cr0 = sregs.cr0;
        vmcb.state.cr3 = sregs.cr3;
        vmcb.state.cr4 = sregs.cr4;
        vmcb.state.efer = avm_efer_to_svm(sregs.efer);
        vmcb.state.rip = sregs.rip;
        vmcb.state.rsp = sregs.rsp;
        vmcb.state.rflags = sregs.rflags;
    }
    vcpu.guest_gprs.rsp = sregs.rsp;

    true
}

pub fn inject_irq(vm_id: u32, vcpu_id: u32, vector: u8) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let idx = match vcpu_index(vcpu_id) {
        Some(idx) => idx,
        None => return false,
    };
    let vcpu = match vm.vcpus[idx].as_mut() {
        Some(v) => v,
        None => return false,
    };

    unsafe {
        let vmcb = &mut *(super::super::phys_to_virt(vcpu.vmcb_phys) as *mut Vmcb);
        vmcb.control.event_inj = (vector as u64) | (1u64 << 31);
    }
    if vcpu.mp_state == VcpuMpState::Halted {
        vcpu.mp_state = VcpuMpState::Runnable;
    }
    true
}

pub fn inject_exception(vm_id: u32, vcpu_id: u32, vector: u8, error_code: u32) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let idx = match vcpu_index(vcpu_id) {
        Some(idx) => idx,
        None => return false,
    };
    let vcpu = match vm.vcpus[idx].as_mut() {
        Some(v) => v,
        None => return false,
    };

    unsafe {
        let vmcb = &mut *(super::super::phys_to_virt(vcpu.vmcb_phys) as *mut Vmcb);
        let has_error = matches!(vector, 8 | 10 | 11 | 12 | 13 | 14 | 17);
        let mut info: u64 = (vector as u64) | (3 << 8) | (1u64 << 31);
        if has_error {
            info |= (1u64 << 11) | ((error_code as u64) << 32);
        }
        vmcb.control.event_inj = info;
    }
    if vcpu.mp_state == VcpuMpState::Halted {
        vcpu.mp_state = VcpuMpState::Runnable;
    }
    true
}

pub fn inject_nmi(vm_id: u32, vcpu_id: u32) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let idx = match vcpu_index(vcpu_id) {
        Some(idx) => idx,
        None => return false,
    };
    let vcpu = match vm.vcpus[idx].as_mut() {
        Some(v) => v,
        None => return false,
    };

    unsafe {
        let vmcb = &mut *(super::super::phys_to_virt(vcpu.vmcb_phys) as *mut Vmcb);
        vmcb.control.event_inj = 2 | (2u64 << 8) | (1u64 << 31);
    }
    if vcpu.mp_state == VcpuMpState::Halted {
        vcpu.mp_state = VcpuMpState::Runnable;
    }
    true
}

pub fn vcpu_pause(vm_id: u32, vcpu_id: u32) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let idx = match vcpu_index(vcpu_id) {
        Some(idx) => idx,
        None => return false,
    };
    let vcpu = match vm.vcpus[idx].as_mut() {
        Some(v) => v,
        None => return false,
    };
    vcpu.paused = true;
    true
}

pub fn vcpu_resume(vm_id: u32, vcpu_id: u32) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let idx = match vcpu_index(vcpu_id) {
        Some(idx) => idx,
        None => return false,
    };
    let vcpu = match vm.vcpus[idx].as_mut() {
        Some(v) => v,
        None => return false,
    };
    vcpu.paused = false;
    if vcpu.mp_state == VcpuMpState::Halted {
        vcpu.mp_state = VcpuMpState::Runnable;
    }
    true
}

pub fn get_fpu(vm_id: u32, vcpu_id: u32) -> Option<GuestFpuState> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_index(vcpu_id)?].as_ref()?;
    Some(vcpu.guest_fpu)
}

pub fn set_fpu(vm_id: u32, vcpu_id: u32, fpu: &GuestFpuState) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let idx = match vcpu_index(vcpu_id) {
        Some(idx) => idx,
        None => return false,
    };
    let vcpu = match vm.vcpus[idx].as_mut() {
        Some(v) => v,
        None => return false,
    };
    vcpu.guest_fpu = *fpu;
    true
}

pub fn get_mp_state(vm_id: u32, vcpu_id: u32) -> Option<VcpuMpState> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_index(vcpu_id)?].as_ref()?;
    Some(vcpu.mp_state)
}

pub fn set_mp_state(vm_id: u32, vcpu_id: u32, state: VcpuMpState) -> bool {
    let mut vms = VMS.lock();
    let vm = match find_vm_mut(&mut vms, vm_id) {
        Some(v) => v,
        None => return false,
    };
    let idx = match vcpu_index(vcpu_id) {
        Some(idx) => idx,
        None => return false,
    };
    let vcpu = match vm.vcpus[idx].as_mut() {
        Some(v) => v,
        None => return false,
    };
    vcpu.mp_state = state;
    true
}

pub fn translate_gva(vm_id: u32, vcpu_id: u32, gva: u64) -> Option<u64> {
    let vms = VMS.lock();
    let vm = find_vm(&vms, vm_id)?;
    let vcpu = vm.vcpus[vcpu_index(vcpu_id)?].as_ref()?;

    unsafe {
        let vmcb = &*(super::super::phys_to_virt(vcpu.vmcb_phys) as *const Vmcb);
        let cr3 = vmcb.state.cr3 & !0xFFF;
        let efer = vmcb.state.efer;

        if efer & 0x500 != 0x500 {
            return None;
        }

        let walk_gpa = |table_gpa: u64, idx: usize| -> Option<u64> {
            let hpa = super::super::ept::npt_translate(vm.npt_root, table_gpa)?;
            let virt = super::super::phys_to_virt(hpa & !0xFFF) as *const u64;
            Some(unsafe { *virt.add(idx) })
        };

        let pml4_idx = ((gva >> 39) & 0x1FF) as usize;
        let pml4e = walk_gpa(cr3, pml4_idx)?;
        if pml4e & 1 == 0 {
            return None;
        }

        let pdpt_gpa = pml4e & 0x000F_FFFF_FFFF_F000;
        let pdpt_idx = ((gva >> 30) & 0x1FF) as usize;
        let pdpte = walk_gpa(pdpt_gpa, pdpt_idx)?;
        if pdpte & 1 == 0 {
            return None;
        }
        if pdpte & (1 << 7) != 0 {
            return Some((pdpte & 0x000F_FFFC_0000_0000) | (gva & 0x3FFF_FFFF));
        }

        let pd_gpa = pdpte & 0x000F_FFFF_FFFF_F000;
        let pd_idx = ((gva >> 21) & 0x1FF) as usize;
        let pde = walk_gpa(pd_gpa, pd_idx)?;
        if pde & 1 == 0 {
            return None;
        }
        if pde & (1 << 7) != 0 {
            return Some((pde & 0x000F_FFFF_FFE0_0000) | (gva & 0x1F_FFFF));
        }

        let pt_gpa = pde & 0x000F_FFFF_FFFF_F000;
        let pt_idx = ((gva >> 12) & 0x1FF) as usize;
        let pte = walk_gpa(pt_gpa, pt_idx)?;
        if pte & 1 == 0 {
            return None;
        }
        Some((pte & 0x000F_FFFF_FFFF_F000) | (gva & 0xFFF))
    }
}

pub fn get_dirty_log(vm_id: u32, slot: u32, bitmap: &mut [u64]) -> Option<u32> {
    let mut vms = VMS.lock();
    let vm = find_vm_mut(&mut vms, vm_id)?;
    let slot_idx = slot as usize;
    if slot_idx >= DIRTY_LOG_SLOTS {
        return None;
    }

    let slot_offset = slot_idx * DIRTY_LOG_WORDS_PER_SLOT;
    let copy_words = bitmap.len().min(DIRTY_LOG_WORDS_PER_SLOT);
    let mut dirty_count = 0u32;
    for i in 0..copy_words {
        let w = vm.dirty_log[slot_offset + i];
        bitmap[i] = w;
        dirty_count += w.count_ones();
    }
    for i in 0..DIRTY_LOG_WORDS_PER_SLOT {
        vm.dirty_log[slot_offset + i] = 0;
    }
    Some(dirty_count)
}
