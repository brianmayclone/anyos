//! Shared data types for hardware virtualization backends.

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VcpuRegs {
    pub rax: u64, pub rbx: u64, pub rcx: u64, pub rdx: u64,
    pub rsi: u64, pub rdi: u64, pub rbp: u64, pub rsp: u64,
    pub r8: u64, pub r9: u64, pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rip: u64, pub rflags: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct SegmentReg {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub type_: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct DescriptorTable {
    pub base: u64,
    pub limit: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VcpuSregs {
    pub cs: SegmentReg, pub ds: SegmentReg, pub es: SegmentReg,
    pub fs: SegmentReg, pub gs: SegmentReg, pub ss: SegmentReg,
    pub tr: SegmentReg, pub ldt: SegmentReg,
    pub gdt: DescriptorTable, pub idt: DescriptorTable,
    pub cr0: u64, pub cr2: u64, pub cr3: u64, pub cr4: u64, pub efer: u64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuidEntry {
    pub function: u32,
    pub index: u32,
    pub flags: u32,
    pub eax: u32, pub ebx: u32, pub ecx: u32, pub edx: u32,
}

#[derive(Debug, Clone)]
pub enum IrqEvent {
    Interrupt(u8),
    Exception(u8, Option<u32>),
    Nmi,
}
