//! KVM backend for Linux.
//!
//! Implements [`HypervisorBackend`] using `/dev/kvm` ioctls.
//! Only compiled when `feature = "host_test"` and `target_os = "linux"`.

use super::{DtableState, HvError, HypervisorBackend, MemoryRegion, SegmentState, VcpuRegs, VmExit};
use core::ptr;

// ════════════════════════════════════════════════════════════════════════
// FFI declarations
// ════════════════════════════════════════════════════════════════════════

extern "C" {
    fn open(pathname: *const u8, flags: i32, ...) -> i32;
    fn close(fd: i32) -> i32;
    fn ioctl(fd: i32, request: u64, ...) -> i32;
    fn mmap(addr: *mut u8, length: usize, prot: i32, flags: i32, fd: i32, offset: i64) -> *mut u8;
    fn munmap(addr: *mut u8, length: usize) -> i32;
}

fn errno() -> i32 {
    // Read errno from the C thread-local. On Linux/glibc, __errno_location()
    // returns a pointer to the thread-local errno.
    extern "C" {
        fn __errno_location() -> *mut i32;
    }
    unsafe { *__errno_location() }
}

// ════════════════════════════════════════════════════════════════════════
// POSIX / mmap constants
// ════════════════════════════════════════════════════════════════════════

const O_RDWR: i32 = 2;
const PROT_READ: i32 = 1;
const PROT_WRITE: i32 = 2;
const MAP_SHARED: i32 = 1;
const MAP_FAILED: *mut u8 = !0usize as *mut u8;

// ════════════════════════════════════════════════════════════════════════
// KVM ioctl numbers
// ════════════════════════════════════════════════════════════════════════

const KVM_GET_API_VERSION: u64 = 0xAE00;
const KVM_CREATE_VM: u64 = 0xAE01;
const KVM_CHECK_EXTENSION: u64 = 0xAE03;
const KVM_GET_VCPU_MMAP_SIZE: u64 = 0xAE04;
const KVM_CREATE_VCPU: u64 = 0xAE41;
const KVM_SET_USER_MEMORY_REGION: u64 = 0x4020AE46;
const KVM_RUN: u64 = 0xAE80;
const KVM_GET_REGS: u64 = 0x8090AE81;
const KVM_SET_REGS: u64 = 0x4090AE82;
const KVM_GET_SREGS: u64 = 0x8138AE83;
const KVM_SET_SREGS: u64 = 0x4138AE84;
const KVM_INTERRUPT: u64 = 0x4004AE86;
const KVM_SET_CPUID2: u64 = 0x4008AE90;
const KVM_GET_SUPPORTED_CPUID: u64 = 0xC008AE05;

// ════════════════════════════════════════════════════════════════════════
// KVM exit reasons
// ════════════════════════════════════════════════════════════════════════

const KVM_EXIT_IO: u32 = 2;
const KVM_EXIT_HLT: u32 = 5;
const KVM_EXIT_MMIO: u32 = 6;
const KVM_EXIT_SHUTDOWN: u32 = 8;
const KVM_EXIT_INTERNAL_ERROR: u32 = 17;

/// IO direction: in (read from device).
const KVM_EXIT_IO_IN: u8 = 0;
/// IO direction: out (write to device).
const KVM_EXIT_IO_OUT: u8 = 1;

// ════════════════════════════════════════════════════════════════════════
// KVM ABI structs
// ════════════════════════════════════════════════════════════════════════

/// KVM general-purpose registers, matching the kernel's `struct kvm_regs`.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
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

/// KVM segment descriptor, matching the kernel's `struct kvm_segment`.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
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
    padding: u8,
}

/// KVM descriptor table register, matching the kernel's `struct kvm_dtable`.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
struct KvmDtable {
    base: u64,
    limit: u16,
    padding: [u16; 3],
}

/// KVM special registers, matching the kernel's `struct kvm_sregs`.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
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
    interrupt_bitmap: [u64; 4],
}

impl Default for KvmSregs {
    fn default() -> Self {
        // Safety: all-zeros is valid for this repr(C) struct.
        unsafe { core::mem::zeroed() }
    }
}

/// Memory region descriptor for `KVM_SET_USER_MEMORY_REGION`.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
struct KvmUserspaceMemoryRegion {
    slot: u32,
    flags: u32,
    guest_phys_addr: u64,
    memory_size: u64,
    userspace_addr: u64,
}

/// Interrupt injection for `KVM_INTERRUPT`.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
struct KvmInterrupt {
    irq: u32,
}

// ════════════════════════════════════════════════════════════════════════
// kvm_run offsets (for reading fields from the mmap'd shared page)
//
// We access kvm_run fields at known offsets rather than defining the
// entire union-heavy struct. This avoids complex repr(C) unions while
// remaining correct per the stable kernel ABI.
// ════════════════════════════════════════════════════════════════════════

/// Offset of `request_interrupt_window` (u8) in kvm_run.
const RUN_REQUEST_INTERRUPT_WINDOW: usize = 0;
/// Offset of `immediate_exit` (u8) in kvm_run.
const RUN_IMMEDIATE_EXIT: usize = 1;
/// Offset of `exit_reason` (u32) in kvm_run.
const RUN_EXIT_REASON: usize = 32;
/// Offset of `ready_for_interrupt_injection` (u8) in kvm_run (after if_flag).
const RUN_IF_FLAG: usize = 40;
/// Offset of the exit data union in kvm_run.
const RUN_EXIT_DATA: usize = 48;

// IO exit sub-offsets (relative to RUN_EXIT_DATA):
//   direction: u8, size: u8, port: u16, count: u32, data_offset: u64
const IO_DIRECTION: usize = 0;
const IO_SIZE: usize = 1;
const IO_PORT: usize = 2;
const IO_COUNT: usize = 4;
const IO_DATA_OFFSET: usize = 8;

// MMIO exit sub-offsets (relative to RUN_EXIT_DATA):
//   phys_addr: u64, data: [u8; 8], len: u32, is_write: u8
const MMIO_PHYS_ADDR: usize = 0;
const MMIO_DATA: usize = 8;
const MMIO_LEN: usize = 16;
const MMIO_IS_WRITE: usize = 20;

/// KVM memory region flag: mark region as read-only.
const KVM_MEM_READONLY: u32 = 1 << 1;

// ════════════════════════════════════════════════════════════════════════
// KvmBackend
// ════════════════════════════════════════════════════════════════════════

/// KVM-based hypervisor backend for Linux hosts.
pub struct KvmBackend {
    /// File descriptor for `/dev/kvm`.
    kvm_fd: i32,
    /// File descriptor for the VM, obtained from `KVM_CREATE_VM`.
    vm_fd: i32,
    /// File descriptor for the vCPU, obtained from `KVM_CREATE_VCPU`.
    vcpu_fd: i32,
    /// Pointer to the mmap'd `kvm_run` shared page.
    kvm_run: *mut u8,
    /// Size of the mmap'd kvm_run region.
    kvm_run_size: usize,
    /// Whether a stop has been requested for the current run loop.
    stop_requested: bool,
}

// Safety: The raw pointers and file descriptors are only accessed through
// &mut self methods (which require exclusive access), except for `kvm_run`
// reads in `&self` methods which are safe because KVM does not concurrently
// modify the run struct while the vCPU is not running.
unsafe impl Send for KvmBackend {}

/// Path to the KVM device node.
const KVM_DEV_PATH: &[u8] = b"/dev/kvm\0";

/// Expected KVM API version.
const KVM_API_VERSION: i32 = 12;

impl KvmBackend {
    /// Open `/dev/kvm` and verify the API version.
    pub fn new() -> Result<Self, HvError> {
        let kvm_fd = unsafe { open(KVM_DEV_PATH.as_ptr(), O_RDWR) };
        if kvm_fd < 0 {
            return Err(HvError::SystemError(errno()));
        }

        let version = unsafe { ioctl(kvm_fd, KVM_GET_API_VERSION) };
        if version != KVM_API_VERSION {
            unsafe { close(kvm_fd); }
            return Err(HvError::NotSupported);
        }

        Ok(KvmBackend {
            kvm_fd,
            vm_fd: -1,
            vcpu_fd: -1,
            kvm_run: ptr::null_mut(),
            kvm_run_size: 0,
            stop_requested: false,
        })
    }

    /// Read a value of type `T` from the kvm_run page at the given byte offset.
    ///
    /// # Safety
    ///
    /// The caller must ensure `offset + size_of::<T>()` is within `kvm_run_size`
    /// and that `T` is a plain data type valid for any bit pattern.
    unsafe fn run_read<T: Copy>(&self, offset: usize) -> T {
        let ptr = self.kvm_run.add(offset) as *const T;
        ptr::read_unaligned(ptr)
    }

    /// Write a value of type `T` to the kvm_run page at the given byte offset.
    ///
    /// # Safety
    ///
    /// Same requirements as `run_read`.
    unsafe fn run_write<T: Copy>(&mut self, offset: usize, val: T) {
        let ptr = self.kvm_run.add(offset) as *mut T;
        ptr::write_unaligned(ptr, val);
    }

    /// Convert a [`KvmSegment`] to the backend-agnostic [`SegmentState`].
    fn kvm_segment_to_state(seg: &KvmSegment) -> SegmentState {
        // Pack access rights in the same format as VMX uses:
        //   bits 3:0  = type
        //   bit  4    = S (descriptor type)
        //   bits 6:5  = DPL
        //   bit  7    = Present
        //   bit  12   = AVL
        //   bit  13   = L (long mode)
        //   bit  14   = D/B
        //   bit  15   = G
        //   bit  16   = Unusable
        let ar = (seg.type_ as u32)
            | ((seg.s as u32) << 4)
            | ((seg.dpl as u32) << 5)
            | ((seg.present as u32) << 7)
            | ((seg.avl as u32) << 12)
            | ((seg.l as u32) << 13)
            | ((seg.db as u32) << 14)
            | ((seg.g as u32) << 15)
            | ((seg.unusable as u32) << 16);
        SegmentState {
            selector: seg.selector,
            base: seg.base,
            limit: seg.limit,
            access_rights: ar,
        }
    }

    /// Convert a [`SegmentState`] to a [`KvmSegment`].
    fn state_to_kvm_segment(state: &SegmentState) -> KvmSegment {
        let ar = state.access_rights;
        KvmSegment {
            base: state.base,
            limit: state.limit,
            selector: state.selector,
            type_: (ar & 0xF) as u8,
            s: ((ar >> 4) & 1) as u8,
            dpl: ((ar >> 5) & 3) as u8,
            present: ((ar >> 7) & 1) as u8,
            avl: ((ar >> 12) & 1) as u8,
            l: ((ar >> 13) & 1) as u8,
            db: ((ar >> 14) & 1) as u8,
            g: ((ar >> 15) & 1) as u8,
            unusable: ((ar >> 16) & 1) as u8,
            padding: 0,
        }
    }
}

impl HypervisorBackend for KvmBackend {
    fn create_vm(&mut self) -> Result<(), HvError> {
        let vm_fd = unsafe { ioctl(self.kvm_fd, KVM_CREATE_VM, 0i32) };
        if vm_fd < 0 {
            return Err(HvError::SystemError(errno()));
        }
        self.vm_fd = vm_fd;
        Ok(())
    }

    fn create_vcpu(&mut self, vcpu_id: u32) -> Result<(), HvError> {
        if self.vm_fd < 0 {
            return Err(HvError::Other("VM not created"));
        }

        let vcpu_fd = unsafe { ioctl(self.vm_fd, KVM_CREATE_VCPU, vcpu_id as i32) };
        if vcpu_fd < 0 {
            return Err(HvError::SystemError(errno()));
        }

        // Query the required mmap size for kvm_run.
        let mmap_size = unsafe { ioctl(self.kvm_fd, KVM_GET_VCPU_MMAP_SIZE) };
        if mmap_size <= 0 {
            unsafe { close(vcpu_fd); }
            return Err(HvError::SystemError(errno()));
        }
        let mmap_size = mmap_size as usize;

        // Map the kvm_run shared page.
        let run_ptr = unsafe {
            mmap(
                ptr::null_mut(),
                mmap_size,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                vcpu_fd,
                0,
            )
        };
        if run_ptr == MAP_FAILED {
            unsafe { close(vcpu_fd); }
            return Err(HvError::MemoryError);
        }

        self.vcpu_fd = vcpu_fd;
        self.kvm_run = run_ptr;
        self.kvm_run_size = mmap_size;
        Ok(())
    }

    fn map_memory(&mut self, region: &MemoryRegion) -> Result<(), HvError> {
        if self.vm_fd < 0 {
            return Err(HvError::Other("VM not created"));
        }

        let mut flags: u32 = 0;
        if region.readonly {
            flags |= KVM_MEM_READONLY;
        }

        let mem_region = KvmUserspaceMemoryRegion {
            slot: region.slot,
            flags,
            guest_phys_addr: region.guest_phys_addr,
            memory_size: region.memory_size,
            userspace_addr: region.userspace_addr as u64,
        };

        let ret = unsafe {
            ioctl(
                self.vm_fd,
                KVM_SET_USER_MEMORY_REGION,
                &mem_region as *const KvmUserspaceMemoryRegion,
            )
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }
        Ok(())
    }

    fn unmap_memory(&mut self, slot: u32) -> Result<(), HvError> {
        // To unmap, set memory_size to 0 for the given slot.
        let mem_region = KvmUserspaceMemoryRegion {
            slot,
            flags: 0,
            guest_phys_addr: 0,
            memory_size: 0,
            userspace_addr: 0,
        };

        let ret = unsafe {
            ioctl(
                self.vm_fd,
                KVM_SET_USER_MEMORY_REGION,
                &mem_region as *const KvmUserspaceMemoryRegion,
            )
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }
        Ok(())
    }

    fn get_regs(&self, vcpu_id: u32) -> Result<VcpuRegs, HvError> {
        if self.vcpu_fd < 0 {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id; // Single vCPU design — validated by having a valid vcpu_fd.

        let mut kregs = KvmRegs::default();
        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_GET_REGS, &mut kregs as *mut KvmRegs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }

        let mut ksregs = KvmSregs::default();
        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_GET_SREGS, &mut ksregs as *mut KvmSregs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }

        Ok(VcpuRegs {
            rax: kregs.rax,
            rcx: kregs.rcx,
            rdx: kregs.rdx,
            rbx: kregs.rbx,
            rsp: kregs.rsp,
            rbp: kregs.rbp,
            rsi: kregs.rsi,
            rdi: kregs.rdi,
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
            cs: Self::kvm_segment_to_state(&ksregs.cs),
            ds: Self::kvm_segment_to_state(&ksregs.ds),
            es: Self::kvm_segment_to_state(&ksregs.es),
            fs: Self::kvm_segment_to_state(&ksregs.fs),
            gs: Self::kvm_segment_to_state(&ksregs.gs),
            ss: Self::kvm_segment_to_state(&ksregs.ss),
            tr: Self::kvm_segment_to_state(&ksregs.tr),
            ldtr: Self::kvm_segment_to_state(&ksregs.ldt),
            gdtr: DtableState {
                base: ksregs.gdt.base,
                limit: ksregs.gdt.limit,
            },
            idtr: DtableState {
                base: ksregs.idt.base,
                limit: ksregs.idt.limit,
            },
            cr0: ksregs.cr0,
            cr2: ksregs.cr2,
            cr3: ksregs.cr3,
            cr4: ksregs.cr4,
            cr8: ksregs.cr8,
            efer: ksregs.efer,
            apic_base: ksregs.apic_base,
            pat: 0,
            sysenter_cs: 0,
            sysenter_esp: 0,
            sysenter_eip: 0,
            star: 0,
            lstar: 0,
            cstar: 0,
            sfmask: 0,
            kernel_gs_base: 0,
            tsc_aux: 0,
        })
    }

    fn set_regs(&mut self, vcpu_id: u32, regs: &VcpuRegs) -> Result<(), HvError> {
        if self.vcpu_fd < 0 {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id;

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
            ioctl(self.vcpu_fd, KVM_SET_REGS, &kregs as *const KvmRegs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }

        // Read current sregs, then update the fields we manage.
        let mut ksregs = KvmSregs::default();
        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_GET_SREGS, &mut ksregs as *mut KvmSregs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }

        ksregs.cs = Self::state_to_kvm_segment(&regs.cs);
        ksregs.ds = Self::state_to_kvm_segment(&regs.ds);
        ksregs.es = Self::state_to_kvm_segment(&regs.es);
        ksregs.fs = Self::state_to_kvm_segment(&regs.fs);
        ksregs.gs = Self::state_to_kvm_segment(&regs.gs);
        ksregs.ss = Self::state_to_kvm_segment(&regs.ss);
        ksregs.tr = Self::state_to_kvm_segment(&regs.tr);
        ksregs.ldt = Self::state_to_kvm_segment(&regs.ldtr);
        ksregs.gdt = KvmDtable {
            base: regs.gdtr.base,
            limit: regs.gdtr.limit,
            padding: [0; 3],
        };
        ksregs.idt = KvmDtable {
            base: regs.idtr.base,
            limit: regs.idtr.limit,
            padding: [0; 3],
        };
        ksregs.cr0 = regs.cr0;
        ksregs.cr2 = regs.cr2;
        ksregs.cr3 = regs.cr3;
        ksregs.cr4 = regs.cr4;
        ksregs.cr8 = regs.cr8;
        ksregs.efer = regs.efer;
        ksregs.apic_base = regs.apic_base;

        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_SET_SREGS, &ksregs as *const KvmSregs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }

        Ok(())
    }

    fn run(&mut self, vcpu_id: u32) -> Result<VmExit, HvError> {
        if self.vcpu_fd < 0 {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id;

        if self.stop_requested {
            self.stop_requested = false;
            return Ok(VmExit::StopRequested);
        }

        let ret = unsafe { ioctl(self.vcpu_fd, KVM_RUN, 0i32) };
        if ret < 0 {
            let err = errno();
            // EINTR (4) means the ioctl was interrupted — treat as a stop.
            if err == 4 {
                return Ok(VmExit::StopRequested);
            }
            return Err(HvError::SystemError(err));
        }

        let exit_reason: u32 = unsafe { self.run_read(RUN_EXIT_REASON) };

        match exit_reason {
            KVM_EXIT_IO => {
                let direction: u8 = unsafe { self.run_read(RUN_EXIT_DATA + IO_DIRECTION) };
                let size: u8 = unsafe { self.run_read(RUN_EXIT_DATA + IO_SIZE) };
                let port: u16 = unsafe { self.run_read(RUN_EXIT_DATA + IO_PORT) };
                let data_offset: u64 = unsafe { self.run_read(RUN_EXIT_DATA + IO_DATA_OFFSET) };

                if direction == KVM_EXIT_IO_OUT {
                    // Read the data the guest wrote from the kvm_run page.
                    let data: u32 = match size {
                        1 => unsafe { self.run_read::<u8>(data_offset as usize) as u32 },
                        2 => unsafe { self.run_read::<u16>(data_offset as usize) as u32 },
                        4 => unsafe { self.run_read::<u32>(data_offset as usize) },
                        _ => 0,
                    };
                    Ok(VmExit::IoOut { port, size, data })
                } else {
                    Ok(VmExit::IoIn { port, size })
                }
            }
            KVM_EXIT_MMIO => {
                let phys_addr: u64 = unsafe { self.run_read(RUN_EXIT_DATA + MMIO_PHYS_ADDR) };
                let len: u32 = unsafe { self.run_read(RUN_EXIT_DATA + MMIO_LEN) };
                let is_write: u8 = unsafe { self.run_read(RUN_EXIT_DATA + MMIO_IS_WRITE) };
                let size = len as u8;

                if is_write != 0 {
                    // Read the data from the mmio data field.
                    let mut data: u64 = 0;
                    unsafe {
                        ptr::copy_nonoverlapping(
                            self.kvm_run.add(RUN_EXIT_DATA + MMIO_DATA),
                            &mut data as *mut u64 as *mut u8,
                            core::cmp::min(len as usize, 8),
                        );
                    }
                    Ok(VmExit::MmioWrite { address: phys_addr, size, data })
                } else {
                    Ok(VmExit::MmioRead { address: phys_addr, size })
                }
            }
            KVM_EXIT_HLT => Ok(VmExit::Hlt),
            KVM_EXIT_SHUTDOWN => Ok(VmExit::Shutdown),
            KVM_EXIT_INTERNAL_ERROR => Ok(VmExit::Shutdown),
            _ => Ok(VmExit::Unknown(exit_reason)),
        }
    }

    fn request_interrupt_window(&mut self, vcpu_id: u32) -> Result<(), HvError> {
        if self.kvm_run.is_null() {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id;
        unsafe { self.run_write::<u8>(RUN_REQUEST_INTERRUPT_WINDOW, 1); }
        Ok(())
    }

    fn inject_interrupt(&mut self, vcpu_id: u32, vector: u8) -> Result<(), HvError> {
        if self.vcpu_fd < 0 {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id;

        let irq = KvmInterrupt { irq: vector as u32 };
        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_INTERRUPT, &irq as *const KvmInterrupt)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }
        // Clear the interrupt window request after successful injection.
        unsafe { self.run_write::<u8>(RUN_REQUEST_INTERRUPT_WINDOW, 0); }
        Ok(())
    }

    fn interrupts_enabled(&self, vcpu_id: u32) -> Result<bool, HvError> {
        if self.kvm_run.is_null() {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id;
        // The `if_flag` field in kvm_run indicates whether IF is set and no
        // interrupt shadow is active.
        let if_flag: u8 = unsafe { self.run_read(RUN_IF_FLAG) };
        Ok(if_flag != 0)
    }

    fn set_io_in_data(&mut self, vcpu_id: u32, data: u32, size: u8) -> Result<(), HvError> {
        if self.kvm_run.is_null() {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id;

        // The data offset for IO exits is stored in the kvm_run IO exit data.
        let data_offset: u64 = unsafe { self.run_read(RUN_EXIT_DATA + IO_DATA_OFFSET) };

        match size {
            1 => unsafe { self.run_write(data_offset as usize, data as u8) },
            2 => unsafe { self.run_write(data_offset as usize, data as u16) },
            4 => unsafe { self.run_write(data_offset as usize, data) },
            _ => return Err(HvError::Other("invalid IO size")),
        }
        Ok(())
    }

    fn set_mmio_read_data(&mut self, vcpu_id: u32, data: u64, size: u8) -> Result<(), HvError> {
        if self.kvm_run.is_null() {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id;

        // Write the data into the mmio.data field of kvm_run.
        let len = core::cmp::min(size as usize, 8);
        unsafe {
            ptr::copy_nonoverlapping(
                &data as *const u64 as *const u8,
                self.kvm_run.add(RUN_EXIT_DATA + MMIO_DATA),
                len,
            );
        }
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
        if self.vcpu_fd < 0 {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id;

        // KVM handles CPUID internally via KVM_SET_CPUID2; when we get a
        // CPUID exit from another backend we need to stuff the result into
        // the guest registers directly.
        let mut kregs = KvmRegs::default();
        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_GET_REGS, &mut kregs as *mut KvmRegs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }

        kregs.rax = eax as u64;
        kregs.rbx = ebx as u64;
        kregs.rcx = ecx as u64;
        kregs.rdx = edx as u64;

        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_SET_REGS, &kregs as *const KvmRegs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }
        Ok(())
    }

    fn set_msr_read_data(&mut self, vcpu_id: u32, value: u64) -> Result<(), HvError> {
        if self.vcpu_fd < 0 {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id;

        // Place the MSR value into RAX:RDX (low 32 in RAX, high 32 in RDX).
        let mut kregs = KvmRegs::default();
        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_GET_REGS, &mut kregs as *mut KvmRegs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }

        kregs.rax = value & 0xFFFF_FFFF;
        kregs.rdx = value >> 32;

        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_SET_REGS, &kregs as *const KvmRegs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }
        Ok(())
    }

    fn advance_rip(&mut self, vcpu_id: u32, len: u8) -> Result<(), HvError> {
        if self.vcpu_fd < 0 {
            return Err(HvError::InvalidVcpu);
        }
        let _ = vcpu_id;

        let mut kregs = KvmRegs::default();
        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_GET_REGS, &mut kregs as *mut KvmRegs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }

        kregs.rip = kregs.rip.wrapping_add(len as u64);

        let ret = unsafe {
            ioctl(self.vcpu_fd, KVM_SET_REGS, &kregs as *const KvmRegs)
        };
        if ret < 0 {
            return Err(HvError::SystemError(errno()));
        }
        Ok(())
    }

    fn request_stop(&mut self, vcpu_id: u32) -> Result<(), HvError> {
        let _ = vcpu_id;
        self.stop_requested = true;
        // Set immediate_exit so the next KVM_RUN returns immediately.
        if !self.kvm_run.is_null() {
            unsafe { self.run_write::<u8>(RUN_IMMEDIATE_EXIT, 1); }
        }
        Ok(())
    }

    fn destroy(&mut self) {
        if !self.kvm_run.is_null() {
            unsafe { munmap(self.kvm_run, self.kvm_run_size); }
            self.kvm_run = ptr::null_mut();
            self.kvm_run_size = 0;
        }
        if self.vcpu_fd >= 0 {
            unsafe { close(self.vcpu_fd); }
            self.vcpu_fd = -1;
        }
        if self.vm_fd >= 0 {
            unsafe { close(self.vm_fd); }
            self.vm_fd = -1;
        }
        if self.kvm_fd >= 0 {
            unsafe { close(self.kvm_fd); }
            self.kvm_fd = -1;
        }
    }
}

impl Drop for KvmBackend {
    fn drop(&mut self) {
        self.destroy();
    }
}
