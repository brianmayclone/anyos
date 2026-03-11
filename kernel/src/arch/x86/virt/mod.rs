//! Hardware virtualization support (Intel VT-x / AMD-V).
//!
//! Provides VMX and SVM backends for the anyOS userspace VM daemon.
//! Feature detection at boot, per-CPU initialization, and VM/vCPU lifecycle.

pub mod ept;
pub mod svm;
pub mod syscalls;
pub mod vmx;

/// Type of hardware virtualization available on this CPU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtType {
    None,
    Vmx,
    Svm,
}

static mut VIRT_TYPE: VirtType = VirtType::None;

/// Called at boot after CPUID detection. Detects VMX or SVM and performs
/// global (one-time) initialization.
pub fn init() {
    let features = crate::arch::x86::cpuid::features();
    if features.vmx {
        unsafe {
            VIRT_TYPE = VirtType::Vmx;
        }
        vmx::global_init();
        crate::serial_println!("[OK] Hardware virtualization: Intel VT-x detected");
    } else if features.svm {
        unsafe {
            VIRT_TYPE = VirtType::Svm;
        }
        svm::global_init();
        crate::serial_println!("[OK] Hardware virtualization: AMD-V (SVM) detected");
    } else {
        crate::serial_println!("[  ] Hardware virtualization: not available");
    }
}

/// Called per-CPU during SMP startup. Enables VMX/SVM on this core.
pub fn per_cpu_init() {
    match virt_type() {
        VirtType::Vmx => vmx::per_cpu_init(),
        VirtType::Svm => svm::per_cpu_init(),
        VirtType::None => {}
    }
}

/// Returns the detected virtualization type.
pub fn virt_type() -> VirtType {
    unsafe { VIRT_TYPE }
}

/// Returns `true` if hardware virtualization is available.
pub fn is_available() -> bool {
    !matches!(virt_type(), VirtType::None)
}

// ── MSR helpers ──────────────────────────────────────────────────────────

#[inline]
pub(crate) unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") lo,
        out("edx") hi,
        options(nostack, nomem, preserves_flags),
    );
    ((hi as u64) << 32) | (lo as u64)
}

#[inline]
pub(crate) unsafe fn wrmsr(msr: u32, value: u64) {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") lo,
        in("edx") hi,
        options(nostack, nomem, preserves_flags),
    );
}

// ── CR helpers ───────────────────────────────────────────────────────────

#[inline]
pub(crate) unsafe fn read_cr0() -> u64 {
    let val: u64;
    core::arch::asm!("mov {}, cr0", out(reg) val, options(nostack, nomem, preserves_flags));
    val
}

#[inline]
pub(crate) unsafe fn read_cr3() -> u64 {
    let val: u64;
    core::arch::asm!("mov {}, cr3", out(reg) val, options(nostack, nomem, preserves_flags));
    val
}

#[inline]
pub(crate) unsafe fn read_cr4() -> u64 {
    let val: u64;
    core::arch::asm!("mov {}, cr4", out(reg) val, options(nostack, nomem, preserves_flags));
    val
}

#[inline]
pub(crate) unsafe fn write_cr4(val: u64) {
    core::arch::asm!("mov cr4, {}", in(reg) val, options(nostack, nomem, preserves_flags));
}

// ── Shared types ─────────────────────────────────────────────────────────

/// CPUID entry for guest CPUID emulation.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct CpuidEntry {
    pub function: u32,
    pub index: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
}

/// Memory region mapping guest physical addresses to host physical addresses.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MemoryRegion {
    pub slot: u32,
    pub guest_phys: u64,
    pub size: u64,
    pub host_phys: u64,
}

/// Information returned to userspace after a VM-exit.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct VmExitInfo {
    /// Exit reason code (VMX or SVM exit reason).
    pub reason: u32,
    /// Qualification / exit_info1 (I/O port, EPT violation details, etc.).
    pub qualification: u64,
    /// Guest physical address (for EPT/NPT violations).
    pub guest_phys_addr: u64,
    /// Instruction length (for instruction-based exits like CPUID, I/O).
    pub instruction_len: u32,
    /// I/O port number (for I/O exits).
    pub io_port: u16,
    /// Access size in bytes (1, 2, 4, 8).
    pub access_size: u8,
    /// Direction: 0 = out/write, 1 = in/read.
    pub is_read: u8,
    /// Data value (for OUT/MMIO write: guest-written value; for CPUID: EAX input).
    pub io_data: u64,
    /// Secondary data (for CPUID: ECX index; for WRMSR: value).
    pub io_data2: u64,
    /// MSR index (for RDMSR/WRMSR exits).
    pub msr_index: u32,
    /// CPUID function (EAX) for CPUID exits.
    pub cpuid_function: u32,
    /// CPUID index (ECX) for CPUID exits.
    pub cpuid_index: u32,
    pub _pad: u32,
}

/// Guest general-purpose registers saved/restored across VM-entry/exit.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct GuestGprs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
}

/// Guest segment register state (for get/set_sregs).
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct GuestSregs {
    pub cs_selector: u16,
    pub cs_base: u64,
    pub cs_limit: u32,
    pub cs_ar: u32,
    pub ds_selector: u16,
    pub ds_base: u64,
    pub ds_limit: u32,
    pub ds_ar: u32,
    pub es_selector: u16,
    pub es_base: u64,
    pub es_limit: u32,
    pub es_ar: u32,
    pub fs_selector: u16,
    pub fs_base: u64,
    pub fs_limit: u32,
    pub fs_ar: u32,
    pub gs_selector: u16,
    pub gs_base: u64,
    pub gs_limit: u32,
    pub gs_ar: u32,
    pub ss_selector: u16,
    pub ss_base: u64,
    pub ss_limit: u32,
    pub ss_ar: u32,
    pub tr_selector: u16,
    pub tr_base: u64,
    pub tr_limit: u32,
    pub tr_ar: u32,
    pub ldtr_selector: u16,
    pub ldtr_base: u64,
    pub ldtr_limit: u32,
    pub ldtr_ar: u32,
    pub gdtr_base: u64,
    pub gdtr_limit: u32,
    pub idtr_base: u64,
    pub idtr_limit: u32,
    pub cr0: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub efer: u64,
    pub rip: u64,
    pub rsp: u64,
    pub rflags: u64,
}

// ── Page allocation helper ───────────────────────────────────────────────

/// Allocate a zeroed 4KB-aligned page suitable for VMCS/VMCB/page tables.
/// Returns the physical address. The page is accessible via identity mapping
/// (phys < 128 MiB).
pub(crate) fn alloc_page_zeroed() -> Option<u64> {
    let phys = crate::memory::physical::alloc_frame()?;
    let ptr = phys.as_u64() as *mut u8;
    unsafe {
        core::ptr::write_bytes(ptr, 0, 4096);
    }
    Some(phys.as_u64())
}

/// Free a page previously allocated with `alloc_page_zeroed`.
pub(crate) fn free_page(phys: u64) {
    crate::memory::physical::free_frame(crate::memory::address::PhysAddr::new(phys));
}
