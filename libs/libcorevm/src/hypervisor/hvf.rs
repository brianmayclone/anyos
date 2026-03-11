//! macOS Hypervisor.framework (HVF) backend.
//!
//! Uses the Hypervisor.framework C API via FFI to provide hardware
//! virtualization on macOS with Intel VT-x. This backend delegates to
//! macOS's built-in hypervisor, which manages VMX on our behalf.

use super::vmx;
use super::{
    DtableState, HvError, HypervisorBackend, MemoryRegion, SegmentState, VcpuRegs, VmExit,
};

// ════════════════════════════════════════════════════════════════════════
// HVF FFI declarations
// ════════════════════════════════════════════════════════════════════════

const HV_SUCCESS: i32 = 0;

// HV memory flags.
const HV_MEMORY_READ: u64 = 1 << 0;
const HV_MEMORY_WRITE: u64 = 1 << 1;
const HV_MEMORY_EXEC: u64 = 1 << 2;

// hv_x86_reg_t — general-purpose and instruction-pointer register IDs.
const HV_X86_RIP: u32 = 0;
const HV_X86_RFLAGS: u32 = 1;
const HV_X86_RAX: u32 = 2;
const HV_X86_RCX: u32 = 3;
const HV_X86_RDX: u32 = 4;
const HV_X86_RBX: u32 = 5;
const HV_X86_RSP: u32 = 6;
const HV_X86_RBP: u32 = 7;
const HV_X86_RSI: u32 = 8;
const HV_X86_RDI: u32 = 9;
const HV_X86_R8: u32 = 10;
const HV_X86_R9: u32 = 11;
const HV_X86_R10: u32 = 12;
const HV_X86_R11: u32 = 13;
const HV_X86_R12: u32 = 14;
const HV_X86_R13: u32 = 15;
const HV_X86_R14: u32 = 16;
const HV_X86_R15: u32 = 17;
const HV_X86_CR0: u32 = 18;
const HV_X86_CR2: u32 = 19;
const HV_X86_CR3: u32 = 20;
const HV_X86_CR4: u32 = 21;
const HV_X86_CR8: u32 = 22;

#[link(name = "Hypervisor", kind = "framework")]
extern "C" {
    fn hv_vm_create(flags: u64) -> i32;
    fn hv_vm_destroy() -> i32;
    fn hv_vcpu_create(vcpu: *mut u32, flags: u64) -> i32;
    fn hv_vcpu_destroy(vcpu: u32) -> i32;
    fn hv_vcpu_run(vcpu: u32) -> i32;
    fn hv_vcpu_run_until(vcpu: u32, deadline: u64) -> i32;
    fn hv_vcpu_interrupt(vcpus: *const u32, count: u32) -> i32;
    fn hv_vcpu_read_register(vcpu: u32, reg: u32, value: *mut u64) -> i32;
    fn hv_vcpu_write_register(vcpu: u32, reg: u32, value: u64) -> i32;
    fn hv_vmx_vcpu_read_vmcs(vcpu: u32, field: u32, value: *mut u64) -> i32;
    fn hv_vmx_vcpu_write_vmcs(vcpu: u32, field: u32, value: u64) -> i32;
    fn hv_vm_map(uva: *const u8, gpa: u64, size: usize, flags: u64) -> i32;
    fn hv_vm_unmap(gpa: u64, size: usize) -> i32;
}

// ════════════════════════════════════════════════════════════════════════
// Helper functions
// ════════════════════════════════════════════════════════════════════════

/// Convert an HVF return code to our error type.
fn check(ret: i32) -> Result<(), HvError> {
    if ret == HV_SUCCESS {
        Ok(())
    } else {
        Err(HvError::SystemError(ret))
    }
}

/// Read an x86 register via hv_vcpu_read_register.
fn read_reg(vcpu: u32, reg: u32) -> Result<u64, HvError> {
    let mut value: u64 = 0;
    check(unsafe { hv_vcpu_read_register(vcpu, reg, &mut value) })?;
    Ok(value)
}

/// Write an x86 register via hv_vcpu_write_register.
fn write_reg(vcpu: u32, reg: u32, value: u64) -> Result<(), HvError> {
    check(unsafe { hv_vcpu_write_register(vcpu, reg, value) })
}

/// Read a VMCS field.
fn read_vmcs(vcpu: u32, field: u32) -> Result<u64, HvError> {
    let mut value: u64 = 0;
    check(unsafe { hv_vmx_vcpu_read_vmcs(vcpu, field, &mut value) })?;
    Ok(value)
}

/// Write a VMCS field.
fn write_vmcs(vcpu: u32, field: u32, value: u64) -> Result<(), HvError> {
    check(unsafe { hv_vmx_vcpu_write_vmcs(vcpu, field, value) })
}

/// Read a segment register state from the VMCS.
fn read_segment(
    vcpu: u32,
    selector_field: u32,
    base_field: u32,
    limit_field: u32,
    ar_field: u32,
) -> Result<SegmentState, HvError> {
    Ok(SegmentState {
        selector: read_vmcs(vcpu, selector_field)? as u16,
        base: read_vmcs(vcpu, base_field)?,
        limit: read_vmcs(vcpu, limit_field)? as u32,
        access_rights: read_vmcs(vcpu, ar_field)? as u32,
    })
}

/// Write a segment register state to the VMCS.
fn write_segment(
    vcpu: u32,
    seg: &SegmentState,
    selector_field: u32,
    base_field: u32,
    limit_field: u32,
    ar_field: u32,
) -> Result<(), HvError> {
    write_vmcs(vcpu, selector_field, seg.selector as u64)?;
    write_vmcs(vcpu, base_field, seg.base)?;
    write_vmcs(vcpu, limit_field, seg.limit as u64)?;
    write_vmcs(vcpu, ar_field, seg.access_rights as u64)?;
    Ok(())
}

/// Read a descriptor-table register (GDTR/IDTR) from the VMCS.
fn read_dtable(vcpu: u32, base_field: u32, limit_field: u32) -> Result<DtableState, HvError> {
    Ok(DtableState {
        base: read_vmcs(vcpu, base_field)?,
        limit: read_vmcs(vcpu, limit_field)? as u16,
    })
}

/// Write a descriptor-table register (GDTR/IDTR) to the VMCS.
fn write_dtable(
    vcpu: u32,
    dt: &DtableState,
    base_field: u32,
    limit_field: u32,
) -> Result<(), HvError> {
    write_vmcs(vcpu, base_field, dt.base)?;
    write_vmcs(vcpu, limit_field, dt.limit as u64)?;
    Ok(())
}

// ════════════════════════════════════════════════════════════════════════
// HvfBackend
// ════════════════════════════════════════════════════════════════════════

/// macOS Hypervisor.framework backend.
pub struct HvfBackend {
    vcpu_id: u32,
    vm_created: bool,
    vcpu_created: bool,
    stop_requested: bool,
    launched: bool,
    /// Pending IO in-data to write back into RAX before the next run.
    pending_io_data: Option<(u32, u8)>,
    /// Pending MMIO read-data to write back before the next run.
    pending_mmio_data: Option<(u64, u8)>,
}

unsafe impl Send for HvfBackend {}

impl HvfBackend {
    /// Create a new, uninitialised HVF backend. Call `create_vm` next.
    pub fn new() -> Result<Self, HvError> {
        Ok(HvfBackend {
            vcpu_id: 0,
            vm_created: false,
            vcpu_created: false,
            stop_requested: false,
            launched: false,
            pending_io_data: None,
            pending_mmio_data: None,
        })
    }

    /// Flush any pending IO/MMIO response data into vCPU registers.
    fn flush_pending_data(&mut self) -> Result<(), HvError> {
        if let Some((data, size)) = self.pending_io_data.take() {
            let mask: u64 = match size {
                1 => 0xFF,
                2 => 0xFFFF,
                _ => 0xFFFF_FFFF,
            };
            let mut rax = read_reg(self.vcpu_id, HV_X86_RAX)?;
            rax = (rax & !mask) | (data as u64 & mask);
            write_reg(self.vcpu_id, HV_X86_RAX, rax)?;
        }
        if let Some((data, size)) = self.pending_mmio_data.take() {
            let mask: u64 = match size {
                1 => 0xFF,
                2 => 0xFFFF,
                4 => 0xFFFF_FFFF,
                _ => 0xFFFF_FFFF_FFFF_FFFF,
            };
            let mut rax = read_reg(self.vcpu_id, HV_X86_RAX)?;
            rax = (rax & !mask) | (data & mask);
            write_reg(self.vcpu_id, HV_X86_RAX, rax)?;
        }
        Ok(())
    }

    /// Configure initial VMCS controls for the vCPU.
    fn setup_vmcs_controls(&self) -> Result<(), HvError> {
        let vcpu = self.vcpu_id;

        // Pin-based controls: external-interrupt exiting.
        write_vmcs(
            vcpu,
            vmx::VMCS_PIN_BASED_CONTROLS,
            vmx::PIN_BASED_EXT_INTR_EXIT as u64,
        )?;

        // Primary proc-based controls: HLT exiting, unconditional I/O exiting,
        // MSR bitmap usage, activate secondary controls.
        let primary = vmx::PROC_BASED_HLT_EXIT
            | vmx::PROC_BASED_ACTIVATE_SECONDARY;
        write_vmcs(
            vcpu,
            vmx::VMCS_PRIMARY_PROC_BASED_CONTROLS,
            primary as u64,
        )?;

        // Secondary proc-based controls: EPT, unrestricted guest.
        let secondary = vmx::PROC2_BASED_ENABLE_EPT | vmx::PROC2_BASED_UNRESTRICTED_GUEST;
        write_vmcs(
            vcpu,
            vmx::VMCS_SECONDARY_PROC_BASED_CONTROLS,
            secondary as u64,
        )?;

        // VM-exit controls: 64-bit host, save/load IA32_EFER.
        let exit_ctls = vmx::EXIT_HOST_ADDR_SPACE_SIZE
            | vmx::EXIT_SAVE_IA32_EFER
            | vmx::EXIT_LOAD_IA32_EFER;
        write_vmcs(vcpu, vmx::VMCS_EXIT_CONTROLS, exit_ctls as u64)?;

        // VM-entry controls: load IA32_EFER, IA-32e mode guest.
        let entry_ctls = vmx::ENTRY_LOAD_IA32_EFER | vmx::ENTRY_IA32E_MODE_GUEST;
        write_vmcs(vcpu, vmx::VMCS_ENTRY_CONTROLS, entry_ctls as u64)?;

        // Link pointer must be all-ones for non-nested operation.
        write_vmcs(vcpu, vmx::VMCS_GUEST_VMCS_LINK_PTR, u64::MAX)?;

        Ok(())
    }
}

impl HypervisorBackend for HvfBackend {
    fn create_vm(&mut self) -> Result<(), HvError> {
        check(unsafe { hv_vm_create(0) })?;
        self.vm_created = true;
        Ok(())
    }

    fn create_vcpu(&mut self, _vcpu_id: u32) -> Result<(), HvError> {
        let mut vcpu: u32 = 0;
        check(unsafe { hv_vcpu_create(&mut vcpu, 0) })?;
        self.vcpu_id = vcpu;
        self.vcpu_created = true;
        self.launched = false;

        // Set up initial VMCS controls.
        self.setup_vmcs_controls()?;

        Ok(())
    }

    fn map_memory(&mut self, region: &MemoryRegion) -> Result<(), HvError> {
        let mut flags = HV_MEMORY_READ | HV_MEMORY_EXEC;
        if !region.readonly {
            flags |= HV_MEMORY_WRITE;
        }
        check(unsafe {
            hv_vm_map(
                region.userspace_addr as *const u8,
                region.guest_phys_addr,
                region.memory_size as usize,
                flags,
            )
        })
    }

    fn unmap_memory(&mut self, _slot: u32) -> Result<(), HvError> {
        // HVF does not use slot-based tracking; the caller must ensure
        // the GPA range is correct. For now, we return an error since
        // we would need the original GPA and size to unmap.
        // In a full implementation, we would maintain a slot-to-region map.
        Err(HvError::Other(
            "hvf: unmap_memory requires slot-to-region tracking",
        ))
    }

    fn get_regs(&self, _vcpu_id: u32) -> Result<VcpuRegs, HvError> {
        let vcpu = self.vcpu_id;
        let mut regs = VcpuRegs::default();

        // General-purpose registers via hv_vcpu_read_register.
        regs.rax = read_reg(vcpu, HV_X86_RAX)?;
        regs.rcx = read_reg(vcpu, HV_X86_RCX)?;
        regs.rdx = read_reg(vcpu, HV_X86_RDX)?;
        regs.rbx = read_reg(vcpu, HV_X86_RBX)?;
        regs.rsp = read_reg(vcpu, HV_X86_RSP)?;
        regs.rbp = read_reg(vcpu, HV_X86_RBP)?;
        regs.rsi = read_reg(vcpu, HV_X86_RSI)?;
        regs.rdi = read_reg(vcpu, HV_X86_RDI)?;
        regs.r8 = read_reg(vcpu, HV_X86_R8)?;
        regs.r9 = read_reg(vcpu, HV_X86_R9)?;
        regs.r10 = read_reg(vcpu, HV_X86_R10)?;
        regs.r11 = read_reg(vcpu, HV_X86_R11)?;
        regs.r12 = read_reg(vcpu, HV_X86_R12)?;
        regs.r13 = read_reg(vcpu, HV_X86_R13)?;
        regs.r14 = read_reg(vcpu, HV_X86_R14)?;
        regs.r15 = read_reg(vcpu, HV_X86_R15)?;

        // Instruction pointer and flags.
        regs.rip = read_reg(vcpu, HV_X86_RIP)?;
        regs.rflags = read_reg(vcpu, HV_X86_RFLAGS)?;

        // Segment registers from VMCS.
        regs.cs = read_segment(
            vcpu,
            vmx::VMCS_GUEST_CS_SELECTOR,
            vmx::VMCS_GUEST_CS_BASE,
            vmx::VMCS_GUEST_CS_LIMIT,
            vmx::VMCS_GUEST_CS_ACCESS_RIGHTS,
        )?;
        regs.ds = read_segment(
            vcpu,
            vmx::VMCS_GUEST_DS_SELECTOR,
            vmx::VMCS_GUEST_DS_BASE,
            vmx::VMCS_GUEST_DS_LIMIT,
            vmx::VMCS_GUEST_DS_ACCESS_RIGHTS,
        )?;
        regs.es = read_segment(
            vcpu,
            vmx::VMCS_GUEST_ES_SELECTOR,
            vmx::VMCS_GUEST_ES_BASE,
            vmx::VMCS_GUEST_ES_LIMIT,
            vmx::VMCS_GUEST_ES_ACCESS_RIGHTS,
        )?;
        regs.fs = read_segment(
            vcpu,
            vmx::VMCS_GUEST_FS_SELECTOR,
            vmx::VMCS_GUEST_FS_BASE,
            vmx::VMCS_GUEST_FS_LIMIT,
            vmx::VMCS_GUEST_FS_ACCESS_RIGHTS,
        )?;
        regs.gs = read_segment(
            vcpu,
            vmx::VMCS_GUEST_GS_SELECTOR,
            vmx::VMCS_GUEST_GS_BASE,
            vmx::VMCS_GUEST_GS_LIMIT,
            vmx::VMCS_GUEST_GS_ACCESS_RIGHTS,
        )?;
        regs.ss = read_segment(
            vcpu,
            vmx::VMCS_GUEST_SS_SELECTOR,
            vmx::VMCS_GUEST_SS_BASE,
            vmx::VMCS_GUEST_SS_LIMIT,
            vmx::VMCS_GUEST_SS_ACCESS_RIGHTS,
        )?;
        regs.tr = read_segment(
            vcpu,
            vmx::VMCS_GUEST_TR_SELECTOR,
            vmx::VMCS_GUEST_TR_BASE,
            vmx::VMCS_GUEST_TR_LIMIT,
            vmx::VMCS_GUEST_TR_ACCESS_RIGHTS,
        )?;
        regs.ldtr = read_segment(
            vcpu,
            vmx::VMCS_GUEST_LDTR_SELECTOR,
            vmx::VMCS_GUEST_LDTR_BASE,
            vmx::VMCS_GUEST_LDTR_LIMIT,
            vmx::VMCS_GUEST_LDTR_ACCESS_RIGHTS,
        )?;

        // Descriptor table registers from VMCS.
        regs.gdtr = read_dtable(vcpu, vmx::VMCS_GUEST_GDTR_BASE, vmx::VMCS_GUEST_GDTR_LIMIT)?;
        regs.idtr = read_dtable(vcpu, vmx::VMCS_GUEST_IDTR_BASE, vmx::VMCS_GUEST_IDTR_LIMIT)?;

        // Control registers.
        regs.cr0 = read_reg(vcpu, HV_X86_CR0)?;
        regs.cr2 = read_reg(vcpu, HV_X86_CR2)?;
        regs.cr3 = read_reg(vcpu, HV_X86_CR3)?;
        regs.cr4 = read_reg(vcpu, HV_X86_CR4)?;
        regs.cr8 = read_reg(vcpu, HV_X86_CR8)?;

        // MSRs from VMCS fields.
        regs.efer = read_vmcs(vcpu, vmx::VMCS_GUEST_IA32_EFER)?;
        regs.pat = read_vmcs(vcpu, vmx::VMCS_GUEST_IA32_PAT)?;

        // Sysenter MSRs from VMCS.
        regs.sysenter_cs = read_vmcs(vcpu, vmx::VMCS_GUEST_SYSENTER_CS)?;
        regs.sysenter_esp = read_vmcs(vcpu, vmx::VMCS_GUEST_SYSENTER_ESP)?;
        regs.sysenter_eip = read_vmcs(vcpu, vmx::VMCS_GUEST_SYSENTER_EIP)?;

        // The remaining MSRs (STAR, LSTAR, CSTAR, SFMASK, KERNEL_GS_BASE,
        // TSC_AUX, APIC_BASE) are not directly accessible through VMCS fields.
        // They must be saved/restored via the MSR load/store mechanism.
        // For now, these fields remain at their default (zero) values and
        // would be populated from the MSR save area in a full implementation.

        Ok(regs)
    }

    fn set_regs(&mut self, _vcpu_id: u32, regs: &VcpuRegs) -> Result<(), HvError> {
        let vcpu = self.vcpu_id;

        // General-purpose registers.
        write_reg(vcpu, HV_X86_RAX, regs.rax)?;
        write_reg(vcpu, HV_X86_RCX, regs.rcx)?;
        write_reg(vcpu, HV_X86_RDX, regs.rdx)?;
        write_reg(vcpu, HV_X86_RBX, regs.rbx)?;
        write_reg(vcpu, HV_X86_RSP, regs.rsp)?;
        write_reg(vcpu, HV_X86_RBP, regs.rbp)?;
        write_reg(vcpu, HV_X86_RSI, regs.rsi)?;
        write_reg(vcpu, HV_X86_RDI, regs.rdi)?;
        write_reg(vcpu, HV_X86_R8, regs.r8)?;
        write_reg(vcpu, HV_X86_R9, regs.r9)?;
        write_reg(vcpu, HV_X86_R10, regs.r10)?;
        write_reg(vcpu, HV_X86_R11, regs.r11)?;
        write_reg(vcpu, HV_X86_R12, regs.r12)?;
        write_reg(vcpu, HV_X86_R13, regs.r13)?;
        write_reg(vcpu, HV_X86_R14, regs.r14)?;
        write_reg(vcpu, HV_X86_R15, regs.r15)?;

        // Instruction pointer and flags.
        write_reg(vcpu, HV_X86_RIP, regs.rip)?;
        write_reg(vcpu, HV_X86_RFLAGS, regs.rflags)?;

        // Segment registers via VMCS.
        write_segment(
            vcpu,
            &regs.cs,
            vmx::VMCS_GUEST_CS_SELECTOR,
            vmx::VMCS_GUEST_CS_BASE,
            vmx::VMCS_GUEST_CS_LIMIT,
            vmx::VMCS_GUEST_CS_ACCESS_RIGHTS,
        )?;
        write_segment(
            vcpu,
            &regs.ds,
            vmx::VMCS_GUEST_DS_SELECTOR,
            vmx::VMCS_GUEST_DS_BASE,
            vmx::VMCS_GUEST_DS_LIMIT,
            vmx::VMCS_GUEST_DS_ACCESS_RIGHTS,
        )?;
        write_segment(
            vcpu,
            &regs.es,
            vmx::VMCS_GUEST_ES_SELECTOR,
            vmx::VMCS_GUEST_ES_BASE,
            vmx::VMCS_GUEST_ES_LIMIT,
            vmx::VMCS_GUEST_ES_ACCESS_RIGHTS,
        )?;
        write_segment(
            vcpu,
            &regs.fs,
            vmx::VMCS_GUEST_FS_SELECTOR,
            vmx::VMCS_GUEST_FS_BASE,
            vmx::VMCS_GUEST_FS_LIMIT,
            vmx::VMCS_GUEST_FS_ACCESS_RIGHTS,
        )?;
        write_segment(
            vcpu,
            &regs.gs,
            vmx::VMCS_GUEST_GS_SELECTOR,
            vmx::VMCS_GUEST_GS_BASE,
            vmx::VMCS_GUEST_GS_LIMIT,
            vmx::VMCS_GUEST_GS_ACCESS_RIGHTS,
        )?;
        write_segment(
            vcpu,
            &regs.ss,
            vmx::VMCS_GUEST_SS_SELECTOR,
            vmx::VMCS_GUEST_SS_BASE,
            vmx::VMCS_GUEST_SS_LIMIT,
            vmx::VMCS_GUEST_SS_ACCESS_RIGHTS,
        )?;
        write_segment(
            vcpu,
            &regs.tr,
            vmx::VMCS_GUEST_TR_SELECTOR,
            vmx::VMCS_GUEST_TR_BASE,
            vmx::VMCS_GUEST_TR_LIMIT,
            vmx::VMCS_GUEST_TR_ACCESS_RIGHTS,
        )?;
        write_segment(
            vcpu,
            &regs.ldtr,
            vmx::VMCS_GUEST_LDTR_SELECTOR,
            vmx::VMCS_GUEST_LDTR_BASE,
            vmx::VMCS_GUEST_LDTR_LIMIT,
            vmx::VMCS_GUEST_LDTR_ACCESS_RIGHTS,
        )?;

        // Descriptor table registers.
        write_dtable(
            vcpu,
            &regs.gdtr,
            vmx::VMCS_GUEST_GDTR_BASE,
            vmx::VMCS_GUEST_GDTR_LIMIT,
        )?;
        write_dtable(
            vcpu,
            &regs.idtr,
            vmx::VMCS_GUEST_IDTR_BASE,
            vmx::VMCS_GUEST_IDTR_LIMIT,
        )?;

        // Control registers.
        write_reg(vcpu, HV_X86_CR0, regs.cr0)?;
        write_reg(vcpu, HV_X86_CR2, regs.cr2)?;
        write_reg(vcpu, HV_X86_CR3, regs.cr3)?;
        write_reg(vcpu, HV_X86_CR4, regs.cr4)?;
        write_reg(vcpu, HV_X86_CR8, regs.cr8)?;

        // MSRs via VMCS.
        write_vmcs(vcpu, vmx::VMCS_GUEST_IA32_EFER, regs.efer)?;
        write_vmcs(vcpu, vmx::VMCS_GUEST_IA32_PAT, regs.pat)?;
        write_vmcs(vcpu, vmx::VMCS_GUEST_SYSENTER_CS, regs.sysenter_cs)?;
        write_vmcs(vcpu, vmx::VMCS_GUEST_SYSENTER_ESP, regs.sysenter_esp)?;
        write_vmcs(vcpu, vmx::VMCS_GUEST_SYSENTER_EIP, regs.sysenter_eip)?;

        Ok(())
    }

    fn run(&mut self, _vcpu_id: u32) -> Result<VmExit, HvError> {
        if self.stop_requested {
            self.stop_requested = false;
            return Ok(VmExit::StopRequested);
        }

        // Flush any pending IO/MMIO response data.
        self.flush_pending_data()?;

        let vcpu = self.vcpu_id;

        // Run the vCPU until it exits.
        check(unsafe { hv_vcpu_run(vcpu) })?;
        self.launched = true;

        // Read the exit reason from the VMCS.
        let exit_reason_raw = read_vmcs(vcpu, vmx::VMCS_EXIT_REASON)? as u32;
        let exit_reason = exit_reason_raw & 0xFFFF; // Basic exit reason is bits 15:0.

        match exit_reason {
            vmx::EXIT_REASON_IO_INSTRUCTION => {
                let qualification = read_vmcs(vcpu, vmx::VMCS_EXIT_QUALIFICATION)?;
                let instr_len = read_vmcs(vcpu, vmx::VMCS_EXIT_INSTRUCTION_LENGTH)? as u8;

                let size = ((qualification & 0x7) + 1) as u8;
                let is_in = (qualification & (1 << 3)) != 0;
                let port = ((qualification >> 16) & 0xFFFF) as u16;

                if is_in {
                    Ok(VmExit::IoIn { port, size })
                } else {
                    let rax = read_reg(vcpu, HV_X86_RAX)?;
                    let data = rax as u32;
                    // Advance RIP past the IO instruction.
                    let rip = read_reg(vcpu, HV_X86_RIP)?;
                    write_reg(vcpu, HV_X86_RIP, rip + instr_len as u64)?;
                    Ok(VmExit::IoOut { port, size, data })
                }
            }

            vmx::EXIT_REASON_EPT_VIOLATION => {
                let qualification = read_vmcs(vcpu, vmx::VMCS_EXIT_QUALIFICATION)?;
                let guest_phys = read_vmcs(vcpu, 0x2400)?; // VMCS_GUEST_PHYSICAL_ADDRESS
                let instr_len = read_vmcs(vcpu, vmx::VMCS_EXIT_INSTRUCTION_LENGTH)? as u8;

                let is_read = (qualification & 1) != 0;
                let is_write = (qualification & 2) != 0;
                // Bit 7: if set, the access was a guest-linear-address translation.
                let is_mmio = (qualification & (1 << 7)) == 0;

                if is_mmio {
                    let size = instr_len; // Approximate; decode would be more accurate.
                    if is_write {
                        let rax = read_reg(vcpu, HV_X86_RAX)?;
                        let rip = read_reg(vcpu, HV_X86_RIP)?;
                        write_reg(vcpu, HV_X86_RIP, rip + instr_len as u64)?;
                        Ok(VmExit::MmioWrite {
                            address: guest_phys,
                            size,
                            data: rax,
                        })
                    } else if is_read {
                        Ok(VmExit::MmioRead {
                            address: guest_phys,
                            size,
                        })
                    } else {
                        Ok(VmExit::EptViolation {
                            guest_phys,
                            is_write,
                        })
                    }
                } else {
                    Ok(VmExit::EptViolation {
                        guest_phys,
                        is_write,
                    })
                }
            }

            vmx::EXIT_REASON_HLT => {
                // Advance past the HLT instruction (1 byte).
                let rip = read_reg(vcpu, HV_X86_RIP)?;
                let instr_len = read_vmcs(vcpu, vmx::VMCS_EXIT_INSTRUCTION_LENGTH)? as u64;
                write_reg(vcpu, HV_X86_RIP, rip + instr_len)?;
                Ok(VmExit::Hlt)
            }

            vmx::EXIT_REASON_CPUID => {
                let eax = read_reg(vcpu, HV_X86_RAX)? as u32;
                let ecx = read_reg(vcpu, HV_X86_RCX)? as u32;
                Ok(VmExit::Cpuid { eax, ecx })
            }

            vmx::EXIT_REASON_MSR_READ => {
                let ecx = read_reg(vcpu, HV_X86_RCX)? as u32;
                Ok(VmExit::MsrRead { index: ecx })
            }

            vmx::EXIT_REASON_MSR_WRITE => {
                let ecx = read_reg(vcpu, HV_X86_RCX)? as u32;
                let eax = read_reg(vcpu, HV_X86_RAX)?;
                let edx = read_reg(vcpu, HV_X86_RDX)?;
                let value = (edx << 32) | (eax & 0xFFFF_FFFF);
                Ok(VmExit::MsrWrite {
                    index: ecx,
                    value,
                })
            }

            vmx::EXIT_REASON_INTERRUPT_WINDOW => Ok(VmExit::InterruptWindow),

            vmx::EXIT_REASON_EXTERNAL_INTERRUPT => {
                // External interrupt — just re-enter the guest; the VMM
                // handles these through the interrupt-window mechanism.
                Ok(VmExit::InterruptWindow)
            }

            vmx::EXIT_REASON_TRIPLE_FAULT => Ok(VmExit::Shutdown),

            other => Ok(VmExit::Unknown(other)),
        }
    }

    fn request_interrupt_window(&mut self, _vcpu_id: u32) -> Result<(), HvError> {
        // Set the interrupt-window exiting bit in primary proc-based controls.
        let vcpu = self.vcpu_id;
        let mut controls = read_vmcs(vcpu, vmx::VMCS_PRIMARY_PROC_BASED_CONTROLS)? as u32;
        controls |= vmx::PROC_BASED_INTERRUPT_WINDOW_EXIT;
        write_vmcs(
            vcpu,
            vmx::VMCS_PRIMARY_PROC_BASED_CONTROLS,
            controls as u64,
        )
    }

    fn inject_interrupt(&mut self, _vcpu_id: u32, vector: u8) -> Result<(), HvError> {
        let vcpu = self.vcpu_id;

        // Clear the interrupt-window exiting bit since we are injecting now.
        let mut controls = read_vmcs(vcpu, vmx::VMCS_PRIMARY_PROC_BASED_CONTROLS)? as u32;
        controls &= !vmx::PROC_BASED_INTERRUPT_WINDOW_EXIT;
        write_vmcs(
            vcpu,
            vmx::VMCS_PRIMARY_PROC_BASED_CONTROLS,
            controls as u64,
        )?;

        // Encode the interrupt into the VM-entry interruption-info field.
        let info = vmx::INTR_INFO_VALID | vmx::INTR_TYPE_EXTERNAL | (vector as u32);
        write_vmcs(vcpu, vmx::VMCS_ENTRY_INTERRUPTION_INFO, info as u64)
    }

    fn interrupts_enabled(&self, _vcpu_id: u32) -> Result<bool, HvError> {
        let vcpu = self.vcpu_id;

        // Check RFLAGS.IF (bit 9).
        let rflags = read_reg(vcpu, HV_X86_RFLAGS)?;
        let if_set = (rflags & (1 << 9)) != 0;

        // Check interruptibility state in VMCS.
        let int_state = read_vmcs(vcpu, vmx::VMCS_GUEST_INTERRUPTIBILITY)? as u32;
        let not_blocked = (int_state
            & (vmx::INTERRUPTIBILITY_STI_BLOCKING | vmx::INTERRUPTIBILITY_MOV_SS_BLOCKING))
            == 0;

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
        _vcpu_id: u32,
        eax: u32,
        ebx: u32,
        ecx: u32,
        edx: u32,
    ) -> Result<(), HvError> {
        let vcpu = self.vcpu_id;
        write_reg(vcpu, HV_X86_RAX, eax as u64)?;
        write_reg(vcpu, HV_X86_RBX, ebx as u64)?;
        write_reg(vcpu, HV_X86_RCX, ecx as u64)?;
        write_reg(vcpu, HV_X86_RDX, edx as u64)?;
        Ok(())
    }

    fn set_msr_read_data(&mut self, _vcpu_id: u32, value: u64) -> Result<(), HvError> {
        let vcpu = self.vcpu_id;
        // MSR read result: low 32 bits in EAX, high 32 bits in EDX.
        write_reg(vcpu, HV_X86_RAX, value & 0xFFFF_FFFF)?;
        write_reg(vcpu, HV_X86_RDX, value >> 32)?;
        Ok(())
    }

    fn advance_rip(&mut self, _vcpu_id: u32, len: u8) -> Result<(), HvError> {
        let vcpu = self.vcpu_id;
        let rip = read_reg(vcpu, HV_X86_RIP)?;
        write_reg(vcpu, HV_X86_RIP, rip + len as u64)
    }

    fn request_stop(&mut self, _vcpu_id: u32) -> Result<(), HvError> {
        self.stop_requested = true;
        // Kick the vCPU out of hv_vcpu_run if it is currently executing.
        check(unsafe { hv_vcpu_interrupt(&self.vcpu_id as *const u32, 1) })
    }

    fn destroy(&mut self) {
        if self.vcpu_created {
            let _ = unsafe { hv_vcpu_destroy(self.vcpu_id) };
            self.vcpu_created = false;
        }
        if self.vm_created {
            let _ = unsafe { hv_vm_destroy() };
            self.vm_created = false;
        }
    }
}

impl Drop for HvfBackend {
    fn drop(&mut self) {
        self.destroy();
    }
}
