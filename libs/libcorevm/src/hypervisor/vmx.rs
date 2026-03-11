//! Intel VT-x (VMX) constants, structures, and VMCS field definitions.
//!
//! This module defines all the hardware constants needed to program the
//! VMCS (Virtual Machine Control Structure) and perform VMX operations
//! on Intel processors.

// ════════════════════════════════════════════════════════════════════════
// MSRs for VMX capability discovery
// ════════════════════════════════════════════════════════════════════════

/// IA32_VMX_BASIC — reports VMCS revision ID, VMCS region size, memory type.
pub const MSR_IA32_VMX_BASIC: u32 = 0x480;
/// IA32_VMX_PINBASED_CTLS — allowed pin-based VM-execution controls.
pub const MSR_IA32_VMX_PINBASED_CTLS: u32 = 0x481;
/// IA32_VMX_PROCBASED_CTLS — allowed primary processor-based controls.
pub const MSR_IA32_VMX_PROCBASED_CTLS: u32 = 0x482;
/// IA32_VMX_EXIT_CTLS — allowed VM-exit controls.
pub const MSR_IA32_VMX_EXIT_CTLS: u32 = 0x483;
/// IA32_VMX_ENTRY_CTLS — allowed VM-entry controls.
pub const MSR_IA32_VMX_ENTRY_CTLS: u32 = 0x484;
/// IA32_VMX_MISC — miscellaneous VMX info (TSC ratio, CR3 target count, etc.).
pub const MSR_IA32_VMX_MISC: u32 = 0x485;
/// IA32_VMX_CR0_FIXED0 — CR0 bits that must be set in VMX operation.
pub const MSR_IA32_VMX_CR0_FIXED0: u32 = 0x486;
/// IA32_VMX_CR0_FIXED1 — CR0 bits that may be set in VMX operation.
pub const MSR_IA32_VMX_CR0_FIXED1: u32 = 0x487;
/// IA32_VMX_CR4_FIXED0 — CR4 bits that must be set in VMX operation.
pub const MSR_IA32_VMX_CR4_FIXED0: u32 = 0x488;
/// IA32_VMX_CR4_FIXED1 — CR4 bits that may be set in VMX operation.
pub const MSR_IA32_VMX_CR4_FIXED1: u32 = 0x489;
/// IA32_VMX_PROCBASED_CTLS2 — allowed secondary processor-based controls.
pub const MSR_IA32_VMX_PROCBASED_CTLS2: u32 = 0x48B;
/// IA32_VMX_EPT_VPID_CAP — EPT and VPID capabilities.
pub const MSR_IA32_VMX_EPT_VPID_CAP: u32 = 0x48C;
/// IA32_VMX_TRUE_PINBASED_CTLS — true pin-based controls (flexible bits).
pub const MSR_IA32_VMX_TRUE_PINBASED_CTLS: u32 = 0x48D;
/// IA32_VMX_TRUE_PROCBASED_CTLS — true primary proc-based controls.
pub const MSR_IA32_VMX_TRUE_PROCBASED_CTLS: u32 = 0x48E;
/// IA32_VMX_TRUE_EXIT_CTLS — true VM-exit controls.
pub const MSR_IA32_VMX_TRUE_EXIT_CTLS: u32 = 0x48F;
/// IA32_VMX_TRUE_ENTRY_CTLS — true VM-entry controls.
pub const MSR_IA32_VMX_TRUE_ENTRY_CTLS: u32 = 0x490;
/// IA32_FEATURE_CONTROL — VMX enable/lock bit.
pub const MSR_IA32_FEATURE_CONTROL: u32 = 0x3A;

/// Bit in IA32_FEATURE_CONTROL: lock bit (must be set).
pub const FEATURE_CONTROL_LOCKED: u64 = 1 << 0;
/// Bit in IA32_FEATURE_CONTROL: enable VMX outside SMX (normal operation).
pub const FEATURE_CONTROL_VMXON_OUTSIDE_SMX: u64 = 1 << 2;

/// CR4.VMXE bit — must be set before VMXON.
pub const CR4_VMXE: u64 = 1 << 13;

// ════════════════════════════════════════════════════════════════════════
// VMCS Field Encodings (Intel SDM Vol. 3C, Appendix B)
// ════════════════════════════════════════════════════════════════════════

// --- 16-bit control fields ---
pub const VMCS_VPID: u32 = 0x0000;

// --- 16-bit guest-state fields ---
pub const VMCS_GUEST_ES_SELECTOR: u32 = 0x0800;
pub const VMCS_GUEST_CS_SELECTOR: u32 = 0x0802;
pub const VMCS_GUEST_SS_SELECTOR: u32 = 0x0804;
pub const VMCS_GUEST_DS_SELECTOR: u32 = 0x0806;
pub const VMCS_GUEST_FS_SELECTOR: u32 = 0x0808;
pub const VMCS_GUEST_GS_SELECTOR: u32 = 0x080A;
pub const VMCS_GUEST_LDTR_SELECTOR: u32 = 0x080C;
pub const VMCS_GUEST_TR_SELECTOR: u32 = 0x080E;

// --- 16-bit host-state fields ---
pub const VMCS_HOST_ES_SELECTOR: u32 = 0x0C00;
pub const VMCS_HOST_CS_SELECTOR: u32 = 0x0C02;
pub const VMCS_HOST_SS_SELECTOR: u32 = 0x0C04;
pub const VMCS_HOST_DS_SELECTOR: u32 = 0x0C06;
pub const VMCS_HOST_FS_SELECTOR: u32 = 0x0C08;
pub const VMCS_HOST_GS_SELECTOR: u32 = 0x0C0A;
pub const VMCS_HOST_TR_SELECTOR: u32 = 0x0C0C;

// --- 64-bit control fields ---
pub const VMCS_IO_BITMAP_A: u32 = 0x2000;
pub const VMCS_IO_BITMAP_B: u32 = 0x2002;
pub const VMCS_MSR_BITMAP: u32 = 0x2004;
pub const VMCS_EXIT_MSR_STORE_ADDR: u32 = 0x2006;
pub const VMCS_EXIT_MSR_LOAD_ADDR: u32 = 0x2008;
pub const VMCS_ENTRY_MSR_LOAD_ADDR: u32 = 0x200A;
pub const VMCS_EXECUTIVE_VMCS_PTR: u32 = 0x200C;
pub const VMCS_TSC_OFFSET: u32 = 0x2010;
pub const VMCS_EPT_POINTER: u32 = 0x201A;

// --- 64-bit guest-state fields ---
pub const VMCS_GUEST_VMCS_LINK_PTR: u32 = 0x2800;
pub const VMCS_GUEST_IA32_DEBUGCTL: u32 = 0x2802;
pub const VMCS_GUEST_IA32_PAT: u32 = 0x2804;
pub const VMCS_GUEST_IA32_EFER: u32 = 0x2806;

// --- 64-bit host-state fields ---
pub const VMCS_HOST_IA32_PAT: u32 = 0x2C00;
pub const VMCS_HOST_IA32_EFER: u32 = 0x2C02;

// --- 32-bit control fields ---
pub const VMCS_PIN_BASED_CONTROLS: u32 = 0x4000;
pub const VMCS_PRIMARY_PROC_BASED_CONTROLS: u32 = 0x4002;
pub const VMCS_EXCEPTION_BITMAP: u32 = 0x4004;
pub const VMCS_PAGE_FAULT_ERROR_CODE_MASK: u32 = 0x4006;
pub const VMCS_PAGE_FAULT_ERROR_CODE_MATCH: u32 = 0x4008;
pub const VMCS_CR3_TARGET_COUNT: u32 = 0x400A;
pub const VMCS_EXIT_CONTROLS: u32 = 0x400C;
pub const VMCS_EXIT_MSR_STORE_COUNT: u32 = 0x400E;
pub const VMCS_EXIT_MSR_LOAD_COUNT: u32 = 0x4010;
pub const VMCS_ENTRY_CONTROLS: u32 = 0x4012;
pub const VMCS_ENTRY_MSR_LOAD_COUNT: u32 = 0x4014;
pub const VMCS_ENTRY_INTERRUPTION_INFO: u32 = 0x4016;
pub const VMCS_ENTRY_EXCEPTION_ERROR_CODE: u32 = 0x4018;
pub const VMCS_ENTRY_INSTRUCTION_LENGTH: u32 = 0x401A;
pub const VMCS_SECONDARY_PROC_BASED_CONTROLS: u32 = 0x401E;

// --- 32-bit read-only data fields ---
pub const VMCS_VM_INSTRUCTION_ERROR: u32 = 0x4400;
pub const VMCS_EXIT_REASON: u32 = 0x4402;
pub const VMCS_EXIT_INTERRUPTION_INFO: u32 = 0x4404;
pub const VMCS_EXIT_INTERRUPTION_ERROR_CODE: u32 = 0x4406;
pub const VMCS_IDT_VECTORING_INFO: u32 = 0x4408;
pub const VMCS_IDT_VECTORING_ERROR_CODE: u32 = 0x440A;
pub const VMCS_EXIT_INSTRUCTION_LENGTH: u32 = 0x440C;
pub const VMCS_EXIT_INSTRUCTION_INFO: u32 = 0x440E;

// --- 32-bit guest-state fields ---
pub const VMCS_GUEST_ES_LIMIT: u32 = 0x4800;
pub const VMCS_GUEST_CS_LIMIT: u32 = 0x4802;
pub const VMCS_GUEST_SS_LIMIT: u32 = 0x4804;
pub const VMCS_GUEST_DS_LIMIT: u32 = 0x4806;
pub const VMCS_GUEST_FS_LIMIT: u32 = 0x4808;
pub const VMCS_GUEST_GS_LIMIT: u32 = 0x480A;
pub const VMCS_GUEST_LDTR_LIMIT: u32 = 0x480C;
pub const VMCS_GUEST_TR_LIMIT: u32 = 0x480E;
pub const VMCS_GUEST_GDTR_LIMIT: u32 = 0x4810;
pub const VMCS_GUEST_IDTR_LIMIT: u32 = 0x4812;
pub const VMCS_GUEST_ES_ACCESS_RIGHTS: u32 = 0x4814;
pub const VMCS_GUEST_CS_ACCESS_RIGHTS: u32 = 0x4816;
pub const VMCS_GUEST_SS_ACCESS_RIGHTS: u32 = 0x4818;
pub const VMCS_GUEST_DS_ACCESS_RIGHTS: u32 = 0x481A;
pub const VMCS_GUEST_FS_ACCESS_RIGHTS: u32 = 0x481C;
pub const VMCS_GUEST_GS_ACCESS_RIGHTS: u32 = 0x481E;
pub const VMCS_GUEST_LDTR_ACCESS_RIGHTS: u32 = 0x4820;
pub const VMCS_GUEST_TR_ACCESS_RIGHTS: u32 = 0x4822;
pub const VMCS_GUEST_INTERRUPTIBILITY: u32 = 0x4824;
pub const VMCS_GUEST_ACTIVITY_STATE: u32 = 0x4826;
pub const VMCS_GUEST_SYSENTER_CS: u32 = 0x482A;

// --- 32-bit host-state fields ---
pub const VMCS_HOST_SYSENTER_CS: u32 = 0x4C00;

// --- Natural-width control fields ---
pub const VMCS_CR0_GUEST_HOST_MASK: u32 = 0x6000;
pub const VMCS_CR4_GUEST_HOST_MASK: u32 = 0x6002;
pub const VMCS_CR0_READ_SHADOW: u32 = 0x6004;
pub const VMCS_CR4_READ_SHADOW: u32 = 0x6006;

// --- 64-bit read-only data fields ---
pub const VMCS_GUEST_PHYSICAL_ADDRESS: u32 = 0x2400;

// --- Natural-width read-only data fields ---
pub const VMCS_EXIT_QUALIFICATION: u32 = 0x6400;
pub const VMCS_IO_RCX: u32 = 0x6402;
pub const VMCS_IO_RSI: u32 = 0x6404;
pub const VMCS_IO_RDI: u32 = 0x6406;
pub const VMCS_IO_RIP: u32 = 0x6408;
pub const VMCS_GUEST_LINEAR_ADDRESS: u32 = 0x640A;

// --- Natural-width guest-state fields ---
pub const VMCS_GUEST_CR0: u32 = 0x6800;
pub const VMCS_GUEST_CR3: u32 = 0x6802;
pub const VMCS_GUEST_CR4: u32 = 0x6804;
pub const VMCS_GUEST_ES_BASE: u32 = 0x6806;
pub const VMCS_GUEST_CS_BASE: u32 = 0x6808;
pub const VMCS_GUEST_SS_BASE: u32 = 0x680A;
pub const VMCS_GUEST_DS_BASE: u32 = 0x680C;
pub const VMCS_GUEST_FS_BASE: u32 = 0x680E;
pub const VMCS_GUEST_GS_BASE: u32 = 0x6810;
pub const VMCS_GUEST_LDTR_BASE: u32 = 0x6812;
pub const VMCS_GUEST_TR_BASE: u32 = 0x6814;
pub const VMCS_GUEST_GDTR_BASE: u32 = 0x6816;
pub const VMCS_GUEST_IDTR_BASE: u32 = 0x6818;
pub const VMCS_GUEST_DR7: u32 = 0x681A;
pub const VMCS_GUEST_RSP: u32 = 0x681C;
pub const VMCS_GUEST_RIP: u32 = 0x681E;
pub const VMCS_GUEST_RFLAGS: u32 = 0x6820;
pub const VMCS_GUEST_PENDING_DBG_EXCEPTIONS: u32 = 0x6822;
pub const VMCS_GUEST_SYSENTER_ESP: u32 = 0x6824;
pub const VMCS_GUEST_SYSENTER_EIP: u32 = 0x6826;

// --- Natural-width host-state fields ---
pub const VMCS_HOST_CR0: u32 = 0x6C00;
pub const VMCS_HOST_CR3: u32 = 0x6C02;
pub const VMCS_HOST_CR4: u32 = 0x6C04;
pub const VMCS_HOST_FS_BASE: u32 = 0x6C06;
pub const VMCS_HOST_GS_BASE: u32 = 0x6C08;
pub const VMCS_HOST_TR_BASE: u32 = 0x6C0A;
pub const VMCS_HOST_GDTR_BASE: u32 = 0x6C0C;
pub const VMCS_HOST_IDTR_BASE: u32 = 0x6C0E;
pub const VMCS_HOST_SYSENTER_ESP: u32 = 0x6C10;
pub const VMCS_HOST_SYSENTER_EIP: u32 = 0x6C12;
pub const VMCS_HOST_RSP: u32 = 0x6C14;
pub const VMCS_HOST_RIP: u32 = 0x6C16;

// ════════════════════════════════════════════════════════════════════════
// Pin-Based VM-Execution Controls
// ════════════════════════════════════════════════════════════════════════
pub const PIN_BASED_EXT_INTR_EXIT: u32 = 1 << 0;
pub const PIN_BASED_NMI_EXIT: u32 = 1 << 3;
pub const PIN_BASED_VIRTUAL_NMIS: u32 = 1 << 5;
pub const PIN_BASED_PREEMPTION_TIMER: u32 = 1 << 6;

// ════════════════════════════════════════════════════════════════════════
// Primary Processor-Based VM-Execution Controls
// ════════════════════════════════════════════════════════════════════════
pub const PROC_BASED_INTERRUPT_WINDOW_EXIT: u32 = 1 << 2;
pub const PROC_BASED_USE_TSC_OFFSETTING: u32 = 1 << 3;
pub const PROC_BASED_HLT_EXIT: u32 = 1 << 7;
pub const PROC_BASED_INVLPG_EXIT: u32 = 1 << 9;
pub const PROC_BASED_MWAIT_EXIT: u32 = 1 << 10;
pub const PROC_BASED_RDPMC_EXIT: u32 = 1 << 11;
pub const PROC_BASED_RDTSC_EXIT: u32 = 1 << 12;
pub const PROC_BASED_CR3_LOAD_EXIT: u32 = 1 << 15;
pub const PROC_BASED_CR3_STORE_EXIT: u32 = 1 << 16;
pub const PROC_BASED_CR8_LOAD_EXIT: u32 = 1 << 19;
pub const PROC_BASED_CR8_STORE_EXIT: u32 = 1 << 20;
pub const PROC_BASED_USE_IO_BITMAPS: u32 = 1 << 25;
pub const PROC_BASED_MONITOR_EXIT: u32 = 1 << 29;
pub const PROC_BASED_PAUSE_EXIT: u32 = 1 << 30;
pub const PROC_BASED_ACTIVATE_SECONDARY: u32 = 1 << 31;

// ════════════════════════════════════════════════════════════════════════
// Secondary Processor-Based VM-Execution Controls
// ════════════════════════════════════════════════════════════════════════
pub const PROC2_BASED_ENABLE_EPT: u32 = 1 << 1;
pub const PROC2_BASED_RDTSCP: u32 = 1 << 3;
pub const PROC2_BASED_ENABLE_VPID: u32 = 1 << 5;
pub const PROC2_BASED_UNRESTRICTED_GUEST: u32 = 1 << 7;
pub const PROC2_BASED_ENABLE_INVPCID: u32 = 1 << 12;
pub const PROC2_BASED_ENABLE_XSAVES: u32 = 1 << 20;

// ════════════════════════════════════════════════════════════════════════
// VM-Exit Controls
// ════════════════════════════════════════════════════════════════════════
pub const EXIT_SAVE_DEBUG_CONTROLS: u32 = 1 << 2;
pub const EXIT_HOST_ADDR_SPACE_SIZE: u32 = 1 << 9;
pub const EXIT_SAVE_IA32_PAT: u32 = 1 << 18;
pub const EXIT_LOAD_IA32_PAT: u32 = 1 << 19;
pub const EXIT_SAVE_IA32_EFER: u32 = 1 << 20;
pub const EXIT_LOAD_IA32_EFER: u32 = 1 << 21;
pub const EXIT_ACK_INTR_ON_EXIT: u32 = 1 << 15;

// ════════════════════════════════════════════════════════════════════════
// VM-Entry Controls
// ════════════════════════════════════════════════════════════════════════
pub const ENTRY_LOAD_DEBUG_CONTROLS: u32 = 1 << 2;
pub const ENTRY_IA32E_MODE_GUEST: u32 = 1 << 9;
pub const ENTRY_LOAD_IA32_PAT: u32 = 1 << 14;
pub const ENTRY_LOAD_IA32_EFER: u32 = 1 << 15;

// ════════════════════════════════════════════════════════════════════════
// VM-Exit Reasons (Basic exit reason field, bits 15:0)
// ════════════════════════════════════════════════════════════════════════
pub const EXIT_REASON_EXCEPTION_NMI: u32 = 0;
pub const EXIT_REASON_EXTERNAL_INTERRUPT: u32 = 1;
pub const EXIT_REASON_TRIPLE_FAULT: u32 = 2;
pub const EXIT_REASON_INIT_SIGNAL: u32 = 3;
pub const EXIT_REASON_SIPI: u32 = 4;
pub const EXIT_REASON_INTERRUPT_WINDOW: u32 = 7;
pub const EXIT_REASON_NMI_WINDOW: u32 = 8;
pub const EXIT_REASON_CPUID: u32 = 10;
pub const EXIT_REASON_HLT: u32 = 12;
pub const EXIT_REASON_INVLPG: u32 = 14;
pub const EXIT_REASON_RDPMC: u32 = 15;
pub const EXIT_REASON_RDTSC: u32 = 16;
pub const EXIT_REASON_VMCALL: u32 = 18;
pub const EXIT_REASON_CR_ACCESS: u32 = 28;
pub const EXIT_REASON_IO_INSTRUCTION: u32 = 30;
pub const EXIT_REASON_MSR_READ: u32 = 31;
pub const EXIT_REASON_MSR_WRITE: u32 = 32;
pub const EXIT_REASON_INVALID_GUEST_STATE: u32 = 33;
pub const EXIT_REASON_MSR_LOADING: u32 = 34;
pub const EXIT_REASON_MONITOR_TRAP_FLAG: u32 = 37;
pub const EXIT_REASON_PAUSE: u32 = 40;
pub const EXIT_REASON_EPT_VIOLATION: u32 = 48;
pub const EXIT_REASON_EPT_MISCONFIG: u32 = 49;
pub const EXIT_REASON_INVEPT: u32 = 50;
pub const EXIT_REASON_RDTSCP: u32 = 51;
pub const EXIT_REASON_PREEMPTION_TIMER: u32 = 52;
pub const EXIT_REASON_XSAVES: u32 = 63;
pub const EXIT_REASON_XRSTORS: u32 = 64;

// ════════════════════════════════════════════════════════════════════════
// VM-Entry Interruption-Information Field
// ════════════════════════════════════════════════════════════════════════

/// Bit 31: Valid — the field contains a valid event to inject.
pub const INTR_INFO_VALID: u32 = 1 << 31;
/// Bit 11: Deliver error code.
pub const INTR_INFO_DELIVER_ERROR_CODE: u32 = 1 << 11;
/// Interruption type (bits 10:8):
pub const INTR_TYPE_EXTERNAL: u32 = 0 << 8;
pub const INTR_TYPE_NMI: u32 = 2 << 8;
pub const INTR_TYPE_HARD_EXCEPTION: u32 = 3 << 8;
pub const INTR_TYPE_SOFT_INTERRUPT: u32 = 4 << 8;

// ════════════════════════════════════════════════════════════════════════
// EPT (Extended Page Tables) constants
// ════════════════════════════════════════════════════════════════════════

/// EPT memory type: Write-Back.
pub const EPT_MEMORY_TYPE_WB: u64 = 6;
/// EPT page walk length (4 levels, value = 3 in field).
pub const EPT_PAGE_WALK_4: u64 = 3 << 3;
/// EPT entry: read permission.
pub const EPT_READ: u64 = 1 << 0;
/// EPT entry: write permission.
pub const EPT_WRITE: u64 = 1 << 1;
/// EPT entry: execute permission.
pub const EPT_EXECUTE: u64 = 1 << 2;
/// EPT entry: memory type field position.
pub const EPT_MEMTYPE_SHIFT: u64 = 3;
/// EPT entry: 2MB large page.
pub const EPT_LARGE_PAGE: u64 = 1 << 7;

// ════════════════════════════════════════════════════════════════════════
// Activity State
// ════════════════════════════════════════════════════════════════════════
pub const ACTIVITY_STATE_ACTIVE: u32 = 0;
pub const ACTIVITY_STATE_HLT: u32 = 1;
pub const ACTIVITY_STATE_SHUTDOWN: u32 = 2;
pub const ACTIVITY_STATE_WAIT_SIPI: u32 = 3;

// ════════════════════════════════════════════════════════════════════════
// Interruptibility State
// ════════════════════════════════════════════════════════════════════════
pub const INTERRUPTIBILITY_STI_BLOCKING: u32 = 1 << 0;
pub const INTERRUPTIBILITY_MOV_SS_BLOCKING: u32 = 1 << 1;
pub const INTERRUPTIBILITY_SMI_BLOCKING: u32 = 1 << 2;
pub const INTERRUPTIBILITY_NMI_BLOCKING: u32 = 1 << 3;

// ════════════════════════════════════════════════════════════════════════
// Access Rights encoding for segment registers in VMCS
// ════════════════════════════════════════════════════════════════════════

/// Unusable segment (bit 16 set).
pub const SEG_ACCESS_UNUSABLE: u32 = 1 << 16;
/// Present bit.
pub const SEG_ACCESS_PRESENT: u32 = 1 << 7;
/// S flag (1 = code/data, 0 = system).
pub const SEG_ACCESS_S: u32 = 1 << 4;
/// DPL field shift.
pub const SEG_ACCESS_DPL_SHIFT: u32 = 5;

/// Encode segment access rights from descriptor fields for VMCS format.
///
/// VMCS access-rights format:
/// Bits 3:0 = type, Bit 4 = S, Bits 6:5 = DPL, Bit 7 = P,
/// Bits 11:8 = reserved, Bit 12 = AVL, Bit 13 = L, Bit 14 = D/B,
/// Bit 15 = G, Bit 16 = unusable.
pub fn encode_access_rights(access: u8, flags: u8, present: bool) -> u32 {
    if !present {
        return SEG_ACCESS_UNUSABLE;
    }
    let ar_type = (access & 0x0F) as u32;
    let s = ((access >> 4) & 1) as u32;
    let dpl = ((access >> 5) & 3) as u32;
    let p = ((access >> 7) & 1) as u32;
    let avl = (flags & 1) as u32;
    let l = ((flags >> 1) & 1) as u32;
    let db = ((flags >> 2) & 1) as u32;
    let g = ((flags >> 3) & 1) as u32;
    ar_type | (s << 4) | (dpl << 5) | (p << 7) | (avl << 12) | (l << 13) | (db << 14) | (g << 15)
}

/// Decode VMCS access-rights back to descriptor access byte and flags nibble.
pub fn decode_access_rights(ar: u32) -> (u8, u8, bool) {
    if ar & SEG_ACCESS_UNUSABLE != 0 {
        return (0, 0, false);
    }
    let access = (ar & 0xFF) as u8;
    let avl = ((ar >> 12) & 1) as u8;
    let l = ((ar >> 13) & 1) as u8;
    let db = ((ar >> 14) & 1) as u8;
    let g = ((ar >> 15) & 1) as u8;
    let flags = avl | (l << 1) | (db << 2) | (g << 3);
    let present = (ar & SEG_ACCESS_PRESENT) != 0;
    (access, flags, present)
}

// ════════════════════════════════════════════════════════════════════════
// Aliases for convenience (used by bare-metal backend)
// ════════════════════════════════════════════════════════════════════════
pub const VMCS_PROC_BASED_CONTROLS: u32 = VMCS_PRIMARY_PROC_BASED_CONTROLS;
pub const VMCS_GUEST_EFER: u32 = VMCS_GUEST_IA32_EFER;
pub const VMCS_GUEST_INTERRUPTIBILITY_STATE: u32 = VMCS_GUEST_INTERRUPTIBILITY;
pub const VMCS_GUEST_VMCS_LINK_POINTER: u32 = VMCS_GUEST_VMCS_LINK_PTR;
pub const VMCS_HOST_EFER: u32 = VMCS_HOST_IA32_EFER;
pub const VMCS_HOST_PAT: u32 = VMCS_HOST_IA32_PAT;
