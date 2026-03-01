//! CPU register file for all x86 modes.
//!
//! Contains the full architectural register state: general-purpose registers,
//! segment registers with cached descriptors, control registers, debug
//! registers, and model-specific registers (MSRs).

use alloc::collections::BTreeMap;
use crate::flags::OperandSize;

/// General-purpose register indices matching x86 encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GprIndex {
    Rax = 0,
    Rcx = 1,
    Rdx = 2,
    Rbx = 3,
    Rsp = 4,
    Rbp = 5,
    Rsi = 6,
    Rdi = 7,
    R8 = 8,
    R9 = 9,
    R10 = 10,
    R11 = 11,
    R12 = 12,
    R13 = 13,
    R14 = 14,
    R15 = 15,
}

/// Segment register index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SegReg {
    Es = 0,
    Cs = 1,
    Ss = 2,
    Ds = 3,
    Fs = 4,
    Gs = 5,
}

impl SegReg {
    /// Convert a 3-bit segment register encoding to SegReg.
    pub fn from_encoding(val: u8) -> Option<SegReg> {
        match val & 0x07 {
            0 => Some(SegReg::Es),
            1 => Some(SegReg::Cs),
            2 => Some(SegReg::Ss),
            3 => Some(SegReg::Ds),
            4 => Some(SegReg::Fs),
            5 => Some(SegReg::Gs),
            _ => None,
        }
    }
}

/// Cached segment descriptor (hidden part of a segment register).
///
/// When a segment register is loaded (MOV, far JMP/CALL, IRET, etc.),
/// the CPU reads the GDT/LDT entry and caches the descriptor here.
#[derive(Debug, Clone, Copy)]
pub struct SegmentDescriptor {
    /// Visible selector value.
    pub selector: u16,
    /// Base address (decoded from descriptor).
    pub base: u64,
    /// Segment limit (in bytes, accounting for granularity).
    pub limit: u32,
    /// Raw access byte from the descriptor.
    pub access: u8,
    /// Flags nibble (G, D/B, L, AVL).
    pub flags: u8,
    /// Descriptor privilege level (0-3).
    pub dpl: u8,
    /// Segment is present.
    pub present: bool,
    /// Code segment (true) vs data segment (false).
    pub is_code: bool,
    /// Conforming code segment.
    pub is_conforming: bool,
    /// Code segment: readable. Data segment: always true.
    pub readable: bool,
    /// Data segment: writable. Code segment: always false.
    pub writable: bool,
    /// D/B bit: 32-bit default operand/stack size.
    pub big: bool,
    /// L bit: 64-bit code segment (long mode).
    pub long_mode: bool,
    /// G bit: limit granularity (true = 4 KiB pages).
    pub granularity: bool,
}

impl SegmentDescriptor {
    /// Create a flat real-mode segment descriptor.
    pub fn real_mode(selector: u16) -> Self {
        SegmentDescriptor {
            selector,
            base: (selector as u64) << 4,
            limit: 0xFFFF,
            access: 0x93,
            flags: 0,
            dpl: 0,
            present: true,
            is_code: false,
            is_conforming: false,
            readable: true,
            writable: true,
            big: false,
            long_mode: false,
            granularity: false,
        }
    }

    /// Create a flat real-mode code segment descriptor.
    pub fn real_mode_code(selector: u16) -> Self {
        let mut desc = Self::real_mode(selector);
        desc.is_code = true;
        desc.writable = false;
        desc.access = 0x9B;
        desc
    }

    /// Decode a segment descriptor from an 8-byte GDT/LDT entry.
    pub fn from_raw(selector: u16, raw: u64) -> Self {
        let base_low = ((raw >> 16) & 0xFFFF) as u32;
        let base_mid = ((raw >> 32) & 0xFF) as u32;
        let base_high = ((raw >> 56) & 0xFF) as u32;
        let base = (base_low | (base_mid << 16) | (base_high << 24)) as u64;

        let limit_low = (raw & 0xFFFF) as u32;
        let limit_high = ((raw >> 48) & 0x0F) as u32;
        let mut limit = limit_low | (limit_high << 16);

        let access = ((raw >> 40) & 0xFF) as u8;
        let flags_nibble = ((raw >> 52) & 0x0F) as u8;

        let granularity = (flags_nibble & 0x08) != 0;
        if granularity {
            limit = (limit << 12) | 0xFFF;
        }

        let dpl = (access >> 5) & 0x03;
        let present = (access & 0x80) != 0;
        let is_system = (access & 0x10) == 0;
        let is_code = !is_system && (access & 0x08) != 0;
        let is_conforming = is_code && (access & 0x04) != 0;
        let readable = if is_code { (access & 0x02) != 0 } else { true };
        let writable = if is_code { false } else { (access & 0x02) != 0 };
        let big = (flags_nibble & 0x04) != 0;
        let long_mode = (flags_nibble & 0x02) != 0;

        SegmentDescriptor {
            selector,
            base,
            limit,
            access,
            flags: flags_nibble,
            dpl,
            present,
            is_code,
            is_conforming,
            readable,
            writable,
            big,
            long_mode,
            granularity,
        }
    }
}

/// GDTR/IDTR register pair (base + limit).
#[derive(Debug, Clone, Copy, Default)]
pub struct TableRegister {
    /// Linear base address of the table.
    pub base: u64,
    /// Table size limit in bytes.
    pub limit: u16,
}

/// CPU register file — all architectural register state.
pub struct RegisterFile {
    /// General-purpose registers (64-bit each, indexed by GprIndex).
    pub gpr: [u64; 16],

    /// Instruction pointer.
    pub rip: u64,

    /// RFLAGS register.
    pub rflags: u64,

    /// Segment registers (visible selector + hidden cached descriptor).
    pub seg: [SegmentDescriptor; 6],

    /// Control registers.
    pub cr0: u64,
    /// Page fault linear address.
    pub cr2: u64,
    /// Page directory base register.
    pub cr3: u64,
    /// CR4 extensions.
    pub cr4: u64,
    /// Task priority register (64-bit mode).
    pub cr8: u64,

    /// Debug registers (DR0-DR3: breakpoint addresses, DR6: status, DR7: control).
    pub dr: [u64; 8],

    /// Global Descriptor Table Register.
    pub gdtr: TableRegister,
    /// Interrupt Descriptor Table Register.
    pub idtr: TableRegister,
    /// Local Descriptor Table Register (selector).
    pub ldtr: u16,
    /// Task Register (selector).
    pub tr: u16,

    /// Model-Specific Registers (sparse storage).
    pub msr: BTreeMap<u32, u64>,

    /// Current privilege level (0-3).
    pub cpl: u8,
}

// ── Well-known MSR addresses ──

/// Extended Feature Enable Register.
pub const MSR_EFER: u32 = 0xC000_0080;
/// SYSCALL target CS/SS and return CS/SS.
pub const MSR_STAR: u32 = 0xC000_0081;
/// SYSCALL target RIP (64-bit mode).
pub const MSR_LSTAR: u32 = 0xC000_0082;
/// SYSCALL target RIP (compatibility mode).
pub const MSR_CSTAR: u32 = 0xC000_0083;
/// SYSCALL RFLAGS mask.
pub const MSR_SFMASK: u32 = 0xC000_0084;
/// FS.base MSR.
pub const MSR_FS_BASE: u32 = 0xC000_0100;
/// GS.base MSR.
pub const MSR_GS_BASE: u32 = 0xC000_0101;
/// KernelGSBase MSR (swapped by SWAPGS).
pub const MSR_KERNEL_GS_BASE: u32 = 0xC000_0102;
/// Time Stamp Counter.
pub const MSR_TSC: u32 = 0x0000_0010;

// ── EFER bits ──

/// SYSCALL/SYSRET enable.
pub const EFER_SCE: u64 = 1 << 0;
/// Long Mode Enable.
pub const EFER_LME: u64 = 1 << 8;
/// Long Mode Active (set by CPU when LME=1 and CR0.PG=1).
pub const EFER_LMA: u64 = 1 << 10;
/// No-Execute Enable.
pub const EFER_NXE: u64 = 1 << 11;

// ── CR0 bits ──

/// Protection Enable.
pub const CR0_PE: u64 = 1 << 0;
/// Monitor Coprocessor.
pub const CR0_MP: u64 = 1 << 1;
/// Emulation (no x87 FPU).
pub const CR0_EM: u64 = 1 << 2;
/// Task Switched.
pub const CR0_TS: u64 = 1 << 3;
/// Extension Type (always 1 on 486+).
pub const CR0_ET: u64 = 1 << 4;
/// Numeric Error (native x87 error reporting).
pub const CR0_NE: u64 = 1 << 5;
/// Write Protect (supervisor writes to RO user pages fault).
pub const CR0_WP: u64 = 1 << 16;
/// Alignment Mask.
pub const CR0_AM: u64 = 1 << 18;
/// Not Write-through.
pub const CR0_NW: u64 = 1 << 29;
/// Cache Disable.
pub const CR0_CD: u64 = 1 << 30;
/// Paging.
pub const CR0_PG: u64 = 1u64 << 31;

// ── CR4 bits ──

/// Virtual-8086 Mode Extensions.
pub const CR4_VME: u64 = 1 << 0;
/// Protected-Mode Virtual Interrupts.
pub const CR4_PVI: u64 = 1 << 1;
/// Time Stamp Disable (RDTSC in Ring 3).
pub const CR4_TSD: u64 = 1 << 2;
/// Debugging Extensions.
pub const CR4_DE: u64 = 1 << 3;
/// Page Size Extensions (4 MiB pages in 32-bit mode).
pub const CR4_PSE: u64 = 1 << 4;
/// Physical Address Extension (PAE paging).
pub const CR4_PAE: u64 = 1 << 5;
/// Machine-Check Enable.
pub const CR4_MCE: u64 = 1 << 6;
/// Page Global Enable.
pub const CR4_PGE: u64 = 1 << 7;
/// OS support for FXSAVE/FXRSTOR.
pub const CR4_OSFXSR: u64 = 1 << 9;
/// OS support for unmasked SIMD exceptions.
pub const CR4_OSXMMEXCPT: u64 = 1 << 10;
/// PCID enable.
pub const CR4_PCIDE: u64 = 1 << 17;

impl RegisterFile {
    /// Create a new register file with power-on reset defaults.
    ///
    /// Matches the Intel SDM "Table 10-1. IA-32 and Intel 64 Processor States
    /// Following Power-up" for the relevant subset.
    pub fn new() -> Self {
        let mut regs = RegisterFile {
            gpr: [0u64; 16],
            rip: 0xFFF0,
            rflags: crate::flags::RFLAGS_FIXED,
            seg: [
                SegmentDescriptor::real_mode(0),      // ES
                SegmentDescriptor::real_mode_code(0xF000), // CS
                SegmentDescriptor::real_mode(0),      // SS
                SegmentDescriptor::real_mode(0),      // DS
                SegmentDescriptor::real_mode(0),      // FS
                SegmentDescriptor::real_mode(0),      // GS
            ],
            cr0: CR0_ET, // ET=1 (486+), PE=0 (real mode)
            cr2: 0,
            cr3: 0,
            cr4: 0,
            cr8: 0,
            dr: [0u64; 8],
            gdtr: TableRegister::default(),
            idtr: TableRegister { base: 0, limit: 0x3FF }, // Real-mode IVT
            ldtr: 0,
            tr: 0,
            msr: BTreeMap::new(),
            cpl: 0,
        };
        // EDX contains processor identification on reset (we report a generic P6)
        regs.gpr[GprIndex::Rdx as usize] = 0x0000_0600;
        // DR6 initial value: all breakpoint conditions clear
        regs.dr[6] = 0xFFFF_0FF0;
        // DR7 initial value: all breakpoints disabled
        regs.dr[7] = 0x0000_0400;
        regs
    }

    // ── GPR access by specific size ──

    /// Read an 8-bit register.
    ///
    /// Without REX: indices 0-3 = AL/CL/DL/BL, 4-7 = AH/CH/DH/BH.
    /// With REX: indices 0-3 = AL/CL/DL/BL, 4-7 = SPL/BPL/SIL/DIL, 8-15 = R8B-R15B.
    #[inline]
    pub fn read_gpr8(&self, index: u8, has_rex: bool) -> u8 {
        if !has_rex && index >= 4 && index <= 7 {
            // AH, CH, DH, BH — high byte of the low word
            ((self.gpr[(index - 4) as usize] >> 8) & 0xFF) as u8
        } else {
            (self.gpr[index as usize] & 0xFF) as u8
        }
    }

    /// Write an 8-bit register.
    #[inline]
    pub fn write_gpr8(&mut self, index: u8, has_rex: bool, val: u8) {
        if !has_rex && index >= 4 && index <= 7 {
            // AH, CH, DH, BH
            let reg = &mut self.gpr[(index - 4) as usize];
            *reg = (*reg & !0xFF00) | ((val as u64) << 8);
        } else {
            let reg = &mut self.gpr[index as usize];
            *reg = (*reg & !0xFF) | (val as u64);
        }
    }

    /// Read a 16-bit register.
    #[inline]
    pub fn read_gpr16(&self, index: u8) -> u16 {
        (self.gpr[index as usize] & 0xFFFF) as u16
    }

    /// Write a 16-bit register (preserves upper 48 bits).
    #[inline]
    pub fn write_gpr16(&mut self, index: u8, val: u16) {
        let reg = &mut self.gpr[index as usize];
        *reg = (*reg & !0xFFFF) | (val as u64);
    }

    /// Read a 32-bit register.
    #[inline]
    pub fn read_gpr32(&self, index: u8) -> u32 {
        (self.gpr[index as usize] & 0xFFFF_FFFF) as u32
    }

    /// Write a 32-bit register.
    ///
    /// In 64-bit mode, writing a 32-bit register zero-extends to 64 bits.
    /// In 32-bit mode, the behavior is the same (upper 32 bits are zero).
    #[inline]
    pub fn write_gpr32(&mut self, index: u8, val: u32) {
        self.gpr[index as usize] = val as u64;
    }

    /// Read a 64-bit register.
    #[inline]
    pub fn read_gpr64(&self, index: u8) -> u64 {
        self.gpr[index as usize]
    }

    /// Write a 64-bit register.
    #[inline]
    pub fn write_gpr64(&mut self, index: u8, val: u64) {
        self.gpr[index as usize] = val;
    }

    // ── Operand-width polymorphic access ──

    /// Read a GPR at the specified operand width.
    #[inline]
    pub fn read_gpr(&self, index: u8, width: OperandSize, has_rex: bool) -> u64 {
        match width {
            OperandSize::Byte => self.read_gpr8(index, has_rex) as u64,
            OperandSize::Word => self.read_gpr16(index) as u64,
            OperandSize::Dword => self.read_gpr32(index) as u64,
            OperandSize::Qword => self.read_gpr64(index),
        }
    }

    /// Write a GPR at the specified operand width.
    #[inline]
    pub fn write_gpr(&mut self, index: u8, width: OperandSize, has_rex: bool, val: u64) {
        match width {
            OperandSize::Byte => self.write_gpr8(index, has_rex, val as u8),
            OperandSize::Word => self.write_gpr16(index, val as u16),
            OperandSize::Dword => self.write_gpr32(index, val as u32),
            OperandSize::Qword => self.write_gpr64(index, val),
        }
    }

    // ── MSR access ──

    /// Read an MSR. Returns 0 for undefined MSRs.
    #[inline]
    pub fn read_msr(&self, index: u32) -> u64 {
        self.msr.get(&index).copied().unwrap_or(0)
    }

    /// Write an MSR.
    #[inline]
    pub fn write_msr(&mut self, index: u32, val: u64) {
        self.msr.insert(index, val);
    }

    // ── Segment register helpers ──

    /// Get a reference to a segment descriptor by SegReg.
    #[inline]
    pub fn segment(&self, reg: SegReg) -> &SegmentDescriptor {
        &self.seg[reg as usize]
    }

    /// Get a mutable reference to a segment descriptor.
    #[inline]
    pub fn segment_mut(&mut self, reg: SegReg) -> &mut SegmentDescriptor {
        &mut self.seg[reg as usize]
    }

    /// Load a segment register from a selector and raw descriptor.
    pub fn load_segment(&mut self, reg: SegReg, selector: u16, raw_descriptor: u64) {
        self.seg[reg as usize] = SegmentDescriptor::from_raw(selector, raw_descriptor);
    }

    /// Load a segment register in real mode (base = selector << 4).
    pub fn load_segment_real(&mut self, reg: SegReg, selector: u16) {
        if reg == SegReg::Cs {
            self.seg[reg as usize] = SegmentDescriptor::real_mode_code(selector);
        } else {
            self.seg[reg as usize] = SegmentDescriptor::real_mode(selector);
        }
    }

    // ── Stack pointer helpers ──

    /// Read the stack pointer at the current stack width.
    #[inline]
    pub fn sp(&self) -> u64 {
        self.gpr[GprIndex::Rsp as usize]
    }

    /// Write the stack pointer.
    #[inline]
    pub fn set_sp(&mut self, val: u64) {
        self.gpr[GprIndex::Rsp as usize] = val;
    }
}
