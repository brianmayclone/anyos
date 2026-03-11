//! Windows Hypervisor Platform (WHP) backend.
//!
//! Uses runtime-loaded WinHvPlatform.dll via LoadLibraryA/GetProcAddress.

extern crate std;

use std::vec::Vec;
use std::boxed::Box;
use core::ffi::c_void;
use super::{VmBackend, VmError, VmExitReason};
use super::types::*;

// --- WHP types ---

type WHV_PARTITION_HANDLE = *mut c_void;

#[repr(C, align(16))]
#[derive(Copy, Clone)]
union WHV_REGISTER_VALUE {
    reg64: u64,
    reg128: [u64; 2],
    segment: WhvSegment,
    table: WhvTable,
}

impl Default for WHV_REGISTER_VALUE {
    fn default() -> Self {
        WHV_REGISTER_VALUE { reg128: [0; 2] }
    }
}

impl WHV_REGISTER_VALUE {
    /// Create a register value from a u64, with upper 64 bits zeroed.
    fn from_u64(v: u64) -> Self {
        WHV_REGISTER_VALUE { reg128: [v, 0] }
    }
    /// Create a register value from a segment descriptor (16 bytes, no leftover).
    fn from_seg(s: WhvSegment) -> Self {
        WHV_REGISTER_VALUE { segment: s }
    }
    /// Create a register value from a table descriptor (16 bytes, no leftover).
    fn from_table(t: WhvTable) -> Self {
        WHV_REGISTER_VALUE { table: t }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct WhvSegment {
    base: u64,
    limit: u32,
    selector: u16,
    attributes: u16,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct WhvTable {
    _pad: [u16; 3],
    limit: u16,
    base: u64,
}

// WHV_VP_EXIT_CONTEXT: ExecutionState(2) + InstructionLength:4+Cr8:4(1) + Reserved(1) + Reserved2(4) + Cs(16) + Rip(8) + Rflags(8) = 40 bytes
const VP_CONTEXT_SIZE: usize = 40;

#[repr(C)]
struct WHV_RUN_VP_EXIT_CONTEXT {
    exit_reason: u32,
    _reserved: u32,
    vp_context: [u8; VP_CONTEXT_SIZE],
    exit_data: [u8; 256],
}

// --- WHP constants ---

const WHV_PROPERTY_PROCESSOR_COUNT: u32 = 0x00001FFF; // WHvPartitionPropertyCodeProcessorCount
const WHV_PROPERTY_EXTENDED_VM_EXITS: u32 = 0x00000001; // WHvPartitionPropertyCodeExtendedVmExits

const WHV_MAP_GPA_RANGE_FLAG_READ: u32 = 0x1;
const WHV_MAP_GPA_RANGE_FLAG_WRITE: u32 = 0x2;
const WHV_MAP_GPA_RANGE_FLAG_EXECUTE: u32 = 0x4;

const WHV_EXIT_REASON_NONE: u32 = 0x00000000;
const WHV_EXIT_REASON_MEMORY_ACCESS: u32 = 0x00000001;
const WHV_EXIT_REASON_IO_PORT: u32 = 0x00000002;
const WHV_EXIT_REASON_HALT: u32 = 0x00000008;
const WHV_EXIT_REASON_CANCELED: u32 = 0x00002001;
const WHV_EXIT_REASON_MSR: u32 = 0x00001000;
const WHV_EXIT_REASON_CPUID: u32 = 0x00001001;

// WHV_REGISTER_NAME constants (from Windows SDK)
const REG_RAX: u32 = 0x00000000;
const REG_RCX: u32 = 0x00000001;
const REG_RDX: u32 = 0x00000002;
const REG_RBX: u32 = 0x00000003;
const REG_RSP: u32 = 0x00000004;
const REG_RBP: u32 = 0x00000005;
const REG_RSI: u32 = 0x00000006;
const REG_RDI: u32 = 0x00000007;
const REG_R8: u32 = 0x00000008;
const REG_R9: u32 = 0x00000009;
const REG_R10: u32 = 0x0000000A;
const REG_R11: u32 = 0x0000000B;
const REG_R12: u32 = 0x0000000C;
const REG_R13: u32 = 0x0000000D;
const REG_R14: u32 = 0x0000000E;
const REG_R15: u32 = 0x0000000F;
const REG_RIP: u32 = 0x00000010;
const REG_RFLAGS: u32 = 0x00000011;

const REG_ES: u32 = 0x00000012;
const REG_CS: u32 = 0x00000013;
const REG_SS: u32 = 0x00000014;
const REG_DS: u32 = 0x00000015;
const REG_FS: u32 = 0x00000016;
const REG_GS: u32 = 0x00000017;
const REG_LDTR: u32 = 0x00000018;
const REG_TR: u32 = 0x00000019;
const REG_IDTR: u32 = 0x0000001A;
const REG_GDTR: u32 = 0x0000001B;

const REG_CR0: u32 = 0x0000001C;
const REG_CR2: u32 = 0x0000001D;
const REG_CR3: u32 = 0x0000001E;
const REG_CR4: u32 = 0x0000001F;
const REG_EFER: u32 = 0x00002001;

const REG_PENDING_INTERRUPTION: u32 = 0x80000000;

// GP register names in order for get/set
const GP_REG_NAMES: [u32; 18] = [
    REG_RAX, REG_RBX, REG_RCX, REG_RDX,
    REG_RSI, REG_RDI, REG_RBP, REG_RSP,
    REG_R8, REG_R9, REG_R10, REG_R11,
    REG_R12, REG_R13, REG_R14, REG_R15,
    REG_RIP, REG_RFLAGS,
];

const SREG_NAMES: [u32; 13] = [
    REG_CS, REG_DS, REG_ES, REG_FS, REG_GS, REG_SS,
    REG_TR, REG_LDTR, REG_GDTR, REG_IDTR,
    REG_CR0, REG_CR2, REG_CR3,
];

const SREG_NAMES_EXT: [u32; 2] = [REG_CR4, REG_EFER];

// --- WHP function pointer types ---

type FnGetCapability = extern "system" fn(u32, *mut u8, u32, *mut u32) -> i32;
type FnCreatePartition = extern "system" fn(*mut WHV_PARTITION_HANDLE) -> i32;
type FnSetupPartition = extern "system" fn(WHV_PARTITION_HANDLE) -> i32;
type FnDeletePartition = extern "system" fn(WHV_PARTITION_HANDLE) -> i32;
type FnSetPartitionProperty = extern "system" fn(WHV_PARTITION_HANDLE, u32, *const u8, u32) -> i32;
type FnMapGpaRange = extern "system" fn(WHV_PARTITION_HANDLE, *mut u8, u64, u64, u32) -> i32;
type FnUnmapGpaRange = extern "system" fn(WHV_PARTITION_HANDLE, u64, u64) -> i32;
type FnCreateVirtualProcessor = extern "system" fn(WHV_PARTITION_HANDLE, u32, u32) -> i32;
type FnDeleteVirtualProcessor = extern "system" fn(WHV_PARTITION_HANDLE, u32) -> i32;
type FnRunVirtualProcessor = extern "system" fn(WHV_PARTITION_HANDLE, u32, *mut u8, u32) -> i32;
type FnGetVirtualProcessorRegisters = extern "system" fn(WHV_PARTITION_HANDLE, u32, *const u32, u32, *mut WHV_REGISTER_VALUE) -> i32;
type FnSetVirtualProcessorRegisters = extern "system" fn(WHV_PARTITION_HANDLE, u32, *const u32, u32, *const WHV_REGISTER_VALUE) -> i32;
// WHvRequestInterrupt(partition, *interrupt_control, size) -> HRESULT
type FnRequestInterrupt = extern "system" fn(WHV_PARTITION_HANDLE, *const u8, u32) -> i32;

struct WhpApi {
    get_capability: FnGetCapability,
    create_partition: FnCreatePartition,
    setup_partition: FnSetupPartition,
    delete_partition: FnDeletePartition,
    set_property: FnSetPartitionProperty,
    map_gpa: FnMapGpaRange,
    unmap_gpa: FnUnmapGpaRange,
    create_vp: FnCreateVirtualProcessor,
    delete_vp: FnDeleteVirtualProcessor,
    run_vp: FnRunVirtualProcessor,
    get_regs: FnGetVirtualProcessorRegisters,
    set_regs: FnSetVirtualProcessorRegisters,
    request_interrupt: Option<FnRequestInterrupt>,
}

// Windows API imports for DLL loading
extern "system" {
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

struct MemorySlot {
    slot: u32,
    guest_phys: u64,
    size: u64,
    host_ptr: *mut u8,
}

/// Minimal Local APIC state for handling MMIO at 0xFEE00000.
struct SoftLapic {
    regs: [u32; 64], // 64 registers at 16-byte spacing (offset >> 4)
}

impl SoftLapic {
    fn new() -> Self {
        let mut regs = [0u32; 64];
        regs[0x02] = 0x20;       // ID = 0 (bits 24-27)
        regs[0x03] = 0x00050014; // Version: max_lvt=5, version=20 (Pentium 4)
        regs[0x0E] = 0x0FFFFFFF; // Logical Destination Format: flat model
        regs[0x0F] = 0x000001FF; // Spurious Vector: APIC enabled (bit 8) + vector 0xFF
        regs[0x08] = 0;          // Task Priority Register
        SoftLapic { regs }
    }

    fn read(&self, offset: u64) -> u32 {
        let idx = ((offset & 0xFFF) >> 4) as usize;
        if idx < 64 { self.regs[idx] } else { 0 }
    }

    fn write(&mut self, offset: u64, val: u32) {
        let idx = ((offset & 0xFFF) >> 4) as usize;
        match idx {
            0x0B => {} // EOI: write-only, acknowledge interrupt
            0x30 => {} // ICR write (trigger IPI) - ignore for single CPU
            0x0F => {  // SVR: preserve APIC enable bit
                if idx < 64 { self.regs[idx] = val; }
            }
            _ => {
                if idx < 64 { self.regs[idx] = val; }
            }
        }
    }
}

/// Minimal IOAPIC state for indirect register access at 0xFEC00000.
struct SoftIoapic {
    ioregsel: u32,
    regs: [u32; 64], // indirect registers
}

impl SoftIoapic {
    fn new() -> Self {
        let mut regs = [0u32; 64];
        regs[0x00] = 0x00; // IOAPICID
        regs[0x01] = 0x00170020; // IOAPICVER: version=0x20, max_redir=23 (24 entries)
        regs[0x02] = 0x00; // IOAPICARB
        // Redirection entries (2 regs each, starting at index 0x10):
        // Default: all masked (bit 16 = 1)
        for i in 0..24 {
            regs[0x10 + i * 2] = 0x00010000; // masked, edge, fixed, physical
            regs[0x10 + i * 2 + 1] = 0x00000000; // destination = 0
        }
        SoftIoapic { ioregsel: 0, regs }
    }

    fn read(&self, offset: u64) -> u32 {
        match offset & 0xFF {
            0x00 => self.ioregsel,
            0x10 => {
                let idx = self.ioregsel as usize;
                if idx < 64 { self.regs[idx] } else { 0 }
            }
            _ => 0,
        }
    }

    fn write(&mut self, offset: u64, val: u32) {
        match offset & 0xFF {
            0x00 => self.ioregsel = val,
            0x10 => {
                let idx = self.ioregsel as usize;
                if idx < 64 { self.regs[idx] = val; }
            }
            _ => {}
        }
    }
}

pub struct WhpBackend {
    partition: WHV_PARTITION_HANDLE,
    memory_slots: Vec<MemorySlot>,
    api: WhpApi,
    lapic: SoftLapic,
    ioapic: SoftIoapic,
}

unsafe impl Send for WhpBackend {}

fn check(hr: i32) -> Result<(), VmError> {
    if hr >= 0 { Ok(()) } else { Err(VmError::BackendError(hr)) }
}

impl WhpBackend {
    pub fn new(_ram_bytes: usize) -> Result<Self, VmError> {
        unsafe {
            let dll = LoadLibraryA(b"WinHvPlatform.dll\0".as_ptr());
            if dll.is_null() {
                return Err(VmError::NoHardwareSupport);
            }

            macro_rules! load {
                ($name:expr) => {{
                    let p = GetProcAddress(dll, concat!($name, "\0").as_ptr());
                    if p.is_null() {
                        return Err(VmError::NoHardwareSupport);
                    }
                    core::mem::transmute(p)
                }};
            }

            let api = WhpApi {
                get_capability: load!("WHvGetCapability"),
                create_partition: load!("WHvCreatePartition"),
                setup_partition: load!("WHvSetupPartition"),
                delete_partition: load!("WHvDeletePartition"),
                set_property: load!("WHvSetPartitionProperty"),
                map_gpa: load!("WHvMapGpaRange"),
                unmap_gpa: load!("WHvUnmapGpaRange"),
                create_vp: load!("WHvCreateVirtualProcessor"),
                delete_vp: load!("WHvDeleteVirtualProcessor"),
                run_vp: load!("WHvRunVirtualProcessor"),
                get_regs: load!("WHvGetVirtualProcessorRegisters"),
                set_regs: load!("WHvSetVirtualProcessorRegisters"),
                request_interrupt: {
                    let p = GetProcAddress(dll, b"WHvRequestInterrupt\0".as_ptr());
                    if p.is_null() { None } else { Some(core::mem::transmute(p)) }
                },
            };

            // Check if the hypervisor is present
            // WHvCapabilityCodeHypervisorPresent = 0x00000000
            let mut present: u32 = 0;
            let mut written: u32 = 0;
            let hr = (api.get_capability)(
                0x00000000, // WHvCapabilityCodeHypervisorPresent
                &mut present as *mut u32 as *mut u8,
                core::mem::size_of::<u32>() as u32,
                &mut written,
            );
            if hr < 0 || present == 0 {
                return Err(VmError::NoHardwareSupport);
            }

            let mut partition: WHV_PARTITION_HANDLE = core::ptr::null_mut();
            let hr = (api.create_partition)(&mut partition);
            if hr < 0 {
                return Err(VmError::BackendErrorCtx(hr, "WHvCreatePartition"));
            }

            // Set processor count = 1
            let count: u32 = 1;
            let hr = (api.set_property)(
                partition,
                WHV_PROPERTY_PROCESSOR_COUNT,
                &count as *const u32 as *const u8,
                core::mem::size_of::<u32>() as u32,
            );
            if hr < 0 {
                (api.delete_partition)(partition);
                return Err(VmError::BackendErrorCtx(hr, "WHvSetPartitionProperty(ProcessorCount)"));
            }

            // Enable extended VM exits for CPUID and MSR interception
            let extended_exits: u64 = 0x3; // bit 0 = X64CpuidExit, bit 1 = X64MsrExit
            let hr = (api.set_property)(
                partition,
                WHV_PROPERTY_EXTENDED_VM_EXITS,
                &extended_exits as *const u64 as *const u8,
                core::mem::size_of::<u64>() as u32,
            );
            if hr < 0 {
                // Non-fatal: some WHP versions may not support extended exits
            }

            // Enable Local APIC emulation (XApic mode)
            // WHvPartitionPropertyCodeLocalApicEmulationMode = 0x00001005
            // WHvX64LocalApicEmulationModeXApic = 1
            let apic_mode: u32 = 1;
            let hr = (api.set_property)(
                partition,
                0x00001005, // WHvPartitionPropertyCodeLocalApicEmulationMode
                &apic_mode as *const u32 as *const u8,
                core::mem::size_of::<u32>() as u32,
            );
            if hr < 0 {
                // Non-fatal: older WHP versions may not support APIC emulation
            }

            let hr = (api.setup_partition)(partition);
            if hr < 0 {
                (api.delete_partition)(partition);
                return Err(VmError::BackendErrorCtx(hr, "WHvSetupPartition"));
            }

            Ok(WhpBackend {
                partition,
                memory_slots: Vec::new(),
                api,
                lapic: SoftLapic::new(),
                ioapic: SoftIoapic::new(),
            })
        }
    }

    fn get_regs_raw(&self, id: u32, names: &[u32], values: &mut [WHV_REGISTER_VALUE]) -> Result<(), VmError> {
        check(unsafe {
            (self.api.get_regs)(
                self.partition, id,
                names.as_ptr(), names.len() as u32,
                values.as_mut_ptr(),
            )
        })
    }

    fn set_regs_raw(&self, id: u32, names: &[u32], values: &[WHV_REGISTER_VALUE]) -> Result<(), VmError> {
        check(unsafe {
            (self.api.set_regs)(
                self.partition, id,
                names.as_ptr(), names.len() as u32,
                values.as_ptr(),
            )
        })
    }
}

/// Decode an MMIO instruction to extract access size and write data.
/// WHP doesn't provide these in the exit context, so we parse the instruction bytes.
/// Returns (access_size, write_data). write_data is 0 for reads.
fn decode_mmio_instruction(instr: &[u8], regs: &VcpuRegs) -> (u8, u64) {
    if instr.is_empty() {
        return (4, 0);
    }

    let mut i = 0;
    let mut operand_size_override = false;
    let mut rex = 0u8;

    // Skip prefixes
    while i < instr.len() {
        match instr[i] {
            0x66 => { operand_size_override = true; i += 1; }
            0x67 | 0xF0 | 0xF2 | 0xF3 | 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 => { i += 1; }
            0x40..=0x4F => { rex = instr[i]; i += 1; }
            _ => break,
        }
    }

    if i >= instr.len() {
        return (4, 0);
    }

    let rex_w = (rex & 0x08) != 0;

    let size: u8 = if rex_w { 8 } else if operand_size_override { 2 } else { 4 };

    let opcode = instr[i];
    match opcode {
        // MOV r/m8, r8 (write)
        0x88 => {
            let reg_val = get_reg_from_modrm(instr.get(i + 1).copied().unwrap_or(0), regs, true);
            (1, reg_val)
        }
        // MOV r/m16/32/64, r16/32/64 (write)
        0x89 => {
            let reg_val = get_reg_from_modrm(instr.get(i + 1).copied().unwrap_or(0), regs, false);
            (size, reg_val)
        }
        // MOV r8, r/m8 (read)
        0x8A => (1, 0),
        // MOV r16/32/64, r/m16/32/64 (read)
        0x8B => (size, 0),
        // MOV r/m8, imm8 (write)
        0xC6 => {
            let (_, imm_off) = skip_modrm(&instr[i..]);
            let imm = instr.get(i + imm_off).copied().unwrap_or(0) as u64;
            (1, imm)
        }
        // MOV r/m16/32/64, imm16/32 (write)
        0xC7 => {
            let (_, imm_off) = skip_modrm(&instr[i..]);
            let imm_size = if operand_size_override { 2 } else { 4 };
            let imm = read_imm(&instr[i + imm_off..], imm_size);
            (size, imm)
        }
        // MOV AL, moffs (read) / MOV moffs, AL (write)
        0xA0 => (1, 0),
        0xA1 => (size, 0),
        0xA2 => (1, regs.rax & 0xFF),
        0xA3 => (size, regs.rax),
        // MOVS (REP prefix handled above, single step)
        0xA4 => (1, 0), // MOVSB - read+write, treat as read
        0xA5 => (size, 0),
        // STOS
        0xAA => (1, regs.rax & 0xFF),
        0xAB => (size, regs.rax),
        // Two-byte opcodes (0x0F prefix)
        0x0F if i + 1 < instr.len() => {
            match instr[i + 1] {
                // MOVZX r, r/m8
                0xB6 => (1, 0),
                // MOVZX r, r/m16
                0xB7 => (2, 0),
                // MOVSX r, r/m8
                0xBE => (1, 0),
                // MOVSX r, r/m16
                0xBF => (2, 0),
                _ => (size, 0),
            }
        }
        _ => (size, 0),
    }
}

/// Extract the register value indicated by the reg field of ModR/M byte
fn get_reg_from_modrm(modrm: u8, regs: &VcpuRegs, byte_reg: bool) -> u64 {
    let reg = ((modrm >> 3) & 7) as usize;
    let vals = [regs.rax, regs.rcx, regs.rdx, regs.rbx, regs.rsp, regs.rbp, regs.rsi, regs.rdi];
    let v = if reg < 8 { vals[reg] } else { 0 };
    if byte_reg { v & 0xFF } else { v }
}

/// Skip past the ModR/M and SIB bytes + displacement, return (modrm_byte, total_bytes_consumed)
fn skip_modrm(instr: &[u8]) -> (u8, usize) {
    if instr.len() < 2 { return (0, 2); }
    let modrm = instr[1];
    let mod_bits = (modrm >> 6) & 3;
    let rm = modrm & 7;
    let mut off = 2usize; // opcode + modrm
    if mod_bits != 3 {
        if rm == 4 { off += 1; } // SIB byte
        match mod_bits {
            0 if rm == 5 => { off += 4; } // disp32
            1 => { off += 1; } // disp8
            2 => { off += 4; } // disp32
            _ => {}
        }
    }
    (modrm, off)
}

/// Read an immediate value of the given size
fn read_imm(data: &[u8], size: u8) -> u64 {
    match size {
        1 if data.len() >= 1 => data[0] as u64,
        2 if data.len() >= 2 => u16::from_le_bytes([data[0], data[1]]) as u64,
        4 if data.len() >= 4 => u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as u64,
        _ => 0,
    }
}

fn seg_to_whv(seg: &SegmentReg) -> WhvSegment {
    let attrs: u16 =
        (seg.type_ as u16 & 0xF)
        | ((seg.s as u16 & 1) << 4)
        | ((seg.dpl as u16 & 3) << 5)
        | ((seg.present as u16 & 1) << 7)
        | ((seg.avl as u16 & 1) << 12)
        | ((seg.l as u16 & 1) << 13)
        | ((seg.db as u16 & 1) << 14)
        | ((seg.g as u16 & 1) << 15);
    WhvSegment {
        base: seg.base,
        limit: seg.limit,
        selector: seg.selector,
        attributes: attrs,
    }
}

fn whv_to_seg(s: &WhvSegment) -> SegmentReg {
    let a = s.attributes;
    SegmentReg {
        base: s.base,
        limit: s.limit,
        selector: s.selector,
        type_: (a & 0xF) as u8,
        s: ((a >> 4) & 1) as u8,
        dpl: ((a >> 5) & 3) as u8,
        present: ((a >> 7) & 1) as u8,
        avl: ((a >> 12) & 1) as u8,
        l: ((a >> 13) & 1) as u8,
        db: ((a >> 14) & 1) as u8,
        g: ((a >> 15) & 1) as u8,
    }
}

impl VmBackend for WhpBackend {
    fn destroy(&mut self) {
        unsafe {
            (self.api.delete_partition)(self.partition);
        }
        self.partition = core::ptr::null_mut();
        self.memory_slots.clear();
    }

    fn reset(&mut self) -> Result<(), VmError> {
        // WHP has no direct reset; caller recreates partition
        Ok(())
    }

    fn set_memory_region(&mut self, slot: u32, guest_phys: u64, size: u64, host_ptr: *mut u8) -> Result<(), VmError> {
        // If slot exists, unmap first
        if let Some(pos) = self.memory_slots.iter().position(|s| s.slot == slot) {
            let old = &self.memory_slots[pos];
            let _ = unsafe { (self.api.unmap_gpa)(self.partition, old.guest_phys, old.size) };
            self.memory_slots.remove(pos);
        }

        if size == 0 {
            return Ok(());
        }

        let flags = WHV_MAP_GPA_RANGE_FLAG_READ | WHV_MAP_GPA_RANGE_FLAG_WRITE | WHV_MAP_GPA_RANGE_FLAG_EXECUTE;
        check(unsafe {
            (self.api.map_gpa)(self.partition, host_ptr, guest_phys, size, flags)
        })?;

        self.memory_slots.push(MemorySlot { slot, guest_phys, size, host_ptr });
        Ok(())
    }

    fn read_phys(&self, addr: u64, buf: &mut [u8]) -> Result<(), VmError> {
        for slot in &self.memory_slots {
            if addr >= slot.guest_phys && addr + buf.len() as u64 <= slot.guest_phys + slot.size {
                let offset = (addr - slot.guest_phys) as usize;
                unsafe {
                    core::ptr::copy_nonoverlapping(slot.host_ptr.add(offset), buf.as_mut_ptr(), buf.len());
                }
                return Ok(());
            }
        }
        Err(VmError::MemoryMapFailed)
    }

    fn write_phys(&mut self, addr: u64, buf: &[u8]) -> Result<(), VmError> {
        for slot in &self.memory_slots {
            if addr >= slot.guest_phys && addr + buf.len() as u64 <= slot.guest_phys + slot.size {
                let offset = (addr - slot.guest_phys) as usize;
                unsafe {
                    core::ptr::copy_nonoverlapping(buf.as_ptr(), slot.host_ptr.add(offset), buf.len());
                }
                return Ok(());
            }
        }
        Err(VmError::MemoryMapFailed)
    }

    fn create_vcpu(&mut self, id: u32) -> Result<(), VmError> {
        check(unsafe { (self.api.create_vp)(self.partition, id, 0) })?;

        // Smoke test: try reading a single register (RIP) to verify the VP works
        let mut val = WHV_REGISTER_VALUE::default();
        let name = REG_RIP;
        let hr = unsafe {
            (self.api.get_regs)(self.partition, id, &name, 1, &mut val)
        };
        if hr < 0 {
            return Err(VmError::BackendErrorCtx(hr, "WHvGetVirtualProcessorRegisters(RIP) after create"));
        }
        Ok(())
    }

    fn destroy_vcpu(&mut self, id: u32) -> Result<(), VmError> {
        check(unsafe { (self.api.delete_vp)(self.partition, id) })
    }

    fn run_vcpu(&mut self, id: u32) -> Result<VmExitReason, VmError> {
        loop {
            let mut exit_ctx = core::mem::MaybeUninit::<WHV_RUN_VP_EXIT_CONTEXT>::uninit();
            check(unsafe {
                (self.api.run_vp)(
                    self.partition, id,
                    exit_ctx.as_mut_ptr() as *mut u8,
                    core::mem::size_of::<WHV_RUN_VP_EXIT_CONTEXT>() as u32,
                )
            })?;

            let ctx = unsafe { exit_ctx.assume_init() };
            // Instruction length is lower 4 bits of vp_context[2] (upper 4 bits = Cr8)
            let instr_len = (ctx.vp_context[2] & 0x0F) as u64;

            match ctx.exit_reason {
                WHV_EXIT_REASON_HALT => return Ok(VmExitReason::Halted),
                WHV_EXIT_REASON_IO_PORT => {
                    // WHV_X64_IO_PORT_ACCESS_CONTEXT layout (from Windows SDK):
                    // [0x00]     InstructionByteCount: u8
                    // [0x01..04] Reserved: [u8; 3]
                    // [0x04..14] InstructionBytes: [u8; 16]
                    // [0x14..18] AccessInfo: u32 (bit 0=IsWrite, bits 1-3=AccessSize)
                    // [0x18..1A] PortNumber: u16
                    // [0x1A..20] Reserved
                    // [0x20..28] Rax: u64
                    let data = &ctx.exit_data;
                    let access_info = u32::from_le_bytes([data[0x14], data[0x15], data[0x16], data[0x17]]);
                    let is_write = (access_info & 1) != 0;
                    let access_size = ((access_info >> 1) & 0x7) as u8;
                    let access_size = if access_size == 0 { 1 } else { access_size };
                    let port = u16::from_le_bytes([data[0x18], data[0x19]]);
                    let rax = u64::from_le_bytes([
                        data[0x20], data[0x21], data[0x22], data[0x23],
                        data[0x24], data[0x25], data[0x26], data[0x27],
                    ]);

                    // WHP does not auto-advance RIP for I/O exits — advance now
                    let mut regs = self.get_vcpu_regs(id)?;
                    regs.rip += instr_len;
                    self.set_vcpu_regs(id, &regs)?;

                    return if is_write {
                        Ok(VmExitReason::IoOut { port, size: access_size, data: rax as u32 })
                    } else {
                        Ok(VmExitReason::IoIn { port, size: access_size })
                    };
                }
                WHV_EXIT_REASON_MEMORY_ACCESS => {
                    // WHV_MEMORY_ACCESS_CONTEXT layout:
                    // [0x00]     InstructionByteCount: u8
                    // [0x01..04] Reserved: [u8; 3]
                    // [0x04..14] InstructionBytes: [u8; 16]
                    // [0x14..18] AccessInfo: u32 (bits 0-1=AccessType: 0=Read,1=Write,2=Execute)
                    // [0x18..20] Gpa: u64
                    // [0x20..28] Gva: u64
                    let data = &ctx.exit_data;
                    let instr_byte_count = data[0x00] as usize;
                    let instr_bytes = &data[0x04..0x04 + instr_byte_count.min(16)];
                    let access_info = u32::from_le_bytes([data[0x14], data[0x15], data[0x16], data[0x17]]);
                    let access_type = access_info & 0x3;
                    let is_write = access_type == 1;
                    let gpa = u64::from_le_bytes([
                        data[0x18], data[0x19], data[0x1A], data[0x1B],
                        data[0x1C], data[0x1D], data[0x1E], data[0x1F],
                    ]);

                    // Decode instruction to get access size and write data
                    let regs = self.get_vcpu_regs(id)?;
                    let (access_size, write_data) = decode_mmio_instruction(instr_bytes, &regs);

                    // Handle LAPIC MMIO internally (0xFEE00000-0xFEE00FFF)
                    if gpa >= 0xFEE0_0000 && gpa < 0xFEE0_1000 {
                        let mut new_regs = regs;
                        new_regs.rip += instr_len;
                        if is_write {
                            self.lapic.write(gpa, write_data as u32);
                        } else {
                            // LAPIC read: put result in destination register
                            // For MOV r, [mem] the destination is the reg field of ModR/M
                            let val = self.lapic.read(gpa) as u64;
                            // The most common pattern is MOV EAX, [addr] — put in RAX
                            // More precisely, decode the dest reg from ModR/M
                            if instr_byte_count > 0 {
                                let mut pi = 0;
                                while pi < instr_bytes.len() {
                                    match instr_bytes[pi] {
                                        0x66 | 0x67 | 0xF0 | 0xF2 | 0xF3
                                        | 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65
                                        | 0x40..=0x4F => { pi += 1; }
                                        _ => break,
                                    }
                                }
                                if pi < instr_bytes.len() {
                                    let op = instr_bytes[pi];
                                    if (op == 0x8B || op == 0x8A) && pi + 1 < instr_bytes.len() {
                                        let modrm = instr_bytes[pi + 1];
                                        let dest = ((modrm >> 3) & 7) as usize;
                                        match dest {
                                            0 => new_regs.rax = val,
                                            1 => new_regs.rcx = val,
                                            2 => new_regs.rdx = val,
                                            3 => new_regs.rbx = val,
                                            4 => new_regs.rsp = val,
                                            5 => new_regs.rbp = val,
                                            6 => new_regs.rsi = val,
                                            7 => new_regs.rdi = val,
                                            _ => new_regs.rax = val,
                                        }
                                    } else {
                                        new_regs.rax = val;
                                    }
                                } else {
                                    new_regs.rax = val;
                                }
                            } else {
                                new_regs.rax = val;
                            }
                        }
                        self.set_vcpu_regs(id, &new_regs)?;
                        continue; // re-enter guest
                    }

                    // Handle IOAPIC MMIO internally (0xFEC00000-0xFEC00FFF)
                    if gpa >= 0xFEC0_0000 && gpa < 0xFEC0_1000 {
                        let mmio_off = gpa - 0xFEC0_0000;
                        let mut new_regs = regs;
                        new_regs.rip += instr_len;
                        if is_write {
                            self.ioapic.write(mmio_off, write_data as u32);
                        } else {
                            let val = self.ioapic.read(mmio_off) as u64;
                            // Decode dest register same as LAPIC
                            if instr_byte_count > 0 {
                                let mut pi = 0;
                                while pi < instr_bytes.len() {
                                    match instr_bytes[pi] {
                                        0x66 | 0x67 | 0xF0 | 0xF2 | 0xF3
                                        | 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65
                                        | 0x40..=0x4F => { pi += 1; }
                                        _ => break,
                                    }
                                }
                                if pi < instr_bytes.len() && (instr_bytes[pi] == 0x8B || instr_bytes[pi] == 0x8A) && pi + 1 < instr_bytes.len() {
                                    let dest = ((instr_bytes[pi + 1] >> 3) & 7) as usize;
                                    match dest {
                                        0 => new_regs.rax = val,
                                        1 => new_regs.rcx = val,
                                        2 => new_regs.rdx = val,
                                        3 => new_regs.rbx = val,
                                        4 => new_regs.rsp = val,
                                        5 => new_regs.rbp = val,
                                        6 => new_regs.rsi = val,
                                        7 => new_regs.rdi = val,
                                        _ => new_regs.rax = val,
                                    }
                                } else {
                                    new_regs.rax = val;
                                }
                            } else {
                                new_regs.rax = val;
                            }
                        }
                        self.set_vcpu_regs(id, &new_regs)?;
                        continue; // re-enter guest
                    }

                    // WHP does not auto-advance RIP for MMIO exits
                    let mut new_regs = regs;
                    new_regs.rip += instr_len;
                    self.set_vcpu_regs(id, &new_regs)?;

                    return if is_write {
                        Ok(VmExitReason::MmioWrite { addr: gpa, size: access_size, data: write_data })
                    } else {
                        Ok(VmExitReason::MmioRead { addr: gpa, size: access_size })
                    };
                }
                WHV_EXIT_REASON_MSR => {
                    // WHV_X64_MSR_ACCESS_CONTEXT layout:
                    // [0x00..04] AccessInfo: u32 (bit 0=IsWrite)
                    // [0x04..08] MsrNumber: u32
                    // [0x08..10] Rax: u64
                    // [0x10..18] Rdx: u64
                    let data = &ctx.exit_data;
                    let access_info = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                    let is_write = (access_info & 1) != 0;

                    if !is_write {
                        // RDMSR: return 0 in RAX:RDX
                        let mut regs = self.get_vcpu_regs(id)?;
                        regs.rax = 0;
                        regs.rdx = 0;
                        regs.rip += instr_len;
                        self.set_vcpu_regs(id, &regs)?;
                    } else {
                        // WRMSR: ignore, just advance RIP
                        let mut regs = self.get_vcpu_regs(id)?;
                        regs.rip += instr_len;
                        self.set_vcpu_regs(id, &regs)?;
                    }
                    // Re-enter guest
                    continue;
                }
                WHV_EXIT_REASON_CPUID => {
                    // Handle CPUID internally: execute native CPUID and return results.
                    // WHV_X64_CPUID_ACCESS_CONTEXT layout:
                    // [0x00..08] Rax (=leaf): u64
                    // [0x08..10] Rcx (=subleaf): u64
                    // [0x10..18] Rdx: u64
                    // [0x18..20] Rbx: u64
                    // [0x20..28] DefaultResultRax: u64
                    // [0x28..30] DefaultResultRcx: u64
                    // [0x30..38] DefaultResultRdx: u64
                    // [0x38..40] DefaultResultRbx: u64
                    let data = &ctx.exit_data;
                    let leaf = u64::from_le_bytes([
                        data[0x00], data[0x01], data[0x02], data[0x03],
                        data[0x04], data[0x05], data[0x06], data[0x07],
                    ]) as u32;
                    let subleaf = u64::from_le_bytes([
                        data[0x08], data[0x09], data[0x0A], data[0x0B],
                        data[0x0C], data[0x0D], data[0x0E], data[0x0F],
                    ]) as u32;

                    // Execute CPUID on host and pass through (with some filtering)
                    let (mut eax, mut ebx, mut ecx, mut edx) = (0u32, 0u32, 0u32, 0u32);
                    unsafe {
                        core::arch::asm!(
                            "push rbx",
                            "cpuid",
                            "mov {ebx_out:e}, ebx",
                            "pop rbx",
                            inout("eax") leaf => eax,
                            ebx_out = out(reg) ebx,
                            inout("ecx") subleaf => ecx,
                            out("edx") edx,
                            options(nostack),
                        );
                    }

                    // Filter out features we don't want to expose to guest
                    if leaf == 1 {
                        ecx &= !(1 << 26); // Remove XSAVE
                        ecx &= !(1 << 27); // Remove OSXSAVE
                        ecx &= !(1 << 5);  // Remove VMX
                    }
                    if leaf == 7 && subleaf == 0 {
                        ebx &= !(1 << 3);  // Remove BMI1
                        ebx &= !(1 << 8);  // Remove BMI2
                    }

                    let mut regs = self.get_vcpu_regs(id)?;
                    regs.rax = eax as u64;
                    regs.rbx = ebx as u64;
                    regs.rcx = ecx as u64;
                    regs.rdx = edx as u64;
                    regs.rip += instr_len;
                    self.set_vcpu_regs(id, &regs)?;
                    // Re-enter guest
                    continue;
                }
                WHV_EXIT_REASON_CANCELED => return Ok(VmExitReason::Halted),
                WHV_EXIT_REASON_NONE => return Ok(VmExitReason::Error),
                _ => return Ok(VmExitReason::Error),
            }
        }
    }

    fn get_vcpu_regs(&self, id: u32) -> Result<VcpuRegs, VmError> {
        let mut vals = [WHV_REGISTER_VALUE::default(); 18];
        self.get_regs_raw(id, &GP_REG_NAMES, &mut vals)?;

        Ok(VcpuRegs {
            rax: unsafe { vals[0].reg64 },
            rbx: unsafe { vals[1].reg64 },
            rcx: unsafe { vals[2].reg64 },
            rdx: unsafe { vals[3].reg64 },
            rsi: unsafe { vals[4].reg64 },
            rdi: unsafe { vals[5].reg64 },
            rbp: unsafe { vals[6].reg64 },
            rsp: unsafe { vals[7].reg64 },
            r8:  unsafe { vals[8].reg64 },
            r9:  unsafe { vals[9].reg64 },
            r10: unsafe { vals[10].reg64 },
            r11: unsafe { vals[11].reg64 },
            r12: unsafe { vals[12].reg64 },
            r13: unsafe { vals[13].reg64 },
            r14: unsafe { vals[14].reg64 },
            r15: unsafe { vals[15].reg64 },
            rip: unsafe { vals[16].reg64 },
            rflags: unsafe { vals[17].reg64 },
        })
    }

    fn set_vcpu_regs(&mut self, id: u32, regs: &VcpuRegs) -> Result<(), VmError> {
        let vals = [
            WHV_REGISTER_VALUE::from_u64(regs.rax),
            WHV_REGISTER_VALUE::from_u64(regs.rbx),
            WHV_REGISTER_VALUE::from_u64(regs.rcx),
            WHV_REGISTER_VALUE::from_u64(regs.rdx),
            WHV_REGISTER_VALUE::from_u64(regs.rsi),
            WHV_REGISTER_VALUE::from_u64(regs.rdi),
            WHV_REGISTER_VALUE::from_u64(regs.rbp),
            WHV_REGISTER_VALUE::from_u64(regs.rsp),
            WHV_REGISTER_VALUE::from_u64(regs.r8),
            WHV_REGISTER_VALUE::from_u64(regs.r9),
            WHV_REGISTER_VALUE::from_u64(regs.r10),
            WHV_REGISTER_VALUE::from_u64(regs.r11),
            WHV_REGISTER_VALUE::from_u64(regs.r12),
            WHV_REGISTER_VALUE::from_u64(regs.r13),
            WHV_REGISTER_VALUE::from_u64(regs.r14),
            WHV_REGISTER_VALUE::from_u64(regs.r15),
            WHV_REGISTER_VALUE::from_u64(regs.rip),
            WHV_REGISTER_VALUE::from_u64(regs.rflags),
        ];
        self.set_regs_raw(id, &GP_REG_NAMES, &vals)
    }

    fn get_vcpu_sregs(&self, id: u32) -> Result<VcpuSregs, VmError> {
        let mut vals = [WHV_REGISTER_VALUE::default(); 13];
        self.get_regs_raw(id, &SREG_NAMES, &mut vals)?;

        let mut ext_vals = [WHV_REGISTER_VALUE::default(); 2];
        self.get_regs_raw(id, &SREG_NAMES_EXT, &mut ext_vals)?;

        Ok(VcpuSregs {
            cs:  whv_to_seg(unsafe { &vals[0].segment }),
            ds:  whv_to_seg(unsafe { &vals[1].segment }),
            es:  whv_to_seg(unsafe { &vals[2].segment }),
            fs:  whv_to_seg(unsafe { &vals[3].segment }),
            gs:  whv_to_seg(unsafe { &vals[4].segment }),
            ss:  whv_to_seg(unsafe { &vals[5].segment }),
            tr:  whv_to_seg(unsafe { &vals[6].segment }),
            ldt: whv_to_seg(unsafe { &vals[7].segment }),
            gdt: DescriptorTable {
                base: unsafe { vals[8].table.base },
                limit: unsafe { vals[8].table.limit },
            },
            idt: DescriptorTable {
                base: unsafe { vals[9].table.base },
                limit: unsafe { vals[9].table.limit },
            },
            cr0: unsafe { vals[10].reg64 },
            cr2: unsafe { vals[11].reg64 },
            cr3: unsafe { vals[12].reg64 },
            cr4: unsafe { ext_vals[0].reg64 },
            efer: unsafe { ext_vals[1].reg64 },
        })
    }

    fn set_vcpu_sregs(&mut self, id: u32, sregs: &VcpuSregs) -> Result<(), VmError> {
        let vals: [WHV_REGISTER_VALUE; 13] = [
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.cs)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.ds)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.es)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.fs)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.gs)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.ss)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.tr)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.ldt)),
            WHV_REGISTER_VALUE::from_table(WhvTable { _pad: [0; 3], limit: sregs.gdt.limit, base: sregs.gdt.base }),
            WHV_REGISTER_VALUE::from_table(WhvTable { _pad: [0; 3], limit: sregs.idt.limit, base: sregs.idt.base }),
            WHV_REGISTER_VALUE::from_u64(sregs.cr0),
            WHV_REGISTER_VALUE::from_u64(sregs.cr2),
            WHV_REGISTER_VALUE::from_u64(sregs.cr3),
        ];
        self.set_regs_raw(id, &SREG_NAMES, &vals)?;

        let ext_vals = [
            WHV_REGISTER_VALUE::from_u64(sregs.cr4),
            WHV_REGISTER_VALUE::from_u64(sregs.efer),
        ];
        self.set_regs_raw(id, &SREG_NAMES_EXT, &ext_vals)
    }

    fn inject_interrupt(&mut self, id: u32, vector: u8) -> Result<(), VmError> {
        if let Some(req_int) = self.api.request_interrupt {
            // WHV_INTERRUPT_CONTROL structure (8 bytes):
            // bits[1:0]  = Type (0=Fixed)
            // bit 2      = DestinationMode (0=Physical)
            // bit 3      = TriggerMode (0=Edge)
            // bits[15:8] = Vector
            // bits[63:32]= Destination (0 = BSP APIC ID)
            let ctrl: u64 = (vector as u64) << 8; // Fixed, Physical, Edge, dest=0
            let hr = (req_int)(self.partition, &ctrl as *const u64 as *const u8, 8);
            if hr < 0 {
                return Err(VmError::BackendError(hr));
            }
            Ok(())
        } else {
            // Fallback: set pending interruption register directly
            let val = WHV_REGISTER_VALUE::from_u64((vector as u64) | (0u64 << 8) | (1u64 << 12));
            self.set_regs_raw(id, &[REG_PENDING_INTERRUPTION], &[val])
        }
    }

    fn inject_exception(&mut self, id: u32, vector: u8, error_code: Option<u32>) -> Result<(), VmError> {
        // type=3 (hardware exception), bit 12=deliver
        let mut val: u64 = (vector as u64) | (3u64 << 8) | (1u64 << 12);
        if let Some(ec) = error_code {
            val |= (ec as u64) << 32;
            val |= 1u64 << 13; // has error code
        }
        let reg_val = WHV_REGISTER_VALUE::from_u64(val);
        self.set_regs_raw(id, &[REG_PENDING_INTERRUPTION], &[reg_val])
    }

    fn inject_nmi(&mut self, id: u32) -> Result<(), VmError> {
        // type=2 (NMI), bit 12=deliver
        let val = WHV_REGISTER_VALUE::from_u64((2u64 << 8) | (1u64 << 12));
        self.set_regs_raw(id, &[REG_PENDING_INTERRUPTION], &[val])
    }

    fn request_interrupt_window(&mut self, _id: u32, _enable: bool) -> Result<(), VmError> {
        // WHP handles interrupt windows via extended VM exits property
        // This requires setting WHvPartitionPropertyCodeExtendedVmExits
        // For now, this is a no-op; the caller polls interrupt readiness
        Ok(())
    }

    fn set_cpuid(&mut self, _entries: &[CpuidEntry]) -> Result<(), VmError> {
        // WHP does not support custom CPUID configuration directly.
        // CPUID exits must be handled via WHvRunVpExitReasonX64Cpuid exit reason
        // after enabling CPUID exits in extended VM exits.
        Ok(())
    }
}

impl Drop for WhpBackend {
    fn drop(&mut self) {
        if !self.partition.is_null() {
            self.destroy();
        }
    }
}
