use crate::errors::AsldError;

use super::VmExitInfo;
#[cfg(not(target_os = "linux"))]
use super::{memory::read_guest_bytes, VmInstance};

const MAX_INSTRUCTION_BYTES: usize = 15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Register {
    Gpr(u8),
    High8(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MmioAccess {
    Read {
        dest: Register,
        width: u8,
        instruction_len: u32,
    },
    Write {
        value: u64,
        width: u8,
        instruction_len: u32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PreparedMmioExit {
    pub(super) exit: VmExitInfo,
    access: Option<MmioAccess>,
}

impl PreparedMmioExit {
    pub(super) fn instruction_len(&self) -> u32 {
        match self.access {
            Some(MmioAccess::Read {
                instruction_len, ..
            })
            | Some(MmioAccess::Write {
                instruction_len, ..
            }) => instruction_len,
            None => self.exit.instruction_len,
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(super) fn prepare_mmio_exit(
    instance: &VmInstance,
    vcpu: &libavm::AvmVcpu,
    exit: &VmExitInfo,
) -> Result<PreparedMmioExit, AsldError> {
    let mut prepared = PreparedMmioExit {
        exit: *exit,
        access: None,
    };
    let regs = vcpu
        .regs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_regs failed"))?;
    let sregs = vcpu
        .sregs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_sregs failed"))?;

    let mut instruction = [0u8; MAX_INSTRUCTION_BYTES];
    for index in 0..MAX_INSTRUCTION_BYTES {
        read_guest_bytes(
            instance,
            vcpu,
            sregs.rip.wrapping_add(index as u64),
            &mut instruction[index..index + 1],
        )?;
    }

    if let Some(access) = decode_mmio_access(&instruction, &regs, sregs.efer) {
        match access {
            MmioAccess::Read {
                width,
                instruction_len,
                ..
            } => {
                prepared.exit.is_read = 1;
                prepared.exit.access_size = width;
                prepared.exit.instruction_len = instruction_len;
            }
            MmioAccess::Write {
                value,
                width,
                instruction_len,
            } => {
                prepared.exit.is_read = 0;
                prepared.exit.access_size = width;
                prepared.exit.instruction_len = instruction_len;
                prepared.exit.io_data = value;
            }
        }
        prepared.access = Some(access);
    }

    Ok(prepared)
}

pub(super) fn complete_mmio_read(
    vcpu: &libavm::AvmVcpu,
    prepared: &PreparedMmioExit,
    value: u32,
) -> Result<(), AsldError> {
    let mut regs = vcpu
        .regs()
        .map_err(|_| AsldError::BackendUnavailable("avm get_regs failed"))?;
    match prepared.access {
        Some(MmioAccess::Read { dest, width, .. }) => {
            write_register(&mut regs, dest, width, value as u64);
        }
        _ => {
            write_register(
                &mut regs,
                Register::Gpr(0),
                prepared.exit.access_size,
                value as u64,
            );
        }
    }
    vcpu.set_regs(&regs)
        .map_err(|_| AsldError::BackendUnavailable("avm set_regs failed"))
}

fn decode_mmio_access(bytes: &[u8], regs: &libavm::AvmRegs, efer: u64) -> Option<MmioAccess> {
    let long_mode = (efer & (1 << 10)) != 0;
    let mut index = 0usize;
    let mut operand16 = false;
    let mut address16 = false;
    let mut rex = 0u8;

    while index < bytes.len() {
        let byte = bytes[index];
        match byte {
            0x66 => operand16 = true,
            0x67 => address16 = true,
            0x26 | 0x2e | 0x36 | 0x3e | 0x64 | 0x65 | 0xf0 | 0xf2 | 0xf3 => {}
            0x40..=0x4f if long_mode => rex = byte,
            _ => break,
        }
        index += 1;
    }

    let opcode = *bytes.get(index)?;
    index += 1;

    match opcode {
        0x88 | 0x89 | 0x8a | 0x8b => {
            let modrm = *bytes.get(index)?;
            let memory_len = modrm_memory_len(bytes, index, address16)?;
            let reg = decode_reg((modrm >> 3) & 0x7, rex, opcode == 0x88 || opcode == 0x8a);
            let width = if opcode == 0x88 || opcode == 0x8a {
                1
            } else {
                operand_width(operand16, rex, long_mode)
            };
            let instruction_len = memory_len as u32;

            if opcode == 0x8a || opcode == 0x8b {
                Some(MmioAccess::Read {
                    dest: reg,
                    width,
                    instruction_len,
                })
            } else {
                Some(MmioAccess::Write {
                    value: read_register(regs, reg, width),
                    width,
                    instruction_len,
                })
            }
        }
        0xc6 | 0xc7 => {
            let modrm = *bytes.get(index)?;
            if ((modrm >> 3) & 0x7) != 0 {
                return None;
            }
            let memory_len = modrm_memory_len(bytes, index, address16)?;
            let width = if opcode == 0xc6 {
                1
            } else {
                operand_width(operand16, rex, long_mode)
            };
            let imm_len = if opcode == 0xc6 {
                1
            } else if width == 2 {
                2
            } else {
                4
            };
            let immediate_offset = memory_len;
            let end = immediate_offset.checked_add(imm_len)?;
            let immediate = read_immediate(bytes.get(immediate_offset..end)?, imm_len);
            Some(MmioAccess::Write {
                value: immediate,
                width,
                instruction_len: end as u32,
            })
        }
        0xa0 | 0xa1 | 0xa2 | 0xa3 => {
            let width = if opcode == 0xa0 || opcode == 0xa2 {
                1
            } else {
                operand_width(operand16, rex, long_mode)
            };
            let address_len = if address16 {
                2
            } else if long_mode {
                8
            } else {
                4
            };
            let instruction_len = index.checked_add(address_len)? as u32;
            if opcode == 0xa0 || opcode == 0xa1 {
                Some(MmioAccess::Read {
                    dest: Register::Gpr(0),
                    width,
                    instruction_len,
                })
            } else {
                Some(MmioAccess::Write {
                    value: read_register(regs, Register::Gpr(0), width),
                    width,
                    instruction_len,
                })
            }
        }
        _ => None,
    }
}

fn modrm_memory_len(bytes: &[u8], modrm_index: usize, address16: bool) -> Option<usize> {
    let modrm = *bytes.get(modrm_index)?;
    let mode = modrm >> 6;
    if mode == 0b11 {
        return None;
    }

    if address16 {
        let rm = modrm & 0x7;
        let disp = match (mode, rm) {
            (0, 6) => 2,
            (0, _) => 0,
            (1, _) => 1,
            (2, _) => 2,
            _ => return None,
        };
        return checked_len(bytes, modrm_index + 1, disp);
    }

    let rm = modrm & 0x7;
    let mut index = modrm_index + 1;
    let mut base = rm;
    if rm == 4 {
        let sib = *bytes.get(index)?;
        index += 1;
        base = sib & 0x7;
    }

    let disp = match (mode, base) {
        (0, 5) => 4,
        (0, _) => 0,
        (1, _) => 1,
        (2, _) => 4,
        _ => return None,
    };
    checked_len(bytes, index, disp)
}

fn checked_len(bytes: &[u8], offset: usize, tail: usize) -> Option<usize> {
    let end = offset.checked_add(tail)?;
    if end <= bytes.len() {
        Some(end)
    } else {
        None
    }
}

fn operand_width(operand16: bool, rex: u8, long_mode: bool) -> u8 {
    if long_mode && (rex & 0x08) != 0 {
        8
    } else if operand16 {
        2
    } else {
        4
    }
}

fn decode_reg(encoded: u8, rex: u8, byte_operand: bool) -> Register {
    let extended = encoded | (((rex >> 2) & 1) << 3);
    if byte_operand && rex == 0 && (4..=7).contains(&encoded) {
        Register::High8(encoded - 4)
    } else {
        Register::Gpr(extended)
    }
}

fn read_register(regs: &libavm::AvmRegs, reg: Register, width: u8) -> u64 {
    let value = match reg {
        Register::Gpr(0) => regs.rax,
        Register::Gpr(1) => regs.rcx,
        Register::Gpr(2) => regs.rdx,
        Register::Gpr(3) => regs.rbx,
        Register::Gpr(4) => regs.rsp,
        Register::Gpr(5) => regs.rbp,
        Register::Gpr(6) => regs.rsi,
        Register::Gpr(7) => regs.rdi,
        Register::Gpr(8) => regs.r8,
        Register::Gpr(9) => regs.r9,
        Register::Gpr(10) => regs.r10,
        Register::Gpr(11) => regs.r11,
        Register::Gpr(12) => regs.r12,
        Register::Gpr(13) => regs.r13,
        Register::Gpr(14) => regs.r14,
        Register::Gpr(15) => regs.r15,
        Register::Gpr(_) => 0,
        Register::High8(0) => (regs.rax >> 8) & 0xff,
        Register::High8(1) => (regs.rcx >> 8) & 0xff,
        Register::High8(2) => (regs.rdx >> 8) & 0xff,
        Register::High8(3) => (regs.rbx >> 8) & 0xff,
        Register::High8(_) => 0,
    };
    match width {
        1 => value & 0xff,
        2 => value & 0xffff,
        4 => value & 0xffff_ffff,
        _ => value,
    }
}

fn write_register(regs: &mut libavm::AvmRegs, reg: Register, width: u8, value: u64) {
    match reg {
        Register::Gpr(index) => {
            let slot = match index {
                0 => &mut regs.rax,
                1 => &mut regs.rcx,
                2 => &mut regs.rdx,
                3 => &mut regs.rbx,
                4 => &mut regs.rsp,
                5 => &mut regs.rbp,
                6 => &mut regs.rsi,
                7 => &mut regs.rdi,
                8 => &mut regs.r8,
                9 => &mut regs.r9,
                10 => &mut regs.r10,
                11 => &mut regs.r11,
                12 => &mut regs.r12,
                13 => &mut regs.r13,
                14 => &mut regs.r14,
                15 => &mut regs.r15,
                _ => return,
            };
            *slot = merge_register_value(*slot, width, value);
        }
        Register::High8(index) => {
            let slot = match index {
                0 => &mut regs.rax,
                1 => &mut regs.rcx,
                2 => &mut regs.rdx,
                3 => &mut regs.rbx,
                _ => return,
            };
            *slot = (*slot & !0xff00) | ((value & 0xff) << 8);
        }
    }
}

fn merge_register_value(original: u64, width: u8, value: u64) -> u64 {
    match width {
        1 => (original & !0xff) | (value & 0xff),
        2 => (original & !0xffff) | (value & 0xffff),
        4 => value & 0xffff_ffff,
        _ => value,
    }
}

fn read_immediate(bytes: &[u8], len: usize) -> u64 {
    let mut value = 0u64;
    for (index, byte) in bytes.iter().take(len).enumerate() {
        value |= (*byte as u64) << (index * 8);
    }
    value
}

#[cfg(test)]
mod tests {
    use super::{decode_mmio_access, write_register, MmioAccess, Register};

    const EFER_LMA: u64 = 1 << 10;

    fn regs() -> libavm::AvmRegs {
        libavm::AvmRegs {
            rax: 0x1111_2222_3333_4444,
            rbx: 0x5555_6666_7777_8888,
            rcx: 0x9999_aaaa_bbbb_cccc,
            rdx: 0xdddd_eeee_ffff_0000,
            r8: 0x0123_4567_89ab_cdef,
            ..Default::default()
        }
    }

    #[test]
    fn decodes_mov_mmio_from_extended_register() {
        let access = decode_mmio_access(&[0x44, 0x89, 0x03], &regs(), EFER_LMA).unwrap();
        assert_eq!(
            access,
            MmioAccess::Write {
                value: 0x89ab_cdef,
                width: 4,
                instruction_len: 3,
            }
        );
    }

    #[test]
    fn decodes_mov_mmio_to_register() {
        let access = decode_mmio_access(&[0x8b, 0x4b, 0x10], &regs(), EFER_LMA).unwrap();
        assert_eq!(
            access,
            MmioAccess::Read {
                dest: Register::Gpr(1),
                width: 4,
                instruction_len: 3,
            }
        );
    }

    #[test]
    fn decodes_rex_w_64bit_mmio_width() {
        let access = decode_mmio_access(&[0x48, 0x89, 0x18], &regs(), EFER_LMA).unwrap();
        assert_eq!(
            access,
            MmioAccess::Write {
                value: 0x5555_6666_7777_8888,
                width: 8,
                instruction_len: 3,
            }
        );
    }

    #[test]
    fn decodes_immediate_mmio_write() {
        let access = decode_mmio_access(
            &[0xc7, 0x43, 0x10, 0xef, 0xbe, 0xad, 0xde],
            &regs(),
            EFER_LMA,
        )
        .unwrap();
        assert_eq!(
            access,
            MmioAccess::Write {
                value: 0xdead_beef,
                width: 4,
                instruction_len: 7,
            }
        );
    }

    #[test]
    fn register_write_zero_extends_32bit_values() {
        let mut regs = regs();
        write_register(&mut regs, Register::Gpr(0), 4, 0xfeed_beef);
        assert_eq!(regs.rax, 0xfeed_beef);
    }

    #[test]
    fn register_write_preserves_upper_bits_for_16bit_values() {
        let mut regs = regs();
        write_register(&mut regs, Register::Gpr(3), 2, 0xbeef);
        assert_eq!(regs.rbx, 0x5555_6666_7777_beef);
    }
}
