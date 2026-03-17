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

// ── Portable exit reason codes ────────────────────────────────────────────
//
// These are the normalized exit reasons reported to userspace in VmExitInfo.reason.
// The kernel maps hardware-specific VMX/SVM codes to these values so that a
// userspace VMM can be written against a single ABI regardless of CPU vendor.
//
// Values 0-63 mirror the VMX exit-reason numbering (Intel SDM Vol. 3C §27.2)
// where applicable, because VMX is more fine-grained.  SVM-specific or
// synthetic reasons start at 0x100.

pub mod exit_reason {
    /// External interrupt (host interrupt while in guest — requires NMI-window or APIC).
    pub const EXTERNAL_INTERRUPT: u32 = 1;
    /// Guest triple-fault.
    pub const TRIPLE_FAULT: u32 = 2;
    /// INIT signal (AP startup, before SIPI).
    pub const INIT_SIGNAL: u32 = 3;
    /// SIPI signal (startup IPI, carries vector in qualification).
    pub const SIPI: u32 = 4;
    /// CPUID instruction.
    pub const CPUID: u32 = 10;
    /// HLT instruction.
    pub const HLT: u32 = 12;
    /// INVD instruction.
    pub const INVD: u32 = 13;
    /// INVLPG instruction.
    pub const INVLPG: u32 = 14;
    /// RDPMC instruction.
    pub const RDPMC: u32 = 15;
    /// RDTSC instruction.
    pub const RDTSC: u32 = 16;
    /// RSM instruction (resume from SMM).
    pub const RSM: u32 = 17;
    /// VMCALL / VMMCALL — hypercall from guest.
    pub const VMCALL: u32 = 18;
    /// Control-register access (CR read/write).
    pub const CR_ACCESS: u32 = 28;
    /// Debug-register access (DR read/write).
    pub const DR_ACCESS: u32 = 29;
    /// I/O instruction (IN/OUT).
    pub const IO_INSTRUCTION: u32 = 30;
    /// RDMSR instruction.
    pub const RDMSR: u32 = 31;
    /// WRMSR instruction.
    pub const WRMSR: u32 = 32;
    /// VM-entry failure due to invalid guest state.
    pub const INVALID_GUEST_STATE: u32 = 33;
    /// PAUSE instruction (spin-loop hint).
    pub const PAUSE: u32 = 40;
    /// EPT / NPT page-not-present or permission violation (MMIO or missing mapping).
    pub const EPT_VIOLATION: u32 = 48;
    /// EPT / NPT misconfiguration.
    pub const EPT_MISCONFIG: u32 = 49;
    /// RDTSCP instruction.
    pub const RDTSCP: u32 = 51;
    /// Preemption timer expiry (VMX only; mapped to SYNTHETIC_PREEMPTION_TIMER for SVM).
    pub const PREEMPTION_TIMER: u32 = 52;
    /// WBINVD instruction.
    pub const WBINVD: u32 = 54;
    /// XSETBV instruction.
    pub const XSETBV: u32 = 55;
    /// RDRAND instruction.
    pub const RDRAND: u32 = 57;
    /// INVPCID instruction.
    pub const INVPCID: u32 = 58;
    /// RDSEED instruction.
    pub const RDSEED: u32 = 61;
    // ── Synthetic codes (0x100+) ──────────────────────────────────────────
    /// Shutdown / triple-fault (SVM 0x7F, mapped here for portability).
    pub const SHUTDOWN: u32 = 0x100;
    /// SMI (System Management Interrupt).
    pub const SMI: u32 = 0x101;
    /// NMI window — NMI can now be injected.
    pub const NMI_WINDOW: u32 = 0x102;
    /// Interrupt window — external interrupt can now be injected.
    pub const IRQ_WINDOW: u32 = 0x103;
    /// Internal CPUID exit was handled by kernel; userspace receives this as a
    /// non-actionable notification (can be ignored).
    pub const CPUID_EMULATED: u32 = 0x104;
    /// Internal HLT handled by kernel (vCPU now in Halted MP state).
    pub const HLT_EMULATED: u32 = 0x105;
}

/// Information returned to userspace after a VM-exit.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct VmExitInfo {
    /// Normalized exit reason code (see `exit_reason::*` constants).
    /// This is the portable anyOS value, not the raw VMX/SVM code.
    pub reason: u32,
    /// Raw hardware exit reason (VMX exit-reason field or SVM EXITCODE).
    /// Userspace can use this for vendor-specific handling.
    pub hw_reason: u32,
    /// Qualification / exit_info1 (I/O port, EPT/CR access details, etc.).
    pub qualification: u64,
    /// Guest physical address (for EPT/NPT violations; 0 otherwise).
    pub guest_phys_addr: u64,
    /// Guest virtual address (for EPT/NPT violations where GLA is valid; 0 otherwise).
    pub guest_virt_addr: u64,
    /// Instruction length (for instruction-based exits like CPUID, I/O, MSR).
    pub instruction_len: u32,
    /// I/O port number (for IO_INSTRUCTION exits).
    pub io_port: u16,
    /// Access size in bytes: 1, 2, or 4 for I/O; 1/2/4/8 for MMIO.
    pub access_size: u8,
    /// Direction: 0 = out/write, 1 = in/read.
    pub is_read: u8,
    /// Data value:
    ///   - OUT / MMIO write: value written by guest.
    ///   - RDMSR: EAX part of MSR value (low 32 bits).
    ///   - CPUID exit: input EAX.
    pub io_data: u64,
    /// Secondary data:
    ///   - WRMSR: full MSR value (EDX:EAX).
    ///   - CPUID: input ECX (subleaf).
    pub io_data2: u64,
    /// MSR index for RDMSR / WRMSR exits.
    pub msr_index: u32,
    /// CPUID function (EAX) for CPUID exits.
    pub cpuid_function: u32,
    /// CPUID index (ECX) for CPUID exits.
    pub cpuid_index: u32,
    /// For CR_ACCESS exits: CR number (0/3/4/8).
    pub cr_number: u8,
    /// For CR_ACCESS exits: 0=write, 1=read.
    pub cr_is_read: u8,
    /// For DR_ACCESS exits: DR number (0-7).
    pub dr_number: u8,
    /// For DR_ACCESS exits: 0=write, 1=read.
    pub dr_is_read: u8,
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

/// Guest FPU/SSE/AVX state in FXSAVE layout (Intel SDM Vol. 1 §13.4).
/// The 512-byte region is exactly what FXSAVE/FXRSTOR expects:
///   offset  0: FCW, FSW, FTW, FOP, FPU IP, FPU CS, FPU DP, FPU DS
///   offset 32: MXCSRr, MXCSR_MASK
///   offset 32+8: ST0–ST7 (80-bit, padded to 16 bytes each)
///   offset 160: XMM0–XMM15 (128-bit each)
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub struct GuestFpuState {
    pub data: [u8; 512],
}

impl Default for GuestFpuState {
    fn default() -> Self {
        // FPU control word = 0x037F (all exceptions masked, 64-bit precision)
        // MXCSR = 0x1F80 (all SSE exceptions masked, round-nearest)
        let mut d = [0u8; 512];
        d[0] = 0x7F; d[1] = 0x03; // FCW
        d[24] = 0x80; d[25] = 0x1F; // MXCSR low word
        d[28] = 0xFF; d[29] = 0xFF; // MXCSR_MASK (all bits valid)
        Self { data: d }
    }
}

impl core::fmt::Debug for GuestFpuState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "GuestFpuState {{ fcw: {:#06x}, mxcsr: {:#010x} }}",
            u16::from_le_bytes([self.data[0], self.data[1]]),
            u32::from_le_bytes([self.data[24], self.data[25], self.data[26], self.data[27]]))
    }
}

/// Multi-processor state of a vCPU (mirrors KVM_MP_STATE_* values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u32)]
pub enum VcpuMpState {
    /// vCPU is running (executing guest code or waiting for an interrupt).
    #[default]
    Runnable = 0,
    /// vCPU has never been initialized (waiting for INIT-SIPI sequence).
    Uninitialized = 1,
    /// vCPU executed HLT and is halted until an interrupt arrives.
    Halted = 2,
    /// vCPU received INIT and is waiting for SIPI.
    InitReceived = 3,
}

impl VcpuMpState {
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Runnable),
            1 => Some(Self::Uninitialized),
            2 => Some(Self::Halted),
            3 => Some(Self::InitReceived),
            _ => None,
        }
    }
}

/// Request structure for SYS_VCPU_TRANSLATE — passed from userspace.
#[derive(Debug, Clone, Copy, Default)]
#[repr(C)]
pub struct TranslateRequest {
    /// Input: guest virtual address to translate.
    pub gva: u64,
    /// Output: corresponding guest physical address (valid only if `valid != 0`).
    pub out_gpa: u64,
    /// Output: 1 if the translation succeeded, 0 if the GVA is not mapped.
    pub out_valid: u32,
    pub _pad: u32,
}

/// Dirty-log descriptor — passed from userspace for SYS_VM_GET_DIRTY_LOG.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct DirtyLogRequest {
    /// Memory slot index (must match a slot registered with SYS_VM_SET_MEMORY).
    pub slot: u32,
    pub _pad: u32,
    /// Pointer to a caller-allocated bitmap (1 bit per 4KB page, LSB-first).
    /// Must be large enough to hold `ceil(region_pages / 8)` bytes.
    pub bitmap_ptr: u64,
    /// Size of the bitmap in bytes (kernel validates against actual region size).
    pub bitmap_size: u64,
}

// ── Page allocation helper ───────────────────────────────────────────────

// ── Phys→Virt lookup table for virt-subsystem pages ─────────────────────
//
// EPT/NPT page-table walkers store physical addresses in entries (required by
// hardware) but need virtual addresses for kernel pointer access.  We maintain
// a small static lookup table so `phys_to_virt(phys)` can resolve any page
// allocated by `alloc_page_zeroed`.
//
// Maximum entries: 64 VMs × ~16 NPT/EPT pages + VMCS/VMCB/bitmaps ≈ 2048.
// Each entry is 2 × u64 = 16 bytes → 32 KiB total — negligible.

const VPAGE_TABLE_SIZE: usize = 2048;

#[derive(Copy, Clone)]
struct VPageEntry {
    phys: u64,
    virt: u64,
}

static VPAGE_TABLE: crate::sync::spinlock::Spinlock<[VPageEntry; VPAGE_TABLE_SIZE]> =
    crate::sync::spinlock::Spinlock::new(
        [const { VPageEntry { phys: 0, virt: 0 } }; VPAGE_TABLE_SIZE]
    );

fn vpage_insert(phys: u64, virt: u64) {
    let mut tbl = VPAGE_TABLE.lock();
    for entry in tbl.iter_mut() {
        if entry.phys == 0 {
            entry.phys = phys;
            entry.virt = virt;
            return;
        }
    }
    // Table full — should never happen with VPAGE_TABLE_SIZE = 2048
    crate::serial_println!("[virt] WARNING: vpage table full, phys={:#x}", phys);
}

fn vpage_remove(phys: u64) {
    let mut tbl = VPAGE_TABLE.lock();
    for entry in tbl.iter_mut() {
        if entry.phys == phys {
            entry.phys = 0;
            entry.virt = 0;
            return;
        }
    }
}

/// Translate a physical address (allocated by `alloc_page_zeroed`) to its
/// kernel virtual address for pointer access.
pub(crate) fn phys_to_virt(phys: u64) -> u64 {
    let tbl = VPAGE_TABLE.lock();
    for entry in tbl.iter() {
        if entry.phys == phys {
            return entry.virt;
        }
    }
    // Fallback: identity mapping (for pages allocated before this mechanism
    // was in place, or pages in the first 128 MiB). If phys < 128 MiB, the
    // identity map makes phys == virt.
    phys
}

/// Allocate a zeroed 4KB-aligned page suitable for VMCS/VMCB/EPT/NPT tables.
/// Returns the physical address. Use `phys_to_virt(phys)` to get the kernel VA.
///
/// Allocates any physical frame, maps it into permanent kernel virtual address
/// space, zeros it via the virtual address, and registers the phys→virt mapping
/// in the lookup table so EPT/NPT walkers can dereference entries by phys.
pub(crate) fn alloc_page_zeroed() -> Option<u64> {
    let phys = crate::memory::physical::alloc_frame()?;
    let virt = crate::memory::virtual_mem::map_kernel_phys_page(phys)?;
    unsafe {
        core::ptr::write_bytes(virt.as_u64() as *mut u8, 0, 4096);
    }
    vpage_insert(phys.as_u64(), virt.as_u64());
    Some(phys.as_u64())
}

/// Free a page previously allocated with `alloc_page_zeroed`.
pub(crate) fn free_page(phys: u64) {
    vpage_remove(phys);
    crate::memory::physical::free_frame(crate::memory::address::PhysAddr::new(phys));
}
