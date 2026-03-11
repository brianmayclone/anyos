//! AMD-V (SVM) constants, structures, and VMCB definitions.
//!
//! This module defines all hardware constants needed to program the
//! VMCB (Virtual Machine Control Block) and perform SVM operations
//! on AMD processors.

// ════════════════════════════════════════════════════════════════════════
// MSRs for SVM
// ════════════════════════════════════════════════════════════════════════

/// VM_CR MSR — controls SVM enable/disable.
pub const MSR_VM_CR: u32 = 0xC001_0114;
/// VM_HSAVE_PA MSR — physical address of host save area.
pub const MSR_VM_HSAVE_PA: u32 = 0xC001_0117;
/// IA32_EFER.SVME bit — must be set to enable SVM.
pub const EFER_SVME: u64 = 1 << 12;
/// VM_CR.SVMDIS — if set, SVM cannot be enabled.
pub const VM_CR_SVMDIS: u64 = 1 << 4;

// ════════════════════════════════════════════════════════════════════════
// VMCB Layout — Control Area (offset 0x000 - 0x3FF)
// ════════════════════════════════════════════════════════════════════════

/// Intercept CR reads (bits 0-15 for CR0-CR15).
pub const VMCB_CR_READ_INTERCEPTS: usize = 0x000;
/// Intercept CR writes (bits 0-15 for CR0-CR15).
pub const VMCB_CR_WRITE_INTERCEPTS: usize = 0x004;
/// Intercept DR reads.
pub const VMCB_DR_READ_INTERCEPTS: usize = 0x008;
/// Intercept DR writes.
pub const VMCB_DR_WRITE_INTERCEPTS: usize = 0x00C;
/// Intercept exception vectors (bitmap, bits 0-31 for vectors 0-31).
pub const VMCB_EXCEPTION_INTERCEPTS: usize = 0x010;
/// Miscellaneous intercepts (INTR, NMI, SMI, INIT, VINTR, etc.).
pub const VMCB_INTERCEPT_MISC1: usize = 0x014;
/// More intercepts (VMRUN, VMMCALL, VMLOAD, etc.).
pub const VMCB_INTERCEPT_MISC2: usize = 0x018;

// Bits in INTERCEPT_MISC1
pub const INTERCEPT_INTR: u32 = 1 << 0;
pub const INTERCEPT_NMI: u32 = 1 << 1;
pub const INTERCEPT_SMI: u32 = 1 << 2;
pub const INTERCEPT_INIT: u32 = 1 << 3;
pub const INTERCEPT_VINTR: u32 = 1 << 4;
pub const INTERCEPT_CR0_SEL_WRITE: u32 = 1 << 5;
pub const INTERCEPT_IDTR_READ: u32 = 1 << 6;
pub const INTERCEPT_GDTR_READ: u32 = 1 << 7;
pub const INTERCEPT_LDTR_READ: u32 = 1 << 8;
pub const INTERCEPT_TR_READ: u32 = 1 << 9;
pub const INTERCEPT_IDTR_WRITE: u32 = 1 << 10;
pub const INTERCEPT_GDTR_WRITE: u32 = 1 << 11;
pub const INTERCEPT_LDTR_WRITE: u32 = 1 << 12;
pub const INTERCEPT_TR_WRITE: u32 = 1 << 13;
pub const INTERCEPT_RDTSC: u32 = 1 << 14;
pub const INTERCEPT_RDPMC: u32 = 1 << 15;
pub const INTERCEPT_PUSHF: u32 = 1 << 16;
pub const INTERCEPT_POPF: u32 = 1 << 17;
pub const INTERCEPT_CPUID: u32 = 1 << 18;
pub const INTERCEPT_RSM: u32 = 1 << 19;
pub const INTERCEPT_IRET: u32 = 1 << 20;
pub const INTERCEPT_SWINT: u32 = 1 << 21;
pub const INTERCEPT_INVD: u32 = 1 << 22;
pub const INTERCEPT_PAUSE: u32 = 1 << 23;
pub const INTERCEPT_HLT: u32 = 1 << 24;
pub const INTERCEPT_INVLPG: u32 = 1 << 25;
pub const INTERCEPT_INVLPGA: u32 = 1 << 26;
pub const INTERCEPT_IOIO: u32 = 1 << 27;
pub const INTERCEPT_MSR: u32 = 1 << 28;
pub const INTERCEPT_TASK_SWITCH: u32 = 1 << 29;
pub const INTERCEPT_FERR_FREEZE: u32 = 1 << 30;
pub const INTERCEPT_SHUTDOWN: u32 = 1 << 31;

// Bits in INTERCEPT_MISC2
pub const INTERCEPT_VMRUN: u32 = 1 << 0;
pub const INTERCEPT_VMMCALL: u32 = 1 << 1;
pub const INTERCEPT_VMLOAD: u32 = 1 << 2;
pub const INTERCEPT_VMSAVE: u32 = 1 << 3;
pub const INTERCEPT_STGI: u32 = 1 << 4;
pub const INTERCEPT_CLGI: u32 = 1 << 5;
pub const INTERCEPT_SKINIT: u32 = 1 << 6;
pub const INTERCEPT_RDTSCP: u32 = 1 << 7;
pub const INTERCEPT_ICEBP: u32 = 1 << 8;
pub const INTERCEPT_WBINVD: u32 = 1 << 9;
pub const INTERCEPT_MONITOR: u32 = 1 << 10;
pub const INTERCEPT_MWAIT: u32 = 1 << 11;
pub const INTERCEPT_XSETBV: u32 = 1 << 13;

/// I/O permissions map physical address.
pub const VMCB_IOPM_BASE_PA: usize = 0x040;
/// MSR permissions map physical address.
pub const VMCB_MSRPM_BASE_PA: usize = 0x048;
/// TSC offset.
pub const VMCB_TSC_OFFSET: usize = 0x050;
/// Guest ASID (Address Space ID).
pub const VMCB_GUEST_ASID: usize = 0x058;
/// TLB control.
pub const VMCB_TLB_CONTROL: usize = 0x05C;
/// Virtual interrupt control (V_TPR, V_IRQ, V_INTR_VECTOR, etc.).
pub const VMCB_V_INTR: usize = 0x060;
/// Interrupt shadow.
pub const VMCB_INTERRUPT_SHADOW: usize = 0x068;
/// Exit code.
pub const VMCB_EXITCODE: usize = 0x070;
/// Exit info 1.
pub const VMCB_EXITINFO1: usize = 0x078;
/// Exit info 2.
pub const VMCB_EXITINFO2: usize = 0x080;
/// Exit interrupt info.
pub const VMCB_EXIT_INT_INFO: usize = 0x088;
/// Nested paging enable and other features.
pub const VMCB_NP_ENABLE: usize = 0x090;
/// Event injection.
pub const VMCB_EVENT_INJ: usize = 0x0A8;
/// Nested CR3 (nCR3) for nested page tables.
pub const VMCB_N_CR3: usize = 0x0B0;
/// Virtual VMLOAD/VMSAVE enable.
pub const VMCB_LBR_VIRT_ENABLE: usize = 0x0B8;
/// VMCB clean bits.
pub const VMCB_CLEAN_BITS: usize = 0x0C0;
/// Next RIP (saved by hardware on intercept).
pub const VMCB_NEXT_RIP: usize = 0x0C8;
/// Number of bytes fetched (for IOIO intercept).
pub const VMCB_NUM_BYTES_FETCHED: usize = 0x0D0;

// ════════════════════════════════════════════════════════════════════════
// VMCB Layout — State Save Area (offset 0x400 - 0x5FF)
// ════════════════════════════════════════════════════════════════════════

pub const VMCB_ES: usize = 0x400;
pub const VMCB_CS: usize = 0x410;
pub const VMCB_SS: usize = 0x420;
pub const VMCB_DS: usize = 0x430;
pub const VMCB_FS: usize = 0x440;
pub const VMCB_GS: usize = 0x450;
pub const VMCB_GDTR: usize = 0x460;
pub const VMCB_LDTR: usize = 0x470;
pub const VMCB_IDTR: usize = 0x480;
pub const VMCB_TR: usize = 0x490;
// Each segment entry is 16 bytes: selector(2) + attrib(2) + limit(4) + base(8)

pub const VMCB_CPL: usize = 0x4CB;
pub const VMCB_EFER: usize = 0x4D0;
pub const VMCB_CR4: usize = 0x548;
pub const VMCB_CR3: usize = 0x550;
pub const VMCB_CR0: usize = 0x558;
pub const VMCB_DR7: usize = 0x560;
pub const VMCB_DR6: usize = 0x568;
pub const VMCB_RFLAGS: usize = 0x570;
pub const VMCB_RIP: usize = 0x578;
pub const VMCB_RSP: usize = 0x5D8;
pub const VMCB_RAX: usize = 0x5F8;
pub const VMCB_STAR: usize = 0x600;
pub const VMCB_LSTAR: usize = 0x608;
pub const VMCB_CSTAR: usize = 0x610;
pub const VMCB_SFMASK: usize = 0x618;
pub const VMCB_KERNEL_GS_BASE: usize = 0x620;
pub const VMCB_SYSENTER_CS: usize = 0x628;
pub const VMCB_SYSENTER_ESP: usize = 0x630;
pub const VMCB_SYSENTER_EIP: usize = 0x638;
pub const VMCB_CR2: usize = 0x640;
pub const VMCB_PAT: usize = 0x668;

// ════════════════════════════════════════════════════════════════════════
// SVM Exit Codes
// ════════════════════════════════════════════════════════════════════════

pub const VMEXIT_CR0_READ: u64 = 0x000;
pub const VMEXIT_CR0_WRITE: u64 = 0x010;
pub const VMEXIT_EXCEPTION_DE: u64 = 0x040;
pub const VMEXIT_EXCEPTION_DB: u64 = 0x041;
pub const VMEXIT_EXCEPTION_NMI: u64 = 0x042;
pub const VMEXIT_EXCEPTION_BP: u64 = 0x043;
pub const VMEXIT_EXCEPTION_UD: u64 = 0x046;
pub const VMEXIT_EXCEPTION_DF: u64 = 0x048;
pub const VMEXIT_EXCEPTION_GP: u64 = 0x04D;
pub const VMEXIT_EXCEPTION_PF: u64 = 0x04E;
pub const VMEXIT_INTR: u64 = 0x060;
pub const VMEXIT_NMI: u64 = 0x061;
pub const VMEXIT_SMI: u64 = 0x062;
pub const VMEXIT_INIT: u64 = 0x063;
pub const VMEXIT_VINTR: u64 = 0x064;
pub const VMEXIT_CPUID: u64 = 0x072;
pub const VMEXIT_HLT: u64 = 0x078;
pub const VMEXIT_INVLPG: u64 = 0x079;
pub const VMEXIT_IOIO: u64 = 0x07B;
pub const VMEXIT_MSR: u64 = 0x07C;
pub const VMEXIT_TASK_SWITCH: u64 = 0x07D;
pub const VMEXIT_SHUTDOWN: u64 = 0x07F;
pub const VMEXIT_VMRUN: u64 = 0x080;
pub const VMEXIT_VMMCALL: u64 = 0x081;
pub const VMEXIT_NPF: u64 = 0x400;
pub const VMEXIT_INVALID: u64 = u64::MAX;

// ════════════════════════════════════════════════════════════════════════
// Nested Paging constants
// ════════════════════════════════════════════════════════════════════════

/// Nested paging enable bit in VMCB_NP_ENABLE.
pub const NP_ENABLE: u64 = 1 << 0;

/// NPT entry: present (valid).
pub const NPT_PRESENT: u64 = 1 << 0;
/// NPT entry: writable.
pub const NPT_WRITE: u64 = 1 << 1;
/// NPT entry: user accessible.
pub const NPT_USER: u64 = 1 << 2;
/// NPT entry: page size (2MB/1GB large page).
pub const NPT_LARGE_PAGE: u64 = 1 << 7;
/// NPT entry: no-execute.
pub const NPT_NX: u64 = 1 << 63;

// ════════════════════════════════════════════════════════════════════════
// Virtual Interrupt Control
// ════════════════════════════════════════════════════════════════════════

/// V_IRQ bit — virtual interrupt pending.
pub const V_IRQ: u64 = 1 << 8;
/// V_INTR_MASKING — use virtual IF instead of physical IF.
pub const V_INTR_MASKING: u64 = 1 << 24;
/// V_INTR_VECTOR shift (bits 7:0 of the virtual interrupt field at +0x060).
pub const V_INTR_VECTOR_SHIFT: u64 = 0;
/// V_TPR field (bits 7:0 at byte offset 0x060): the virtual task priority.
pub const V_TPR_SHIFT: u64 = 0;

// ════════════════════════════════════════════════════════════════════════
// Event Injection
// ════════════════════════════════════════════════════════════════════════

/// Event valid bit.
pub const EVENT_INJ_VALID: u64 = 1 << 31;
/// Event type field (bits 10:8).
pub const EVENT_INJ_TYPE_INTR: u64 = 0 << 8;
pub const EVENT_INJ_TYPE_NMI: u64 = 2 << 8;
pub const EVENT_INJ_TYPE_EXCEPTION: u64 = 3 << 8;
pub const EVENT_INJ_TYPE_SOFT: u64 = 4 << 8;
/// Error code valid bit.
pub const EVENT_INJ_EV: u64 = 1 << 11;

// ════════════════════════════════════════════════════════════════════════
// TLB Control values
// ════════════════════════════════════════════════════════════════════════

pub const TLB_CONTROL_DO_NOTHING: u32 = 0;
pub const TLB_CONTROL_FLUSH_ALL: u32 = 1;
pub const TLB_CONTROL_FLUSH_GUEST: u32 = 3;
pub const TLB_CONTROL_FLUSH_GUEST_NONGLOBAL: u32 = 7;

// ════════════════════════════════════════════════════════════════════════
// VMCB Clean Bits
// ════════════════════════════════════════════════════════════════════════

pub const VMCB_CLEAN_INTERCEPTS: u32 = 1 << 0;
pub const VMCB_CLEAN_IOPM: u32 = 1 << 1;
pub const VMCB_CLEAN_ASID: u32 = 1 << 2;
pub const VMCB_CLEAN_TPR: u32 = 1 << 3;
pub const VMCB_CLEAN_NP: u32 = 1 << 4;
pub const VMCB_CLEAN_CRX: u32 = 1 << 5;
pub const VMCB_CLEAN_DRX: u32 = 1 << 6;
pub const VMCB_CLEAN_DT: u32 = 1 << 7;
pub const VMCB_CLEAN_SEG: u32 = 1 << 8;
pub const VMCB_CLEAN_CR2: u32 = 1 << 9;
pub const VMCB_CLEAN_LBR: u32 = 1 << 10;
pub const VMCB_CLEAN_AVIC: u32 = 1 << 11;

/// Size of the VMCB structure (4 KiB page-aligned).
pub const VMCB_SIZE: usize = 4096;
/// Size of the host save area.
pub const HOST_SAVE_AREA_SIZE: usize = 4096;

/// SVM I/O Permissions Map size (12 KiB = 3 pages covering 64K ports × 2 bits).
pub const IOPM_SIZE: usize = 12288;
/// SVM MSR Permissions Map size (8 KiB = 2 pages).
pub const MSRPM_SIZE: usize = 8192;
