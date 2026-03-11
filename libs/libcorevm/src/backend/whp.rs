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

#[repr(C)]
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
    base: u64,
    limit: u16,
    _pad: [u16; 3],
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
const WHV_PROPERTY_EXTENDED_VM_EXITS: u32 = 0x00000002; // WHvPartitionPropertyCodeExtendedVmExits

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

// Register name constants
const REG_RAX: u32 = 0x00020000;
const REG_RCX: u32 = 0x00020001;
const REG_RDX: u32 = 0x00020002;
const REG_RBX: u32 = 0x00020003;
const REG_RSP: u32 = 0x00020004;
const REG_RBP: u32 = 0x00020005;
const REG_RSI: u32 = 0x00020006;
const REG_RDI: u32 = 0x00020007;
const REG_R8: u32 = 0x00020008;
const REG_R9: u32 = 0x00020009;
const REG_R10: u32 = 0x0002000A;
const REG_R11: u32 = 0x0002000B;
const REG_R12: u32 = 0x0002000C;
const REG_R13: u32 = 0x0002000D;
const REG_R14: u32 = 0x0002000E;
const REG_R15: u32 = 0x0002000F;
const REG_RIP: u32 = 0x00020010;
const REG_RFLAGS: u32 = 0x00020011;

const REG_CS: u32 = 0x00030000;
const REG_DS: u32 = 0x00030001;
const REG_ES: u32 = 0x00030002;
const REG_FS: u32 = 0x00030003;
const REG_GS: u32 = 0x00030004;
const REG_SS: u32 = 0x00030005;
const REG_TR: u32 = 0x00030006;
const REG_LDTR: u32 = 0x00030007;
const REG_GDTR: u32 = 0x00030008;
const REG_IDTR: u32 = 0x00030009;

const REG_CR0: u32 = 0x00040000;
const REG_CR2: u32 = 0x00040001;
const REG_CR3: u32 = 0x00040002;
const REG_CR4: u32 = 0x00040003;
const REG_EFER: u32 = 0x00050001;

const REG_PENDING_INTERRUPTION: u32 = 0x00080015;

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

pub struct WhpBackend {
    partition: WHV_PARTITION_HANDLE,
    memory_slots: Vec<MemorySlot>,
    api: WhpApi,
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

            let hr = (api.setup_partition)(partition);
            if hr < 0 {
                (api.delete_partition)(partition);
                return Err(VmError::BackendErrorCtx(hr, "WHvSetupPartition"));
            }

            Ok(WhpBackend {
                partition,
                memory_slots: Vec::new(),
                api,
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
        check(unsafe { (self.api.create_vp)(self.partition, id, 0) })
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
                    let access_size_enc = ((access_info >> 1) & 0x7) as u8;
                    let access_size = match access_size_enc { 0 => 1, 1 => 2, 3 => 4, _ => 1 };
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
                    let access_info = u32::from_le_bytes([data[0x14], data[0x15], data[0x16], data[0x17]]);
                    let access_type = access_info & 0x3;
                    let is_write = access_type == 1;
                    let gpa = u64::from_le_bytes([
                        data[0x18], data[0x19], data[0x1A], data[0x1B],
                        data[0x1C], data[0x1D], data[0x1E], data[0x1F],
                    ]);
                    let size = if instr_len > 0 { instr_len as u8 } else { 4 };

                    // WHP does not auto-advance RIP for MMIO exits
                    let mut regs = self.get_vcpu_regs(id)?;
                    regs.rip += instr_len;
                    self.set_vcpu_regs(id, &regs)?;

                    return if is_write {
                        Ok(VmExitReason::MmioWrite { addr: gpa, size, data: 0 })
                    } else {
                        Ok(VmExitReason::MmioRead { addr: gpa, size })
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
            WHV_REGISTER_VALUE { reg64: regs.rax },
            WHV_REGISTER_VALUE { reg64: regs.rbx },
            WHV_REGISTER_VALUE { reg64: regs.rcx },
            WHV_REGISTER_VALUE { reg64: regs.rdx },
            WHV_REGISTER_VALUE { reg64: regs.rsi },
            WHV_REGISTER_VALUE { reg64: regs.rdi },
            WHV_REGISTER_VALUE { reg64: regs.rbp },
            WHV_REGISTER_VALUE { reg64: regs.rsp },
            WHV_REGISTER_VALUE { reg64: regs.r8 },
            WHV_REGISTER_VALUE { reg64: regs.r9 },
            WHV_REGISTER_VALUE { reg64: regs.r10 },
            WHV_REGISTER_VALUE { reg64: regs.r11 },
            WHV_REGISTER_VALUE { reg64: regs.r12 },
            WHV_REGISTER_VALUE { reg64: regs.r13 },
            WHV_REGISTER_VALUE { reg64: regs.r14 },
            WHV_REGISTER_VALUE { reg64: regs.r15 },
            WHV_REGISTER_VALUE { reg64: regs.rip },
            WHV_REGISTER_VALUE { reg64: regs.rflags },
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
            WHV_REGISTER_VALUE { segment: seg_to_whv(&sregs.cs) },
            WHV_REGISTER_VALUE { segment: seg_to_whv(&sregs.ds) },
            WHV_REGISTER_VALUE { segment: seg_to_whv(&sregs.es) },
            WHV_REGISTER_VALUE { segment: seg_to_whv(&sregs.fs) },
            WHV_REGISTER_VALUE { segment: seg_to_whv(&sregs.gs) },
            WHV_REGISTER_VALUE { segment: seg_to_whv(&sregs.ss) },
            WHV_REGISTER_VALUE { segment: seg_to_whv(&sregs.tr) },
            WHV_REGISTER_VALUE { segment: seg_to_whv(&sregs.ldt) },
            WHV_REGISTER_VALUE { table: WhvTable { base: sregs.gdt.base, limit: sregs.gdt.limit, _pad: [0; 3] } },
            WHV_REGISTER_VALUE { table: WhvTable { base: sregs.idt.base, limit: sregs.idt.limit, _pad: [0; 3] } },
            WHV_REGISTER_VALUE { reg64: sregs.cr0 },
            WHV_REGISTER_VALUE { reg64: sregs.cr2 },
            WHV_REGISTER_VALUE { reg64: sregs.cr3 },
        ];
        self.set_regs_raw(id, &SREG_NAMES, &vals)?;

        let ext_vals = [
            WHV_REGISTER_VALUE { reg64: sregs.cr4 },
            WHV_REGISTER_VALUE { reg64: sregs.efer },
        ];
        self.set_regs_raw(id, &SREG_NAMES_EXT, &ext_vals)
    }

    fn inject_interrupt(&mut self, id: u32, vector: u8) -> Result<(), VmError> {
        // WHV_INTERRUPT_CONTROL: bits[7:0]=vector, bits[11:8]=type(0=ext int), bit 12=deliver
        let val = WHV_REGISTER_VALUE {
            reg64: (vector as u64) | (0u64 << 8) | (1u64 << 12),
        };
        let name = REG_PENDING_INTERRUPTION;
        self.set_regs_raw(id, &[name], &[val])
    }

    fn inject_exception(&mut self, id: u32, vector: u8, error_code: Option<u32>) -> Result<(), VmError> {
        // type=3 (hardware exception), bit 12=deliver
        let mut val: u64 = (vector as u64) | (3u64 << 8) | (1u64 << 12);
        if let Some(ec) = error_code {
            val |= (ec as u64) << 32;
            val |= 1u64 << 13; // has error code
        }
        let reg_val = WHV_REGISTER_VALUE { reg64: val };
        self.set_regs_raw(id, &[REG_PENDING_INTERRUPTION], &[reg_val])
    }

    fn inject_nmi(&mut self, id: u32) -> Result<(), VmError> {
        // type=2 (NMI), bit 12=deliver
        let val = WHV_REGISTER_VALUE {
            reg64: (2u64 << 8) | (1u64 << 12),
        };
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
