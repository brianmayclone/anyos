//! Windows Hypervisor Platform (WHP) backend.
//!
//! Uses the WHP API from WinHvPlatform.dll via FFI to provide hardware
//! virtualization on Windows 10+ with Hyper-V enabled.

use alloc::collections::BTreeMap;
use alloc::vec;

use super::{
    DtableState, HvError, HypervisorBackend, MemoryRegion, SegmentState, VcpuRegs, VmExit,
};

// ════════════════════════════════════════════════════════════════════════
// WHP FFI types and constants
// ════════════════════════════════════════════════════════════════════════

type WHV_PARTITION_HANDLE = *mut core::ffi::c_void;
type HRESULT = i32;

/// 128-bit register value used by WHP get/set register APIs.
#[repr(C)]
#[derive(Copy, Clone)]
union WHV_REGISTER_VALUE {
    reg64: u64,
    reg32: u32,
    reg128: [u64; 2],
    segment: WhvSegmentRegister,
    table: WhvTableRegister,
}

impl Default for WHV_REGISTER_VALUE {
    fn default() -> Self {
        WHV_REGISTER_VALUE { reg128: [0; 2] }
    }
}

/// WHP segment register layout within WHV_REGISTER_VALUE.
#[repr(C)]
#[derive(Copy, Clone, Default)]
struct WhvSegmentRegister {
    base: u64,
    limit: u32,
    selector: u16,
    attributes: u16,
}

/// WHP table register layout (GDTR/IDTR).
#[repr(C)]
#[derive(Copy, Clone, Default)]
struct WhvTableRegister {
    _pad: [u16; 3],
    limit: u16,
    base: u64,
}

const S_OK: HRESULT = 0;

// Partition property codes.
const WHV_PARTITION_PROPERTY_PROCESSOR_COUNT: u32 = 0x00001FFF;
const WHV_PARTITION_PROPERTY_EXTENDED_VM_EXITS: u32 = 0x00001006;

// Extended VM exits bitmask — enable CPUID and MSR exits.
const WHV_EXTENDED_VM_EXIT_CPUID: u64 = 1 << 1;
const WHV_EXTENDED_VM_EXIT_MSR: u64 = 1 << 3;

// GPA range mapping flags.
const WHV_MAP_GPA_RANGE_FLAG_READ: u32 = 0x1;
const WHV_MAP_GPA_RANGE_FLAG_WRITE: u32 = 0x2;
const WHV_MAP_GPA_RANGE_FLAG_EXECUTE: u32 = 0x4;

// WHV_RUN_VP_EXIT_REASON values.
const WHV_EXIT_REASON_NONE: u32 = 0;
const WHV_EXIT_REASON_MEMORY_ACCESS: u32 = 1;
const WHV_EXIT_REASON_IO_PORT_ACCESS: u32 = 2;
const WHV_EXIT_REASON_HALT: u32 = 8;
const WHV_EXIT_REASON_CANCELED: u32 = 16;
const WHV_EXIT_REASON_CPUID: u32 = 17;
const WHV_EXIT_REASON_MSR_ACCESS: u32 = 18;
const WHV_EXIT_REASON_INTERRUPT_WINDOW: u32 = 19;

// WHV_REGISTER_NAME values for x86-64 registers.
const WHV_X64_REGISTER_RAX: u32 = 0x00000000;
const WHV_X64_REGISTER_RCX: u32 = 0x00000001;
const WHV_X64_REGISTER_RDX: u32 = 0x00000002;
const WHV_X64_REGISTER_RBX: u32 = 0x00000003;
const WHV_X64_REGISTER_RSP: u32 = 0x00000004;
const WHV_X64_REGISTER_RBP: u32 = 0x00000005;
const WHV_X64_REGISTER_RSI: u32 = 0x00000006;
const WHV_X64_REGISTER_RDI: u32 = 0x00000007;
const WHV_X64_REGISTER_R8: u32 = 0x00000008;
const WHV_X64_REGISTER_R9: u32 = 0x00000009;
const WHV_X64_REGISTER_R10: u32 = 0x0000000A;
const WHV_X64_REGISTER_R11: u32 = 0x0000000B;
const WHV_X64_REGISTER_R12: u32 = 0x0000000C;
const WHV_X64_REGISTER_R13: u32 = 0x0000000D;
const WHV_X64_REGISTER_R14: u32 = 0x0000000E;
const WHV_X64_REGISTER_R15: u32 = 0x0000000F;
const WHV_X64_REGISTER_RIP: u32 = 0x00000010;
const WHV_X64_REGISTER_RFLAGS: u32 = 0x00000011;

// Segment registers.
const WHV_X64_REGISTER_ES: u32 = 0x00000012;
const WHV_X64_REGISTER_CS: u32 = 0x00000013;
const WHV_X64_REGISTER_SS: u32 = 0x00000014;
const WHV_X64_REGISTER_DS: u32 = 0x00000015;
const WHV_X64_REGISTER_FS: u32 = 0x00000016;
const WHV_X64_REGISTER_GS: u32 = 0x00000017;
const WHV_X64_REGISTER_LDTR: u32 = 0x00000018;
const WHV_X64_REGISTER_TR: u32 = 0x00000019;

// Descriptor table registers.
const WHV_X64_REGISTER_IDTR: u32 = 0x0000001A;
const WHV_X64_REGISTER_GDTR: u32 = 0x0000001B;

// Control registers.
const WHV_X64_REGISTER_CR0: u32 = 0x00000020;
const WHV_X64_REGISTER_CR2: u32 = 0x00000021;
const WHV_X64_REGISTER_CR3: u32 = 0x00000022;
const WHV_X64_REGISTER_CR4: u32 = 0x00000023;
const WHV_X64_REGISTER_CR8: u32 = 0x00000024;

// MSRs exposed as register names.
const WHV_X64_REGISTER_EFER: u32 = 0x00001001;
const WHV_X64_REGISTER_APIC_BASE: u32 = 0x00001002;
const WHV_X64_REGISTER_PAT: u32 = 0x00001003;
const WHV_X64_REGISTER_SYSENTER_CS: u32 = 0x00001004;
const WHV_X64_REGISTER_SYSENTER_EIP: u32 = 0x00001005;
const WHV_X64_REGISTER_SYSENTER_ESP: u32 = 0x00001006;
const WHV_X64_REGISTER_STAR: u32 = 0x00001007;
const WHV_X64_REGISTER_LSTAR: u32 = 0x00001008;
const WHV_X64_REGISTER_CSTAR: u32 = 0x00001009;
const WHV_X64_REGISTER_SFMASK: u32 = 0x0000100A;
const WHV_X64_REGISTER_KERNEL_GS_BASE: u32 = 0x0000100B;
const WHV_X64_REGISTER_TSC_AUX: u32 = 0x0000100C;

// Interrupt state registers.
const WHV_X64_REGISTER_PENDING_INTERRUPTION: u32 = 0x00000080;
const WHV_X64_REGISTER_INTERRUPT_STATE: u32 = 0x00000081;
const WHV_X64_REGISTER_DELIVERABILITY_NOTIFICATIONS: u32 = 0x00000084;

// WHP exit context size — 256 bytes covers the union.
const EXIT_CONTEXT_SIZE: u32 = 256;

// Memory access type indicators from WHP exit context.
const WHV_MEMORY_ACCESS_WRITE: u8 = 1;

#[link(name = "WinHvPlatform")]
extern "system" {
    fn WHvCreatePartition(partition: *mut WHV_PARTITION_HANDLE) -> HRESULT;
    fn WHvSetupPartition(partition: WHV_PARTITION_HANDLE) -> HRESULT;
    fn WHvDeletePartition(partition: WHV_PARTITION_HANDLE);
    fn WHvSetPartitionProperty(
        partition: WHV_PARTITION_HANDLE,
        code: u32,
        buf: *const u8,
        size: u32,
    ) -> HRESULT;
    fn WHvCreateVirtualProcessor(
        partition: WHV_PARTITION_HANDLE,
        index: u32,
        flags: u32,
    ) -> HRESULT;
    fn WHvDeleteVirtualProcessor(partition: WHV_PARTITION_HANDLE, index: u32) -> HRESULT;
    fn WHvRunVirtualProcessor(
        partition: WHV_PARTITION_HANDLE,
        index: u32,
        buf: *mut u8,
        buf_size: u32,
    ) -> HRESULT;
    fn WHvGetVirtualProcessorRegisters(
        partition: WHV_PARTITION_HANDLE,
        index: u32,
        names: *const u32,
        count: u32,
        values: *mut WHV_REGISTER_VALUE,
    ) -> HRESULT;
    fn WHvSetVirtualProcessorRegisters(
        partition: WHV_PARTITION_HANDLE,
        index: u32,
        names: *const u32,
        count: u32,
        values: *const WHV_REGISTER_VALUE,
    ) -> HRESULT;
    fn WHvMapGpaRange(
        partition: WHV_PARTITION_HANDLE,
        process: usize,
        source: *const u8,
        guest_addr: u64,
        size: u64,
        flags: u32,
    ) -> HRESULT;
    fn WHvUnmapGpaRange(
        partition: WHV_PARTITION_HANDLE,
        guest_addr: u64,
        size: u64,
    ) -> HRESULT;
    fn WHvCancelRunVirtualProcessor(
        partition: WHV_PARTITION_HANDLE,
        index: u32,
        flags: u32,
    ) -> HRESULT;
}

// ════════════════════════════════════════════════════════════════════════
// Exit context structures (parsed from the raw buffer)
// ════════════════════════════════════════════════════════════════════════

/// Parse a little-endian u32 from a byte slice at the given offset.
fn read_u32(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// Parse a little-endian u16 from a byte slice at the given offset.
fn read_u16(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

/// Parse a little-endian u64 from a byte slice at the given offset.
fn read_u64(buf: &[u8], off: usize) -> u64 {
    u64::from_le_bytes([
        buf[off],
        buf[off + 1],
        buf[off + 2],
        buf[off + 3],
        buf[off + 4],
        buf[off + 5],
        buf[off + 6],
        buf[off + 7],
    ])
}

// ════════════════════════════════════════════════════════════════════════
// WhpBackend
// ════════════════════════════════════════════════════════════════════════

/// Windows Hypervisor Platform backend.
pub struct WhpBackend {
    partition: WHV_PARTITION_HANDLE,
    stop_requested: bool,
    /// Pending IO in-data to write back into RAX before the next run.
    pending_io_data: Option<(u32, u8)>,
    /// Pending MMIO read-data to write back before the next run.
    pending_mmio_data: Option<(u64, u8)>,
    /// Tracks mapped regions by slot: (guest_phys_addr, size).
    mapped_regions: BTreeMap<u32, (u64, u64)>,
}

// SAFETY: The WHP partition handle is not thread-safe in itself, but our
// API ensures only one thread at a time calls into the backend (the VMM
// run-loop is single-threaded per vCPU). The Send bound is required by
// the trait.
unsafe impl Send for WhpBackend {}

impl WhpBackend {
    /// Create a new, uninitialised WHP backend. Call `create_vm` next.
    pub fn new() -> Result<Self, HvError> {
        Ok(WhpBackend {
            partition: core::ptr::null_mut(),
            stop_requested: false,
            pending_io_data: None,
            pending_mmio_data: None,
            mapped_regions: BTreeMap::new(),
        })
    }

    /// Helper: convert HRESULT to our error type.
    fn check(hr: HRESULT) -> Result<(), HvError> {
        if hr == S_OK {
            Ok(())
        } else {
            Err(HvError::SystemError(hr))
        }
    }

    /// Get a single 64-bit register.
    fn get_reg64(&self, vcpu_id: u32, name: u32) -> Result<u64, HvError> {
        let mut val = WHV_REGISTER_VALUE::default();
        let hr = unsafe {
            WHvGetVirtualProcessorRegisters(
                self.partition,
                vcpu_id,
                &name as *const u32,
                1,
                &mut val as *mut WHV_REGISTER_VALUE,
            )
        };
        Self::check(hr)?;
        Ok(unsafe { val.reg64 })
    }

    /// Set a single 64-bit register.
    fn set_reg64(&mut self, vcpu_id: u32, name: u32, value: u64) -> Result<(), HvError> {
        let val = WHV_REGISTER_VALUE { reg64: value };
        let hr = unsafe {
            WHvSetVirtualProcessorRegisters(
                self.partition,
                vcpu_id,
                &name as *const u32,
                1,
                &val as *const WHV_REGISTER_VALUE,
            )
        };
        Self::check(hr)
    }

    /// Read a segment register from WHP into our SegmentState.
    fn get_segment(&self, vcpu_id: u32, name: u32) -> Result<SegmentState, HvError> {
        let mut val = WHV_REGISTER_VALUE::default();
        let hr = unsafe {
            WHvGetVirtualProcessorRegisters(
                self.partition,
                vcpu_id,
                &name as *const u32,
                1,
                &mut val as *mut WHV_REGISTER_VALUE,
            )
        };
        Self::check(hr)?;
        let seg = unsafe { val.segment };
        Ok(SegmentState {
            selector: seg.selector,
            base: seg.base,
            limit: seg.limit,
            // WHP stores attributes in a 16-bit compact form; expand to our
            // 32-bit format by zero-extending.
            access_rights: seg.attributes as u32,
        })
    }

    /// Write a segment register from our SegmentState into WHP.
    fn set_segment(
        &mut self,
        vcpu_id: u32,
        name: u32,
        seg: &SegmentState,
    ) -> Result<(), HvError> {
        let val = WHV_REGISTER_VALUE {
            segment: WhvSegmentRegister {
                base: seg.base,
                limit: seg.limit,
                selector: seg.selector,
                attributes: seg.access_rights as u16,
            },
        };
        let hr = unsafe {
            WHvSetVirtualProcessorRegisters(
                self.partition,
                vcpu_id,
                &name as *const u32,
                1,
                &val as *const WHV_REGISTER_VALUE,
            )
        };
        Self::check(hr)
    }

    /// Read a descriptor table register (GDTR/IDTR).
    fn get_dtable(&self, vcpu_id: u32, name: u32) -> Result<DtableState, HvError> {
        let mut val = WHV_REGISTER_VALUE::default();
        let hr = unsafe {
            WHvGetVirtualProcessorRegisters(
                self.partition,
                vcpu_id,
                &name as *const u32,
                1,
                &mut val as *mut WHV_REGISTER_VALUE,
            )
        };
        Self::check(hr)?;
        let tbl = unsafe { val.table };
        Ok(DtableState {
            base: tbl.base,
            limit: tbl.limit,
        })
    }

    /// Write a descriptor table register (GDTR/IDTR).
    fn set_dtable(
        &mut self,
        vcpu_id: u32,
        name: u32,
        dt: &DtableState,
    ) -> Result<(), HvError> {
        let val = WHV_REGISTER_VALUE {
            table: WhvTableRegister {
                _pad: [0; 3],
                limit: dt.limit,
                base: dt.base,
            },
        };
        let hr = unsafe {
            WHvSetVirtualProcessorRegisters(
                self.partition,
                vcpu_id,
                &name as *const u32,
                1,
                &val as *const WHV_REGISTER_VALUE,
            )
        };
        Self::check(hr)
    }

    /// Flush any pending IO/MMIO response data into vCPU registers
    /// before the next WHvRunVirtualProcessor call.
    fn flush_pending_data(&mut self, vcpu_id: u32) -> Result<(), HvError> {
        if let Some((data, size)) = self.pending_io_data.take() {
            // IO in data goes into RAX (masked to the appropriate size).
            let mask = match size {
                1 => 0xFF,
                2 => 0xFFFF,
                _ => 0xFFFF_FFFF,
            };
            let mut rax = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RAX)?;
            rax = (rax & !mask) | (data as u64 & mask);
            self.set_reg64(vcpu_id, WHV_X64_REGISTER_RAX, rax)?;
        }
        if let Some((data, size)) = self.pending_mmio_data.take() {
            // MMIO read data also goes into RAX.
            let mask: u64 = match size {
                1 => 0xFF,
                2 => 0xFFFF,
                4 => 0xFFFF_FFFF,
                _ => 0xFFFF_FFFF_FFFF_FFFF,
            };
            let mut rax = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RAX)?;
            rax = (rax & !mask) | (data & mask);
            self.set_reg64(vcpu_id, WHV_X64_REGISTER_RAX, rax)?;
        }
        Ok(())
    }
}

impl HypervisorBackend for WhpBackend {
    fn create_vm(&mut self) -> Result<(), HvError> {
        let hr = unsafe { WHvCreatePartition(&mut self.partition as *mut WHV_PARTITION_HANDLE) };
        Self::check(hr)?;

        // Set processor count to 1 (we add more vCPUs via create_vcpu).
        let count: u32 = 1;
        let hr = unsafe {
            WHvSetPartitionProperty(
                self.partition,
                WHV_PARTITION_PROPERTY_PROCESSOR_COUNT,
                &count as *const u32 as *const u8,
                core::mem::size_of::<u32>() as u32,
            )
        };
        Self::check(hr)?;

        // Enable extended exits for CPUID and MSR intercepts.
        let exits: u64 = WHV_EXTENDED_VM_EXIT_CPUID | WHV_EXTENDED_VM_EXIT_MSR;
        let hr = unsafe {
            WHvSetPartitionProperty(
                self.partition,
                WHV_PARTITION_PROPERTY_EXTENDED_VM_EXITS,
                &exits as *const u64 as *const u8,
                core::mem::size_of::<u64>() as u32,
            )
        };
        Self::check(hr)?;

        // Finalise the partition setup.
        let hr = unsafe { WHvSetupPartition(self.partition) };
        Self::check(hr)?;

        Ok(())
    }

    fn create_vcpu(&mut self, vcpu_id: u32) -> Result<(), HvError> {
        let hr = unsafe { WHvCreateVirtualProcessor(self.partition, vcpu_id, 0) };
        Self::check(hr)
    }

    fn map_memory(&mut self, region: &MemoryRegion) -> Result<(), HvError> {
        let mut flags = WHV_MAP_GPA_RANGE_FLAG_READ | WHV_MAP_GPA_RANGE_FLAG_EXECUTE;
        if !region.readonly {
            flags |= WHV_MAP_GPA_RANGE_FLAG_WRITE;
        }

        // GetCurrentProcess() returns -1 as a pseudo-handle.
        let current_process: usize = usize::MAX; // -1 as usize

        let hr = unsafe {
            WHvMapGpaRange(
                self.partition,
                current_process,
                region.userspace_addr as *const u8,
                region.guest_phys_addr,
                region.memory_size,
                flags,
            )
        };
        Self::check(hr)?;

        self.mapped_regions
            .insert(region.slot, (region.guest_phys_addr, region.memory_size));
        Ok(())
    }

    fn unmap_memory(&mut self, slot: u32) -> Result<(), HvError> {
        let (guest_addr, size) = self
            .mapped_regions
            .remove(&slot)
            .ok_or(HvError::MemoryError)?;

        let hr = unsafe { WHvUnmapGpaRange(self.partition, guest_addr, size) };
        Self::check(hr)
    }

    fn get_regs(&self, vcpu_id: u32) -> Result<VcpuRegs, HvError> {
        let mut regs = VcpuRegs::default();

        // General-purpose registers.
        regs.rax = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RAX)?;
        regs.rcx = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RCX)?;
        regs.rdx = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RDX)?;
        regs.rbx = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RBX)?;
        regs.rsp = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RSP)?;
        regs.rbp = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RBP)?;
        regs.rsi = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RSI)?;
        regs.rdi = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RDI)?;
        regs.r8 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_R8)?;
        regs.r9 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_R9)?;
        regs.r10 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_R10)?;
        regs.r11 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_R11)?;
        regs.r12 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_R12)?;
        regs.r13 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_R13)?;
        regs.r14 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_R14)?;
        regs.r15 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_R15)?;

        // Instruction pointer and flags.
        regs.rip = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RIP)?;
        regs.rflags = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RFLAGS)?;

        // Segment registers.
        regs.cs = self.get_segment(vcpu_id, WHV_X64_REGISTER_CS)?;
        regs.ds = self.get_segment(vcpu_id, WHV_X64_REGISTER_DS)?;
        regs.es = self.get_segment(vcpu_id, WHV_X64_REGISTER_ES)?;
        regs.fs = self.get_segment(vcpu_id, WHV_X64_REGISTER_FS)?;
        regs.gs = self.get_segment(vcpu_id, WHV_X64_REGISTER_GS)?;
        regs.ss = self.get_segment(vcpu_id, WHV_X64_REGISTER_SS)?;
        regs.tr = self.get_segment(vcpu_id, WHV_X64_REGISTER_TR)?;
        regs.ldtr = self.get_segment(vcpu_id, WHV_X64_REGISTER_LDTR)?;

        // Descriptor table registers.
        regs.gdtr = self.get_dtable(vcpu_id, WHV_X64_REGISTER_GDTR)?;
        regs.idtr = self.get_dtable(vcpu_id, WHV_X64_REGISTER_IDTR)?;

        // Control registers.
        regs.cr0 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_CR0)?;
        regs.cr2 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_CR2)?;
        regs.cr3 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_CR3)?;
        regs.cr4 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_CR4)?;
        regs.cr8 = self.get_reg64(vcpu_id, WHV_X64_REGISTER_CR8)?;

        // MSRs.
        regs.efer = self.get_reg64(vcpu_id, WHV_X64_REGISTER_EFER)?;
        regs.apic_base = self.get_reg64(vcpu_id, WHV_X64_REGISTER_APIC_BASE)?;
        regs.pat = self.get_reg64(vcpu_id, WHV_X64_REGISTER_PAT)?;
        regs.sysenter_cs = self.get_reg64(vcpu_id, WHV_X64_REGISTER_SYSENTER_CS)?;
        regs.sysenter_esp = self.get_reg64(vcpu_id, WHV_X64_REGISTER_SYSENTER_ESP)?;
        regs.sysenter_eip = self.get_reg64(vcpu_id, WHV_X64_REGISTER_SYSENTER_EIP)?;
        regs.star = self.get_reg64(vcpu_id, WHV_X64_REGISTER_STAR)?;
        regs.lstar = self.get_reg64(vcpu_id, WHV_X64_REGISTER_LSTAR)?;
        regs.cstar = self.get_reg64(vcpu_id, WHV_X64_REGISTER_CSTAR)?;
        regs.sfmask = self.get_reg64(vcpu_id, WHV_X64_REGISTER_SFMASK)?;
        regs.kernel_gs_base = self.get_reg64(vcpu_id, WHV_X64_REGISTER_KERNEL_GS_BASE)?;
        regs.tsc_aux = self.get_reg64(vcpu_id, WHV_X64_REGISTER_TSC_AUX)?;

        Ok(regs)
    }

    fn set_regs(&mut self, vcpu_id: u32, regs: &VcpuRegs) -> Result<(), HvError> {
        // General-purpose registers.
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RAX, regs.rax)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RCX, regs.rcx)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RDX, regs.rdx)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RBX, regs.rbx)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RSP, regs.rsp)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RBP, regs.rbp)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RSI, regs.rsi)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RDI, regs.rdi)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_R8, regs.r8)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_R9, regs.r9)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_R10, regs.r10)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_R11, regs.r11)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_R12, regs.r12)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_R13, regs.r13)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_R14, regs.r14)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_R15, regs.r15)?;

        // Instruction pointer and flags.
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RIP, regs.rip)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RFLAGS, regs.rflags)?;

        // Segment registers.
        self.set_segment(vcpu_id, WHV_X64_REGISTER_CS, &regs.cs)?;
        self.set_segment(vcpu_id, WHV_X64_REGISTER_DS, &regs.ds)?;
        self.set_segment(vcpu_id, WHV_X64_REGISTER_ES, &regs.es)?;
        self.set_segment(vcpu_id, WHV_X64_REGISTER_FS, &regs.fs)?;
        self.set_segment(vcpu_id, WHV_X64_REGISTER_GS, &regs.gs)?;
        self.set_segment(vcpu_id, WHV_X64_REGISTER_SS, &regs.ss)?;
        self.set_segment(vcpu_id, WHV_X64_REGISTER_TR, &regs.tr)?;
        self.set_segment(vcpu_id, WHV_X64_REGISTER_LDTR, &regs.ldtr)?;

        // Descriptor table registers.
        self.set_dtable(vcpu_id, WHV_X64_REGISTER_GDTR, &regs.gdtr)?;
        self.set_dtable(vcpu_id, WHV_X64_REGISTER_IDTR, &regs.idtr)?;

        // Control registers.
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_CR0, regs.cr0)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_CR2, regs.cr2)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_CR3, regs.cr3)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_CR4, regs.cr4)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_CR8, regs.cr8)?;

        // MSRs.
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_EFER, regs.efer)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_APIC_BASE, regs.apic_base)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_PAT, regs.pat)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_SYSENTER_CS, regs.sysenter_cs)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_SYSENTER_ESP, regs.sysenter_esp)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_SYSENTER_EIP, regs.sysenter_eip)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_STAR, regs.star)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_LSTAR, regs.lstar)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_CSTAR, regs.cstar)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_SFMASK, regs.sfmask)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_KERNEL_GS_BASE, regs.kernel_gs_base)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_TSC_AUX, regs.tsc_aux)?;

        Ok(())
    }

    fn run(&mut self, vcpu_id: u32) -> Result<VmExit, HvError> {
        // Check for pending stop before entering the guest.
        if self.stop_requested {
            self.stop_requested = false;
            return Ok(VmExit::StopRequested);
        }

        // Flush any pending IO/MMIO response data.
        self.flush_pending_data(vcpu_id)?;

        let mut exit_buf = vec![0u8; EXIT_CONTEXT_SIZE as usize];
        let hr = unsafe {
            WHvRunVirtualProcessor(
                self.partition,
                vcpu_id,
                exit_buf.as_mut_ptr(),
                EXIT_CONTEXT_SIZE,
            )
        };
        Self::check(hr)?;

        // Parse the exit context. The first 4 bytes are the exit reason.
        let exit_reason = read_u32(&exit_buf, 0);

        match exit_reason {
            WHV_EXIT_REASON_IO_PORT_ACCESS => {
                // Byte 32: access info (direction, size).
                // Bytes 36-37: port number.
                // Bytes 40-43: data (for OUT).
                let access_info = exit_buf[32];
                let is_in = (access_info & 1) != 0;
                let size = ((access_info >> 1) & 0x7) as u8;
                let port = read_u16(&exit_buf, 36);
                let rip_delta = exit_buf[44];

                if is_in {
                    Ok(VmExit::IoIn { port, size })
                } else {
                    let data = read_u32(&exit_buf, 40);
                    // Advance RIP past the IO instruction.
                    if rip_delta > 0 {
                        let rip = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RIP)?;
                        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RIP, rip + rip_delta as u64)?;
                    }
                    Ok(VmExit::IoOut { port, size, data })
                }
            }

            WHV_EXIT_REASON_MEMORY_ACCESS => {
                // Byte 32: access info (read/write, instruction length, etc.).
                // Bytes 40-47: guest physical address.
                // Bytes 48-55: guest virtual address.
                let access_info = exit_buf[32];
                let is_write = (access_info & WHV_MEMORY_ACCESS_WRITE) != 0;
                let instr_len = exit_buf[33];
                let size = exit_buf[34];
                let gpa = read_u64(&exit_buf, 40);

                if is_write {
                    let data = read_u64(&exit_buf, 56);
                    // Advance RIP past the faulting instruction.
                    if instr_len > 0 {
                        let rip = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RIP)?;
                        self.set_reg64(
                            vcpu_id,
                            WHV_X64_REGISTER_RIP,
                            rip + instr_len as u64,
                        )?;
                    }
                    Ok(VmExit::MmioWrite {
                        address: gpa,
                        size,
                        data,
                    })
                } else {
                    Ok(VmExit::MmioRead {
                        address: gpa,
                        size,
                    })
                }
            }

            WHV_EXIT_REASON_HALT => Ok(VmExit::Hlt),

            WHV_EXIT_REASON_CPUID => {
                let eax = read_u32(&exit_buf, 32);
                let ecx = read_u32(&exit_buf, 40);
                Ok(VmExit::Cpuid { eax, ecx })
            }

            WHV_EXIT_REASON_MSR_ACCESS => {
                let is_write = exit_buf[32] != 0;
                let msr_index = read_u32(&exit_buf, 36);
                if is_write {
                    let value = read_u64(&exit_buf, 40);
                    Ok(VmExit::MsrWrite {
                        index: msr_index,
                        value,
                    })
                } else {
                    Ok(VmExit::MsrRead { index: msr_index })
                }
            }

            WHV_EXIT_REASON_INTERRUPT_WINDOW => Ok(VmExit::InterruptWindow),

            WHV_EXIT_REASON_CANCELED => {
                self.stop_requested = false;
                Ok(VmExit::StopRequested)
            }

            WHV_EXIT_REASON_NONE => Ok(VmExit::Shutdown),

            other => Ok(VmExit::Unknown(other)),
        }
    }

    fn request_interrupt_window(&mut self, vcpu_id: u32) -> Result<(), HvError> {
        // Set the deliverability-notifications register to request an
        // interrupt-window exit.
        let val = WHV_REGISTER_VALUE { reg64: 1 };
        let hr = unsafe {
            WHvSetVirtualProcessorRegisters(
                self.partition,
                vcpu_id,
                &WHV_X64_REGISTER_DELIVERABILITY_NOTIFICATIONS as *const u32,
                1,
                &val as *const WHV_REGISTER_VALUE,
            )
        };
        Self::check(hr)
    }

    fn inject_interrupt(&mut self, vcpu_id: u32, vector: u8) -> Result<(), HvError> {
        // Encode an external interrupt into the pending-interruption register.
        // Bits 7:0 = vector, bits 10:8 = type (0 = external), bit 31 = valid.
        let value: u64 = (vector as u64) | (1u64 << 31);
        let val = WHV_REGISTER_VALUE { reg64: value };
        let hr = unsafe {
            WHvSetVirtualProcessorRegisters(
                self.partition,
                vcpu_id,
                &WHV_X64_REGISTER_PENDING_INTERRUPTION as *const u32,
                1,
                &val as *const WHV_REGISTER_VALUE,
            )
        };
        Self::check(hr)
    }

    fn interrupts_enabled(&self, vcpu_id: u32) -> Result<bool, HvError> {
        // Check RFLAGS.IF (bit 9) and interruptibility state.
        let rflags = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RFLAGS)?;
        let if_set = (rflags & (1 << 9)) != 0;

        let int_state = self.get_reg64(vcpu_id, WHV_X64_REGISTER_INTERRUPT_STATE)?;
        let not_blocked = (int_state & 0x3) == 0; // STI/MOV-SS blocking

        Ok(if_set && not_blocked)
    }

    fn set_io_in_data(&mut self, _vcpu_id: u32, data: u32, size: u8) -> Result<(), HvError> {
        self.pending_io_data = Some((data, size));
        Ok(())
    }

    fn set_mmio_read_data(&mut self, _vcpu_id: u32, data: u64, size: u8) -> Result<(), HvError> {
        self.pending_mmio_data = Some((data, size));
        Ok(())
    }

    fn set_cpuid_response(
        &mut self,
        vcpu_id: u32,
        eax: u32,
        ebx: u32,
        ecx: u32,
        edx: u32,
    ) -> Result<(), HvError> {
        // Write CPUID results into the four output registers.
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RAX, eax as u64)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RBX, ebx as u64)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RCX, ecx as u64)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RDX, edx as u64)?;
        Ok(())
    }

    fn set_msr_read_data(&mut self, vcpu_id: u32, value: u64) -> Result<(), HvError> {
        // MSR read result: low 32 bits in EAX, high 32 bits in EDX.
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RAX, value & 0xFFFF_FFFF)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RDX, value >> 32)?;
        Ok(())
    }

    fn advance_rip(&mut self, vcpu_id: u32, len: u8) -> Result<(), HvError> {
        let rip = self.get_reg64(vcpu_id, WHV_X64_REGISTER_RIP)?;
        self.set_reg64(vcpu_id, WHV_X64_REGISTER_RIP, rip + len as u64)
    }

    fn request_stop(&mut self, vcpu_id: u32) -> Result<(), HvError> {
        self.stop_requested = true;
        // Cancel any in-progress WHvRunVirtualProcessor call.
        let hr = unsafe { WHvCancelRunVirtualProcessor(self.partition, vcpu_id, 0) };
        Self::check(hr)
    }

    fn destroy(&mut self) {
        if !self.partition.is_null() {
            unsafe {
                WHvDeletePartition(self.partition);
            }
            self.partition = core::ptr::null_mut();
        }
    }
}

impl Drop for WhpBackend {
    fn drop(&mut self) {
        self.destroy();
    }
}
