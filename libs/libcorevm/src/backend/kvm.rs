//! Linux KVM backend for hardware-accelerated virtualization.

use super::{CpuidEntry, DescriptorTable, SegmentReg, VcpuRegs, VcpuSregs, VmBackend, VmError, VmExitReason};
use alloc::vec::Vec;

// ── KVM ioctl numbers (x86_64 Linux) ──────────────────────────────────────

const KVM_GET_API_VERSION: u64 = 0xAE00;
const KVM_CREATE_VM: u64 = 0xAE01;
const KVM_GET_VCPU_MMAP_SIZE: u64 = 0xAE04;
const KVM_CREATE_VCPU: u64 = 0xAE41;
const KVM_SET_USER_MEMORY_REGION: u64 = 0x4020_AE46;
const KVM_RUN: u64 = 0xAE80;
const KVM_GET_REGS: u64 = 0x8090_AE81;
const KVM_SET_REGS: u64 = 0x4090_AE82;
const KVM_GET_SREGS: u64 = 0x8138_AE83;
const KVM_SET_SREGS: u64 = 0x4138_AE84;
const KVM_INTERRUPT: u64 = 0x4004_AE86;
const KVM_SET_CPUID2: u64 = 0x4008_AE90;
const KVM_GET_VCPU_EVENTS: u64 = 0x8040_AE9F;
const KVM_SET_VCPU_EVENTS: u64 = 0x4040_AEA0;

// Exit reasons
const KVM_EXIT_IO: u32 = 2;
const KVM_EXIT_DEBUG: u32 = 4;
const KVM_EXIT_HLT: u32 = 5;
const KVM_EXIT_MMIO: u32 = 6;
const KVM_EXIT_IRQ_WINDOW_OPEN: u32 = 7;
const KVM_EXIT_SHUTDOWN: u32 = 8;
const KVM_EXIT_INTERNAL_ERROR: u32 = 17;

const KVM_EXIT_IO_IN: u8 = 0;
const KVM_EXIT_IO_OUT: u8 = 1;

// mmap constants
const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const MAP_SHARED: i32 = 1;
const O_RDWR: i32 = 2;

// ── Raw syscall helpers ───────────────────────────────────────────────────

unsafe fn sys_ioctl(fd: i32, request: u64, arg: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") 16_u64,
        in("rdi") fd as u64,
        in("rsi") request,
        in("rdx") arg,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack)
    );
    ret
}

unsafe fn sys_mmap(addr: u64, len: u64, prot: i32, flags: i32, fd: i32, offset: i64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") 9_u64,
        in("rdi") addr,
        in("rsi") len,
        in("rdx") prot as u64,
        in("r10") flags as u64,
        in("r8") fd as u64,
        in("r9") offset as u64,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack)
    );
    ret
}

unsafe fn sys_munmap(addr: u64, len: u64) -> i64 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") 11_u64,
        in("rdi") addr,
        in("rsi") len,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack)
    );
    ret
}

unsafe fn sys_open(path: *const u8, flags: i32) -> i32 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") 2_u64,
        in("rdi") path as u64,
        in("rsi") flags as u64,
        in("rdx") 0_u64,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack)
    );
    ret as i32
}

unsafe fn sys_close(fd: i32) -> i32 {
    let ret: i64;
    core::arch::asm!(
        "syscall",
        in("rax") 3_u64,
        in("rdi") fd as u64,
        lateout("rax") ret,
        out("rcx") _,
        out("r11") _,
        options(nostack)
    );
    ret as i32
}

// ── KVM structs (repr C, matching Linux headers) ──────────────────────────

#[repr(C)]
struct KvmUserspaceMemoryRegion {
    slot: u32,
    flags: u32,
    guest_phys_addr: u64,
    memory_size: u64,
    userspace_addr: u64,
}

/// kvm_regs — note KVM field order: rax rbx rcx rdx rsi rdi rsp rbp r8-r15 rip rflags
#[repr(C)]
#[derive(Default)]
struct KvmRegs {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rsp: u64,
    rbp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
    rflags: u64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KvmSegment {
    base: u64,
    limit: u32,
    selector: u16,
    type_: u8,
    present: u8,
    dpl: u8,
    db: u8,
    s: u8,
    l: u8,
    g: u8,
    avl: u8,
    unusable: u8,
    _padding: u8,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct KvmDtable {
    base: u64,
    limit: u16,
    _padding: [u16; 3],
}

#[repr(C)]
#[derive(Default)]
struct KvmSregs {
    cs: KvmSegment,
    ds: KvmSegment,
    es: KvmSegment,
    fs: KvmSegment,
    gs: KvmSegment,
    ss: KvmSegment,
    tr: KvmSegment,
    ldt: KvmSegment,
    gdt: KvmDtable,
    idt: KvmDtable,
    cr0: u64,
    cr2: u64,
    cr3: u64,
    cr4: u64,
    cr8: u64,
    efer: u64,
    apic_base: u64,
    interrupt_bitmap: [u64; 4], // 256 bits
}

#[repr(C)]
struct KvmVcpuEvents {
    exception: KvmVcpuEventException,
    interrupt: KvmVcpuEventInterrupt,
    nmi: KvmVcpuEventNmi,
    sipi_vector: u32,
    flags: u32,
    smi: KvmVcpuEventSmi,
    _reserved: [u8; 27],
    exception_has_payload: u8,
    exception_payload: u64,
}

#[repr(C)]
struct KvmVcpuEventException {
    injected: u8,
    nr: u8,
    has_error_code: u8,
    pending: u8,
    error_code: u32,
}

#[repr(C)]
struct KvmVcpuEventInterrupt {
    injected: u8,
    nr: u8,
    soft: u8,
    shadow: u8,
}

#[repr(C)]
struct KvmVcpuEventNmi {
    injected: u8,
    pending: u8,
    masked: u8,
    _pad: u8,
}

#[repr(C)]
struct KvmVcpuEventSmi {
    smm: u8,
    pending: u8,
    smm_inside_nmi: u8,
    latched_init: u8,
}

/// kvm_run shared page — offsets for exit data sub-structs.
#[repr(C)]
struct KvmRun {
    request_interrupt_window: u8,
    immediate_exit: u8,
    _padding1: [u8; 6],
    exit_reason: u32,
    ready_for_interrupt_injection: u8,
    if_flag: u8,
    flags: u16,
    cr8: u64,
    apic_base: u64,
    // offset 32: union of exit info — we access via raw pointer offsets
    exit_data: [u8; 256],
}

/// IO exit sub-struct at kvm_run offset 32
#[repr(C)]
struct KvmRunExitIo {
    direction: u8,
    size: u8,
    port: u16,
    count: u32,
    data_offset: u64,
}

/// MMIO exit sub-struct at kvm_run offset 32
#[repr(C)]
struct KvmRunExitMmio {
    phys_addr: u64,
    data: [u8; 8],
    len: u32,
    is_write: u8,
}

#[repr(C)]
struct KvmCpuidEntry2 {
    function: u32,
    index: u32,
    flags: u32,
    eax: u32,
    ebx: u32,
    ecx: u32,
    edx: u32,
    _padding: [u32; 3],
}

// ── Memory region tracking for read_phys / write_phys ─────────────────────

struct MemorySlot {
    guest_phys: u64,
    size: u64,
    host_ptr: *mut u8,
}

// ── vCPU ──────────────────────────────────────────────────────────────────

struct KvmVcpu {
    fd: i32,
    kvm_run: *mut KvmRun,
    mmap_size: usize,
}

impl Drop for KvmVcpu {
    fn drop(&mut self) {
        unsafe {
            if !self.kvm_run.is_null() {
                sys_munmap(self.kvm_run as u64, self.mmap_size as u64);
            }
            sys_close(self.fd);
        }
    }
}

// ── KvmBackend ────────────────────────────────────────────────────────────

pub struct KvmBackend {
    kvm_fd: i32,
    vm_fd: i32,
    vcpus: Vec<Option<KvmVcpu>>,
    mmap_size: usize,
    memory_slots: Vec<MemorySlot>,
}

impl KvmBackend {
    pub fn new() -> Result<Self, VmError> {
        unsafe {
            let kvm_fd = sys_open(b"/dev/kvm\0".as_ptr(), O_RDWR);
            if kvm_fd < 0 {
                return Err(VmError::NoHardwareSupport);
            }

            let api_ver = sys_ioctl(kvm_fd, KVM_GET_API_VERSION, 0);
            if api_ver != 12 {
                sys_close(kvm_fd);
                return Err(VmError::NoHardwareSupport);
            }

            let mmap_size = sys_ioctl(kvm_fd, KVM_GET_VCPU_MMAP_SIZE, 0);
            if mmap_size <= 0 {
                sys_close(kvm_fd);
                return Err(VmError::BackendError(mmap_size as i32));
            }

            let vm_fd = sys_ioctl(kvm_fd, KVM_CREATE_VM, 0) as i32;
            if vm_fd < 0 {
                sys_close(kvm_fd);
                return Err(VmError::BackendError(vm_fd));
            }

            Ok(Self {
                kvm_fd,
                vm_fd,
                vcpus: Vec::new(),
                mmap_size: mmap_size as usize,
                memory_slots: Vec::new(),
            })
        }
    }

    fn get_vcpu(&self, id: u32) -> Result<&KvmVcpu, VmError> {
        self.vcpus
            .get(id as usize)
            .and_then(|v| v.as_ref())
            .ok_or(VmError::InvalidVcpuId)
    }

    fn get_vcpu_mut(&mut self, id: u32) -> Result<&mut KvmVcpu, VmError> {
        self.vcpus
            .get_mut(id as usize)
            .and_then(|v| v.as_mut())
            .ok_or(VmError::InvalidVcpuId)
    }

    /// Write I/O response data into the kvm_run shared page for an IoIn exit.
    /// Must be called before the next `run_vcpu`.
    pub fn set_io_response(&mut self, vcpu_id: u32, data: &[u8]) {
        if let Ok(vcpu) = self.get_vcpu(vcpu_id) {
            unsafe {
                let run = &*vcpu.kvm_run;
                let io = &*(run.exit_data.as_ptr() as *const KvmRunExitIo);
                let dst = (vcpu.kvm_run as *mut u8).add(io.data_offset as usize);
                let len = data.len().min(io.size as usize * io.count as usize);
                core::ptr::copy_nonoverlapping(data.as_ptr(), dst, len);
            }
        }
    }

    /// Write MMIO response data into the kvm_run shared page for an MmioRead exit.
    /// Must be called before the next `run_vcpu`.
    pub fn set_mmio_response(&mut self, vcpu_id: u32, data: &[u8]) {
        if let Ok(vcpu) = self.get_vcpu(vcpu_id) {
            unsafe {
                let mmio = &mut *((*vcpu.kvm_run).exit_data.as_ptr() as *mut KvmRunExitMmio);
                let len = data.len().min(mmio.len as usize).min(8);
                mmio.data[..len].copy_from_slice(&data[..len]);
            }
        }
    }

    fn translate_phys(&self, addr: u64) -> Option<*mut u8> {
        for slot in &self.memory_slots {
            if addr >= slot.guest_phys && addr < slot.guest_phys + slot.size {
                let offset = (addr - slot.guest_phys) as usize;
                return Some(unsafe { slot.host_ptr.add(offset) });
            }
        }
        None
    }
}

impl Drop for KvmBackend {
    fn drop(&mut self) {
        self.destroy();
    }
}

// ── Segment conversion helpers ────────────────────────────────────────────

fn seg_to_kvm(s: &SegmentReg) -> KvmSegment {
    KvmSegment {
        base: s.base,
        limit: s.limit,
        selector: s.selector,
        type_: s.type_,
        present: s.present,
        dpl: s.dpl,
        db: s.db,
        s: s.s,
        l: s.l,
        g: s.g,
        avl: s.avl,
        unusable: if s.present != 0 { 0 } else { 1 },
        _padding: 0,
    }
}

fn seg_from_kvm(k: &KvmSegment) -> SegmentReg {
    SegmentReg {
        base: k.base,
        limit: k.limit,
        selector: k.selector,
        type_: k.type_,
        present: k.present,
        dpl: k.dpl,
        db: k.db,
        s: k.s,
        l: k.l,
        g: k.g,
        avl: k.avl,
    }
}

fn dt_from_kvm(k: &KvmDtable) -> DescriptorTable {
    DescriptorTable {
        base: k.base,
        limit: k.limit,
    }
}

fn dt_to_kvm(d: &DescriptorTable) -> KvmDtable {
    KvmDtable {
        base: d.base,
        limit: d.limit,
        _padding: [0; 3],
    }
}

// ── VmBackend implementation ──────────────────────────────────────────────

impl VmBackend for KvmBackend {
    fn destroy(&mut self) {
        self.vcpus.clear();
        if self.vm_fd >= 0 {
            unsafe { sys_close(self.vm_fd); }
            self.vm_fd = -1;
        }
        if self.kvm_fd >= 0 {
            unsafe { sys_close(self.kvm_fd); }
            self.kvm_fd = -1;
        }
    }

    fn reset(&mut self) -> Result<(), VmError> {
        // KVM doesn't have a global VM reset; vCPUs must be re-created.
        Ok(())
    }

    fn set_memory_region(
        &mut self,
        slot: u32,
        guest_phys: u64,
        size: u64,
        host_ptr: *mut u8,
    ) -> Result<(), VmError> {
        let region = KvmUserspaceMemoryRegion {
            slot,
            flags: 0,
            guest_phys_addr: guest_phys,
            memory_size: size,
            userspace_addr: host_ptr as u64,
        };
        let ret = unsafe {
            sys_ioctl(
                self.vm_fd,
                KVM_SET_USER_MEMORY_REGION,
                &region as *const _ as u64,
            )
        };
        if ret < 0 {
            return Err(VmError::MemoryMapFailed);
        }
        // Track for read_phys/write_phys
        self.memory_slots.retain(|s| s.guest_phys != guest_phys);
        if size > 0 {
            self.memory_slots.push(MemorySlot {
                guest_phys,
                size,
                host_ptr,
            });
        }
        Ok(())
    }

    fn read_phys(&self, addr: u64, buf: &mut [u8]) -> Result<(), VmError> {
        let ptr = self.translate_phys(addr).ok_or(VmError::MemoryMapFailed)?;
        unsafe {
            core::ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), buf.len());
        }
        Ok(())
    }

    fn write_phys(&mut self, addr: u64, buf: &[u8]) -> Result<(), VmError> {
        let ptr = self.translate_phys(addr).ok_or(VmError::MemoryMapFailed)?;
        unsafe {
            core::ptr::copy_nonoverlapping(buf.as_ptr(), ptr, buf.len());
        }
        Ok(())
    }

    fn create_vcpu(&mut self, id: u32) -> Result<(), VmError> {
        unsafe {
            let vcpu_fd = sys_ioctl(self.vm_fd, KVM_CREATE_VCPU, id as u64) as i32;
            if vcpu_fd < 0 {
                return Err(VmError::BackendError(vcpu_fd));
            }

            let run_ptr = sys_mmap(
                0,
                self.mmap_size as u64,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                vcpu_fd,
                0,
            );
            if run_ptr < 0 || (run_ptr as u64) >= 0xFFFF_FFFF_FFFF_F000 {
                sys_close(vcpu_fd);
                return Err(VmError::BackendError(-1));
            }

            let vcpu = KvmVcpu {
                fd: vcpu_fd,
                kvm_run: run_ptr as *mut KvmRun,
                mmap_size: self.mmap_size,
            };

            let idx = id as usize;
            while self.vcpus.len() <= idx {
                self.vcpus.push(None);
            }
            self.vcpus[idx] = Some(vcpu);
        }
        Ok(())
    }

    fn destroy_vcpu(&mut self, id: u32) -> Result<(), VmError> {
        let idx = id as usize;
        if idx < self.vcpus.len() {
            self.vcpus[idx] = None; // Drop handles cleanup
            Ok(())
        } else {
            Err(VmError::InvalidVcpuId)
        }
    }

    fn run_vcpu(&mut self, id: u32) -> Result<VmExitReason, VmError> {
        let vcpu = self.get_vcpu(id)?;
        let fd = vcpu.fd;
        let run = vcpu.kvm_run;

        let ret = unsafe { sys_ioctl(fd, KVM_RUN, 0) };
        if ret < 0 {
            // EINTR (4) is retriable but we report it; EAGAIN similarly
            let errno = (-ret) as i32;
            return Err(VmError::BackendError(errno));
        }

        unsafe {
            let exit_reason = (*run).exit_reason;
            match exit_reason {
                KVM_EXIT_IO => {
                    let io = &*((*run).exit_data.as_ptr() as *const KvmRunExitIo);
                    let data_ptr = (run as *const u8).add(io.data_offset as usize);
                    if io.direction == KVM_EXIT_IO_OUT {
                        let mut val: u32 = 0;
                        core::ptr::copy_nonoverlapping(
                            data_ptr,
                            &mut val as *mut u32 as *mut u8,
                            io.size as usize,
                        );
                        Ok(VmExitReason::IoOut {
                            port: io.port,
                            size: io.size,
                            data: val,
                        })
                    } else {
                        Ok(VmExitReason::IoIn {
                            port: io.port,
                            size: io.size,
                        })
                    }
                }
                KVM_EXIT_MMIO => {
                    let mmio = &*((*run).exit_data.as_ptr() as *const KvmRunExitMmio);
                    if mmio.is_write != 0 {
                        let mut val: u64 = 0;
                        core::ptr::copy_nonoverlapping(
                            mmio.data.as_ptr(),
                            &mut val as *mut u64 as *mut u8,
                            (mmio.len as usize).min(8),
                        );
                        Ok(VmExitReason::MmioWrite {
                            addr: mmio.phys_addr,
                            size: mmio.len as u8,
                            data: val,
                        })
                    } else {
                        Ok(VmExitReason::MmioRead {
                            addr: mmio.phys_addr,
                            size: mmio.len as u8,
                        })
                    }
                }
                KVM_EXIT_HLT => Ok(VmExitReason::Halted),
                KVM_EXIT_SHUTDOWN => Ok(VmExitReason::Shutdown),
                KVM_EXIT_IRQ_WINDOW_OPEN => Ok(VmExitReason::InterruptWindow),
                KVM_EXIT_DEBUG => Ok(VmExitReason::Debug),
                KVM_EXIT_INTERNAL_ERROR => Ok(VmExitReason::Error),
                _ => Ok(VmExitReason::Error),
            }
        }
    }

    fn get_vcpu_regs(&self, id: u32) -> Result<VcpuRegs, VmError> {
        let vcpu = self.get_vcpu(id)?;
        let mut kregs = KvmRegs::default();
        let ret = unsafe {
            sys_ioctl(vcpu.fd, KVM_GET_REGS, &mut kregs as *mut _ as u64)
        };
        if ret < 0 {
            return Err(VmError::BackendError(ret as i32));
        }
        Ok(VcpuRegs {
            rax: kregs.rax,
            rbx: kregs.rbx,
            rcx: kregs.rcx,
            rdx: kregs.rdx,
            rsi: kregs.rsi,
            rdi: kregs.rdi,
            rbp: kregs.rbp,
            rsp: kregs.rsp,
            r8: kregs.r8,
            r9: kregs.r9,
            r10: kregs.r10,
            r11: kregs.r11,
            r12: kregs.r12,
            r13: kregs.r13,
            r14: kregs.r14,
            r15: kregs.r15,
            rip: kregs.rip,
            rflags: kregs.rflags,
        })
    }

    fn set_vcpu_regs(&mut self, id: u32, regs: &VcpuRegs) -> Result<(), VmError> {
        let vcpu = self.get_vcpu(id)?;
        let kregs = KvmRegs {
            rax: regs.rax,
            rbx: regs.rbx,
            rcx: regs.rcx,
            rdx: regs.rdx,
            rsi: regs.rsi,
            rdi: regs.rdi,
            rsp: regs.rsp,
            rbp: regs.rbp,
            r8: regs.r8,
            r9: regs.r9,
            r10: regs.r10,
            r11: regs.r11,
            r12: regs.r12,
            r13: regs.r13,
            r14: regs.r14,
            r15: regs.r15,
            rip: regs.rip,
            rflags: regs.rflags,
        };
        let ret = unsafe {
            sys_ioctl(vcpu.fd, KVM_SET_REGS, &kregs as *const _ as u64)
        };
        if ret < 0 {
            return Err(VmError::BackendError(ret as i32));
        }
        Ok(())
    }

    fn get_vcpu_sregs(&self, id: u32) -> Result<VcpuSregs, VmError> {
        let vcpu = self.get_vcpu(id)?;
        let mut ks = KvmSregs::default();
        let ret = unsafe {
            sys_ioctl(vcpu.fd, KVM_GET_SREGS, &mut ks as *mut _ as u64)
        };
        if ret < 0 {
            return Err(VmError::BackendError(ret as i32));
        }
        Ok(VcpuSregs {
            cs: seg_from_kvm(&ks.cs),
            ds: seg_from_kvm(&ks.ds),
            es: seg_from_kvm(&ks.es),
            fs: seg_from_kvm(&ks.fs),
            gs: seg_from_kvm(&ks.gs),
            ss: seg_from_kvm(&ks.ss),
            tr: seg_from_kvm(&ks.tr),
            ldt: seg_from_kvm(&ks.ldt),
            gdt: dt_from_kvm(&ks.gdt),
            idt: dt_from_kvm(&ks.idt),
            cr0: ks.cr0,
            cr2: ks.cr2,
            cr3: ks.cr3,
            cr4: ks.cr4,
            efer: ks.efer,
        })
    }

    fn set_vcpu_sregs(&mut self, id: u32, sregs: &VcpuSregs) -> Result<(), VmError> {
        let vcpu = self.get_vcpu(id)?;
        // Read current to preserve fields we don't expose (cr8, apic_base, interrupt_bitmap)
        let mut ks = KvmSregs::default();
        let ret = unsafe {
            sys_ioctl(vcpu.fd, KVM_GET_SREGS, &mut ks as *mut _ as u64)
        };
        if ret < 0 {
            return Err(VmError::BackendError(ret as i32));
        }

        ks.cs = seg_to_kvm(&sregs.cs);
        ks.ds = seg_to_kvm(&sregs.ds);
        ks.es = seg_to_kvm(&sregs.es);
        ks.fs = seg_to_kvm(&sregs.fs);
        ks.gs = seg_to_kvm(&sregs.gs);
        ks.ss = seg_to_kvm(&sregs.ss);
        ks.tr = seg_to_kvm(&sregs.tr);
        ks.ldt = seg_to_kvm(&sregs.ldt);
        ks.gdt = dt_to_kvm(&sregs.gdt);
        ks.idt = dt_to_kvm(&sregs.idt);
        ks.cr0 = sregs.cr0;
        ks.cr2 = sregs.cr2;
        ks.cr3 = sregs.cr3;
        ks.cr4 = sregs.cr4;
        ks.efer = sregs.efer;

        let ret = unsafe {
            sys_ioctl(vcpu.fd, KVM_SET_SREGS, &ks as *const _ as u64)
        };
        if ret < 0 {
            return Err(VmError::BackendError(ret as i32));
        }
        Ok(())
    }

    fn inject_interrupt(&mut self, id: u32, vector: u8) -> Result<(), VmError> {
        let vcpu = self.get_vcpu(id)?;
        let irq: u32 = vector as u32;
        let ret = unsafe {
            sys_ioctl(vcpu.fd, KVM_INTERRUPT, &irq as *const u32 as u64)
        };
        if ret < 0 {
            return Err(VmError::BackendError(ret as i32));
        }
        Ok(())
    }

    fn inject_exception(&mut self, id: u32, vector: u8, error_code: Option<u32>) -> Result<(), VmError> {
        let vcpu = self.get_vcpu(id)?;

        // Read current events
        let mut events: KvmVcpuEvents = unsafe { core::mem::zeroed() };
        let ret = unsafe {
            sys_ioctl(vcpu.fd, KVM_GET_VCPU_EVENTS, &mut events as *mut _ as u64)
        };
        if ret < 0 {
            return Err(VmError::BackendError(ret as i32));
        }

        events.exception.injected = 1;
        events.exception.nr = vector;
        events.exception.has_error_code = if error_code.is_some() { 1 } else { 0 };
        events.exception.error_code = error_code.unwrap_or(0);

        let ret = unsafe {
            sys_ioctl(vcpu.fd, KVM_SET_VCPU_EVENTS, &events as *const _ as u64)
        };
        if ret < 0 {
            return Err(VmError::BackendError(ret as i32));
        }
        Ok(())
    }

    fn inject_nmi(&mut self, id: u32) -> Result<(), VmError> {
        let vcpu = self.get_vcpu(id)?;

        let mut events: KvmVcpuEvents = unsafe { core::mem::zeroed() };
        let ret = unsafe {
            sys_ioctl(vcpu.fd, KVM_GET_VCPU_EVENTS, &mut events as *mut _ as u64)
        };
        if ret < 0 {
            return Err(VmError::BackendError(ret as i32));
        }

        events.nmi.injected = 1;

        let ret = unsafe {
            sys_ioctl(vcpu.fd, KVM_SET_VCPU_EVENTS, &events as *const _ as u64)
        };
        if ret < 0 {
            return Err(VmError::BackendError(ret as i32));
        }
        Ok(())
    }

    fn request_interrupt_window(&mut self, id: u32, enable: bool) -> Result<(), VmError> {
        let vcpu = self.get_vcpu_mut(id)?;
        unsafe {
            (*vcpu.kvm_run).request_interrupt_window = if enable { 1 } else { 0 };
        }
        Ok(())
    }

    fn set_cpuid(&mut self, entries: &[CpuidEntry]) -> Result<(), VmError> {
        // Build a buffer: header (nent: u32, padding: u32) + N entries
        let header_size = 8usize; // nent + padding
        let entry_size = core::mem::size_of::<KvmCpuidEntry2>();
        let total = header_size + entries.len() * entry_size;
        let mut buf = vec![0u8; total];

        // Write nent
        let nent = entries.len() as u32;
        buf[0..4].copy_from_slice(&nent.to_ne_bytes());

        // Write entries
        for (i, e) in entries.iter().enumerate() {
            let off = header_size + i * entry_size;
            let ke = KvmCpuidEntry2 {
                function: e.function,
                index: e.index,
                flags: 0,
                eax: e.eax,
                ebx: e.ebx,
                ecx: e.ecx,
                edx: e.edx,
                _padding: [0; 3],
            };
            unsafe {
                core::ptr::copy_nonoverlapping(
                    &ke as *const _ as *const u8,
                    buf.as_mut_ptr().add(off),
                    entry_size,
                );
            }
        }

        // Apply to all existing vCPUs
        for vcpu_opt in &self.vcpus {
            if let Some(vcpu) = vcpu_opt {
                let ret = unsafe {
                    sys_ioctl(vcpu.fd, KVM_SET_CPUID2, buf.as_ptr() as u64)
                };
                if ret < 0 {
                    return Err(VmError::BackendError(ret as i32));
                }
            }
        }
        Ok(())
    }
}
