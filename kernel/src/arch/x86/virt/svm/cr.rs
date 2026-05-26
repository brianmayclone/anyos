use super::super::{exit_reason, GuestGprs, VmExitInfo};
use super::{SvmVcpu, Vmcb};
use crate::memory::{address::PhysAddr, physmap};

pub(super) const CR_WRITE_INTERCEPTS: u16 = (1 << 0) | (1 << 3) | (1 << 4) | (1 << 8);

const VMEXIT_CR_WRITE_BASE: u64 = 0x010;
const EFER_LME: u64 = 1 << 8;
const EFER_LMA: u64 = 1 << 10;
const CS_ATTR_LONG: u16 = 1 << 9;
const CR0_NE: u64 = 1 << 5;
const CR0_PG: u64 = 1 << 31;

pub(super) fn is_cr_access_exit(exit_code: u64) -> bool {
    matches!(exit_code, 0x000..=0x00f | 0x010..=0x01f)
}

pub(super) fn cr_number(exit_code: u64) -> u8 {
    (exit_code & 0x0f) as u8
}

pub(super) fn cr_is_read(exit_code: u64) -> u8 {
    if exit_code < VMEXIT_CR_WRITE_BASE {
        1
    } else {
        0
    }
}

pub(super) fn emulated_exit(
    exit_code: u64,
    qualification: u64,
    instruction_len: u32,
) -> VmExitInfo {
    VmExitInfo {
        reason: exit_reason::CR_ACCESS_EMULATED,
        hw_reason: exit_code as u32,
        qualification,
        instruction_len,
        cr_number: cr_number(exit_code),
        cr_is_read: cr_is_read(exit_code),
        ..Default::default()
    }
}

pub(super) unsafe fn emulate_cr_access(
    exit_code: u64,
    instruction_len: u32,
    vcpu: &mut SvmVcpu,
    vmcb: &mut Vmcb,
    npt_root: u64,
) -> bool {
    let cr = cr_number(exit_code);
    let mut written = None;
    let decoded_len;
    if exit_code >= VMEXIT_CR_WRITE_BASE {
        let Some(instr) = decode_cr_instruction(vmcb, cr, true, npt_root) else {
            return false;
        };
        decoded_len = instr.len;
        let raw_value = guest_gpr_read(&vcpu.guest_gprs, vmcb, instr.gpr);
        let value = normalize_cr_operand(vmcb, raw_value);
        if !write_guest_cr(vmcb, cr, value) {
            return false;
        }
        written = Some((raw_value, value));
    } else {
        let Some(instr) = decode_cr_instruction(vmcb, cr, false, npt_root) else {
            return false;
        };
        decoded_len = instr.len;
        let Some(mut value) = read_guest_cr(vmcb, cr) else {
            return false;
        };
        value = normalize_cr_operand(vmcb, value);
        guest_gpr_write(&mut vcpu.guest_gprs, vmcb, instr.gpr, value);
    }

    let advance = if (1..=15).contains(&instruction_len) {
        instruction_len
    } else {
        decoded_len
    };
    if advance == 0 {
        return false;
    }

    vmcb.state.rip = vmcb.state.rip.wrapping_add(advance as u64);
    vcpu.guest_gprs.rsp = vmcb.state.rsp;
    if let Some((raw_value, value)) = written {
        crate::serial_verbose_println!(
            "[svm] cr{} write raw={:#x} value={:#x} rip={:#x} cr0={:#x} cr3={:#x} cr4={:#x} efer={:#x}",
            cr,
            raw_value,
            value,
            vmcb.state.rip,
            vmcb.state.cr0,
            vmcb.state.cr3,
            vmcb.state.cr4,
            vmcb.state.efer
        );
    }
    true
}

fn normalize_cr_operand(vmcb: &Vmcb, value: u64) -> u64 {
    if vmcb.state.efer & EFER_LMA != 0 && vmcb.state.cs.attrib & CS_ATTR_LONG != 0 {
        value
    } else {
        value & 0xffff_ffff
    }
}

struct CrInstruction {
    gpr: u8,
    len: u32,
}

unsafe fn decode_cr_instruction(
    vmcb: &Vmcb,
    expected_cr: u8,
    write: bool,
    npt_root: u64,
) -> Option<CrInstruction> {
    let len = vmcb.control.fetched_instruction_len as usize;
    if (3..=vmcb.control.fetched_instruction.len()).contains(&len) {
        let bytes = &vmcb.control.fetched_instruction[..len];
        if let Some(instr) = decode_cr_instruction_bytes(bytes, expected_cr, write) {
            return Some(instr);
        }
    }

    decode_cr_instruction_from_guest(vmcb, expected_cr, write, npt_root)
}

fn decode_cr_instruction_bytes(
    bytes: &[u8],
    expected_cr: u8,
    write: bool,
) -> Option<CrInstruction> {
    let mut index = 0usize;
    let mut rex = 0u8;

    while index < bytes.len() {
        let byte = bytes[index];
        if (0x40..=0x4f).contains(&byte) {
            rex = byte;
            index += 1;
            continue;
        }
        if matches!(
            byte,
            0x26 | 0x2e | 0x36 | 0x3e | 0x64 | 0x65 | 0x66 | 0x67 | 0xf0 | 0xf2 | 0xf3
        ) {
            index += 1;
            continue;
        }
        break;
    }

    if index + 2 >= bytes.len() || bytes[index] != 0x0f {
        return None;
    }
    let opcode = bytes[index + 1];
    let expected_opcode = if write { 0x22 } else { 0x20 };
    if opcode != expected_opcode {
        return None;
    }

    let modrm = bytes[index + 2];
    if (modrm >> 6) != 0b11 {
        return None;
    }
    let cr = ((modrm >> 3) & 0x07) | (((rex >> 2) & 0x01) << 3);
    if cr != expected_cr {
        return None;
    }
    let low = modrm & 0x07;
    let high = (rex & 0x01) << 3;
    Some(CrInstruction {
        gpr: low | high,
        len: (index + 3) as u32,
    })
}

unsafe fn decode_cr_instruction_from_guest(
    vmcb: &Vmcb,
    expected_cr: u8,
    write: bool,
    npt_root: u64,
) -> Option<CrInstruction> {
    let mut bytes = [0u8; 15];
    for offset in 0..bytes.len() {
        let gva = vmcb
            .state
            .cs
            .base
            .wrapping_add(vmcb.state.rip)
            .wrapping_add(offset as u64);
        let gpa = guest_linear_to_physical(vmcb, npt_root, gva)?;
        let hpa = super::super::ept::npt_translate(npt_root, gpa)?;
        bytes[offset] = read_guest_hpa_u8(hpa)?;
        if offset >= 2 {
            if let Some(instr) = decode_cr_instruction_bytes(&bytes[..=offset], expected_cr, write)
            {
                return Some(instr);
            }
        }
    }
    None
}

unsafe fn guest_linear_to_physical(vmcb: &Vmcb, npt_root: u64, gva: u64) -> Option<u64> {
    if vmcb.state.cr0 & CR0_PG == 0 {
        return Some(gva);
    }

    if vmcb.state.efer & (EFER_LME | EFER_LMA) != (EFER_LME | EFER_LMA) {
        return None;
    }

    let pml4e = read_guest_u64(
        npt_root,
        (vmcb.state.cr3 & !0xfff) + (((gva >> 39) & 0x1ff) * 8),
    )?;
    if pml4e & 1 == 0 {
        return None;
    }

    let pdpte = read_guest_u64(
        npt_root,
        (pml4e & 0x000f_ffff_ffff_f000) + (((gva >> 30) & 0x1ff) * 8),
    )?;
    if pdpte & 1 == 0 {
        return None;
    }
    if pdpte & (1 << 7) != 0 {
        return Some((pdpte & 0x000f_fffc_0000_0000) | (gva & 0x3fff_ffff));
    }

    let pde = read_guest_u64(
        npt_root,
        (pdpte & 0x000f_ffff_ffff_f000) + (((gva >> 21) & 0x1ff) * 8),
    )?;
    if pde & 1 == 0 {
        return None;
    }
    if pde & (1 << 7) != 0 {
        return Some((pde & 0x000f_ffff_ffe0_0000) | (gva & 0x1f_ffff));
    }

    let pte = read_guest_u64(
        npt_root,
        (pde & 0x000f_ffff_ffff_f000) + (((gva >> 12) & 0x1ff) * 8),
    )?;
    if pte & 1 == 0 {
        return None;
    }
    Some((pte & 0x000f_ffff_ffff_f000) | (gva & 0xfff))
}

unsafe fn read_guest_u64(npt_root: u64, gpa: u64) -> Option<u64> {
    if (gpa & 0xfff) > 0xff8 {
        return None;
    }
    let hpa = super::super::ept::npt_translate(npt_root, gpa)?;
    read_guest_hpa_u64(hpa)
}

fn read_guest_hpa_u8(hpa: u64) -> Option<u8> {
    physmap::phys_to_virt(PhysAddr::new(hpa))
        .map(|ptr| unsafe { core::ptr::read_volatile(ptr as *const u8) })
}

fn read_guest_hpa_u64(hpa: u64) -> Option<u64> {
    physmap::phys_to_virt(PhysAddr::new(hpa))
        .map(|ptr| unsafe { core::ptr::read_unaligned(ptr as *const u64) })
}

unsafe fn read_guest_cr(vmcb: &Vmcb, cr: u8) -> Option<u64> {
    match cr {
        0 => Some(vmcb.state.cr0),
        3 => Some(vmcb.state.cr3),
        4 => Some(vmcb.state.cr4),
        8 => Some(0),
        _ => None,
    }
}

unsafe fn write_guest_cr(vmcb: &mut Vmcb, cr: u8, value: u64) -> bool {
    match cr {
        0 => write_guest_cr0(vmcb, value),
        3 => vmcb.state.cr3 = value,
        4 => vmcb.state.cr4 = value,
        8 => {}
        _ => return false,
    }
    vmcb.control.tlb_control = 1;
    true
}

unsafe fn write_guest_cr0(vmcb: &mut Vmcb, value: u64) {
    let cr0 = value | CR0_NE;
    vmcb.state.cr0 = cr0;
    if (cr0 & CR0_PG) != 0 && (vmcb.state.efer & EFER_LME) != 0 {
        vmcb.state.efer |= EFER_LMA;
    } else {
        vmcb.state.efer &= !EFER_LMA;
    }
}

unsafe fn guest_gpr_read(gprs: &GuestGprs, vmcb: &Vmcb, reg: u8) -> u64 {
    match reg {
        0 => gprs.rax,
        1 => gprs.rcx,
        2 => gprs.rdx,
        3 => gprs.rbx,
        4 => vmcb.state.rsp,
        5 => gprs.rbp,
        6 => gprs.rsi,
        7 => gprs.rdi,
        8 => gprs.r8,
        9 => gprs.r9,
        10 => gprs.r10,
        11 => gprs.r11,
        12 => gprs.r12,
        13 => gprs.r13,
        14 => gprs.r14,
        15 => gprs.r15,
        _ => 0,
    }
}

unsafe fn guest_gpr_write(gprs: &mut GuestGprs, vmcb: &mut Vmcb, reg: u8, value: u64) {
    match reg {
        0 => gprs.rax = value,
        1 => gprs.rcx = value,
        2 => gprs.rdx = value,
        3 => gprs.rbx = value,
        4 => {
            gprs.rsp = value;
            vmcb.state.rsp = value;
        }
        5 => gprs.rbp = value,
        6 => gprs.rsi = value,
        7 => gprs.rdi = value,
        8 => gprs.r8 = value,
        9 => gprs.r9 = value,
        10 => gprs.r10 = value,
        11 => gprs.r11 = value,
        12 => gprs.r12 = value,
        13 => gprs.r13 = value,
        14 => gprs.r14 = value,
        15 => gprs.r15 = value,
        _ => {}
    }
}
