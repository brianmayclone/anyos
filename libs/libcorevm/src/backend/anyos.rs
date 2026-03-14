//! anyOS hardware virtualization backend.
//!
//! Uses kernel syscalls (SYS_VM_CREATE etc.) to access VMX/SVM hardware
//! via the anyOS kernel virt module.

use super::{CpuidEntry, DescriptorTable, SegmentReg, VcpuRegs, VcpuSregs, VmBackend, VmError, VmExitReason};
use alloc::vec::Vec;

// ── Syscall numbers ──────────────────────────────────────────────────────

const SYS_VM_HW_INFO: u64 = 613;
const SYS_VM_CREATE: u64 = 600;
const SYS_VM_DESTROY: u64 = 601;
const SYS_VM_SET_MEMORY: u64 = 602;
const SYS_VCPU_CREATE: u64 = 603;
const SYS_VCPU_RUN: u64 = 604;
const SYS_VCPU_GET_REGS: u64 = 605;
const SYS_VCPU_SET_REGS: u64 = 606;
const SYS_VCPU_GET_SREGS: u64 = 607;
const SYS_VCPU_SET_SREGS: u64 = 608;
const SYS_VCPU_INJECT_IRQ: u64 = 609;
const SYS_VCPU_INJECT_EXCEPTION: u64 = 610;
const SYS_VCPU_INJECT_NMI: u64 = 611;
const SYS_VM_SET_CPUID: u64 = 612;

// ── VMX exit reasons (must match kernel/src/arch/x86/virt/vmx.rs) ────────

const EXIT_REASON_EXTERNAL_INTERRUPT: u32 = 1;
const EXIT_REASON_TRIPLE_FAULT: u32 = 2;
const EXIT_REASON_CPUID: u32 = 10;
const EXIT_REASON_HLT: u32 = 12;
const EXIT_REASON_IO_INSTRUCTION: u32 = 30;
const EXIT_REASON_RDMSR: u32 = 31;
const EXIT_REASON_WRMSR: u32 = 32;
const EXIT_REASON_EPT_VIOLATION: u32 = 48;

// ── Raw syscall helpers ──────────────────────────────────────────────────
//
// anyOS 64-bit SYSCALL convention:
//   RAX = syscall number
//   RBX = arg1, R10 = arg2, RDX = arg3, RSI = arg4, RDI = arg5
//   Return value in RAX.

#[inline]
unsafe fn syscall0(nr: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "syscall",
        in("rax") nr,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[inline]
unsafe fn syscall1(nr: u64, a1: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "push rbx",
        "mov rbx, {a1}",
        "syscall",
        "pop rbx",
        a1 = in(reg) a1,
        in("rax") nr,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[inline]
unsafe fn syscall2(nr: u64, a1: u64, a2: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "push rbx",
        "mov rbx, {a1}",
        "syscall",
        "pop rbx",
        a1 = in(reg) a1,
        in("rax") nr,
        in("r10") a2,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[inline]
unsafe fn syscall3(nr: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "push rbx",
        "mov rbx, {a1}",
        "syscall",
        "pop rbx",
        a1 = in(reg) a1,
        in("rax") nr,
        in("r10") a2,
        in("rdx") a3,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[inline]
#[allow(dead_code)]
unsafe fn syscall4(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "push rbx",
        "mov rbx, {a1}",
        "syscall",
        "pop rbx",
        a1 = in(reg) a1,
        in("rax") nr,
        in("r10") a2,
        in("rdx") a3,
        in("rsi") a4,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

#[inline]
#[allow(dead_code)]
unsafe fn syscall5(nr: u64, a1: u64, a2: u64, a3: u64, a4: u64, a5: u64) -> u64 {
    let ret: u64;
    core::arch::asm!(
        "push rbx",
        "mov rbx, {a1}",
        "syscall",
        "pop rbx",
        a1 = in(reg) a1,
        in("rax") nr,
        in("r10") a2,
        in("rdx") a3,
        in("rsi") a4,
        in("rdi") a5,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack),
    );
    ret
}

// ── Kernel-matching structs (must be #[repr(C)] and match kernel layout) ─

/// Memory region descriptor passed to SYS_VM_SET_MEMORY.
/// Matches `MemRegionDesc` in kernel/src/arch/x86/virt/syscalls.rs.
#[repr(C)]
struct MemRegionDesc {
    guest_phys: u64,
    size: u64,
    host_phys: u64,
}

/// VM exit info returned by the kernel after SYS_VCPU_RUN.
/// Matches `VmExitInfo` in kernel/src/arch/x86/virt/mod.rs.
#[repr(C)]
#[derive(Default)]
struct KernelVmExitInfo {
    reason: u32,
    qualification: u64,
    guest_phys_addr: u64,
    instruction_len: u32,
    io_port: u16,
    access_size: u8,
    is_read: u8,
    io_data: u64,
    io_data2: u64,
    msr_index: u32,
    cpuid_function: u32,
    cpuid_index: u32,
    _pad: u32,
}

/// Guest GPRs as the kernel sees them.
/// Matches `GuestGprs` in kernel/src/arch/x86/virt/mod.rs.
#[repr(C)]
#[derive(Default)]
struct KernelGuestGprs {
    rax: u64, rbx: u64, rcx: u64, rdx: u64,
    rsi: u64, rdi: u64, rbp: u64,
    r8: u64, r9: u64, r10: u64, r11: u64,
    r12: u64, r13: u64, r14: u64, r15: u64,
}

/// Guest segment/control register state as the kernel sees them.
/// Matches `GuestSregs` in kernel/src/arch/x86/virt/mod.rs.
#[repr(C)]
#[derive(Default)]
struct KernelGuestSregs {
    cs_selector: u16, cs_base: u64, cs_limit: u32, cs_ar: u32,
    ds_selector: u16, ds_base: u64, ds_limit: u32, ds_ar: u32,
    es_selector: u16, es_base: u64, es_limit: u32, es_ar: u32,
    fs_selector: u16, fs_base: u64, fs_limit: u32, fs_ar: u32,
    gs_selector: u16, gs_base: u64, gs_limit: u32, gs_ar: u32,
    ss_selector: u16, ss_base: u64, ss_limit: u32, ss_ar: u32,
    tr_selector: u16, tr_base: u64, tr_limit: u32, tr_ar: u32,
    ldtr_selector: u16, ldtr_base: u64, ldtr_limit: u32, ldtr_ar: u32,
    gdtr_base: u64, gdtr_limit: u32,
    idtr_base: u64, idtr_limit: u32,
    cr0: u64, cr3: u64, cr4: u64, efer: u64,
    rip: u64, rsp: u64, rflags: u64,
}

// ── Memory slot tracking ─────────────────────────────────────────────────

struct MemorySlot {
    guest_phys: u64,
    size: u64,
    host_ptr: *mut u8,
}

// ── AnyOsBackend ─────────────────────────────────────────────────────────

pub struct AnyOsBackend {
    vm_id: u32,
    memory_slots: Vec<MemorySlot>,
}

/// Query hardware virtualization type from the kernel.
/// Returns 0 = none, 1 = VMX (Intel VT-x), 2 = SVM (AMD-V).
pub fn syscall_vm_hw_info() -> u32 {
    unsafe { syscall0(SYS_VM_HW_INFO) as u32 }
}

impl AnyOsBackend {
    pub fn new(_ram_bytes: usize) -> Result<Self, VmError> {
        // Check hardware support first so we can give a precise error.
        if syscall_vm_hw_info() == 0 {
            return Err(VmError::NoHardwareSupport);
        }
        let vm_id = unsafe { syscall0(SYS_VM_CREATE) } as u32;
        if vm_id == 0 || vm_id == u32::MAX {
            return Err(VmError::VmCreateFailed);
        }
        Ok(Self {
            vm_id,
            memory_slots: Vec::new(),
        })
    }

    fn translate_phys(&self, addr: u64, len: usize) -> Option<*mut u8> {
        for slot in &self.memory_slots {
            if addr >= slot.guest_phys && addr + len as u64 <= slot.guest_phys + slot.size {
                let offset = (addr - slot.guest_phys) as usize;
                return Some(unsafe { slot.host_ptr.add(offset) });
            }
        }
        None
    }

    fn check_result(ret: u64) -> Result<(), VmError> {
        if ret as u32 == u32::MAX {
            Err(VmError::BackendError(ret as i32))
        } else {
            Ok(())
        }
    }
}

impl Drop for AnyOsBackend {
    fn drop(&mut self) {
        self.destroy();
    }
}

// ── Segment register access‐rights conversion ───────────────────────────
//
// The kernel stores VMX-style access rights (a single u32), while the
// VcpuSregs/SegmentReg type uses decomposed fields.

fn seg_to_kernel(seg: &SegmentReg) -> (u16, u64, u32, u32) {
    let mut ar: u32 = 0;
    ar |= (seg.type_ as u32) & 0xF;           // bits 3:0
    ar |= ((seg.s as u32) & 1) << 4;          // bit 4
    ar |= ((seg.dpl as u32) & 3) << 5;        // bits 6:5
    ar |= ((seg.present as u32) & 1) << 7;    // bit 7
    ar |= ((seg.avl as u32) & 1) << 12;       // bit 12
    ar |= ((seg.l as u32) & 1) << 13;         // bit 13
    ar |= ((seg.db as u32) & 1) << 14;        // bit 14
    ar |= ((seg.g as u32) & 1) << 15;         // bit 15
    (seg.selector, seg.base, seg.limit, ar)
}

fn kernel_to_seg(selector: u16, base: u64, limit: u32, ar: u32) -> SegmentReg {
    SegmentReg {
        base,
        limit,
        selector,
        type_: (ar & 0xF) as u8,
        s: ((ar >> 4) & 1) as u8,
        dpl: ((ar >> 5) & 3) as u8,
        present: ((ar >> 7) & 1) as u8,
        avl: ((ar >> 12) & 1) as u8,
        l: ((ar >> 13) & 1) as u8,
        db: ((ar >> 14) & 1) as u8,
        g: ((ar >> 15) & 1) as u8,
    }
}

// ── VmBackend implementation ─────────────────────────────────────────────

impl VmBackend for AnyOsBackend {
    fn destroy(&mut self) {
        if self.vm_id != 0 {
            unsafe { syscall1(SYS_VM_DESTROY, self.vm_id as u64); }
            self.vm_id = 0;
        }
    }

    fn reset(&mut self) -> Result<(), VmError> {
        // Destroy and recreate the VM.
        let old_slots: Vec<MemorySlot> = core::mem::take(&mut self.memory_slots);
        self.destroy();
        let vm_id = unsafe { syscall0(SYS_VM_CREATE) } as u32;
        if vm_id == 0 || vm_id == u32::MAX {
            return Err(VmError::VmCreateFailed);
        }
        self.vm_id = vm_id;
        // Re-register memory regions.
        for (i, slot) in old_slots.iter().enumerate() {
            self.set_memory_region(i as u32, slot.guest_phys, slot.size, slot.host_ptr)?;
        }
        Ok(())
    }

    fn set_memory_region(&mut self, slot: u32, guest_phys: u64, size: u64, host_ptr: *mut u8) -> Result<(), VmError> {
        let desc = MemRegionDesc {
            guest_phys,
            size,
            host_phys: host_ptr as u64,
        };
        let ret = unsafe {
            syscall3(
                SYS_VM_SET_MEMORY,
                self.vm_id as u64,
                slot as u64,
                &desc as *const _ as u64,
            )
        };
        Self::check_result(ret)?;
        // Track for read_phys/write_phys.
        self.memory_slots.retain(|s| s.guest_phys != guest_phys);
        if size > 0 {
            self.memory_slots.push(MemorySlot { guest_phys, size, host_ptr });
        }
        Ok(())
    }

    fn read_phys(&self, addr: u64, buf: &mut [u8]) -> Result<(), VmError> {
        let ptr = self.translate_phys(addr, buf.len()).ok_or(VmError::MemoryMapFailed)?;
        unsafe { core::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), buf.len()); }
        Ok(())
    }

    fn write_phys(&mut self, addr: u64, buf: &[u8]) -> Result<(), VmError> {
        let ptr = self.translate_phys(addr, buf.len()).ok_or(VmError::MemoryMapFailed)?;
        unsafe { core::ptr::copy_nonoverlapping(buf.as_ptr(), ptr, buf.len()); }
        Ok(())
    }

    fn create_vcpu(&mut self, id: u32) -> Result<(), VmError> {
        let ret = unsafe { syscall2(SYS_VCPU_CREATE, self.vm_id as u64, id as u64) };
        Self::check_result(ret)
    }

    fn destroy_vcpu(&mut self, _id: u32) -> Result<(), VmError> {
        // The kernel destroys vCPUs when the VM is destroyed.
        Ok(())
    }

    fn run_vcpu(&mut self, id: u32) -> Result<VmExitReason, VmError> {
        let mut exit_info = KernelVmExitInfo::default();
        let ret = unsafe {
            syscall3(
                SYS_VCPU_RUN,
                self.vm_id as u64,
                id as u64,
                &mut exit_info as *mut _ as u64,
            )
        };
        if ret as u32 == u32::MAX {
            return Err(VmError::VmEntryFailed(0));
        }

        let reason = exit_info.reason & 0xFFFF;
        match reason {
            EXIT_REASON_IO_INSTRUCTION => {
                let port = exit_info.io_port;
                let size = exit_info.access_size;
                if exit_info.is_read != 0 {
                    Ok(VmExitReason::IoIn { port, size, count: 1 })
                } else {
                    Ok(VmExitReason::IoOut { port, size, data: exit_info.io_data as u32, count: 1 })
                }
            }
            EXIT_REASON_EPT_VIOLATION => {
                let addr = exit_info.guest_phys_addr;
                let size = exit_info.access_size;
                if exit_info.is_read != 0 {
                    Ok(VmExitReason::MmioRead { addr, size, dest_reg: 0, instr_len: 0 })
                } else {
                    Ok(VmExitReason::MmioWrite { addr, size, data: exit_info.io_data })
                }
            }
            EXIT_REASON_RDMSR => {
                Ok(VmExitReason::MsrRead { index: exit_info.msr_index })
            }
            EXIT_REASON_WRMSR => {
                Ok(VmExitReason::MsrWrite { index: exit_info.msr_index, value: exit_info.io_data })
            }
            EXIT_REASON_CPUID => {
                Ok(VmExitReason::CpuidExit { function: exit_info.cpuid_function, index: exit_info.cpuid_index })
            }
            EXIT_REASON_HLT => Ok(VmExitReason::Halted),
            EXIT_REASON_TRIPLE_FAULT => Ok(VmExitReason::Shutdown),
            EXIT_REASON_EXTERNAL_INTERRUPT => Ok(VmExitReason::InterruptWindow),
            _ => Ok(VmExitReason::Error),
        }
    }

    fn get_vcpu_regs(&self, id: u32) -> Result<VcpuRegs, VmError> {
        let mut gprs = KernelGuestGprs::default();
        let ret = unsafe {
            syscall3(
                SYS_VCPU_GET_REGS,
                self.vm_id as u64,
                id as u64,
                &mut gprs as *mut _ as u64,
            )
        };
        Self::check_result(ret)?;
        // Also need sregs for rip/rsp/rflags since the kernel stores those in VMCS.
        let mut ksregs = KernelGuestSregs::default();
        let ret2 = unsafe {
            syscall3(
                SYS_VCPU_GET_SREGS,
                self.vm_id as u64,
                id as u64,
                &mut ksregs as *mut _ as u64,
            )
        };
        Self::check_result(ret2)?;
        Ok(VcpuRegs {
            rax: gprs.rax, rbx: gprs.rbx, rcx: gprs.rcx, rdx: gprs.rdx,
            rsi: gprs.rsi, rdi: gprs.rdi, rbp: gprs.rbp, rsp: ksregs.rsp,
            r8: gprs.r8, r9: gprs.r9, r10: gprs.r10, r11: gprs.r11,
            r12: gprs.r12, r13: gprs.r13, r14: gprs.r14, r15: gprs.r15,
            rip: ksregs.rip, rflags: ksregs.rflags,
        })
    }

    fn set_vcpu_regs(&mut self, id: u32, regs: &VcpuRegs) -> Result<(), VmError> {
        let gprs = KernelGuestGprs {
            rax: regs.rax, rbx: regs.rbx, rcx: regs.rcx, rdx: regs.rdx,
            rsi: regs.rsi, rdi: regs.rdi, rbp: regs.rbp,
            r8: regs.r8, r9: regs.r9, r10: regs.r10, r11: regs.r11,
            r12: regs.r12, r13: regs.r13, r14: regs.r14, r15: regs.r15,
        };
        let ret = unsafe {
            syscall3(
                SYS_VCPU_SET_REGS,
                self.vm_id as u64,
                id as u64,
                &gprs as *const _ as u64,
            )
        };
        Self::check_result(ret)?;
        // Set rip/rsp/rflags via sregs.
        let mut ksregs = KernelGuestSregs::default();
        // Read current sregs first so we don't clobber segment state.
        let ret2 = unsafe {
            syscall3(
                SYS_VCPU_GET_SREGS,
                self.vm_id as u64,
                id as u64,
                &mut ksregs as *mut _ as u64,
            )
        };
        Self::check_result(ret2)?;
        ksregs.rip = regs.rip;
        ksregs.rsp = regs.rsp;
        ksregs.rflags = regs.rflags;
        let ret3 = unsafe {
            syscall3(
                SYS_VCPU_SET_SREGS,
                self.vm_id as u64,
                id as u64,
                &ksregs as *const _ as u64,
            )
        };
        Self::check_result(ret3)
    }

    fn get_vcpu_sregs(&self, id: u32) -> Result<VcpuSregs, VmError> {
        let mut ksregs = KernelGuestSregs::default();
        let ret = unsafe {
            syscall3(
                SYS_VCPU_GET_SREGS,
                self.vm_id as u64,
                id as u64,
                &mut ksregs as *mut _ as u64,
            )
        };
        Self::check_result(ret)?;
        Ok(VcpuSregs {
            cs: kernel_to_seg(ksregs.cs_selector, ksregs.cs_base, ksregs.cs_limit, ksregs.cs_ar),
            ds: kernel_to_seg(ksregs.ds_selector, ksregs.ds_base, ksregs.ds_limit, ksregs.ds_ar),
            es: kernel_to_seg(ksregs.es_selector, ksregs.es_base, ksregs.es_limit, ksregs.es_ar),
            fs: kernel_to_seg(ksregs.fs_selector, ksregs.fs_base, ksregs.fs_limit, ksregs.fs_ar),
            gs: kernel_to_seg(ksregs.gs_selector, ksregs.gs_base, ksregs.gs_limit, ksregs.gs_ar),
            ss: kernel_to_seg(ksregs.ss_selector, ksregs.ss_base, ksregs.ss_limit, ksregs.ss_ar),
            tr: kernel_to_seg(ksregs.tr_selector, ksregs.tr_base, ksregs.tr_limit, ksregs.tr_ar),
            ldt: kernel_to_seg(ksregs.ldtr_selector, ksregs.ldtr_base, ksregs.ldtr_limit, ksregs.ldtr_ar),
            gdt: DescriptorTable { base: ksregs.gdtr_base, limit: ksregs.gdtr_limit as u16 },
            idt: DescriptorTable { base: ksregs.idtr_base, limit: ksregs.idtr_limit as u16 },
            cr0: ksregs.cr0, cr2: 0, cr3: ksregs.cr3, cr4: ksregs.cr4, efer: ksregs.efer,
        })
    }

    fn set_vcpu_sregs(&mut self, id: u32, sregs: &VcpuSregs) -> Result<(), VmError> {
        // Read current kernel sregs to preserve rip/rsp/rflags.
        let mut ksregs = KernelGuestSregs::default();
        let ret = unsafe {
            syscall3(
                SYS_VCPU_GET_SREGS,
                self.vm_id as u64,
                id as u64,
                &mut ksregs as *mut _ as u64,
            )
        };
        Self::check_result(ret)?;

        let (sel, base, limit, ar) = seg_to_kernel(&sregs.cs);
        ksregs.cs_selector = sel; ksregs.cs_base = base; ksregs.cs_limit = limit; ksregs.cs_ar = ar;
        let (sel, base, limit, ar) = seg_to_kernel(&sregs.ds);
        ksregs.ds_selector = sel; ksregs.ds_base = base; ksregs.ds_limit = limit; ksregs.ds_ar = ar;
        let (sel, base, limit, ar) = seg_to_kernel(&sregs.es);
        ksregs.es_selector = sel; ksregs.es_base = base; ksregs.es_limit = limit; ksregs.es_ar = ar;
        let (sel, base, limit, ar) = seg_to_kernel(&sregs.fs);
        ksregs.fs_selector = sel; ksregs.fs_base = base; ksregs.fs_limit = limit; ksregs.fs_ar = ar;
        let (sel, base, limit, ar) = seg_to_kernel(&sregs.gs);
        ksregs.gs_selector = sel; ksregs.gs_base = base; ksregs.gs_limit = limit; ksregs.gs_ar = ar;
        let (sel, base, limit, ar) = seg_to_kernel(&sregs.ss);
        ksregs.ss_selector = sel; ksregs.ss_base = base; ksregs.ss_limit = limit; ksregs.ss_ar = ar;
        let (sel, base, limit, ar) = seg_to_kernel(&sregs.tr);
        ksregs.tr_selector = sel; ksregs.tr_base = base; ksregs.tr_limit = limit; ksregs.tr_ar = ar;
        let (sel, base, limit, ar) = seg_to_kernel(&sregs.ldt);
        ksregs.ldtr_selector = sel; ksregs.ldtr_base = base; ksregs.ldtr_limit = limit; ksregs.ldtr_ar = ar;

        ksregs.gdtr_base = sregs.gdt.base;
        ksregs.gdtr_limit = sregs.gdt.limit as u32;
        ksregs.idtr_base = sregs.idt.base;
        ksregs.idtr_limit = sregs.idt.limit as u32;

        ksregs.cr0 = sregs.cr0;
        ksregs.cr3 = sregs.cr3;
        ksregs.cr4 = sregs.cr4;
        ksregs.efer = sregs.efer;

        let ret2 = unsafe {
            syscall3(
                SYS_VCPU_SET_SREGS,
                self.vm_id as u64,
                id as u64,
                &ksregs as *const _ as u64,
            )
        };
        Self::check_result(ret2)
    }

    fn inject_interrupt(&mut self, id: u32, vector: u8) -> Result<(), VmError> {
        let ret = unsafe {
            syscall3(SYS_VCPU_INJECT_IRQ, self.vm_id as u64, id as u64, vector as u64)
        };
        Self::check_result(ret)
    }

    fn inject_exception(&mut self, id: u32, vector: u8, error_code: Option<u32>) -> Result<(), VmError> {
        // Kernel expects: info = vector (bits 7:0) | error_code << 8.
        let ec = error_code.unwrap_or(0);
        let info = (vector as u32) | (ec << 8);
        let ret = unsafe {
            syscall3(SYS_VCPU_INJECT_EXCEPTION, self.vm_id as u64, id as u64, info as u64)
        };
        Self::check_result(ret)
    }

    fn inject_nmi(&mut self, id: u32) -> Result<(), VmError> {
        let ret = unsafe {
            syscall2(SYS_VCPU_INJECT_NMI, self.vm_id as u64, id as u64)
        };
        Self::check_result(ret)
    }

    fn request_interrupt_window(&mut self, _id: u32, _enable: bool) -> Result<(), VmError> {
        // The kernel handles interrupt windowing internally on VMX/SVM.
        // External interrupts cause a VM-exit which we report as InterruptWindow.
        Ok(())
    }

    fn set_cpuid(&mut self, entries: &[CpuidEntry]) -> Result<(), VmError> {
        if entries.is_empty() {
            return Ok(());
        }
        // The kernel CpuidEntry layout matches ours (both are #[repr(C)] with same fields).
        let ret = unsafe {
            syscall3(
                SYS_VM_SET_CPUID,
                self.vm_id as u64,
                entries.as_ptr() as u64,
                entries.len() as u64,
            )
        };
        Self::check_result(ret)
    }
}
