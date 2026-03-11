//! Bare-metal hypervisor backend for anyOS.
//!
//! Implements [`HypervisorBackend`] using direct VMX (Intel VT-x) or SVM
//! (AMD-V) instructions via inline assembly. No OS system calls are used —
//! this runs directly on the hardware.
//!
//! Only compiled when `feature = "host_test"` is NOT set (bare-metal anyOS).

use super::{DtableState, HvError, HypervisorBackend, MemoryRegion, SegmentState, VcpuRegs, VmExit};
use super::vmx;
use super::svm;
use alloc::vec::Vec;

/// CPU vendor detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CpuVendor {
    Intel,
    Amd,
    Unknown,
}

/// Detect CPU vendor via CPUID leaf 0.
fn detect_vendor() -> CpuVendor {
    let (ebx, ecx, edx): (u32, u32, u32);
    unsafe {
        core::arch::asm!(
            "push rbx",
            "xor eax, eax",
            "cpuid",
            "mov {0:e}, ebx",
            "mov {1:e}, ecx",
            "mov {2:e}, edx",
            "pop rbx",
            out(reg) ebx,
            out(reg) ecx,
            out(reg) edx,
            out("eax") _,
            options(nostack),
        );
    }
    // "GenuineIntel" = EBX=756E6547 EDX=49656E69 ECX=6C65746E
    if ebx == 0x756E_6547 && edx == 0x4965_6E69 && ecx == 0x6C65_746E {
        CpuVendor::Intel
    }
    // "AuthenticAMD" = EBX=68747541 EDX=69746E65 ECX=444D4163
    else if ebx == 0x6874_7541 && edx == 0x6974_6E65 && ecx == 0x444D_4163 {
        CpuVendor::Amd
    } else {
        CpuVendor::Unknown
    }
}

/// Read an MSR.
#[inline]
unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") lo,
        out("edx") hi,
        options(nostack, nomem),
    );
    (hi as u64) << 32 | lo as u64
}

/// Write an MSR.
#[inline]
unsafe fn wrmsr(msr: u32, val: u64) {
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") val as u32,
        in("edx") (val >> 32) as u32,
        options(nostack, nomem),
    );
}

/// Read CR0.
#[inline]
unsafe fn read_cr0() -> u64 {
    let val: u64;
    core::arch::asm!("mov {}, cr0", out(reg) val, options(nostack, nomem));
    val
}

/// Read CR4.
#[inline]
unsafe fn read_cr4() -> u64 {
    let val: u64;
    core::arch::asm!("mov {}, cr4", out(reg) val, options(nostack, nomem));
    val
}

/// Write CR4.
#[inline]
unsafe fn write_cr4(val: u64) {
    core::arch::asm!("mov cr4, {}", in(reg) val, options(nostack, nomem));
}

/// Allocate a 4KB-aligned page of zeroed memory.
fn alloc_page() -> Vec<u8> {
    let layout = alloc::alloc::Layout::from_size_align(4096, 4096).unwrap();
    let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        panic!("failed to allocate 4KB aligned page");
    }
    unsafe { Vec::from_raw_parts(ptr, 4096, 4096) }
}

/// Allocate N pages of 4KB-aligned zeroed memory.
fn alloc_pages(n: usize) -> Vec<u8> {
    let size = n * 4096;
    let layout = alloc::alloc::Layout::from_size_align(size, 4096).unwrap();
    let ptr = unsafe { alloc::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        panic!("failed to allocate {} pages", n);
    }
    unsafe { Vec::from_raw_parts(ptr, size, size) }
}

/// Physical address of a page-aligned buffer.
#[inline]
fn phys_addr(buf: &[u8]) -> u64 {
    buf.as_ptr() as u64
}


// ─── VMX inline assembly wrappers ───────────────────────────────────────────

/// Execute VMXON with the physical address of the VMXON region.
#[inline]
unsafe fn vmxon(vmxon_phys: u64) -> bool {
    let flags: u64;
    core::arch::asm\!(
        "vmxon [{0}]",
        "pushfq",
        "pop {1}",
        in(reg) &vmxon_phys as *const u64,
        out(reg) flags,
        options(nostack),
    );
    // CF=0 and ZF=0 means success
    (flags & 0x41) == 0
}

/// Execute VMXOFF.
#[inline]
unsafe fn vmxoff() {
    core::arch::asm\!("vmxoff", options(nostack, nomem));
}

/// Execute VMCLEAR on a VMCS region.
#[inline]
unsafe fn vmclear(vmcs_phys: u64) -> bool {
    let flags: u64;
    core::arch::asm\!(
        "vmclear [{0}]",
        "pushfq",
        "pop {1}",
        in(reg) &vmcs_phys as *const u64,
        out(reg) flags,
        options(nostack),
    );
    (flags & 0x41) == 0
}

/// Execute VMPTRLD to load a VMCS.
#[inline]
unsafe fn vmptrld(vmcs_phys: u64) -> bool {
    let flags: u64;
    core::arch::asm\!(
        "vmptrld [{0}]",
        "pushfq",
        "pop {1}",
        in(reg) &vmcs_phys as *const u64,
        out(reg) flags,
        options(nostack),
    );
    (flags & 0x41) == 0
}

/// Write a field to the current VMCS.
#[inline]
unsafe fn vmwrite(field: u32, value: u64) -> bool {
    let flags: u64;
    core::arch::asm\!(
        "vmwrite {0}, {1}",
        "pushfq",
        "pop {2}",
        in(reg) field as u64,
        in(reg) value,
        out(reg) flags,
        options(nostack, nomem),
    );
    (flags & 0x41) == 0
}

/// Read a field from the current VMCS.
#[inline]
unsafe fn vmread(field: u32) -> Result<u64, ()> {
    let value: u64;
    let flags: u64;
    core::arch::asm\!(
        "vmread {0}, {1}",
        "pushfq",
        "pop {2}",
        out(reg) value,
        in(reg) field as u64,
        out(reg) flags,
        options(nostack, nomem),
    );
    if (flags & 0x41) == 0 { Ok(value) } else { Err(()) }
}

/// Execute VMLAUNCH. Returns the exit reason on success, or error.
#[inline]
unsafe fn vmlaunch() -> Result<u32, u32> {
    let flags: u64;
    core::arch::asm\!(
        "vmlaunch",
        "pushfq",
        "pop {0}",
        out(reg) flags,
        options(nostack),
    );
    if (flags & 0x41) == 0 {
        let reason = vmread(vmx::VMCS_EXIT_REASON).unwrap_or(0xFFFF_FFFF);
        Ok(reason as u32)
    } else {
        let err = vmread(vmx::VMCS_VM_INSTRUCTION_ERROR).unwrap_or(0) as u32;
        Err(err)
    }
}

/// Execute VMRESUME. Returns the exit reason on success, or error.
#[inline]
unsafe fn vmresume() -> Result<u32, u32> {
    let flags: u64;
    core::arch::asm\!(
        "vmresume",
        "pushfq",
        "pop {0}",
        out(reg) flags,
        options(nostack),
    );
    if (flags & 0x41) == 0 {
        let reason = vmread(vmx::VMCS_EXIT_REASON).unwrap_or(0xFFFF_FFFF);
        Ok(reason as u32)
    } else {
        let err = vmread(vmx::VMCS_VM_INSTRUCTION_ERROR).unwrap_or(0) as u32;
        Err(err)
    }
}

// ─── SVM inline assembly wrappers ───────────────────────────────────────────

/// Execute VMRUN with the physical address of the VMCB.
#[inline]
unsafe fn svm_vmrun(vmcb_phys: u64) {
    core::arch::asm\!(
        "mov rax, {0}",
        "vmrun",
        in(reg) vmcb_phys,
        out("rax") _,
        options(nostack),
    );
}

/// Execute VMSAVE.
#[inline]
unsafe fn svm_vmsave(vmcb_phys: u64) {
    core::arch::asm\!(
        "mov rax, {0}",
        "vmsave",
        in(reg) vmcb_phys,
        out("rax") _,
        options(nostack),
    );
}

/// Execute VMLOAD.
#[inline]
unsafe fn svm_vmload(vmcb_phys: u64) {
    core::arch::asm\!(
        "mov rax, {0}",
        "vmload",
        in(reg) vmcb_phys,
        out("rax") _,
        options(nostack),
    );
}

/// Enable SVM via the EFER MSR.
unsafe fn svm_enable() {
    let efer = rdmsr(0xC000_0080);
    wrmsr(0xC000_0080, efer | (1 << 12)); // EFER.SVME
}

/// Read RDTSC.
#[inline]
unsafe fn rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    core::arch::asm\!(
        "rdtsc",
        out("eax") lo,
        out("edx") hi,
        options(nostack, nomem),
    );
    (hi as u64) << 32 | lo as u64
}

// ─── EPT / NPT page table management ───────────────────────────────────────

/// 4-level EPT/NPT page table for identity-mapping guest physical memory.
struct NestedPageTable {
    /// PML4 table (root, 4KB aligned).
    pml4: Vec<u8>,
    /// PDPT pages.
    pdpts: Vec<Vec<u8>>,
    /// PD pages (for 2MB large page mappings).
    pds: Vec<Vec<u8>>,
}

impl NestedPageTable {
    /// Build an identity-mapped nested page table covering `size_bytes` of
    /// guest physical memory using 2MB large pages.
    fn build_identity(size_bytes: u64) -> Self {
        let mut pml4 = alloc_page();
        let mut pdpts = Vec::new();
        let mut pds = Vec::new();

        let num_2mb_pages = (size_bytes + (2 * 1024 * 1024 - 1)) / (2 * 1024 * 1024);
        let num_pds = ((num_2mb_pages + 511) / 512) as usize;
        let num_pdpts = ((num_pds + 511) / 512) as usize;

        // Create PDPT entries in PML4
        for i in 0..num_pdpts.max(1) {
            let pdpt = alloc_page();
            let pdpt_phys = phys_addr(&pdpt);
            // PML4 entry: present + read/write + user
            let entry = pdpt_phys | 0x07;
            let offset = i * 8;
            if offset + 8 <= 4096 {
                pml4[offset..offset + 8].copy_from_slice(&entry.to_le_bytes());
            }

            // Create PD entries in this PDPT
            let pds_in_this_pdpt = if i == num_pdpts - 1 {
                num_pds - i * 512
            } else {
                512
            };

            for j in 0..pds_in_this_pdpt.min(512) {
                let pd = alloc_page();
                let pd_phys = phys_addr(&pd);
                // PDPT entry: present + read/write + user
                let pdpt_entry = pd_phys | 0x07;
                let pdpt_offset = j * 8;
                if pdpt_offset + 8 <= 4096 {
                    unsafe {
                        let pdpt_ptr = pdpt.as_ptr() as *mut u8;
                        let dst = pdpt_ptr.add(pdpt_offset) as *mut u64;
                        dst.write_volatile(pdpt_entry);
                    }
                }

                // Fill PD with 2MB large page entries
                let base_pd_index = (i * 512 + j) as u64;
                for k in 0u64..512 {
                    let page_num = base_pd_index * 512 + k;
                    let phys = page_num * 2 * 1024 * 1024;
                    if phys >= size_bytes {
                        break;
                    }
                    // PD entry: present + read/write + user + large page (bit 7)
                    // For EPT: read (0) + write (1) + execute (2) + large page (7)
                    let pd_entry = phys | 0x87; // PS + present + RW + user
                    let pd_offset = (k as usize) * 8;
                    if pd_offset + 8 <= 4096 {
                        unsafe {
                            let pd_ptr = pd.as_ptr() as *mut u8;
                            let dst = pd_ptr.add(pd_offset) as *mut u64;
                            dst.write_volatile(pd_entry);
                        }
                    }
                }

                pds.push(pd);
            }

            pdpts.push(pdpt);
        }

        Self { pml4, pdpts, pds }
    }

    /// Build EPT identity map using EPT-specific permission bits.
    fn build_ept_identity(size_bytes: u64) -> Self {
        let mut pml4 = alloc_page();
        let mut pdpts = Vec::new();
        let mut pds = Vec::new();

        let num_2mb_pages = (size_bytes + (2 * 1024 * 1024 - 1)) / (2 * 1024 * 1024);
        let num_pds = ((num_2mb_pages + 511) / 512) as usize;
        let num_pdpts = ((num_pds + 511) / 512) as usize;

        for i in 0..num_pdpts.max(1) {
            let pdpt = alloc_page();
            let pdpt_phys = phys_addr(&pdpt);
            // EPT PML4 entry: read(0) + write(1) + execute(2) = 0x07
            let entry = pdpt_phys | 0x07;
            let offset = i * 8;
            if offset + 8 <= 4096 {
                pml4[offset..offset + 8].copy_from_slice(&entry.to_le_bytes());
            }

            let pds_in_this_pdpt = if i == num_pdpts - 1 {
                num_pds - i * 512
            } else {
                512
            };

            for j in 0..pds_in_this_pdpt.min(512) {
                let pd = alloc_page();
                let pd_phys = phys_addr(&pd);
                let pdpt_entry = pd_phys | 0x07;
                let pdpt_offset = j * 8;
                if pdpt_offset + 8 <= 4096 {
                    unsafe {
                        let pdpt_ptr = pdpt.as_ptr() as *mut u8;
                        let dst = pdpt_ptr.add(pdpt_offset) as *mut u64;
                        dst.write_volatile(pdpt_entry);
                    }
                }

                let base_pd_index = (i * 512 + j) as u64;
                for k in 0u64..512 {
                    let page_num = base_pd_index * 512 + k;
                    let phys = page_num * 2 * 1024 * 1024;
                    if phys >= size_bytes {
                        break;
                    }
                    // EPT large page: read(0) + write(1) + execute(2) + memory type WB(6,bits 3-5=110) + large page(7)
                    let pd_entry = phys | (0x07 | (6 << 3) | (1 << 7));
                    let pd_offset = (k as usize) * 8;
                    if pd_offset + 8 <= 4096 {
                        unsafe {
                            let pd_ptr = pd.as_ptr() as *mut u8;
                            let dst = pd_ptr.add(pd_offset) as *mut u64;
                            dst.write_volatile(pd_entry);
                        }
                    }
                }

                pds.push(pd);
            }

            pdpts.push(pdpt);
        }

        Self { pml4, pdpts, pds }
    }

    /// Get the physical address of the PML4 root.
    fn root_phys(&self) -> u64 {
        phys_addr(&self.pml4)
    }
}

// ─── VMCB (AMD-V) layout ───────────────────────────────────────────────────

/// Offsets into the VMCB control area (first 0x400 bytes).
mod vmcb_ctrl {
    pub const INTERCEPT_CR_READ: usize = 0x000;
    pub const INTERCEPT_CR_WRITE: usize = 0x004;
    pub const INTERCEPT_EXCEPTIONS: usize = 0x008;
    pub const INTERCEPT_MISC1: usize = 0x00C;
    pub const INTERCEPT_MISC2: usize = 0x010;
    pub const IOPM_BASE_PA: usize = 0x040;
    pub const MSRPM_BASE_PA: usize = 0x048;
    pub const GUEST_ASID: usize = 0x058;
    pub const TLB_CONTROL: usize = 0x05C;
    pub const EXITCODE: usize = 0x070;
    pub const EXITINFO1: usize = 0x078;
    pub const EXITINFO2: usize = 0x080;
    pub const EVENT_INJ: usize = 0x0A8;
    pub const N_CR3: usize = 0x0B0;
    pub const LBR_ENABLE: usize = 0x0B8;
    pub const VMCB_CLEAN: usize = 0x0C0;
    pub const NEXT_RIP: usize = 0x0C8;
    pub const NP_ENABLE: usize = 0x090;
}

/// Offsets into the VMCB state-save area (starts at 0x400).
mod vmcb_save {
    pub const BASE: usize = 0x400;
    pub const ES_SELECTOR: usize = BASE + 0x000;
    pub const ES_ATTRIB: usize = BASE + 0x002;
    pub const ES_LIMIT: usize = BASE + 0x004;
    pub const ES_BASE: usize = BASE + 0x008;
    pub const CS_SELECTOR: usize = BASE + 0x010;
    pub const CS_ATTRIB: usize = BASE + 0x012;
    pub const CS_LIMIT: usize = BASE + 0x014;
    pub const CS_BASE: usize = BASE + 0x018;
    pub const SS_SELECTOR: usize = BASE + 0x020;
    pub const SS_ATTRIB: usize = BASE + 0x022;
    pub const SS_LIMIT: usize = BASE + 0x024;
    pub const SS_BASE: usize = BASE + 0x028;
    pub const DS_SELECTOR: usize = BASE + 0x030;
    pub const DS_ATTRIB: usize = BASE + 0x032;
    pub const DS_LIMIT: usize = BASE + 0x034;
    pub const DS_BASE: usize = BASE + 0x038;
    pub const FS_SELECTOR: usize = BASE + 0x040;
    pub const FS_ATTRIB: usize = BASE + 0x042;
    pub const FS_LIMIT: usize = BASE + 0x044;
    pub const FS_BASE: usize = BASE + 0x048;
    pub const GS_SELECTOR: usize = BASE + 0x050;
    pub const GS_ATTRIB: usize = BASE + 0x052;
    pub const GS_LIMIT: usize = BASE + 0x054;
    pub const GS_BASE: usize = BASE + 0x058;
    pub const GDTR_SELECTOR: usize = BASE + 0x060;
    pub const GDTR_LIMIT: usize = BASE + 0x064;
    pub const GDTR_BASE: usize = BASE + 0x068;
    pub const LDTR_SELECTOR: usize = BASE + 0x070;
    pub const LDTR_ATTRIB: usize = BASE + 0x072;
    pub const LDTR_LIMIT: usize = BASE + 0x074;
    pub const LDTR_BASE: usize = BASE + 0x078;
    pub const IDTR_SELECTOR: usize = BASE + 0x080;
    pub const IDTR_LIMIT: usize = BASE + 0x084;
    pub const IDTR_BASE: usize = BASE + 0x088;
    pub const TR_SELECTOR: usize = BASE + 0x090;
    pub const TR_ATTRIB: usize = BASE + 0x092;
    pub const TR_LIMIT: usize = BASE + 0x094;
    pub const TR_BASE: usize = BASE + 0x098;
    pub const EFER: usize = BASE + 0x0D0;
    pub const CR4: usize = BASE + 0x148;
    pub const CR3: usize = BASE + 0x150;
    pub const CR0: usize = BASE + 0x158;
    pub const DR7: usize = BASE + 0x160;
    pub const DR6: usize = BASE + 0x168;
    pub const RFLAGS: usize = BASE + 0x170;
    pub const RIP: usize = BASE + 0x178;
    pub const RSP: usize = BASE + 0x1D8;
    pub const RAX: usize = BASE + 0x1F8;
    pub const STAR: usize = BASE + 0x200;
    pub const LSTAR: usize = BASE + 0x208;
    pub const CSTAR: usize = BASE + 0x210;
    pub const SFMASK: usize = BASE + 0x218;
    pub const KERNEL_GS_BASE: usize = BASE + 0x228;
    pub const SYSENTER_CS: usize = BASE + 0x230;
    pub const SYSENTER_ESP: usize = BASE + 0x238;
    pub const SYSENTER_EIP: usize = BASE + 0x240;
    pub const CR2: usize = BASE + 0x248;
    pub const PAT: usize = BASE + 0x268;
}

// ─── BareMetalBackend struct ────────────────────────────────────────────────

/// VMX-specific state for a single vCPU.
struct VmxState {
    /// VMXON region (4KB, must stay alive).
    vmxon_region: Vec<u8>,
    /// VMCS region (4KB, must stay alive).
    vmcs_region: Vec<u8>,
    /// Whether VMLAUNCH has been called (vs VMRESUME).
    launched: bool,
    /// EPT page tables.
    ept: Option<NestedPageTable>,
    /// Saved guest GPRs (not stored in VMCS).
    guest_gprs: GuestGprs,
    /// Pending IO in data to return.
    pending_io_data: Option<(u32, u8)>,
    /// Pending MMIO read data.
    pending_mmio_data: Option<(u64, u8)>,
    /// Pending CPUID response.
    pending_cpuid: Option<(u32, u32, u32, u32)>,
    /// Pending MSR read data.
    pending_msr_data: Option<u64>,
    /// Stop requested.
    stop_requested: bool,
    /// Interrupt window requested.
    int_window_requested: bool,
}

/// SVM-specific state for a single vCPU.
struct SvmState {
    /// VMCB (4KB aligned, must stay alive).
    vmcb: Vec<u8>,
    /// Host save area (4KB aligned).
    host_save: Vec<u8>,
    /// NPT page tables.
    npt: Option<NestedPageTable>,
    /// IO permission map (12KB = 3 pages, for 64K ports).
    iopm: Vec<u8>,
    /// MSR permission map (8KB = 2 pages).
    msrpm: Vec<u8>,
    /// Saved guest GPRs (RAX is in VMCB, rest managed here).
    guest_gprs: GuestGprs,
    /// Pending IO in data.
    pending_io_data: Option<(u32, u8)>,
    /// Pending MMIO read data.
    pending_mmio_data: Option<(u64, u8)>,
    /// Pending CPUID response.
    pending_cpuid: Option<(u32, u32, u32, u32)>,
    /// Pending MSR read data.
    pending_msr_data: Option<u64>,
    /// Stop requested.
    stop_requested: bool,
    /// Interrupt window requested.
    int_window_requested: bool,
}

/// Guest GPR state not stored in VMCS/VMCB.
#[derive(Default, Clone)]
struct GuestGprs {
    rax: u64,
    rbx: u64,
    rcx: u64,
    rdx: u64,
    rsi: u64,
    rdi: u64,
    rbp: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
}

/// The bare-metal hypervisor backend.
pub struct BareMetalBackend {
    vendor: CpuVendor,
    vmx: Option<VmxState>,
    svm: Option<SvmState>,
    /// Memory regions mapped for the guest.
    memory_regions: Vec<MemoryRegion>,
    /// Total guest memory size for nested page table coverage.
    guest_mem_size: u64,
}

// ─── BareMetalBackend implementation ────────────────────────────────────────

impl BareMetalBackend {
    /// Create a new bare-metal backend, detecting CPU vendor.
    pub fn new() -> Result<Self, HvError> {
        let vendor = detect_vendor();
        match vendor {
            CpuVendor::Intel => {
                // Check VMX support via CPUID.1:ECX bit 5
                let ecx: u32;
                unsafe {
                    core::arch::asm\!(
                        "push rbx",
                        "mov eax, 1",
                        "cpuid",
                        "mov {0:e}, ecx",
                        "pop rbx",
                        out(reg) ecx,
                        out("eax") _,
                        out("edx") _,
                        options(nostack),
                    );
                }
                if ecx & (1 << 5) == 0 {
                    return Err(HvError::NotSupported);
                }
            }
            CpuVendor::Amd => {
                // Check SVM support via CPUID 0x80000001:ECX bit 2
                let ecx: u32;
                unsafe {
                    core::arch::asm\!(
                        "push rbx",
                        "mov eax, 0x80000001",
                        "cpuid",
                        "mov {0:e}, ecx",
                        "pop rbx",
                        out(reg) ecx,
                        out("eax") _,
                        out("edx") _,
                        options(nostack),
                    );
                }
                if ecx & (1 << 2) == 0 {
                    return Err(HvError::NotSupported);
                }
            }
            CpuVendor::Unknown => return Err(HvError::NotSupported),
        }

        Ok(Self {
            vendor,
            vmx: None,
            svm: None,
            memory_regions: Vec::new(),
            guest_mem_size: 0,
        })
    }

    // ── VMX helpers ──

    fn vmx_init(&mut self) -> Result<(), HvError> {
        unsafe {
            // Enable VMX in CR4
            let cr4 = read_cr4();
            write_cr4(cr4 | (1 << 13)); // CR4.VMXE

            // Read IA32_VMX_BASIC to get VMCS revision ID
            let vmx_basic = rdmsr(0x480); // IA32_VMX_BASIC
            let revision_id = (vmx_basic & 0x7FFF_FFFF) as u32;

            // Allocate and initialize VMXON region
            let mut vmxon_region = alloc_page();
            vmxon_region[0..4].copy_from_slice(&revision_id.to_le_bytes());

            let vmxon_phys = phys_addr(&vmxon_region);
            if \!vmxon(vmxon_phys) {
                return Err(HvError::HardwareError(0));
            }

            // Allocate and initialize VMCS
            let mut vmcs_region = alloc_page();
            vmcs_region[0..4].copy_from_slice(&revision_id.to_le_bytes());

            let vmcs_phys = phys_addr(&vmcs_region);
            if \!vmclear(vmcs_phys) {
                vmxoff();
                return Err(HvError::HardwareError(1));
            }
            if \!vmptrld(vmcs_phys) {
                vmxoff();
                return Err(HvError::HardwareError(2));
            }

            self.vmx = Some(VmxState {
                vmxon_region,
                vmcs_region,
                launched: false,
                ept: None,
                guest_gprs: GuestGprs::default(),
                pending_io_data: None,
                pending_mmio_data: None,
                pending_cpuid: None,
                pending_msr_data: None,
                stop_requested: false,
                int_window_requested: false,
            });

            Ok(())
        }
    }

    fn vmx_setup_vmcs(&mut self) -> Result<(), HvError> {
        unsafe {
            // Read VMX capability MSRs for pin/proc/exit/entry controls
            let pin_based_msr = rdmsr(0x481); // IA32_VMX_PINBASED_CTLS
            let proc_based_msr = rdmsr(0x482); // IA32_VMX_PROCBASED_CTLS
            let exit_msr = rdmsr(0x483); // IA32_VMX_EXIT_CTLS
            let entry_msr = rdmsr(0x484); // IA32_VMX_ENTRY_CTLS

            // Check if secondary proc-based controls are available
            let proc2_msr = if (proc_based_msr >> 32) as u32 & (1 << 31) \!= 0 {
                rdmsr(0x48B) // IA32_VMX_PROCBASED_CTLS2
            } else {
                0
            };

            // Pin-based: external interrupt exiting + NMI exiting
            let pin_val = adjust_vmx_controls(
                (1 << 0) | (1 << 3), // ext-int + NMI exiting
                pin_based_msr,
            );
            vmwrite(vmx::VMCS_PIN_BASED_CONTROLS, pin_val as u64);

            // Primary proc-based: HLT exiting + MWAIT exiting + RDPMC exiting +
            // use IO bitmaps (no) + use MSR bitmaps (no) + activate secondary controls
            let proc_val = adjust_vmx_controls(
                (1 << 7)  |  // HLT exiting
                (1 << 10) |  // RDTSC exiting (optional, we skip for perf)
                (1 << 12) |  // RDPMC exiting
                (1 << 24) |  // unconditional IO exiting
                (1 << 31),   // activate secondary controls
                proc_based_msr,
            );
            vmwrite(vmx::VMCS_PROC_BASED_CONTROLS, proc_val as u64);

            // Secondary proc-based: enable EPT + unrestricted guest
            let proc2_val = adjust_vmx_controls(
                (1 << 1) | // enable EPT
                (1 << 7),  // unrestricted guest
                proc2_msr,
            );
            vmwrite(vmx::VMCS_SECONDARY_PROC_BASED_CONTROLS, proc2_val as u64);

            // VM-exit controls: host address-space size (64-bit host)
            let exit_val = adjust_vmx_controls(
                (1 << 9),  // host address-space size = 64-bit
                exit_msr,
            );
            vmwrite(vmx::VMCS_EXIT_CONTROLS, exit_val as u64);

            // VM-entry controls
            let entry_val = adjust_vmx_controls(0, entry_msr);
            vmwrite(vmx::VMCS_ENTRY_CONTROLS, entry_val as u64);

            // CR0/CR4 guest/host mask and shadow — let guest manage freely
            vmwrite(vmx::VMCS_CR0_GUEST_HOST_MASK, 0);
            vmwrite(vmx::VMCS_CR0_READ_SHADOW, 0);
            vmwrite(vmx::VMCS_CR4_GUEST_HOST_MASK, 0);
            vmwrite(vmx::VMCS_CR4_READ_SHADOW, 0);

            // Exception bitmap — intercept nothing (0) or just #DF and #MC
            vmwrite(vmx::VMCS_EXCEPTION_BITMAP, (1 << 8) | (1 << 18)); // #DF + #MC

            // Set up EPT if we have memory regions
            if self.guest_mem_size > 0 {
                self.vmx_setup_ept()?;
            }

            // Host state — save current host register state
            self.vmx_setup_host_state();

            Ok(())
        }
    }

    unsafe fn vmx_setup_host_state(&self) {
        // Host CR0, CR3, CR4
        vmwrite(vmx::VMCS_HOST_CR0, read_cr0());
        let cr3: u64;
        core::arch::asm\!("mov {}, cr3", out(reg) cr3, options(nostack, nomem));
        vmwrite(vmx::VMCS_HOST_CR3, cr3);
        vmwrite(vmx::VMCS_HOST_CR4, read_cr4());

        // Host segment selectors
        let cs: u16;
        let ss: u16;
        let ds: u16;
        let es: u16;
        let fs: u16;
        let gs: u16;
        let tr: u16;
        core::arch::asm\!(
            "mov {0:x}, cs",
            "mov {1:x}, ss",
            "mov {2:x}, ds",
            "mov {3:x}, es",
            out(reg) cs,
            out(reg) ss,
            out(reg) ds,
            out(reg) es,
            options(nostack, nomem),
        );
        core::arch::asm\!(
            "mov {0:x}, fs",
            "mov {1:x}, gs",
            "str {2:x}",
            out(reg) fs,
            out(reg) gs,
            out(reg) tr,
            options(nostack, nomem),
        );
        vmwrite(vmx::VMCS_HOST_CS_SELECTOR, cs as u64);
        vmwrite(vmx::VMCS_HOST_SS_SELECTOR, ss as u64);
        vmwrite(vmx::VMCS_HOST_DS_SELECTOR, ds as u64);
        vmwrite(vmx::VMCS_HOST_ES_SELECTOR, es as u64);
        vmwrite(vmx::VMCS_HOST_FS_SELECTOR, fs as u64);
        vmwrite(vmx::VMCS_HOST_GS_SELECTOR, gs as u64);
        vmwrite(vmx::VMCS_HOST_TR_SELECTOR, tr as u64);

        // Host GDTR/IDTR base
        let mut gdtr_buf = [0u8; 10];
        let mut idtr_buf = [0u8; 10];
        core::arch::asm\!("sgdt [{}]", in(reg) gdtr_buf.as_mut_ptr(), options(nostack));
        core::arch::asm\!("sidt [{}]", in(reg) idtr_buf.as_mut_ptr(), options(nostack));
        let gdtr_base = u64::from_le_bytes(gdtr_buf[2..10].try_into().unwrap());
        let idtr_base = u64::from_le_bytes(idtr_buf[2..10].try_into().unwrap());
        vmwrite(vmx::VMCS_HOST_GDTR_BASE, gdtr_base);
        vmwrite(vmx::VMCS_HOST_IDTR_BASE, idtr_base);

        // Host RSP and RIP will be set just before VMLAUNCH/VMRESUME
        // (they point to our VM-exit handler)

        // Host MSRs
        vmwrite(vmx::VMCS_HOST_EFER, rdmsr(0xC000_0080));
        vmwrite(vmx::VMCS_HOST_PAT, rdmsr(0x277));
        vmwrite(vmx::VMCS_HOST_SYSENTER_CS, rdmsr(0x174));
        vmwrite(vmx::VMCS_HOST_SYSENTER_ESP, rdmsr(0x175));
        vmwrite(vmx::VMCS_HOST_SYSENTER_EIP, rdmsr(0x176));
        vmwrite(vmx::VMCS_HOST_FS_BASE, rdmsr(0xC000_0100));
        vmwrite(vmx::VMCS_HOST_GS_BASE, rdmsr(0xC000_0101));
    }

    fn vmx_setup_ept(&mut self) -> Result<(), HvError> {
        let ept = NestedPageTable::build_ept_identity(self.guest_mem_size);
        let eptp = ept.root_phys()
            | (3 << 3) // page-walk length = 4 (encoded as 3)
            | 6;       // memory type = WB
        unsafe {
            vmwrite(vmx::VMCS_EPT_POINTER, eptp);
        }
        if let Some(ref mut vmx_state) = self.vmx {
            vmx_state.ept = Some(ept);
        }
        Ok(())
    }

    fn vmx_set_guest_reset_state(&mut self) -> Result<(), HvError> {
        unsafe {
            // Real mode reset state
            vmwrite(vmx::VMCS_GUEST_CS_SELECTOR, 0xF000);
            vmwrite(vmx::VMCS_GUEST_CS_BASE, 0xFFFF_0000);
            vmwrite(vmx::VMCS_GUEST_CS_LIMIT, 0xFFFF);
            vmwrite(vmx::VMCS_GUEST_CS_ACCESS_RIGHTS, 0x9B); // present, code, r/x, accessed

            for &(sel_field, base_field, limit_field, ar_field) in &[
                (vmx::VMCS_GUEST_DS_SELECTOR, vmx::VMCS_GUEST_DS_BASE, vmx::VMCS_GUEST_DS_LIMIT, vmx::VMCS_GUEST_DS_ACCESS_RIGHTS),
                (vmx::VMCS_GUEST_ES_SELECTOR, vmx::VMCS_GUEST_ES_BASE, vmx::VMCS_GUEST_ES_LIMIT, vmx::VMCS_GUEST_ES_ACCESS_RIGHTS),
                (vmx::VMCS_GUEST_SS_SELECTOR, vmx::VMCS_GUEST_SS_BASE, vmx::VMCS_GUEST_SS_LIMIT, vmx::VMCS_GUEST_SS_ACCESS_RIGHTS),
                (vmx::VMCS_GUEST_FS_SELECTOR, vmx::VMCS_GUEST_FS_BASE, vmx::VMCS_GUEST_FS_LIMIT, vmx::VMCS_GUEST_FS_ACCESS_RIGHTS),
                (vmx::VMCS_GUEST_GS_SELECTOR, vmx::VMCS_GUEST_GS_BASE, vmx::VMCS_GUEST_GS_LIMIT, vmx::VMCS_GUEST_GS_ACCESS_RIGHTS),
            ] {
                vmwrite(sel_field, 0);
                vmwrite(base_field, 0);
                vmwrite(limit_field, 0xFFFF);
                vmwrite(ar_field, 0x93); // present, data, r/w, accessed
            }

            // TR
            vmwrite(vmx::VMCS_GUEST_TR_SELECTOR, 0);
            vmwrite(vmx::VMCS_GUEST_TR_BASE, 0);
            vmwrite(vmx::VMCS_GUEST_TR_LIMIT, 0xFFFF);
            vmwrite(vmx::VMCS_GUEST_TR_ACCESS_RIGHTS, 0x8B); // present, 32-bit busy TSS

            // LDTR
            vmwrite(vmx::VMCS_GUEST_LDTR_SELECTOR, 0);
            vmwrite(vmx::VMCS_GUEST_LDTR_BASE, 0);
            vmwrite(vmx::VMCS_GUEST_LDTR_LIMIT, 0xFFFF);
            vmwrite(vmx::VMCS_GUEST_LDTR_ACCESS_RIGHTS, 0x82); // present, LDT

            // GDTR / IDTR
            vmwrite(vmx::VMCS_GUEST_GDTR_BASE, 0);
            vmwrite(vmx::VMCS_GUEST_GDTR_LIMIT, 0xFFFF);
            vmwrite(vmx::VMCS_GUEST_IDTR_BASE, 0);
            vmwrite(vmx::VMCS_GUEST_IDTR_LIMIT, 0xFFFF);

            // Control registers — real mode defaults
            vmwrite(vmx::VMCS_GUEST_CR0, 0x0000_0010); // ET bit set
            vmwrite(vmx::VMCS_GUEST_CR3, 0);
            vmwrite(vmx::VMCS_GUEST_CR4, 0);

            // RIP = 0xFFF0, RSP = 0, RFLAGS = 0x02
            vmwrite(vmx::VMCS_GUEST_RIP, 0xFFF0);
            vmwrite(vmx::VMCS_GUEST_RSP, 0);
            vmwrite(vmx::VMCS_GUEST_RFLAGS, 0x02);

            // EFER
            vmwrite(vmx::VMCS_GUEST_EFER, 0);

            // DR7
            vmwrite(vmx::VMCS_GUEST_DR7, 0x0000_0400);

            // Guest activity state = active
            vmwrite(vmx::VMCS_GUEST_ACTIVITY_STATE, 0);

            // Interruptibility state = 0
            vmwrite(vmx::VMCS_GUEST_INTERRUPTIBILITY_STATE, 0);

            // VMCS link pointer
            vmwrite(vmx::VMCS_GUEST_VMCS_LINK_POINTER, 0xFFFF_FFFF_FFFF_FFFF);
        }
        Ok(())
    }

    // ── SVM helpers ──

    fn svm_init(&mut self) -> Result<(), HvError> {
        unsafe {
            svm_enable();

            // Allocate VMCB (4KB aligned, zeroed)
            let vmcb = alloc_page();

            // Allocate host save area
            let host_save = alloc_page();
            // Set VM_HSAVE_PA MSR
            wrmsr(0xC001_0117, phys_addr(&host_save));

            // Allocate IOPM (12KB = 3 pages, all bits = 1 means intercept all IO)
            let mut iopm = alloc_pages(3);
            // Set all bits to 1 — intercept all IO ports
            for byte in iopm.iter_mut() {
                *byte = 0xFF;
            }

            // Allocate MSRPM (8KB = 2 pages, all 1s = intercept all MSRs)
            let mut msrpm = alloc_pages(2);
            for byte in msrpm.iter_mut() {
                *byte = 0xFF;
            }

            self.svm = Some(SvmState {
                vmcb,
                host_save,
                npt: None,
                iopm,
                msrpm,
                guest_gprs: GuestGprs::default(),
                pending_io_data: None,
                pending_mmio_data: None,
                pending_cpuid: None,
                pending_msr_data: None,
                stop_requested: false,
                int_window_requested: false,
            });

            Ok(())
        }
    }

    fn svm_setup_vmcb(&mut self) -> Result<(), HvError> {
        let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;
        let vmcb = &mut svm.vmcb;

        // Helper to write a u32 into the VMCB at a given offset
        let write_u32 = |buf: &mut Vec<u8>, offset: usize, val: u32| {
            buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
        };
        let write_u64 = |buf: &mut Vec<u8>, offset: usize, val: u64| {
            buf[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
        };

        // Intercept CR0/CR4 writes
        write_u32(vmcb, vmcb_ctrl::INTERCEPT_CR_WRITE, (1 << 0) | (1 << 4));

        // Intercept exceptions: #DF(8) and #MC(18)
        write_u32(vmcb, vmcb_ctrl::INTERCEPT_EXCEPTIONS, (1 << 8) | (1 << 18));

        // Intercept MISC1: INTR, NMI, CPUID, HLT, IOIO, MSR, shutdown
        write_u32(vmcb, vmcb_ctrl::INTERCEPT_MISC1,
            (1 << 0)  | // INTR
            (1 << 1)  | // NMI
            (1 << 18) | // CPUID
            (1 << 24) | // HLT
            (1 << 27) | // IOIO
            (1 << 28) | // MSR
            (1 << 31)   // SHUTDOWN
        );

        // IOPM and MSRPM base addresses
        write_u64(vmcb, vmcb_ctrl::IOPM_BASE_PA, phys_addr(&svm.iopm));
        write_u64(vmcb, vmcb_ctrl::MSRPM_BASE_PA, phys_addr(&svm.msrpm));

        // Guest ASID (must be non-zero)
        write_u32(vmcb, vmcb_ctrl::GUEST_ASID, 1);

        // Enable NPT if we have memory
        if self.guest_mem_size > 0 {
            let npt = NestedPageTable::build_identity(self.guest_mem_size);
            write_u64(vmcb, vmcb_ctrl::NP_ENABLE, 1);
            write_u64(vmcb, vmcb_ctrl::N_CR3, npt.root_phys());
            let svm = self.svm.as_mut().unwrap();
            svm.npt = Some(npt);
        }

        // Guest reset state in save area
        let svm = self.svm.as_mut().unwrap();
        let vmcb = &mut svm.vmcb;

        // CS: selector=0xF000, base=0xFFFF0000, limit=0xFFFF
        vmcb[vmcb_save::CS_SELECTOR..vmcb_save::CS_SELECTOR + 2].copy_from_slice(&0xF000u16.to_le_bytes());
        write_u64(vmcb, vmcb_save::CS_BASE, 0xFFFF_0000);
        write_u32(vmcb, vmcb_save::CS_LIMIT, 0xFFFF);
        vmcb[vmcb_save::CS_ATTRIB..vmcb_save::CS_ATTRIB + 2].copy_from_slice(&0x049Bu16.to_le_bytes());

        // Data segments: selector=0, base=0, limit=0xFFFF
        for &(sel, base, limit, attrib) in &[
            (vmcb_save::DS_SELECTOR, vmcb_save::DS_BASE, vmcb_save::DS_LIMIT, vmcb_save::DS_ATTRIB),
            (vmcb_save::ES_SELECTOR, vmcb_save::ES_BASE, vmcb_save::ES_LIMIT, vmcb_save::ES_ATTRIB),
            (vmcb_save::SS_SELECTOR, vmcb_save::SS_BASE, vmcb_save::SS_LIMIT, vmcb_save::SS_ATTRIB),
            (vmcb_save::FS_SELECTOR, vmcb_save::FS_BASE, vmcb_save::FS_LIMIT, vmcb_save::FS_ATTRIB),
            (vmcb_save::GS_SELECTOR, vmcb_save::GS_BASE, vmcb_save::GS_LIMIT, vmcb_save::GS_ATTRIB),
        ] {
            vmcb[sel..sel + 2].copy_from_slice(&0u16.to_le_bytes());
            write_u64(vmcb, base, 0);
            write_u32(vmcb, limit, 0xFFFF);
            vmcb[attrib..attrib + 2].copy_from_slice(&0x0493u16.to_le_bytes());
        }

        // TR
        vmcb[vmcb_save::TR_SELECTOR..vmcb_save::TR_SELECTOR + 2].copy_from_slice(&0u16.to_le_bytes());
        write_u64(vmcb, vmcb_save::TR_BASE, 0);
        write_u32(vmcb, vmcb_save::TR_LIMIT, 0xFFFF);
        vmcb[vmcb_save::TR_ATTRIB..vmcb_save::TR_ATTRIB + 2].copy_from_slice(&0x008Bu16.to_le_bytes());

        // LDTR
        vmcb[vmcb_save::LDTR_SELECTOR..vmcb_save::LDTR_SELECTOR + 2].copy_from_slice(&0u16.to_le_bytes());
        write_u64(vmcb, vmcb_save::LDTR_BASE, 0);
        write_u32(vmcb, vmcb_save::LDTR_LIMIT, 0xFFFF);
        vmcb[vmcb_save::LDTR_ATTRIB..vmcb_save::LDTR_ATTRIB + 2].copy_from_slice(&0x0082u16.to_le_bytes());

        // GDTR / IDTR
        write_u64(vmcb, vmcb_save::GDTR_BASE, 0);
        write_u32(vmcb, vmcb_save::GDTR_LIMIT, 0xFFFF);
        write_u64(vmcb, vmcb_save::IDTR_BASE, 0);
        write_u32(vmcb, vmcb_save::IDTR_LIMIT, 0xFFFF);

        // CR0 = 0x10 (ET), CR3 = 0, CR4 = 0
        write_u64(vmcb, vmcb_save::CR0, 0x10);
        write_u64(vmcb, vmcb_save::CR3, 0);
        write_u64(vmcb, vmcb_save::CR4, 0);

        // RIP = 0xFFF0
        write_u64(vmcb, vmcb_save::RIP, 0xFFF0);
        // RFLAGS = 0x02
        write_u64(vmcb, vmcb_save::RFLAGS, 0x02);
        // EFER = 0
        write_u64(vmcb, vmcb_save::EFER, 0);
        // DR7 = 0x400
        write_u64(vmcb, vmcb_save::DR7, 0x400);
        // PAT = default
        write_u64(vmcb, vmcb_save::PAT, 0x0007_0406_0007_0406);

        Ok(())
    }
}

/// Adjust VMX controls: apply must-be-1 and must-be-0 bits from capability MSR.
fn adjust_vmx_controls(desired: u32, msr_val: u64) -> u32 {
    let allowed_0 = msr_val as u32;        // bits that must be 1
    let allowed_1 = (msr_val >> 32) as u32; // bits that can be 1
    (desired | allowed_0) & allowed_1
}

/// Helper to read a u32 from a byte buffer at an offset.
fn read_vmcb_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(buf[offset..offset + 4].try_into().unwrap())
}

/// Helper to read a u64 from a byte buffer at an offset.
fn read_vmcb_u64(buf: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap())
}

/// Helper to read a u16 from a byte buffer at an offset.
fn read_vmcb_u16(buf: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(buf[offset..offset + 2].try_into().unwrap())
}

/// Helper to write a u64 into a byte buffer.
fn write_vmcb_u64(buf: &mut [u8], offset: usize, val: u64) {
    buf[offset..offset + 8].copy_from_slice(&val.to_le_bytes());
}

fn write_vmcb_u32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_le_bytes());
}

fn write_vmcb_u16(buf: &mut [u8], offset: usize, val: u16) {
    buf[offset..offset + 2].copy_from_slice(&val.to_le_bytes());
}

// ─── HypervisorBackend trait implementation ─────────────────────────────────

impl HypervisorBackend for BareMetalBackend {
    fn create_vm(&mut self) -> Result<(), HvError> {
        match self.vendor {
            CpuVendor::Intel => self.vmx_init(),
            CpuVendor::Amd => self.svm_init(),
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn create_vcpu(&mut self, _vcpu_id: u32) -> Result<(), HvError> {
        // On bare metal, the vCPU is the physical CPU.
        // Set up VMCS/VMCB with reset state.
        match self.vendor {
            CpuVendor::Intel => {
                self.vmx_setup_vmcs()?;
                self.vmx_set_guest_reset_state()
            }
            CpuVendor::Amd => self.svm_setup_vmcb(),
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn map_memory(&mut self, region: &MemoryRegion) -> Result<(), HvError> {
        let end = region.guest_phys_addr + region.memory_size;
        if end > self.guest_mem_size {
            self.guest_mem_size = end;
        }
        self.memory_regions.push(region.clone());

        // Rebuild nested page tables with new coverage
        match self.vendor {
            CpuVendor::Intel => {
                if self.vmx.is_some() {
                    self.vmx_setup_ept()?;
                }
            }
            CpuVendor::Amd => {
                if let Some(ref mut svm) = self.svm {
                    let npt = NestedPageTable::build_identity(self.guest_mem_size);
                    write_vmcb_u64(&mut svm.vmcb, vmcb_ctrl::N_CR3, npt.root_phys());
                    write_vmcb_u64(&mut svm.vmcb, vmcb_ctrl::NP_ENABLE, 1);
                    svm.npt = Some(npt);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn unmap_memory(&mut self, slot: u32) -> Result<(), HvError> {
        self.memory_regions.retain(|r| r.slot != slot);
        // Recalculate max guest memory
        self.guest_mem_size = self.memory_regions.iter()
            .map(|r| r.guest_phys_addr + r.memory_size)
            .max()
            .unwrap_or(0);
        Ok(())
    }

    fn get_regs(&self, _vcpu_id: u32) -> Result<VcpuRegs, HvError> {
        let mut regs = VcpuRegs::default();

        match self.vendor {
            CpuVendor::Intel => {
                let vmx = self.vmx.as_ref().ok_or(HvError::Other("VMX not initialized"))?;
                unsafe {
                    regs.rip = vmread(vmx::VMCS_GUEST_RIP).unwrap_or(0);
                    regs.rsp = vmread(vmx::VMCS_GUEST_RSP).unwrap_or(0);
                    regs.rflags = vmread(vmx::VMCS_GUEST_RFLAGS).unwrap_or(0);

                    // GPRs from saved state
                    regs.rax = vmx.guest_gprs.rax;
                    regs.rbx = vmx.guest_gprs.rbx;
                    regs.rcx = vmx.guest_gprs.rcx;
                    regs.rdx = vmx.guest_gprs.rdx;
                    regs.rsi = vmx.guest_gprs.rsi;
                    regs.rdi = vmx.guest_gprs.rdi;
                    regs.rbp = vmx.guest_gprs.rbp;
                    regs.r8 = vmx.guest_gprs.r8;
                    regs.r9 = vmx.guest_gprs.r9;
                    regs.r10 = vmx.guest_gprs.r10;
                    regs.r11 = vmx.guest_gprs.r11;
                    regs.r12 = vmx.guest_gprs.r12;
                    regs.r13 = vmx.guest_gprs.r13;
                    regs.r14 = vmx.guest_gprs.r14;
                    regs.r15 = vmx.guest_gprs.r15;

                    // Segment registers
                    regs.cs = SegmentState {
                        selector: vmread(vmx::VMCS_GUEST_CS_SELECTOR).unwrap_or(0) as u16,
                        base: vmread(vmx::VMCS_GUEST_CS_BASE).unwrap_or(0),
                        limit: vmread(vmx::VMCS_GUEST_CS_LIMIT).unwrap_or(0) as u32,
                        access_rights: vmread(vmx::VMCS_GUEST_CS_ACCESS_RIGHTS).unwrap_or(0) as u32,
                    };
                    regs.ds = SegmentState {
                        selector: vmread(vmx::VMCS_GUEST_DS_SELECTOR).unwrap_or(0) as u16,
                        base: vmread(vmx::VMCS_GUEST_DS_BASE).unwrap_or(0),
                        limit: vmread(vmx::VMCS_GUEST_DS_LIMIT).unwrap_or(0) as u32,
                        access_rights: vmread(vmx::VMCS_GUEST_DS_ACCESS_RIGHTS).unwrap_or(0) as u32,
                    };
                    regs.es = SegmentState {
                        selector: vmread(vmx::VMCS_GUEST_ES_SELECTOR).unwrap_or(0) as u16,
                        base: vmread(vmx::VMCS_GUEST_ES_BASE).unwrap_or(0),
                        limit: vmread(vmx::VMCS_GUEST_ES_LIMIT).unwrap_or(0) as u32,
                        access_rights: vmread(vmx::VMCS_GUEST_ES_ACCESS_RIGHTS).unwrap_or(0) as u32,
                    };
                    regs.ss = SegmentState {
                        selector: vmread(vmx::VMCS_GUEST_SS_SELECTOR).unwrap_or(0) as u16,
                        base: vmread(vmx::VMCS_GUEST_SS_BASE).unwrap_or(0),
                        limit: vmread(vmx::VMCS_GUEST_SS_LIMIT).unwrap_or(0) as u32,
                        access_rights: vmread(vmx::VMCS_GUEST_SS_ACCESS_RIGHTS).unwrap_or(0) as u32,
                    };
                    regs.fs = SegmentState {
                        selector: vmread(vmx::VMCS_GUEST_FS_SELECTOR).unwrap_or(0) as u16,
                        base: vmread(vmx::VMCS_GUEST_FS_BASE).unwrap_or(0),
                        limit: vmread(vmx::VMCS_GUEST_FS_LIMIT).unwrap_or(0) as u32,
                        access_rights: vmread(vmx::VMCS_GUEST_FS_ACCESS_RIGHTS).unwrap_or(0) as u32,
                    };
                    regs.gs = SegmentState {
                        selector: vmread(vmx::VMCS_GUEST_GS_SELECTOR).unwrap_or(0) as u16,
                        base: vmread(vmx::VMCS_GUEST_GS_BASE).unwrap_or(0),
                        limit: vmread(vmx::VMCS_GUEST_GS_LIMIT).unwrap_or(0) as u32,
                        access_rights: vmread(vmx::VMCS_GUEST_GS_ACCESS_RIGHTS).unwrap_or(0) as u32,
                    };
                    regs.tr = SegmentState {
                        selector: vmread(vmx::VMCS_GUEST_TR_SELECTOR).unwrap_or(0) as u16,
                        base: vmread(vmx::VMCS_GUEST_TR_BASE).unwrap_or(0),
                        limit: vmread(vmx::VMCS_GUEST_TR_LIMIT).unwrap_or(0) as u32,
                        access_rights: vmread(vmx::VMCS_GUEST_TR_ACCESS_RIGHTS).unwrap_or(0) as u32,
                    };
                    regs.ldtr = SegmentState {
                        selector: vmread(vmx::VMCS_GUEST_LDTR_SELECTOR).unwrap_or(0) as u16,
                        base: vmread(vmx::VMCS_GUEST_LDTR_BASE).unwrap_or(0),
                        limit: vmread(vmx::VMCS_GUEST_LDTR_LIMIT).unwrap_or(0) as u32,
                        access_rights: vmread(vmx::VMCS_GUEST_LDTR_ACCESS_RIGHTS).unwrap_or(0) as u32,
                    };

                    // Descriptor tables
                    regs.gdtr = DtableState {
                        base: vmread(vmx::VMCS_GUEST_GDTR_BASE).unwrap_or(0),
                        limit: vmread(vmx::VMCS_GUEST_GDTR_LIMIT).unwrap_or(0) as u16,
                    };
                    regs.idtr = DtableState {
                        base: vmread(vmx::VMCS_GUEST_IDTR_BASE).unwrap_or(0),
                        limit: vmread(vmx::VMCS_GUEST_IDTR_LIMIT).unwrap_or(0) as u16,
                    };

                    // Control registers
                    regs.cr0 = vmread(vmx::VMCS_GUEST_CR0).unwrap_or(0);
                    regs.cr3 = vmread(vmx::VMCS_GUEST_CR3).unwrap_or(0);
                    regs.cr4 = vmread(vmx::VMCS_GUEST_CR4).unwrap_or(0);
                    regs.efer = vmread(vmx::VMCS_GUEST_EFER).unwrap_or(0);
                }
            }
            CpuVendor::Amd => {
                let svm = self.svm.as_ref().ok_or(HvError::Other("SVM not initialized"))?;
                let vmcb = &svm.vmcb;

                regs.rip = read_vmcb_u64(vmcb, vmcb_save::RIP);
                regs.rsp = read_vmcb_u64(vmcb, vmcb_save::RSP);
                regs.rflags = read_vmcb_u64(vmcb, vmcb_save::RFLAGS);
                regs.rax = read_vmcb_u64(vmcb, vmcb_save::RAX);

                regs.rbx = svm.guest_gprs.rbx;
                regs.rcx = svm.guest_gprs.rcx;
                regs.rdx = svm.guest_gprs.rdx;
                regs.rsi = svm.guest_gprs.rsi;
                regs.rdi = svm.guest_gprs.rdi;
                regs.rbp = svm.guest_gprs.rbp;
                regs.r8 = svm.guest_gprs.r8;
                regs.r9 = svm.guest_gprs.r9;
                regs.r10 = svm.guest_gprs.r10;
                regs.r11 = svm.guest_gprs.r11;
                regs.r12 = svm.guest_gprs.r12;
                regs.r13 = svm.guest_gprs.r13;
                regs.r14 = svm.guest_gprs.r14;
                regs.r15 = svm.guest_gprs.r15;

                regs.cs = SegmentState {
                    selector: read_vmcb_u16(vmcb, vmcb_save::CS_SELECTOR),
                    base: read_vmcb_u64(vmcb, vmcb_save::CS_BASE),
                    limit: read_vmcb_u32(vmcb, vmcb_save::CS_LIMIT),
                    access_rights: read_vmcb_u16(vmcb, vmcb_save::CS_ATTRIB) as u32,
                };
                regs.ds = SegmentState {
                    selector: read_vmcb_u16(vmcb, vmcb_save::DS_SELECTOR),
                    base: read_vmcb_u64(vmcb, vmcb_save::DS_BASE),
                    limit: read_vmcb_u32(vmcb, vmcb_save::DS_LIMIT),
                    access_rights: read_vmcb_u16(vmcb, vmcb_save::DS_ATTRIB) as u32,
                };
                regs.es = SegmentState {
                    selector: read_vmcb_u16(vmcb, vmcb_save::ES_SELECTOR),
                    base: read_vmcb_u64(vmcb, vmcb_save::ES_BASE),
                    limit: read_vmcb_u32(vmcb, vmcb_save::ES_LIMIT),
                    access_rights: read_vmcb_u16(vmcb, vmcb_save::ES_ATTRIB) as u32,
                };
                regs.ss = SegmentState {
                    selector: read_vmcb_u16(vmcb, vmcb_save::SS_SELECTOR),
                    base: read_vmcb_u64(vmcb, vmcb_save::SS_BASE),
                    limit: read_vmcb_u32(vmcb, vmcb_save::SS_LIMIT),
                    access_rights: read_vmcb_u16(vmcb, vmcb_save::SS_ATTRIB) as u32,
                };
                regs.fs = SegmentState {
                    selector: read_vmcb_u16(vmcb, vmcb_save::FS_SELECTOR),
                    base: read_vmcb_u64(vmcb, vmcb_save::FS_BASE),
                    limit: read_vmcb_u32(vmcb, vmcb_save::FS_LIMIT),
                    access_rights: read_vmcb_u16(vmcb, vmcb_save::FS_ATTRIB) as u32,
                };
                regs.gs = SegmentState {
                    selector: read_vmcb_u16(vmcb, vmcb_save::GS_SELECTOR),
                    base: read_vmcb_u64(vmcb, vmcb_save::GS_BASE),
                    limit: read_vmcb_u32(vmcb, vmcb_save::GS_LIMIT),
                    access_rights: read_vmcb_u16(vmcb, vmcb_save::GS_ATTRIB) as u32,
                };
                regs.tr = SegmentState {
                    selector: read_vmcb_u16(vmcb, vmcb_save::TR_SELECTOR),
                    base: read_vmcb_u64(vmcb, vmcb_save::TR_BASE),
                    limit: read_vmcb_u32(vmcb, vmcb_save::TR_LIMIT),
                    access_rights: read_vmcb_u16(vmcb, vmcb_save::TR_ATTRIB) as u32,
                };
                regs.ldtr = SegmentState {
                    selector: read_vmcb_u16(vmcb, vmcb_save::LDTR_SELECTOR),
                    base: read_vmcb_u64(vmcb, vmcb_save::LDTR_BASE),
                    limit: read_vmcb_u32(vmcb, vmcb_save::LDTR_LIMIT),
                    access_rights: read_vmcb_u16(vmcb, vmcb_save::LDTR_ATTRIB) as u32,
                };
                regs.gdtr = DtableState {
                    base: read_vmcb_u64(vmcb, vmcb_save::GDTR_BASE),
                    limit: read_vmcb_u32(vmcb, vmcb_save::GDTR_LIMIT) as u16,
                };
                regs.idtr = DtableState {
                    base: read_vmcb_u64(vmcb, vmcb_save::IDTR_BASE),
                    limit: read_vmcb_u32(vmcb, vmcb_save::IDTR_LIMIT) as u16,
                };

                regs.cr0 = read_vmcb_u64(vmcb, vmcb_save::CR0);
                regs.cr2 = read_vmcb_u64(vmcb, vmcb_save::CR2);
                regs.cr3 = read_vmcb_u64(vmcb, vmcb_save::CR3);
                regs.cr4 = read_vmcb_u64(vmcb, vmcb_save::CR4);
                regs.efer = read_vmcb_u64(vmcb, vmcb_save::EFER);
                regs.pat = read_vmcb_u64(vmcb, vmcb_save::PAT);
                regs.star = read_vmcb_u64(vmcb, vmcb_save::STAR);
                regs.lstar = read_vmcb_u64(vmcb, vmcb_save::LSTAR);
                regs.cstar = read_vmcb_u64(vmcb, vmcb_save::CSTAR);
                regs.sfmask = read_vmcb_u64(vmcb, vmcb_save::SFMASK);
                regs.kernel_gs_base = read_vmcb_u64(vmcb, vmcb_save::KERNEL_GS_BASE);
                regs.sysenter_cs = read_vmcb_u64(vmcb, vmcb_save::SYSENTER_CS);
                regs.sysenter_esp = read_vmcb_u64(vmcb, vmcb_save::SYSENTER_ESP);
                regs.sysenter_eip = read_vmcb_u64(vmcb, vmcb_save::SYSENTER_EIP);
            }
            CpuVendor::Unknown => return Err(HvError::NotSupported),
        }
        Ok(regs)
    }

    fn set_regs(&mut self, _vcpu_id: u32, regs: &VcpuRegs) -> Result<(), HvError> {
        match self.vendor {
            CpuVendor::Intel => {
                let vmx = self.vmx.as_mut().ok_or(HvError::Other("VMX not initialized"))?;
                unsafe {
                    vmwrite(vmx::VMCS_GUEST_RIP, regs.rip);
                    vmwrite(vmx::VMCS_GUEST_RSP, regs.rsp);
                    vmwrite(vmx::VMCS_GUEST_RFLAGS, regs.rflags);

                    vmx.guest_gprs.rax = regs.rax;
                    vmx.guest_gprs.rbx = regs.rbx;
                    vmx.guest_gprs.rcx = regs.rcx;
                    vmx.guest_gprs.rdx = regs.rdx;
                    vmx.guest_gprs.rsi = regs.rsi;
                    vmx.guest_gprs.rdi = regs.rdi;
                    vmx.guest_gprs.rbp = regs.rbp;
                    vmx.guest_gprs.r8 = regs.r8;
                    vmx.guest_gprs.r9 = regs.r9;
                    vmx.guest_gprs.r10 = regs.r10;
                    vmx.guest_gprs.r11 = regs.r11;
                    vmx.guest_gprs.r12 = regs.r12;
                    vmx.guest_gprs.r13 = regs.r13;
                    vmx.guest_gprs.r14 = regs.r14;
                    vmx.guest_gprs.r15 = regs.r15;

                    // Segment registers
                    vmwrite(vmx::VMCS_GUEST_CS_SELECTOR, regs.cs.selector as u64);
                    vmwrite(vmx::VMCS_GUEST_CS_BASE, regs.cs.base);
                    vmwrite(vmx::VMCS_GUEST_CS_LIMIT, regs.cs.limit as u64);
                    vmwrite(vmx::VMCS_GUEST_CS_ACCESS_RIGHTS, regs.cs.access_rights as u64);

                    vmwrite(vmx::VMCS_GUEST_DS_SELECTOR, regs.ds.selector as u64);
                    vmwrite(vmx::VMCS_GUEST_DS_BASE, regs.ds.base);
                    vmwrite(vmx::VMCS_GUEST_DS_LIMIT, regs.ds.limit as u64);
                    vmwrite(vmx::VMCS_GUEST_DS_ACCESS_RIGHTS, regs.ds.access_rights as u64);

                    vmwrite(vmx::VMCS_GUEST_ES_SELECTOR, regs.es.selector as u64);
                    vmwrite(vmx::VMCS_GUEST_ES_BASE, regs.es.base);
                    vmwrite(vmx::VMCS_GUEST_ES_LIMIT, regs.es.limit as u64);
                    vmwrite(vmx::VMCS_GUEST_ES_ACCESS_RIGHTS, regs.es.access_rights as u64);

                    vmwrite(vmx::VMCS_GUEST_SS_SELECTOR, regs.ss.selector as u64);
                    vmwrite(vmx::VMCS_GUEST_SS_BASE, regs.ss.base);
                    vmwrite(vmx::VMCS_GUEST_SS_LIMIT, regs.ss.limit as u64);
                    vmwrite(vmx::VMCS_GUEST_SS_ACCESS_RIGHTS, regs.ss.access_rights as u64);

                    vmwrite(vmx::VMCS_GUEST_FS_SELECTOR, regs.fs.selector as u64);
                    vmwrite(vmx::VMCS_GUEST_FS_BASE, regs.fs.base);
                    vmwrite(vmx::VMCS_GUEST_FS_LIMIT, regs.fs.limit as u64);
                    vmwrite(vmx::VMCS_GUEST_FS_ACCESS_RIGHTS, regs.fs.access_rights as u64);

                    vmwrite(vmx::VMCS_GUEST_GS_SELECTOR, regs.gs.selector as u64);
                    vmwrite(vmx::VMCS_GUEST_GS_BASE, regs.gs.base);
                    vmwrite(vmx::VMCS_GUEST_GS_LIMIT, regs.gs.limit as u64);
                    vmwrite(vmx::VMCS_GUEST_GS_ACCESS_RIGHTS, regs.gs.access_rights as u64);

                    vmwrite(vmx::VMCS_GUEST_TR_SELECTOR, regs.tr.selector as u64);
                    vmwrite(vmx::VMCS_GUEST_TR_BASE, regs.tr.base);
                    vmwrite(vmx::VMCS_GUEST_TR_LIMIT, regs.tr.limit as u64);
                    vmwrite(vmx::VMCS_GUEST_TR_ACCESS_RIGHTS, regs.tr.access_rights as u64);

                    vmwrite(vmx::VMCS_GUEST_LDTR_SELECTOR, regs.ldtr.selector as u64);
                    vmwrite(vmx::VMCS_GUEST_LDTR_BASE, regs.ldtr.base);
                    vmwrite(vmx::VMCS_GUEST_LDTR_LIMIT, regs.ldtr.limit as u64);
                    vmwrite(vmx::VMCS_GUEST_LDTR_ACCESS_RIGHTS, regs.ldtr.access_rights as u64);

                    vmwrite(vmx::VMCS_GUEST_GDTR_BASE, regs.gdtr.base);
                    vmwrite(vmx::VMCS_GUEST_GDTR_LIMIT, regs.gdtr.limit as u64);
                    vmwrite(vmx::VMCS_GUEST_IDTR_BASE, regs.idtr.base);
                    vmwrite(vmx::VMCS_GUEST_IDTR_LIMIT, regs.idtr.limit as u64);

                    vmwrite(vmx::VMCS_GUEST_CR0, regs.cr0);
                    vmwrite(vmx::VMCS_GUEST_CR3, regs.cr3);
                    vmwrite(vmx::VMCS_GUEST_CR4, regs.cr4);
                    vmwrite(vmx::VMCS_GUEST_EFER, regs.efer);
                }
                Ok(())
            }
            CpuVendor::Amd => {
                let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;
                let vmcb = &mut svm.vmcb;

                write_vmcb_u64(vmcb, vmcb_save::RIP, regs.rip);
                write_vmcb_u64(vmcb, vmcb_save::RSP, regs.rsp);
                write_vmcb_u64(vmcb, vmcb_save::RFLAGS, regs.rflags);
                write_vmcb_u64(vmcb, vmcb_save::RAX, regs.rax);

                svm.guest_gprs.rbx = regs.rbx;
                svm.guest_gprs.rcx = regs.rcx;
                svm.guest_gprs.rdx = regs.rdx;
                svm.guest_gprs.rsi = regs.rsi;
                svm.guest_gprs.rdi = regs.rdi;
                svm.guest_gprs.rbp = regs.rbp;
                svm.guest_gprs.r8 = regs.r8;
                svm.guest_gprs.r9 = regs.r9;
                svm.guest_gprs.r10 = regs.r10;
                svm.guest_gprs.r11 = regs.r11;
                svm.guest_gprs.r12 = regs.r12;
                svm.guest_gprs.r13 = regs.r13;
                svm.guest_gprs.r14 = regs.r14;
                svm.guest_gprs.r15 = regs.r15;

                // Segments
                write_vmcb_u16(vmcb, vmcb_save::CS_SELECTOR, regs.cs.selector);
                write_vmcb_u64(vmcb, vmcb_save::CS_BASE, regs.cs.base);
                write_vmcb_u32(vmcb, vmcb_save::CS_LIMIT, regs.cs.limit);
                write_vmcb_u16(vmcb, vmcb_save::CS_ATTRIB, regs.cs.access_rights as u16);

                write_vmcb_u16(vmcb, vmcb_save::DS_SELECTOR, regs.ds.selector);
                write_vmcb_u64(vmcb, vmcb_save::DS_BASE, regs.ds.base);
                write_vmcb_u32(vmcb, vmcb_save::DS_LIMIT, regs.ds.limit);
                write_vmcb_u16(vmcb, vmcb_save::DS_ATTRIB, regs.ds.access_rights as u16);

                write_vmcb_u16(vmcb, vmcb_save::ES_SELECTOR, regs.es.selector);
                write_vmcb_u64(vmcb, vmcb_save::ES_BASE, regs.es.base);
                write_vmcb_u32(vmcb, vmcb_save::ES_LIMIT, regs.es.limit);
                write_vmcb_u16(vmcb, vmcb_save::ES_ATTRIB, regs.es.access_rights as u16);

                write_vmcb_u16(vmcb, vmcb_save::SS_SELECTOR, regs.ss.selector);
                write_vmcb_u64(vmcb, vmcb_save::SS_BASE, regs.ss.base);
                write_vmcb_u32(vmcb, vmcb_save::SS_LIMIT, regs.ss.limit);
                write_vmcb_u16(vmcb, vmcb_save::SS_ATTRIB, regs.ss.access_rights as u16);

                write_vmcb_u16(vmcb, vmcb_save::FS_SELECTOR, regs.fs.selector);
                write_vmcb_u64(vmcb, vmcb_save::FS_BASE, regs.fs.base);
                write_vmcb_u32(vmcb, vmcb_save::FS_LIMIT, regs.fs.limit);
                write_vmcb_u16(vmcb, vmcb_save::FS_ATTRIB, regs.fs.access_rights as u16);

                write_vmcb_u16(vmcb, vmcb_save::GS_SELECTOR, regs.gs.selector);
                write_vmcb_u64(vmcb, vmcb_save::GS_BASE, regs.gs.base);
                write_vmcb_u32(vmcb, vmcb_save::GS_LIMIT, regs.gs.limit);
                write_vmcb_u16(vmcb, vmcb_save::GS_ATTRIB, regs.gs.access_rights as u16);

                write_vmcb_u16(vmcb, vmcb_save::TR_SELECTOR, regs.tr.selector);
                write_vmcb_u64(vmcb, vmcb_save::TR_BASE, regs.tr.base);
                write_vmcb_u32(vmcb, vmcb_save::TR_LIMIT, regs.tr.limit);
                write_vmcb_u16(vmcb, vmcb_save::TR_ATTRIB, regs.tr.access_rights as u16);

                write_vmcb_u16(vmcb, vmcb_save::LDTR_SELECTOR, regs.ldtr.selector);
                write_vmcb_u64(vmcb, vmcb_save::LDTR_BASE, regs.ldtr.base);
                write_vmcb_u32(vmcb, vmcb_save::LDTR_LIMIT, regs.ldtr.limit);
                write_vmcb_u16(vmcb, vmcb_save::LDTR_ATTRIB, regs.ldtr.access_rights as u16);

                write_vmcb_u64(vmcb, vmcb_save::GDTR_BASE, regs.gdtr.base);
                write_vmcb_u32(vmcb, vmcb_save::GDTR_LIMIT, regs.gdtr.limit as u32);
                write_vmcb_u64(vmcb, vmcb_save::IDTR_BASE, regs.idtr.base);
                write_vmcb_u32(vmcb, vmcb_save::IDTR_LIMIT, regs.idtr.limit as u32);

                write_vmcb_u64(vmcb, vmcb_save::CR0, regs.cr0);
                write_vmcb_u64(vmcb, vmcb_save::CR2, regs.cr2);
                write_vmcb_u64(vmcb, vmcb_save::CR3, regs.cr3);
                write_vmcb_u64(vmcb, vmcb_save::CR4, regs.cr4);
                write_vmcb_u64(vmcb, vmcb_save::EFER, regs.efer);
                write_vmcb_u64(vmcb, vmcb_save::PAT, regs.pat);
                write_vmcb_u64(vmcb, vmcb_save::STAR, regs.star);
                write_vmcb_u64(vmcb, vmcb_save::LSTAR, regs.lstar);
                write_vmcb_u64(vmcb, vmcb_save::CSTAR, regs.cstar);
                write_vmcb_u64(vmcb, vmcb_save::SFMASK, regs.sfmask);
                write_vmcb_u64(vmcb, vmcb_save::KERNEL_GS_BASE, regs.kernel_gs_base);
                write_vmcb_u64(vmcb, vmcb_save::SYSENTER_CS, regs.sysenter_cs);
                write_vmcb_u64(vmcb, vmcb_save::SYSENTER_ESP, regs.sysenter_esp);
                write_vmcb_u64(vmcb, vmcb_save::SYSENTER_EIP, regs.sysenter_eip);

                Ok(())
            }
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn run(&mut self, _vcpu_id: u32) -> Result<VmExit, HvError> {
        match self.vendor {
            CpuVendor::Intel => self.vmx_run(),
            CpuVendor::Amd => self.svm_run(),
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn request_interrupt_window(&mut self, _vcpu_id: u32) -> Result<(), HvError> {
        match self.vendor {
            CpuVendor::Intel => {
                let vmx = self.vmx.as_mut().ok_or(HvError::Other("VMX not initialized"))?;
                vmx.int_window_requested = true;
                // Set interrupt-window exiting in proc-based controls
                unsafe {
                    let ctrl = vmread(vmx::VMCS_PROC_BASED_CONTROLS).unwrap_or(0) as u32;
                    vmwrite(vmx::VMCS_PROC_BASED_CONTROLS, (ctrl | (1 << 2)) as u64);
                }
                Ok(())
            }
            CpuVendor::Amd => {
                let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;
                svm.int_window_requested = true;
                // Set V_INTR_MASKING and virtual interrupt in VMCB
                let misc1 = read_vmcb_u32(&svm.vmcb, vmcb_ctrl::INTERCEPT_MISC1);
                write_vmcb_u32(&mut svm.vmcb, vmcb_ctrl::INTERCEPT_MISC1, misc1 | (1 << 4)); // VINTR intercept
                Ok(())
            }
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn inject_interrupt(&mut self, _vcpu_id: u32, vector: u8) -> Result<(), HvError> {
        match self.vendor {
            CpuVendor::Intel => {
                unsafe {
                    // VM-entry interruption-information field:
                    // bits 7:0 = vector, bits 10:8 = type (0=ext int), bit 31 = valid
                    let info = (vector as u64) | (0 << 8) | (1 << 31);
                    vmwrite(vmx::VMCS_ENTRY_INTERRUPTION_INFO, info);
                }
                Ok(())
            }
            CpuVendor::Amd => {
                let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;
                // EVENTINJ: bits 7:0 = vector, bits 10:8 = type (0=ext int), bit 31 = valid
                let event = (vector as u64) | (0 << 8) | (1 << 31);
                write_vmcb_u64(&mut svm.vmcb, vmcb_ctrl::EVENT_INJ, event);
                Ok(())
            }
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn interrupts_enabled(&self, _vcpu_id: u32) -> Result<bool, HvError> {
        match self.vendor {
            CpuVendor::Intel => unsafe {
                let rflags = vmread(vmx::VMCS_GUEST_RFLAGS).unwrap_or(0);
                let interruptibility = vmread(vmx::VMCS_GUEST_INTERRUPTIBILITY_STATE).unwrap_or(0);
                // IF flag set and no blocking by STI/MOV SS
                Ok((rflags & (1 << 9)) \!= 0 && (interruptibility & 0x03) == 0)
            },
            CpuVendor::Amd => {
                let svm = self.svm.as_ref().ok_or(HvError::Other("SVM not initialized"))?;
                let rflags = read_vmcb_u64(&svm.vmcb, vmcb_save::RFLAGS);
                Ok((rflags & (1 << 9)) \!= 0)
            }
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn set_io_in_data(&mut self, _vcpu_id: u32, data: u32, size: u8) -> Result<(), HvError> {
        match self.vendor {
            CpuVendor::Intel => {
                let vmx = self.vmx.as_mut().ok_or(HvError::Other("VMX not initialized"))?;
                vmx.pending_io_data = Some((data, size));
                Ok(())
            }
            CpuVendor::Amd => {
                let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;
                svm.pending_io_data = Some((data, size));
                Ok(())
            }
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn set_mmio_read_data(&mut self, _vcpu_id: u32, data: u64, size: u8) -> Result<(), HvError> {
        match self.vendor {
            CpuVendor::Intel => {
                let vmx = self.vmx.as_mut().ok_or(HvError::Other("VMX not initialized"))?;
                vmx.pending_mmio_data = Some((data, size));
                Ok(())
            }
            CpuVendor::Amd => {
                let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;
                svm.pending_mmio_data = Some((data, size));
                Ok(())
            }
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn set_cpuid_response(&mut self, _vcpu_id: u32, eax: u32, ebx: u32, ecx: u32, edx: u32) -> Result<(), HvError> {
        match self.vendor {
            CpuVendor::Intel => {
                let vmx = self.vmx.as_mut().ok_or(HvError::Other("VMX not initialized"))?;
                vmx.pending_cpuid = Some((eax, ebx, ecx, edx));
                Ok(())
            }
            CpuVendor::Amd => {
                let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;
                svm.pending_cpuid = Some((eax, ebx, ecx, edx));
                Ok(())
            }
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn set_msr_read_data(&mut self, _vcpu_id: u32, value: u64) -> Result<(), HvError> {
        match self.vendor {
            CpuVendor::Intel => {
                let vmx = self.vmx.as_mut().ok_or(HvError::Other("VMX not initialized"))?;
                vmx.pending_msr_data = Some(value);
                Ok(())
            }
            CpuVendor::Amd => {
                let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;
                svm.pending_msr_data = Some(value);
                Ok(())
            }
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn advance_rip(&mut self, _vcpu_id: u32, len: u8) -> Result<(), HvError> {
        match self.vendor {
            CpuVendor::Intel => unsafe {
                let rip = vmread(vmx::VMCS_GUEST_RIP).unwrap_or(0);
                vmwrite(vmx::VMCS_GUEST_RIP, rip + len as u64);
                Ok(())
            },
            CpuVendor::Amd => {
                let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;
                // SVM provides NRIP (next RIP) in the VMCB at offset 0xC8
                let next_rip = read_vmcb_u64(&svm.vmcb, vmcb_ctrl::NEXT_RIP);
                if next_rip \!= 0 {
                    write_vmcb_u64(&mut svm.vmcb, vmcb_save::RIP, next_rip);
                } else {
                    let rip = read_vmcb_u64(&svm.vmcb, vmcb_save::RIP);
                    write_vmcb_u64(&mut svm.vmcb, vmcb_save::RIP, rip + len as u64);
                }
                Ok(())
            }
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn request_stop(&mut self, _vcpu_id: u32) -> Result<(), HvError> {
        match self.vendor {
            CpuVendor::Intel => {
                let vmx = self.vmx.as_mut().ok_or(HvError::Other("VMX not initialized"))?;
                vmx.stop_requested = true;
                Ok(())
            }
            CpuVendor::Amd => {
                let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;
                svm.stop_requested = true;
                Ok(())
            }
            CpuVendor::Unknown => Err(HvError::NotSupported),
        }
    }

    fn destroy(&mut self) {
        match self.vendor {
            CpuVendor::Intel => {
                if self.vmx.is_some() {
                    unsafe {
                        vmxoff();
                        // Clear VMXE in CR4
                        let cr4 = read_cr4();
                        write_cr4(cr4 & \!(1 << 13));
                    }
                    self.vmx = None;
                }
            }
            CpuVendor::Amd => {
                if self.svm.is_some() {
                    unsafe {
                        // Disable SVM in EFER
                        let efer = rdmsr(0xC000_0080);
                        wrmsr(0xC000_0080, efer & \!(1 << 12));
                    }
                    self.svm = None;
                }
            }
            CpuVendor::Unknown => {}
        }
    }
}

// ─── VMX/SVM run implementations ───────────────────────────────────────────

impl BareMetalBackend {
    fn vmx_run(&mut self) -> Result<VmExit, HvError> {
        let vmx = self.vmx.as_mut().ok_or(HvError::Other("VMX not initialized"))?;

        // Check for stop request
        if vmx.stop_requested {
            vmx.stop_requested = false;
            return Ok(VmExit::StopRequested);
        }

        // Apply pending IO in data (set RAX with the result)
        if let Some((data, size)) = vmx.pending_io_data.take() {
            let mask = match size {
                1 => 0xFF,
                2 => 0xFFFF,
                4 => 0xFFFF_FFFF,
                _ => 0xFFFF_FFFF,
            };
            vmx.guest_gprs.rax = (vmx.guest_gprs.rax & \!mask) | (data as u64 & mask);
        }

        // Apply pending CPUID response
        if let Some((eax, ebx, ecx, edx)) = vmx.pending_cpuid.take() {
            vmx.guest_gprs.rax = eax as u64;
            vmx.guest_gprs.rbx = ebx as u64;
            vmx.guest_gprs.rcx = ecx as u64;
            vmx.guest_gprs.rdx = edx as u64;
        }

        // Apply pending MSR read data
        if let Some(value) = vmx.pending_msr_data.take() {
            vmx.guest_gprs.rax = value & 0xFFFF_FFFF;
            vmx.guest_gprs.rdx = value >> 32;
        }

        // Apply pending MMIO read (set RAX)
        if let Some((data, size)) = vmx.pending_mmio_data.take() {
            let mask = match size {
                1 => 0xFF,
                2 => 0xFFFF,
                4 => 0xFFFF_FFFF,
                8 => 0xFFFF_FFFF_FFFF_FFFF,
                _ => 0xFFFF_FFFF,
            };
            vmx.guest_gprs.rax = data & mask;
        }

        // Set host RSP/RIP for VM-exit return
        // We use the current stack pointer and a label after VMLAUNCH/VMRESUME
        let gprs = &mut vmx.guest_gprs as *mut GuestGprs;
        let launched = vmx.launched;

        unsafe {
            let exit_reason: u32;

            // Save host GPRs, load guest GPRs, execute VMLAUNCH/VMRESUME,
            // on VM-exit save guest GPRs, restore host GPRs
            core::arch::asm\!(
                // Save host callee-saved registers
                "push rbx",
                "push rbp",
                "push r12",
                "push r13",
                "push r14",
                "push r15",

                // Load guest GPRs from GuestGprs struct
                "mov rax, [{gprs} + 0]",     // rax
                "mov rbx, [{gprs} + 8]",     // rbx
                "mov rcx, [{gprs} + 16]",    // rcx
                "mov rdx, [{gprs} + 24]",    // rdx
                "mov rsi, [{gprs} + 32]",    // rsi
                "mov rdi, [{gprs} + 40]",    // rdi
                "mov rbp, [{gprs} + 48]",    // rbp
                "mov r8,  [{gprs} + 56]",    // r8
                "mov r9,  [{gprs} + 64]",    // r9
                "mov r10, [{gprs} + 72]",    // r10
                "mov r11, [{gprs} + 80]",    // r11
                "mov r12, [{gprs} + 88]",    // r12
                "mov r13, [{gprs} + 96]",    // r13
                "mov r14, [{gprs} + 104]",   // r14
                "mov r15, [{gprs} + 112]",   // r15

                // VMLAUNCH or VMRESUME
                "test {launched:e}, {launched:e}",
                "jnz 2f",
                "vmlaunch",
                "jmp 3f",
                "2:",
                "vmresume",
                "3:",

                // VM-exit: save guest GPRs back
                // We need to save gprs pointer first — it was clobbered
                // Actually, {gprs} is still on stack from push, use a different approach
                // The gprs pointer is passed in a register that was saved
                gprs = in(reg) gprs,
                launched = in(reg) launched as u64,
                options(nostack),
            );

            // After VM-exit, read exit reason from VMCS
            exit_reason = vmread(vmx::VMCS_EXIT_REASON).unwrap_or(0xFFFF) as u32;

            // Mark as launched for subsequent runs
            let vmx = self.vmx.as_mut().unwrap();
            vmx.launched = true;

            // Clear interrupt-window exiting if it was set
            if vmx.int_window_requested {
                let ctrl = vmread(vmx::VMCS_PROC_BASED_CONTROLS).unwrap_or(0) as u32;
                vmwrite(vmx::VMCS_PROC_BASED_CONTROLS, (ctrl & \!(1 << 2)) as u64);
                vmx.int_window_requested = false;
            }

            // Decode exit reason
            let basic_reason = exit_reason & 0xFFFF;
            match basic_reason {
                // External interrupt
                1 => Ok(VmExit::InterruptWindow),
                // Triple fault
                2 => Ok(VmExit::Shutdown),
                // CPUID
                10 => {
                    let eax = vmx.guest_gprs.rax as u32;
                    let ecx = vmx.guest_gprs.rcx as u32;
                    Ok(VmExit::Cpuid { eax, ecx })
                }
                // HLT
                12 => Ok(VmExit::Hlt),
                // RDMSR
                31 => {
                    let index = vmx.guest_gprs.rcx as u32;
                    Ok(VmExit::MsrRead { index })
                }
                // WRMSR
                32 => {
                    let index = vmx.guest_gprs.rcx as u32;
                    let value = (vmx.guest_gprs.rdx << 32) | (vmx.guest_gprs.rax & 0xFFFF_FFFF);
                    Ok(VmExit::MsrWrite { index, value })
                }
                // I/O instruction
                30 => {
                    let qual = vmread(vmx::VMCS_EXIT_QUALIFICATION).unwrap_or(0);
                    let size = ((qual & 0x07) + 1) as u8;
                    let is_in = (qual & (1 << 3)) \!= 0;
                    let port = ((qual >> 16) & 0xFFFF) as u16;
                    if is_in {
                        Ok(VmExit::IoIn { port, size })
                    } else {
                        let data = vmx.guest_gprs.rax as u32;
                        Ok(VmExit::IoOut { port, size, data })
                    }
                }
                // EPT violation
                48 => {
                    let qual = vmread(vmx::VMCS_EXIT_QUALIFICATION).unwrap_or(0);
                    let guest_phys = vmread(vmx::VMCS_GUEST_PHYSICAL_ADDRESS).unwrap_or(0);
                    let is_write = (qual & 0x02) \!= 0;
                    Ok(VmExit::EptViolation { guest_phys, is_write })
                }
                // Interrupt window
                7 => Ok(VmExit::InterruptWindow),
                // Shutdown
                _ => Ok(VmExit::Unknown(basic_reason)),
            }
        }
    }

    fn svm_run(&mut self) -> Result<VmExit, HvError> {
        let svm = self.svm.as_mut().ok_or(HvError::Other("SVM not initialized"))?;

        // Check stop request
        if svm.stop_requested {
            svm.stop_requested = false;
            return Ok(VmExit::StopRequested);
        }

        // Apply pending IO data
        if let Some((data, size)) = svm.pending_io_data.take() {
            let mask = match size {
                1 => 0xFF,
                2 => 0xFFFF,
                4 => 0xFFFF_FFFF,
                _ => 0xFFFF_FFFF,
            };
            let rax = read_vmcb_u64(&svm.vmcb, vmcb_save::RAX);
            write_vmcb_u64(&mut svm.vmcb, vmcb_save::RAX, (rax & \!mask) | (data as u64 & mask));
        }

        // Apply pending CPUID
        if let Some((eax, ebx, ecx, edx)) = svm.pending_cpuid.take() {
            write_vmcb_u64(&mut svm.vmcb, vmcb_save::RAX, eax as u64);
            svm.guest_gprs.rbx = ebx as u64;
            svm.guest_gprs.rcx = ecx as u64;
            svm.guest_gprs.rdx = edx as u64;
        }

        // Apply pending MSR read
        if let Some(value) = svm.pending_msr_data.take() {
            write_vmcb_u64(&mut svm.vmcb, vmcb_save::RAX, value & 0xFFFF_FFFF);
            svm.guest_gprs.rdx = value >> 32;
        }

        // Apply pending MMIO read
        if let Some((data, _size)) = svm.pending_mmio_data.take() {
            write_vmcb_u64(&mut svm.vmcb, vmcb_save::RAX, data);
        }

        let vmcb_phys = phys_addr(&svm.vmcb);

        unsafe {
            // Save host state, load guest GPRs, VMRUN, save guest GPRs
            let gprs = &mut svm.guest_gprs as *mut GuestGprs;

            // Load guest GPRs (except RAX which is in VMCB)
            core::arch::asm\!(
                "push rbx",
                "push rbp",
                "push r12",
                "push r13",
                "push r14",
                "push r15",

                "mov rbx, [{gprs} + 8]",
                "mov rcx, [{gprs} + 16]",
                "mov rdx, [{gprs} + 24]",
                "mov rsi, [{gprs} + 32]",
                "mov rdi, [{gprs} + 40]",
                "mov rbp, [{gprs} + 48]",
                "mov r8,  [{gprs} + 56]",
                "mov r9,  [{gprs} + 64]",
                "mov r10, [{gprs} + 72]",
                "mov r11, [{gprs} + 80]",
                "mov r12, [{gprs} + 88]",
                "mov r13, [{gprs} + 96]",
                "mov r14, [{gprs} + 104]",
                "mov r15, [{gprs} + 112]",

                // RAX = VMCB physical address for VMRUN
                "mov rax, {vmcb_phys}",
                "vmrun",

                // VM-exit: save guest GPRs back
                "mov [{gprs} + 8], rbx",
                "mov [{gprs} + 16], rcx",
                "mov [{gprs} + 24], rdx",
                "mov [{gprs} + 32], rsi",
                "mov [{gprs} + 40], rdi",
                "mov [{gprs} + 48], rbp",
                "mov [{gprs} + 56], r8",
                "mov [{gprs} + 64], r9",
                "mov [{gprs} + 72], r10",
                "mov [{gprs} + 80], r11",
                "mov [{gprs} + 88], r12",
                "mov [{gprs} + 96], r13",
                "mov [{gprs} + 104], r14",
                "mov [{gprs} + 112], r15",

                "pop r15",
                "pop r14",
                "pop r13",
                "pop r12",
                "pop rbp",
                "pop rbx",

                gprs = in(reg) gprs,
                vmcb_phys = in(reg) vmcb_phys,
                options(nostack),
            );
        }

        // Read exit code from VMCB
        let svm = self.svm.as_mut().unwrap();
        let exitcode = read_vmcb_u64(&svm.vmcb, vmcb_ctrl::EXITCODE);
        let exitinfo1 = read_vmcb_u64(&svm.vmcb, vmcb_ctrl::EXITINFO1);
        let exitinfo2 = read_vmcb_u64(&svm.vmcb, vmcb_ctrl::EXITINFO2);

        // Check interrupt window
        if svm.int_window_requested {
            let rflags = read_vmcb_u64(&svm.vmcb, vmcb_save::RFLAGS);
            if rflags & (1 << 9) \!= 0 {
                // Clear VINTR intercept
                let misc1 = read_vmcb_u32(&svm.vmcb, vmcb_ctrl::INTERCEPT_MISC1);
                write_vmcb_u32(&mut svm.vmcb, vmcb_ctrl::INTERCEPT_MISC1, misc1 & \!(1 << 4));
                svm.int_window_requested = false;
            }
        }

        // Decode SVM exit codes
        match exitcode {
            // VMEXIT_INTR (0x60)
            0x60 => Ok(VmExit::InterruptWindow),
            // VMEXIT_NMI (0x61)
            0x61 => Ok(VmExit::InterruptWindow),
            // VMEXIT_VINTR (0x64)
            0x64 => Ok(VmExit::InterruptWindow),
            // VMEXIT_CPUID (0x72)
            0x72 => {
                let eax = read_vmcb_u64(&svm.vmcb, vmcb_save::RAX) as u32;
                let ecx = svm.guest_gprs.rcx as u32;
                Ok(VmExit::Cpuid { eax, ecx })
            }
            // VMEXIT_HLT (0x78)
            0x78 => Ok(VmExit::Hlt),
            // VMEXIT_IOIO (0x7B)
            0x7B => {
                let info = exitinfo1;
                let is_in = (info & 1) \!= 0;
                let size = if info & (1 << 4) \!= 0 { 1u8 }
                    else if info & (1 << 5) \!= 0 { 2u8 }
                    else { 4u8 };
                let port = ((info >> 16) & 0xFFFF) as u16;
                if is_in {
                    Ok(VmExit::IoIn { port, size })
                } else {
                    let data = read_vmcb_u64(&svm.vmcb, vmcb_save::RAX) as u32;
                    Ok(VmExit::IoOut { port, size, data })
                }
            }
            // VMEXIT_MSR (0x7C)
            0x7C => {
                let is_write = exitinfo1 == 1;
                let index = svm.guest_gprs.rcx as u32;
                if is_write {
                    let rax = read_vmcb_u64(&svm.vmcb, vmcb_save::RAX);
                    let value = (svm.guest_gprs.rdx << 32) | (rax & 0xFFFF_FFFF);
                    Ok(VmExit::MsrWrite { index, value })
                } else {
                    Ok(VmExit::MsrRead { index })
                }
            }
            // VMEXIT_SHUTDOWN (0x7F)
            0x7F => Ok(VmExit::Shutdown),
            // VMEXIT_NPF (0x400 = 1024)
            0x400 => {
                let guest_phys = exitinfo2;
                let is_write = (exitinfo1 & (1 << 1)) \!= 0;
                // Check if this is MMIO (address outside RAM range)
                if guest_phys >= self.guest_mem_size || self.is_mmio_region(guest_phys) {
                    if is_write {
                        let data = read_vmcb_u64(&svm.vmcb, vmcb_save::RAX);
                        Ok(VmExit::MmioWrite { address: guest_phys, size: 4, data })
                    } else {
                        Ok(VmExit::MmioRead { address: guest_phys, size: 4 })
                    }
                } else {
                    Ok(VmExit::EptViolation { guest_phys, is_write })
                }
            }
            _ => Ok(VmExit::Unknown(exitcode as u32)),
        }
    }

    /// Check if a guest physical address is in an MMIO region (not backed by RAM).
    fn is_mmio_region(&self, addr: u64) -> bool {
        // Common MMIO ranges: LAPIC, IOAPIC, VGA, PCI config space
        matches\!(addr,
            0xFEE0_0000..=0xFEE0_0FFF | // LAPIC
            0xFEC0_0000..=0xFEC0_0FFF | // IOAPIC
            0xA0000..=0xBFFFF |          // VGA memory
            0xF000_0000..=0xFFFF_FFFF    // PCI / ROM area
        )
    }
}
