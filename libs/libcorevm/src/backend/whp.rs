//! Windows Hypervisor Platform (WHP) backend.
//!
//! Uses runtime-loaded WinHvPlatform.dll via LoadLibraryA/GetProcAddress.

extern crate std;

use std::vec::Vec;
use std::boxed::Box;
use core::ffi::c_void;
use super::{VmBackend, VmError, VmExitReason};
use super::types::*;

/// Global callback for WHP debug output. Set by the host application to
/// route debug messages to its diagnostics UI instead of log files.
static WHP_DEBUG_CB: core::sync::atomic::AtomicPtr<c_void> =
    core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());
static WHP_DEBUG_CTX: core::sync::atomic::AtomicPtr<c_void> =
    core::sync::atomic::AtomicPtr::new(core::ptr::null_mut());

type WhpDebugFn = extern "C" fn(ctx: *mut c_void, msg: *const u8, len: u32);

/// Register a callback to receive WHP debug output.
#[no_mangle]
pub extern "C" fn corevm_set_whp_debug_callback(
    cb: Option<extern "C" fn(*mut c_void, *const u8, u32)>,
    ctx: *mut c_void,
) {
    match cb {
        Some(f) => {
            WHP_DEBUG_CB.store(f as *mut c_void, core::sync::atomic::Ordering::Release);
            WHP_DEBUG_CTX.store(ctx, core::sync::atomic::Ordering::Release);
        }
        None => {
            WHP_DEBUG_CB.store(core::ptr::null_mut(), core::sync::atomic::Ordering::Release);
            WHP_DEBUG_CTX.store(core::ptr::null_mut(), core::sync::atomic::Ordering::Release);
        }
    }
}

/// Write a debug line. Routes to the registered callback if set, otherwise to a log file.
fn whp_debug(args: core::fmt::Arguments) {
    let cb_ptr = WHP_DEBUG_CB.load(core::sync::atomic::Ordering::Acquire);
    if !cb_ptr.is_null() {
        let msg = std::format!("{}\n", args);
        let cb: WhpDebugFn = unsafe { core::mem::transmute(cb_ptr) };
        let ctx = WHP_DEBUG_CTX.load(core::sync::atomic::Ordering::Acquire);
        cb(ctx, msg.as_ptr(), msg.len() as u32);
        return;
    }
    // Fallback: file-based logging
    use std::io::Write;
    static DEBUG_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    let n = DEBUG_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    if n >= 20000 { return; }
    let path = std::env::var("TEMP")
        .map(|t| std::format!("{}\\whp_debug.log", t))
        .unwrap_or_else(|_| std::string::String::from("whp_debug.log"));
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] {}", n, args);
    }
}

// --- WHP types ---

type WHV_PARTITION_HANDLE = *mut c_void;

#[repr(C, align(16))]
#[derive(Copy, Clone)]
union WHV_REGISTER_VALUE {
    reg64: u64,
    reg128: [u64; 2],
    segment: WhvSegment,
    table: WhvTable,
}

impl Default for WHV_REGISTER_VALUE {
    fn default() -> Self {
        WHV_REGISTER_VALUE { reg128: [0; 2] }
    }
}

impl WHV_REGISTER_VALUE {
    /// Create a register value from a u64, with upper 64 bits zeroed.
    fn from_u64(v: u64) -> Self {
        WHV_REGISTER_VALUE { reg128: [v, 0] }
    }
    /// Create a register value from a segment descriptor (16 bytes, no leftover).
    fn from_seg(s: WhvSegment) -> Self {
        WHV_REGISTER_VALUE { segment: s }
    }
    /// Create a register value from a table descriptor (16 bytes, no leftover).
    fn from_table(t: WhvTable) -> Self {
        WHV_REGISTER_VALUE { table: t }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct WhvSegment {
    base: u64,
    limit: u32,
    selector: u16,
    attributes: u16,
}

#[repr(C)]
#[derive(Copy, Clone, Default)]
struct WhvTable {
    _pad: [u16; 3],
    limit: u16,
    base: u64,
}

// WHV_VP_EXIT_CONTEXT: ExecutionState(2) + InstructionLength:4+Cr8:4(1) + Reserved(1) + Reserved2(4) + Cs(16) + Rip(8) + Rflags(8) = 40 bytes
const VP_CONTEXT_SIZE: usize = 40;

#[repr(C)]
struct WHV_RUN_VP_EXIT_CONTEXT {
    exit_reason: u32,
    _reserved: u32,
    vp_context: [u8; VP_CONTEXT_SIZE],
    exit_data: [u8; 256],
}

// --- Thread-safe cancel support ---
// Stored after partition creation so cancel_vcpu_global can be called from any thread.
static CANCEL_PARTITION: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static CANCEL_FN_PTR: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Cancel a running WHvRunVirtualProcessor from any thread.
pub fn cancel_vcpu_global(vcpu_id: u32) -> i32 {
    let partition = CANCEL_PARTITION.load(core::sync::atomic::Ordering::Relaxed);
    let fn_ptr = CANCEL_FN_PTR.load(core::sync::atomic::Ordering::Relaxed);
    if partition == 0 || fn_ptr == 0 {
        return -1;
    }
    let cancel_fn: FnCancelRunVirtualProcessor = unsafe { core::mem::transmute(fn_ptr) };
    let hr = cancel_fn(partition as WHV_PARTITION_HANDLE, vcpu_id, 0);
    if hr >= 0 { 0 } else { -1 }
}

// --- WHP constants ---

const WHV_PROPERTY_PROCESSOR_COUNT: u32 = 0x00001FFF; // WHvPartitionPropertyCodeProcessorCount
const WHV_PROPERTY_EXTENDED_VM_EXITS: u32 = 0x00000002; // WHvPartitionPropertyCodeExtendedVmExits
const WHV_PROPERTY_EXCEPTION_EXIT_BITMAP: u32 = 0x00000003; // WHvPartitionPropertyCodeExceptionExitBitmap

const WHV_MAP_GPA_RANGE_FLAG_READ: u32 = 0x1;
const WHV_MAP_GPA_RANGE_FLAG_WRITE: u32 = 0x2;
const WHV_MAP_GPA_RANGE_FLAG_EXECUTE: u32 = 0x4;

const WHV_EXIT_REASON_NONE: u32 = 0x00000000;
const WHV_EXIT_REASON_MEMORY_ACCESS: u32 = 0x00000001;
const WHV_EXIT_REASON_IO_PORT: u32 = 0x00000002;
const WHV_EXIT_REASON_UNRECOVERABLE_EXCEPTION: u32 = 0x00000004;
const WHV_EXIT_REASON_INVALID_VP_STATE: u32 = 0x00000005;
const WHV_EXIT_REASON_UNSUPPORTED_FEATURE: u32 = 0x00000006;
const WHV_EXIT_REASON_HALT: u32 = 0x00000008;
const WHV_EXIT_REASON_CANCELED: u32 = 0x00002001;
const WHV_EXIT_REASON_MSR: u32 = 0x00001000;
const WHV_EXIT_REASON_CPUID: u32 = 0x00001001;
const WHV_EXIT_REASON_EXCEPTION: u32 = 0x00001002;
const WHV_EXIT_REASON_INTERRUPT_WINDOW: u32 = 0x00000007;
const WHV_EXIT_REASON_APIC_EOI: u32 = 0x00000009;

/// WHV_INTERRUPT_CONTROL for WHvRequestInterrupt (XApic mode).
/// Layout: [0..8] u64 bitfield (Type[0:7], DestMode[8:11], TriggerMode[12:15], Reserved[16:63]),
///         [8..12] Destination (APIC ID), [12..16] Vector.
#[repr(C)]
#[derive(Copy, Clone)]
struct WhvInterruptControl {
    type_and_flags: u64, // bits 0-7: Type (0=Fixed,1=LowestPri), bits 8-11: DestMode, bits 12-15: TriggerMode
    destination: u32,    // target APIC ID
    vector: u32,         // interrupt vector
}

impl WhvInterruptControl {
    /// Create a fixed, edge-triggered, physical-destination interrupt to APIC ID 0.
    fn fixed_edge(vector: u8) -> Self {
        WhvInterruptControl {
            type_and_flags: 0, // Type=Fixed(0), DestMode=Physical(0), TriggerMode=Edge(0)
            destination: 0,    // BSP (APIC ID 0)
            vector: vector as u32,
        }
    }
}

// WHV_REGISTER_NAME constants (from Windows SDK)
const REG_RAX: u32 = 0x00000000;
const REG_RCX: u32 = 0x00000001;
const REG_RDX: u32 = 0x00000002;
const REG_RBX: u32 = 0x00000003;
const REG_RSP: u32 = 0x00000004;
const REG_RBP: u32 = 0x00000005;
const REG_RSI: u32 = 0x00000006;
const REG_RDI: u32 = 0x00000007;
const REG_R8: u32 = 0x00000008;
const REG_R9: u32 = 0x00000009;
const REG_R10: u32 = 0x0000000A;
const REG_R11: u32 = 0x0000000B;
const REG_R12: u32 = 0x0000000C;
const REG_R13: u32 = 0x0000000D;
const REG_R14: u32 = 0x0000000E;
const REG_R15: u32 = 0x0000000F;
const REG_RIP: u32 = 0x00000010;
const REG_RFLAGS: u32 = 0x00000011;

const REG_ES: u32 = 0x00000012;
const REG_CS: u32 = 0x00000013;
const REG_SS: u32 = 0x00000014;
const REG_DS: u32 = 0x00000015;
const REG_FS: u32 = 0x00000016;
const REG_GS: u32 = 0x00000017;
const REG_LDTR: u32 = 0x00000018;
const REG_TR: u32 = 0x00000019;
const REG_IDTR: u32 = 0x0000001A;
const REG_GDTR: u32 = 0x0000001B;

const REG_CR0: u32 = 0x0000001C;
const REG_CR2: u32 = 0x0000001D;
const REG_CR3: u32 = 0x0000001E;
const REG_CR4: u32 = 0x0000001F;
const REG_EFER: u32 = 0x00002001;

// MSR-backed registers
const REG_TSC: u32 = 0x00002000;
// REG_EFER = 0x00002001 (above)
const REG_KERNEL_GS_BASE: u32 = 0x00002002;
const REG_APIC_BASE: u32 = 0x00002003;
const REG_PAT: u32 = 0x00002004;
const REG_SYSENTER_CS: u32 = 0x00002005;
const REG_SYSENTER_EIP: u32 = 0x00002006;
const REG_SYSENTER_ESP: u32 = 0x00002007;
const REG_STAR: u32 = 0x00002008;
const REG_LSTAR: u32 = 0x00002009;
const REG_CSTAR: u32 = 0x0000200A;
const REG_SFMASK: u32 = 0x0000200B;

const REG_PENDING_INTERRUPTION: u32 = 0x80000000;
const REG_PENDING_EVENT: u32 = 0x80000002; // WHvRegisterPendingEvent — 128-bit, for ExtInt injection in XApic mode
const REG_DELIVERABILITY_NOTIFICATIONS: u32 = 0x80000004;
const REG_INTERNAL_ACTIVITY_STATE: u32 = 0x80000005; // WHvX64RegisterInternalActivityState

// GP register names in order for get/set
const GP_REG_NAMES: [u32; 18] = [
    REG_RAX, REG_RBX, REG_RCX, REG_RDX,
    REG_RSI, REG_RDI, REG_RBP, REG_RSP,
    REG_R8, REG_R9, REG_R10, REG_R11,
    REG_R12, REG_R13, REG_R14, REG_R15,
    REG_RIP, REG_RFLAGS,
];

const SREG_NAMES: [u32; 13] = [
    REG_CS, REG_DS, REG_ES, REG_FS, REG_GS, REG_SS,
    REG_TR, REG_LDTR, REG_GDTR, REG_IDTR,
    REG_CR0, REG_CR2, REG_CR3,
];

const SREG_NAMES_EXT: [u32; 2] = [REG_CR4, REG_EFER];

// --- WHP function pointer types ---

type FnGetCapability = extern "system" fn(u32, *mut u8, u32, *mut u32) -> i32;
type FnCreatePartition = extern "system" fn(*mut WHV_PARTITION_HANDLE) -> i32;
type FnSetupPartition = extern "system" fn(WHV_PARTITION_HANDLE) -> i32;
type FnDeletePartition = extern "system" fn(WHV_PARTITION_HANDLE) -> i32;
type FnSetPartitionProperty = extern "system" fn(WHV_PARTITION_HANDLE, u32, *const u8, u32) -> i32;
type FnMapGpaRange = extern "system" fn(WHV_PARTITION_HANDLE, *mut u8, u64, u64, u32) -> i32;
type FnUnmapGpaRange = extern "system" fn(WHV_PARTITION_HANDLE, u64, u64) -> i32;
type FnCreateVirtualProcessor = extern "system" fn(WHV_PARTITION_HANDLE, u32, u32) -> i32;
type FnDeleteVirtualProcessor = extern "system" fn(WHV_PARTITION_HANDLE, u32) -> i32;
type FnRunVirtualProcessor = extern "system" fn(WHV_PARTITION_HANDLE, u32, *mut u8, u32) -> i32;
type FnGetVirtualProcessorRegisters = extern "system" fn(WHV_PARTITION_HANDLE, u32, *const u32, u32, *mut WHV_REGISTER_VALUE) -> i32;
type FnSetVirtualProcessorRegisters = extern "system" fn(WHV_PARTITION_HANDLE, u32, *const u32, u32, *const WHV_REGISTER_VALUE) -> i32;
// WHvRequestInterrupt(partition, *interrupt_control, size) -> HRESULT
type FnRequestInterrupt = extern "system" fn(WHV_PARTITION_HANDLE, *const u8, u32) -> i32;
type FnCancelRunVirtualProcessor = extern "system" fn(WHV_PARTITION_HANDLE, u32, u32) -> i32;

struct WhpApi {
    get_capability: FnGetCapability,
    create_partition: FnCreatePartition,
    setup_partition: FnSetupPartition,
    delete_partition: FnDeletePartition,
    set_property: FnSetPartitionProperty,
    map_gpa: FnMapGpaRange,
    unmap_gpa: FnUnmapGpaRange,
    create_vp: FnCreateVirtualProcessor,
    delete_vp: FnDeleteVirtualProcessor,
    run_vp: FnRunVirtualProcessor,
    get_regs: FnGetVirtualProcessorRegisters,
    set_regs: FnSetVirtualProcessorRegisters,
    request_interrupt: Option<FnRequestInterrupt>,
    cancel_run: FnCancelRunVirtualProcessor,
}

// Windows API imports for DLL loading
extern "system" {
    fn LoadLibraryA(name: *const u8) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
}

struct MemorySlot {
    slot: u32,
    guest_phys: u64,
    size: u64,
    host_ptr: *mut u8,
}

/// Read host TSC for LAPIC timer time-keeping.
#[inline]
fn host_tsc() -> u64 {
    #[cfg(target_arch = "x86_64")]
    unsafe { core::arch::x86_64::_rdtsc() as u64 }
    #[cfg(not(target_arch = "x86_64"))]
    { 0 }
}

/// Minimal Local APIC state for handling MMIO at 0xFEE00000.
/// Timer uses host TSC so current_count reads always reflect real elapsed time,
/// even inside the inner MMIO loop of run_vcpu.
pub struct SoftLapic {
    pub regs: [u32; 64],
    // Timer state — TSC-based
    pub timer_initial: u32,       // guest-written initial count
    pub timer_divide: u32,        // divisor (1,2,4,8,16,32,64,128)
    pub timer_armed: bool,
    timer_tsc_start: u64,     // host TSC when timer was armed
    timer_tsc_period: u64,    // host TSC ticks for one full countdown
    tsc_per_bus_tick: u64,    // host TSC ticks per bus tick (calibrated from CPUID MHz)
    pub timer_irq_pending: bool,
    timer_last_fire_tsc: u64, // TSC of last fire (avoid double-fire in same period)
}

/// LAPIC register indices (offset >> 4)
const LAPIC_IDX_EOI: usize = 0x0B;
const LAPIC_IDX_SVR: usize = 0x0F;
const LAPIC_IDX_ISR0: usize = 0x10;
const LAPIC_IDX_ICR_LO: usize = 0x30;
const LAPIC_IDX_LVT_TIMER: usize = 0x32;
const LAPIC_IDX_INITIAL_COUNT: usize = 0x38;
const LAPIC_IDX_CURRENT_COUNT: usize = 0x39;
const LAPIC_IDX_DIVIDE_CONFIG: usize = 0x3E;

impl SoftLapic {
    fn new() -> Self {
        let mut regs = [0u32; 64];
        regs[0x02] = 0x20;       // ID = 0 (bits 24-27)
        regs[0x03] = 0x00050014; // Version: max_lvt=5, version=20 (Pentium 4)
        regs[0x0E] = 0x0FFFFFFF; // Logical Destination Format: flat model
        regs[0x0F] = 0x000001FF; // Spurious Vector: APIC enabled (bit 8) + vector 0xFF
        regs[0x08] = 0;          // Task Priority Register
        regs[LAPIC_IDX_LVT_TIMER] = 0x00010000; // masked by default

        // Estimate host TSC frequency: use a quick calibration.
        // Assume ~1 GHz bus clock for the guest LAPIC timer.
        // tsc_per_bus_tick = host_tsc_freq / guest_bus_freq.
        // We approximate host TSC freq by measuring a short interval.
        // For simplicity, assume host TSC ≈ guest bus clock (1:1 mapping),
        // which gives ~GHz-class bus frequency. Linux will calibrate against
        // this and adapt.
        SoftLapic {
            regs,
            timer_initial: 0,
            timer_divide: 1,
            timer_armed: false,
            timer_tsc_start: 0,
            timer_tsc_period: 0,
            tsc_per_bus_tick: 1, // 1:1 TSC-to-bus mapping
            timer_irq_pending: false,
            timer_last_fire_tsc: 0,
        }
    }

    fn decode_divide(val: u32) -> u32 {
        let bits = ((val >> 1) & 0b100) | (val & 0b11);
        match bits {
            0b000 => 2,
            0b001 => 4,
            0b010 => 8,
            0b011 => 16,
            0b100 => 32,
            0b101 => 64,
            0b110 => 128,
            0b111 => 1,
            _ => 1,
        }
    }

    /// Compute remaining timer count from host TSC.
    pub fn current_count(&self) -> u32 {
        if !self.timer_armed || self.timer_initial == 0 || self.timer_tsc_period == 0 {
            return 0;
        }
        let now = host_tsc();
        let elapsed = now.wrapping_sub(self.timer_tsc_start);
        let lvt = self.regs[LAPIC_IDX_LVT_TIMER];
        let mode = (lvt >> 17) & 0x3;

        if mode == 1 {
            // Periodic: compute position within current period
            let pos = elapsed % self.timer_tsc_period;
            let remaining_tsc = self.timer_tsc_period - pos;
            let remaining = remaining_tsc / (self.tsc_per_bus_tick * self.timer_divide as u64);
            (remaining as u32).min(self.timer_initial)
        } else {
            // One-shot
            if elapsed >= self.timer_tsc_period {
                0
            } else {
                let remaining_tsc = self.timer_tsc_period - elapsed;
                let remaining = remaining_tsc / (self.tsc_per_bus_tick * self.timer_divide as u64);
                (remaining as u32).min(self.timer_initial)
            }
        }
    }

    fn read(&self, offset: u64) -> u32 {
        let idx = ((offset & 0xFFF) >> 4) as usize;
        match idx {
            LAPIC_IDX_CURRENT_COUNT => self.current_count(),
            _ => {
                if idx < 64 { self.regs[idx] } else { 0 }
            }
        }
    }

    fn write(&mut self, offset: u64, val: u32) {
        let idx = ((offset & 0xFFF) >> 4) as usize;
        match idx {
            LAPIC_IDX_EOI => {
                // EOI: clear highest-priority ISR bit
                for i in (0..8).rev() {
                    let isr_idx = LAPIC_IDX_ISR0 + i;
                    if isr_idx < 64 && self.regs[isr_idx] != 0 {
                        let bit = 31 - self.regs[isr_idx].leading_zeros();
                        self.regs[isr_idx] &= !(1 << bit);
                        break;
                    }
                }
            }
            LAPIC_IDX_ICR_LO => {} // IPI - ignore for single CPU
            LAPIC_IDX_INITIAL_COUNT => {
                self.regs[idx] = val;
                self.timer_initial = val;
                if val == 0 {
                    self.timer_armed = false;
                } else {
                    // Arm timer: compute TSC period for full countdown
                    self.timer_tsc_period = val as u64 * self.timer_divide as u64 * self.tsc_per_bus_tick;
                    self.timer_tsc_start = host_tsc();
                    self.timer_last_fire_tsc = self.timer_tsc_start;
                    self.timer_armed = true;
                }
            }
            LAPIC_IDX_CURRENT_COUNT => {
                // Read-only per Intel SDM
            }
            LAPIC_IDX_DIVIDE_CONFIG => {
                self.regs[idx] = val;
                self.timer_divide = Self::decode_divide(val);
            }
            LAPIC_IDX_LVT_TIMER => {
                self.regs[idx] = val;
            }
            0x02 | 0x03 => {
                // APIC ID (0x020) and Version (0x030) are read-only.
                // Linux verify_local_APIC() writes a test pattern and expects
                // the value to NOT change. If it does, APIC is deemed broken.
            }
            _ => {
                if idx < 64 { self.regs[idx] = val; }
            }
        }
    }

    /// Check if the timer has expired. Call this periodically (from run loop or MMIO handler).
    /// Returns Some(vector) if an interrupt should be delivered.
    pub fn poll_timer(&mut self) -> Option<u8> {
        if !self.timer_armed || self.timer_initial == 0 || self.timer_tsc_period == 0 {
            return None;
        }

        let lvt = self.regs[LAPIC_IDX_LVT_TIMER];
        let masked = (lvt & (1 << 16)) != 0;
        let vector = (lvt & 0xFF) as u8;
        let mode = (lvt >> 17) & 0x3;

        let now = host_tsc();
        let elapsed = now.wrapping_sub(self.timer_tsc_start);

        if mode == 1 {
            // Periodic: fire if we crossed a period boundary since last fire
            let since_last = now.wrapping_sub(self.timer_last_fire_tsc);
            if since_last >= self.timer_tsc_period {
                self.timer_last_fire_tsc = now;
                if !masked && vector >= 16 {
                    self.timer_irq_pending = true;
                    return Some(vector);
                }
            }
        } else {
            // One-shot: fire once when expired
            if elapsed >= self.timer_tsc_period && !self.timer_irq_pending {
                self.timer_armed = false;
                if !masked && vector >= 16 {
                    self.timer_irq_pending = true;
                    return Some(vector);
                }
            }
        }
        None
    }

    /// Check and clear pending timer IRQ.
    pub fn take_timer_irq(&mut self) -> Option<u8> {
        if self.timer_irq_pending {
            self.timer_irq_pending = false;
            let lvt = self.regs[LAPIC_IDX_LVT_TIMER];
            let vector = (lvt & 0xFF) as u8;
            if vector >= 16 { Some(vector) } else { None }
        } else {
            None
        }
    }
}

/// Minimal IOAPIC state for indirect register access at 0xFEC00000.
const IOAPIC_NUM_PINS: usize = 24;

pub struct SoftIoapic {
    pub ioregsel: u32,
    id: u32,
    /// Redirection table: 24 entries, each 64-bit.
    redir: [u64; IOAPIC_NUM_PINS],
    /// Interrupt Request Register — one bit per pin.
    irr: u32,
    /// Pin level state for level-triggered handling.
    irq_level: [bool; IOAPIC_NUM_PINS],
    /// Pending interrupts to deliver via WHvRequestInterrupt.
    /// Each entry: (vector, dest_apic_id, dest_mode, trigger_mode, delivery_mode)
    pub pending: Vec<IoapicInterrupt>,
}

#[derive(Clone, Copy)]
pub struct IoapicInterrupt {
    pub vector: u8,
    pub dest: u8,
    pub dest_mode: u8,   // 0=physical, 1=logical
    pub trigger: u8,     // 0=edge, 1=level
    pub delivery: u8,    // 0=fixed, 1=lowest-pri
}

// Redirection entry bits
const REDIR_MASKED: u64    = 1 << 16;
const REDIR_LEVEL: u64     = 1 << 15;
const REDIR_REMOTE_IRR: u64 = 1 << 14;
const REDIR_DESTMODE: u64  = 1 << 11;
const REDIR_DELIV_STATUS: u64 = 1 << 12;
const REDIR_RO_BITS: u64   = REDIR_REMOTE_IRR | REDIR_DELIV_STATUS;

impl SoftIoapic {
    fn new() -> Self {
        let mut redir = [0u64; IOAPIC_NUM_PINS];
        for e in redir.iter_mut() {
            *e = REDIR_MASKED; // all masked by default
        }
        SoftIoapic {
            ioregsel: 0,
            id: 0,
            redir,
            irr: 0,
            irq_level: [false; IOAPIC_NUM_PINS],
            pending: Vec::new(),
        }
    }

    fn read_reg(&self, index: u32) -> u32 {
        match index {
            0x00 => self.id,
            0x01 => ((IOAPIC_NUM_PINS as u32 - 1) << 16) | 0x20, // version
            0x02 => self.id, // arbitration
            0x10..=0x3F => {
                let pin = ((index - 0x10) / 2) as usize;
                if pin < IOAPIC_NUM_PINS {
                    if (index & 1) == 0 { self.redir[pin] as u32 }
                    else { (self.redir[pin] >> 32) as u32 }
                } else { 0 }
            }
            _ => 0,
        }
    }

    fn write_reg(&mut self, index: u32, val: u32) {
        match index {
            0x00 => self.id = val & 0x0F00_0000,
            0x01 | 0x02 => {} // read-only
            0x10..=0x3F => {
                let pin = ((index - 0x10) / 2) as usize;
                if pin < IOAPIC_NUM_PINS {
                    let entry = &mut self.redir[pin];
                    let ro = *entry & REDIR_RO_BITS;
                    if (index & 1) == 0 {
                        *entry = (*entry & 0xFFFF_FFFF_0000_0000) | ((val as u64) & !REDIR_RO_BITS) | ro;
                    } else {
                        *entry = (*entry & 0x0000_0000_FFFF_FFFF) | ((val as u64) << 32);
                    }
                    // Edge mode clears Remote IRR (Linux workaround)
                    if (*entry & REDIR_LEVEL) == 0 {
                        *entry &= !REDIR_REMOTE_IRR;
                    }
                    self.service();
                }
            }
            _ => {}
        }
    }

    fn read(&self, offset: u64) -> u32 {
        match offset & 0xFF {
            0x00 => self.ioregsel,
            0x10 => self.read_reg(self.ioregsel),
            _ => 0,
        }
    }

    fn write(&mut self, offset: u64, val: u32) {
        match offset & 0xFF {
            0x00 => self.ioregsel = val,
            0x10 => self.write_reg(self.ioregsel, val),
            _ => {}
        }
    }

    /// Assert or deassert an IRQ pin (like QEMU ioapic_set_irq).
    pub fn set_irq(&mut self, pin: u8, level: bool) {
        let i = pin as usize;
        if i >= IOAPIC_NUM_PINS { return; }
        let entry = self.redir[i];
        let is_level = (entry & REDIR_LEVEL) != 0;

        if is_level {
            if level {
                self.irr |= 1 << pin;
                self.irq_level[i] = true;
                if (entry & REDIR_REMOTE_IRR) == 0 {
                    self.service();
                }
            } else {
                self.irq_level[i] = false;
                self.irr &= !(1 << pin);
            }
        } else {
            // Edge-triggered
            if level {
                if (entry & REDIR_MASKED) != 0 { return; }
                self.irr |= 1 << pin;
                self.service();
            }
        }
    }

    /// Service loop: scan IRR and produce pending interrupts for WHvRequestInterrupt.
    fn service(&mut self) {
        for i in 0..IOAPIC_NUM_PINS {
            if (self.irr & (1 << i)) == 0 { continue; }
            let entry = self.redir[i];
            if (entry & REDIR_MASKED) != 0 { continue; }
            let dm = ((entry >> 8) & 7) as u8;
            if dm > 1 { continue; } // only Fixed(0) and LowestPri(1)
            let vector = (entry & 0xFF) as u8;
            // Vectors 0-15 are reserved for exceptions — skip invalid entries
            // (guest may clear entries to 0 before reprogramming).
            if vector < 16 { continue; }
            let is_level = (entry & REDIR_LEVEL) != 0;

            if is_level {
                if (entry & REDIR_REMOTE_IRR) != 0 { continue; }
                self.redir[i] |= REDIR_REMOTE_IRR;
            } else {
                self.irr &= !(1 << i);
            }

            self.pending.push(IoapicInterrupt {
                vector,
                dest: ((entry >> 56) & 0xFF) as u8,
                dest_mode: if (entry & REDIR_DESTMODE) != 0 { 1 } else { 0 },
                trigger: if is_level { 1 } else { 0 },
                delivery: dm,
            });
        }
    }

    /// Take pending interrupts for delivery.
    pub fn take_pending(&mut self) -> Vec<IoapicInterrupt> {
        core::mem::take(&mut self.pending)
    }

    /// EOI broadcast from LAPIC — clear Remote IRR on matching entries.
    pub fn eoi_vector(&mut self, vector: u8) {
        for i in 0..IOAPIC_NUM_PINS {
            let entry = &mut self.redir[i];
            if (*entry & REDIR_LEVEL) == 0 { continue; }
            if (*entry & 0xFF) as u8 != vector { continue; }
            if (*entry & REDIR_REMOTE_IRR) == 0 { continue; }
            *entry &= !REDIR_REMOTE_IRR;
        }
        self.service();
    }
}

pub struct WhpBackend {
    partition: WHV_PARTITION_HANDLE,
    memory_slots: Vec<MemorySlot>,
    api: WhpApi,
    pub lapic: SoftLapic,
    ioapic: SoftIoapic,
    /// Pending MMIO read response: (value, dest_reg).
    /// Set by the FFI handler after dispatching to the device.
    /// Applied at the top of run_vcpu before re-entering the guest.
    pending_mmio_read: Option<(u64, u8)>,
}

unsafe impl Send for WhpBackend {}

fn check(hr: i32) -> Result<(), VmError> {
    if hr >= 0 { Ok(()) } else { Err(VmError::BackendError(hr)) }
}

impl WhpBackend {
    pub fn new(_ram_bytes: usize) -> Result<Self, VmError> {
        unsafe {
            let dll = LoadLibraryA(b"WinHvPlatform.dll\0".as_ptr());
            if dll.is_null() {
                return Err(VmError::NoHardwareSupport);
            }

            macro_rules! load {
                ($name:expr) => {{
                    let p = GetProcAddress(dll, concat!($name, "\0").as_ptr());
                    if p.is_null() {
                        return Err(VmError::NoHardwareSupport);
                    }
                    core::mem::transmute(p)
                }};
            }

            let api = WhpApi {
                get_capability: load!("WHvGetCapability"),
                create_partition: load!("WHvCreatePartition"),
                setup_partition: load!("WHvSetupPartition"),
                delete_partition: load!("WHvDeletePartition"),
                set_property: load!("WHvSetPartitionProperty"),
                map_gpa: load!("WHvMapGpaRange"),
                unmap_gpa: load!("WHvUnmapGpaRange"),
                create_vp: load!("WHvCreateVirtualProcessor"),
                delete_vp: load!("WHvDeleteVirtualProcessor"),
                run_vp: load!("WHvRunVirtualProcessor"),
                get_regs: load!("WHvGetVirtualProcessorRegisters"),
                set_regs: load!("WHvSetVirtualProcessorRegisters"),
                request_interrupt: {
                    let p = GetProcAddress(dll, b"WHvRequestInterrupt\0".as_ptr());
                    if p.is_null() {
                        whp_debug(format_args!("WHvRequestInterrupt NOT found in DLL"));
                        None
                    } else {
                        whp_debug(format_args!("WHvRequestInterrupt found at {:p}", p));
                        Some(core::mem::transmute(p))
                    }
                },
                cancel_run: load!("WHvCancelRunVirtualProcessor"),
            };

            // Check if the hypervisor is present
            // WHvCapabilityCodeHypervisorPresent = 0x00000000
            let mut present: u32 = 0;
            let mut written: u32 = 0;
            let hr = (api.get_capability)(
                0x00000000, // WHvCapabilityCodeHypervisorPresent
                &mut present as *mut u32 as *mut u8,
                core::mem::size_of::<u32>() as u32,
                &mut written,
            );
            if hr < 0 || present == 0 {
                return Err(VmError::NoHardwareSupport);
            }

            let mut partition: WHV_PARTITION_HANDLE = core::ptr::null_mut();
            let hr = (api.create_partition)(&mut partition);
            if hr < 0 {
                return Err(VmError::BackendErrorCtx(hr, "WHvCreatePartition"));
            }

            // Set processor count = 1
            let count: u32 = 1;
            let hr = (api.set_property)(
                partition,
                WHV_PROPERTY_PROCESSOR_COUNT,
                &count as *const u32 as *const u8,
                core::mem::size_of::<u32>() as u32,
            );
            if hr < 0 {
                (api.delete_partition)(partition);
                return Err(VmError::BackendErrorCtx(hr, "WHvSetPartitionProperty(ProcessorCount)"));
            }

            // Enable extended VM exits for CPUID, MSR, and exception interception
            // bit 0 = X64CpuidExit, bit 1 = X64MsrExit, bit 2 = ExceptionExit
            // Try property code 0x2 first (Windows SDK), fall back to 0x1 (older?)
            let extended_exits: u64 = 0x7;
            let hr = (api.set_property)(
                partition,
                0x00000002, // WHvPartitionPropertyCodeExtendedVmExits
                &extended_exits as *const u64 as *const u8,
                core::mem::size_of::<u64>() as u32,
            );
            if hr < 0 {
                whp_debug(format_args!("ExtendedVmExits(0x2) FAILED hr={:#x}, trying 0x1", hr));
                let hr2 = (api.set_property)(
                    partition,
                    0x00000001,
                    &extended_exits as *const u64 as *const u8,
                    core::mem::size_of::<u64>() as u32,
                );
                if hr2 < 0 {
                    whp_debug(format_args!("ExtendedVmExits(0x1) also FAILED hr={:#x}", hr2));
                } else {
                    whp_debug(format_args!("ExtendedVmExits(0x1) OK: {:#x}", extended_exits));
                }
            } else {
                whp_debug(format_args!("ExtendedVmExits(0x2) OK: {:#x}", extended_exits));
            }

            // Exception exit bitmap: catch #DF and #UD.
            // Without this, WHP silently resets the VP on triple fault.
            let exc_bitmap: u64 = (1u64 << 6) | (1u64 << 8);
            let hr = (api.set_property)(
                partition,
                WHV_PROPERTY_EXCEPTION_EXIT_BITMAP,
                &exc_bitmap as *const u64 as *const u8,
                core::mem::size_of::<u64>() as u32,
            );
            if hr < 0 {
                whp_debug(format_args!("ExceptionExitBitmap set FAILED: hr={:#x}", hr));
            } else {
                whp_debug(format_args!("ExceptionExitBitmap set OK: {:#x}", exc_bitmap));
            }

            // XApic emulation — WHP handles LAPIC internally (timer, ISR/IRR, EOI).
            // WHvPartitionPropertyCodeLocalApicEmulationMode = 0x00001005
            let apic_mode: u32 = 1;
            let hr = (api.set_property)(
                partition,
                0x00001005,
                &apic_mode as *const u32 as *const u8,
                core::mem::size_of::<u32>() as u32,
            );
            if hr < 0 {
                whp_debug(format_args!("LocalApicEmulationMode(xApic=1) FAILED hr={:#x}", hr));
            } else {
                whp_debug(format_args!("LocalApicEmulationMode set to xApic(1)"));
            }

            let hr = (api.setup_partition)(partition);
            if hr < 0 {
                (api.delete_partition)(partition);
                return Err(VmError::BackendErrorCtx(hr, "WHvSetupPartition"));
            }

            // Store partition handle and cancel function for thread-safe cancel support.
            CANCEL_PARTITION.store(partition as u64, core::sync::atomic::Ordering::Relaxed);
            CANCEL_FN_PTR.store(api.cancel_run as usize as u64, core::sync::atomic::Ordering::Relaxed);

            Ok(WhpBackend {
                partition,
                memory_slots: Vec::new(),
                api,
                lapic: SoftLapic::new(),
                ioapic: SoftIoapic::new(),
                pending_mmio_read: None,
            })
        }
    }

    /// Store a pending MMIO read response to be applied before the next VM entry.
    pub fn set_pending_mmio_read(&mut self, value: u64, dest_reg: u8) {
        self.pending_mmio_read = Some((value, dest_reg));
    }

    fn get_regs_raw(&self, id: u32, names: &[u32], values: &mut [WHV_REGISTER_VALUE]) -> Result<(), VmError> {
        check(unsafe {
            (self.api.get_regs)(
                self.partition, id,
                names.as_ptr(), names.len() as u32,
                values.as_mut_ptr(),
            )
        })
    }

    fn set_regs_raw(&self, id: u32, names: &[u32], values: &[WHV_REGISTER_VALUE]) -> Result<(), VmError> {
        check(unsafe {
            (self.api.set_regs)(
                self.partition, id,
                names.as_ptr(), names.len() as u32,
                values.as_ptr(),
            )
        })
    }
}

/// Decode an MMIO instruction to extract access size and write data.
/// WHP doesn't provide these in the exit context, so we parse the instruction bytes.
/// Returns (access_size, write_data). write_data is 0 for reads.
fn decode_mmio_instruction(instr: &[u8], regs: &VcpuRegs) -> (u8, u64) {
    if instr.is_empty() {
        return (4, 0);
    }

    let mut i = 0;
    let mut operand_size_override = false;
    let mut rex = 0u8;

    // Skip prefixes
    while i < instr.len() {
        match instr[i] {
            0x66 => { operand_size_override = true; i += 1; }
            0x67 | 0xF0 | 0xF2 | 0xF3 | 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 => { i += 1; }
            0x40..=0x4F => { rex = instr[i]; i += 1; }
            _ => break,
        }
    }

    if i >= instr.len() {
        return (4, 0);
    }

    let rex_w = (rex & 0x08) != 0;

    let size: u8 = if rex_w { 8 } else if operand_size_override { 2 } else { 4 };

    let opcode = instr[i];
    match opcode {
        // MOV r/m8, r8 (write)
        0x88 => {
            let reg_val = get_reg_from_modrm(instr.get(i + 1).copied().unwrap_or(0), regs, true, rex);
            (1, reg_val)
        }
        // MOV r/m16/32/64, r16/32/64 (write)
        0x89 => {
            let reg_val = get_reg_from_modrm(instr.get(i + 1).copied().unwrap_or(0), regs, false, rex);
            (size, reg_val)
        }
        // MOV r8, r/m8 (read)
        0x8A => (1, 0),
        // MOV r16/32/64, r/m16/32/64 (read)
        0x8B => (size, 0),
        // MOV r/m8, imm8 (write)
        0xC6 => {
            let (_, imm_off) = skip_modrm(&instr[i..]);
            let imm = instr.get(i + imm_off).copied().unwrap_or(0) as u64;
            (1, imm)
        }
        // MOV r/m16/32/64, imm16/32 (write)
        0xC7 => {
            let (_, imm_off) = skip_modrm(&instr[i..]);
            let imm_size = if operand_size_override { 2 } else { 4 };
            let imm = read_imm(&instr[i + imm_off..], imm_size);
            (size, imm)
        }
        // MOV AL, moffs (read) / MOV moffs, AL (write)
        0xA0 => (1, 0),
        0xA1 => (size, 0),
        0xA2 => (1, regs.rax & 0xFF),
        0xA3 => (size, regs.rax),
        // MOVS (REP prefix handled above, single step)
        0xA4 => (1, 0), // MOVSB - read+write, treat as read
        0xA5 => (size, 0),
        // STOS
        0xAA => (1, regs.rax & 0xFF),
        0xAB => (size, regs.rax),
        // Two-byte opcodes (0x0F prefix)
        0x0F if i + 1 < instr.len() => {
            match instr[i + 1] {
                // MOVZX r, r/m8
                0xB6 => (1, 0),
                // MOVZX r, r/m16
                0xB7 => (2, 0),
                // MOVSX r, r/m8
                0xBE => (1, 0),
                // MOVSX r, r/m16
                0xBF => (2, 0),
                _ => (size, 0),
            }
        }
        _ => (size, 0),
    }
}

/// Extract the register value indicated by the reg field of ModR/M byte.
/// `rex` is the REX prefix byte (0 if none). REX.R extends the reg field to 4 bits.
fn get_reg_from_modrm(modrm: u8, regs: &VcpuRegs, byte_reg: bool, rex: u8) -> u64 {
    let mut reg = ((modrm >> 3) & 7) as usize;
    if (rex & 0x04) != 0 { reg += 8; } // REX.R
    let vals: [u64; 16] = [
        regs.rax, regs.rcx, regs.rdx, regs.rbx, regs.rsp, regs.rbp, regs.rsi, regs.rdi,
        regs.r8,  regs.r9,  regs.r10, regs.r11, regs.r12, regs.r13, regs.r14, regs.r15,
    ];
    let v = vals[reg];
    if byte_reg { v & 0xFF } else { v }
}

/// Skip past the ModR/M and SIB bytes + displacement, return (modrm_byte, total_bytes_consumed)
fn skip_modrm(instr: &[u8]) -> (u8, usize) {
    if instr.len() < 2 { return (0, 2); }
    let modrm = instr[1];
    let mod_bits = (modrm >> 6) & 3;
    let rm = modrm & 7;
    let mut off = 2usize; // opcode + modrm
    if mod_bits != 3 {
        if rm == 4 { off += 1; } // SIB byte
        match mod_bits {
            0 if rm == 5 => { off += 4; } // disp32
            1 => { off += 1; } // disp8
            2 => { off += 4; } // disp32
            _ => {}
        }
    }
    (modrm, off)
}

/// Read an immediate value of the given size
fn read_imm(data: &[u8], size: u8) -> u64 {
    match size {
        1 if data.len() >= 1 => data[0] as u64,
        2 if data.len() >= 2 => u16::from_le_bytes([data[0], data[1]]) as u64,
        4 if data.len() >= 4 => u32::from_le_bytes([data[0], data[1], data[2], data[3]]) as u64,
        _ => 0,
    }
}

/// Compute instruction length from raw bytes (for MEMORY_ACCESS exits where
/// VP context InstructionLength is 0).
fn compute_instr_len(instr: &[u8]) -> u64 {
    if instr.is_empty() { return 1; }
    let mut i = 0;
    let mut has_addr_override = false;
    let mut has_op_override = false;
    // Skip prefixes
    while i < instr.len() {
        match instr[i] {
            0x67 => { has_addr_override = true; i += 1; }
            0x66 => { has_op_override = true; i += 1; }
            0xF0 | 0xF2 | 0xF3
            | 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65
            | 0x40..=0x4F => { i += 1; }
            _ => break,
        }
    }
    if i >= instr.len() { return i as u64; }
    let prefix_len = i;
    let op = instr[i];
    // MOV AL/AX, moffs (A0/A1) or MOV moffs, AL/AX (A2/A3)
    if op >= 0xA0 && op <= 0xA3 {
        let addr_size: usize = if has_addr_override { 2 } else { 4 };
        return (prefix_len + 1 + addr_size) as u64;
    }
    // Two-byte opcode (0F xx)
    let opcode_len = if op == 0x0F { 2 } else { 1 };
    let modrm_start = prefix_len + opcode_len;
    if modrm_start >= instr.len() { return modrm_start as u64; }
    // Opcodes that have ModR/M
    let has_modrm = match op {
        0x88 | 0x89 | 0x8A | 0x8B | 0xC6 | 0xC7 => true,
        0x0F => true, // MOVZX/MOVSX etc.
        _ => false,
    };
    if !has_modrm {
        return (prefix_len + opcode_len) as u64;
    }
    // Parse ModR/M + SIB + displacement
    let modrm = instr[modrm_start];
    let mod_bits = (modrm >> 6) & 3;
    let rm = modrm & 7;
    let mut off = modrm_start + 1; // past modrm
    if mod_bits != 3 {
        if rm == 4 { off += 1; } // SIB
        match mod_bits {
            0 if rm == 5 => { off += 4; } // disp32
            1 => { off += 1; } // disp8
            2 => { off += 4; } // disp32
            _ => {}
        }
    }
    // Immediate for MOV r/m, imm
    match op {
        0xC6 => { off += 1; } // imm8
        0xC7 => { off += if has_op_override { 2 } else { 4 }; } // imm16/32
        _ => {}
    }
    off as u64
}

/// Decode the destination register index from an MMIO read instruction.
/// Returns 0-7 corresponding to RAX..RDI. Falls back to 0 (RAX).
fn decode_mmio_dest_reg(instr: &[u8]) -> u8 {
    let mut i = 0;
    // Skip prefixes
    while i < instr.len() {
        match instr[i] {
            0x66 | 0x67 | 0xF0 | 0xF2 | 0xF3
            | 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65
            | 0x40..=0x4F => { i += 1; }
            _ => break,
        }
    }
    if i >= instr.len() { return 0; }
    let op = instr[i];
    // MOV r, r/m (0x8B, 0x8A) or MOVZX/MOVSX (0F B6/B7/BE/BF)
    if (op == 0x8B || op == 0x8A) && i + 1 < instr.len() {
        return (instr[i + 1] >> 3) & 7;
    }
    if op == 0x0F && i + 2 < instr.len() {
        let op2 = instr[i + 1];
        if op2 == 0xB6 || op2 == 0xB7 || op2 == 0xBE || op2 == 0xBF {
            return (instr[i + 2] >> 3) & 7;
        }
    }
    0 // default to RAX
}

fn seg_to_whv(seg: &SegmentReg) -> WhvSegment {
    let attrs: u16 =
        (seg.type_ as u16 & 0xF)
        | ((seg.s as u16 & 1) << 4)
        | ((seg.dpl as u16 & 3) << 5)
        | ((seg.present as u16 & 1) << 7)
        | ((seg.avl as u16 & 1) << 12)
        | ((seg.l as u16 & 1) << 13)
        | ((seg.db as u16 & 1) << 14)
        | ((seg.g as u16 & 1) << 15);
    WhvSegment {
        base: seg.base,
        limit: seg.limit,
        selector: seg.selector,
        attributes: attrs,
    }
}

fn whv_to_seg(s: &WhvSegment) -> SegmentReg {
    let a = s.attributes;
    SegmentReg {
        base: s.base,
        limit: s.limit,
        selector: s.selector,
        type_: (a & 0xF) as u8,
        s: ((a >> 4) & 1) as u8,
        dpl: ((a >> 5) & 3) as u8,
        present: ((a >> 7) & 1) as u8,
        avl: ((a >> 12) & 1) as u8,
        l: ((a >> 13) & 1) as u8,
        db: ((a >> 14) & 1) as u8,
        g: ((a >> 15) & 1) as u8,
    }
}

impl WhpBackend {
    /// Read the counter register respecting address size.
    fn read_counter(rcx: u64, addr_size: u8) -> u64 {
        match addr_size {
            2 => rcx & 0xFFFF,
            4 => rcx & 0xFFFF_FFFF,
            _ => rcx,
        }
    }
}

impl WhpBackend {
    /// Deliver pending IOAPIC interrupts via direct PendingEvent ExtInt injection.
    /// WHvRequestInterrupt returns success but WHP's internal LAPIC never delivers,
    /// so we bypass it entirely and inject the same way as PIC interrupts.
    /// PendingEvent can only hold one event, so we inject the first and re-queue the rest.
    fn deliver_ioapic_pending(&mut self) {
        let pending = self.ioapic.take_pending();
        if pending.is_empty() { return; }

        // Check RFLAGS.IF — can only inject if interrupts are enabled
        let mut rflags_val = [WHV_REGISTER_VALUE::default()];
        let if_set = if self.get_regs_raw(0, &[REG_RFLAGS], &mut rflags_val).is_ok() {
            (unsafe { rflags_val[0].reg64 } & 0x200) != 0
        } else {
            false
        };

        if !if_set {
            // Can't inject now — put them all back and request interrupt window
            for intr in pending {
                self.ioapic.pending.push(intr);
            }
            let _ = self.request_interrupt_window(0, true);
            return;
        }

        // Inject the first one via PendingEvent ExtInt (same as PIC path)
        let first = &pending[0];
        {
            static IOAPIC_INTR_CTR: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let cnt = IOAPIC_INTR_CTR.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if cnt < 50 {
                whp_debug(format_args!("IOAPIC direct inject: vec={} dest={} dm={} trig={} del={}",
                    first.vector, first.dest, first.dest_mode, first.trigger, first.delivery));
            }
        }
        let _ = self.inject_interrupt(0, first.vector);

        // Re-queue remaining interrupts
        if pending.len() > 1 {
            for intr in &pending[1..] {
                self.ioapic.pending.push(intr.clone());
            }
            // Request interrupt window so we get called again for the rest
            let _ = self.request_interrupt_window(0, true);
        }
    }

    /// Route a device IRQ through the IOAPIC and deliver via WHvRequestInterrupt.
    pub fn ioapic_set_irq(&mut self, pin: u8, level: bool) {
        self.ioapic.set_irq(pin, level);
        self.deliver_ioapic_pending();
    }
}

impl VmBackend for WhpBackend {
    fn destroy(&mut self) {
        unsafe {
            (self.api.delete_partition)(self.partition);
        }
        self.partition = core::ptr::null_mut();
        self.memory_slots.clear();
    }

    fn reset(&mut self) -> Result<(), VmError> {
        // WHP has no direct reset; caller recreates partition
        Ok(())
    }

    fn set_memory_region(&mut self, slot: u32, guest_phys: u64, size: u64, host_ptr: *mut u8) -> Result<(), VmError> {
        // If slot exists, unmap first
        if let Some(pos) = self.memory_slots.iter().position(|s| s.slot == slot) {
            let old = &self.memory_slots[pos];
            let _ = unsafe { (self.api.unmap_gpa)(self.partition, old.guest_phys, old.size) };
            self.memory_slots.remove(pos);
        }

        if size == 0 {
            return Ok(());
        }

        let flags = WHV_MAP_GPA_RANGE_FLAG_READ | WHV_MAP_GPA_RANGE_FLAG_WRITE | WHV_MAP_GPA_RANGE_FLAG_EXECUTE;
        check(unsafe {
            (self.api.map_gpa)(self.partition, host_ptr, guest_phys, size, flags)
        })?;

        self.memory_slots.push(MemorySlot { slot, guest_phys, size, host_ptr });
        Ok(())
    }

    fn read_phys(&self, addr: u64, buf: &mut [u8]) -> Result<(), VmError> {
        for slot in &self.memory_slots {
            if addr >= slot.guest_phys && addr + buf.len() as u64 <= slot.guest_phys + slot.size {
                let offset = (addr - slot.guest_phys) as usize;
                unsafe {
                    core::ptr::copy_nonoverlapping(slot.host_ptr.add(offset), buf.as_mut_ptr(), buf.len());
                }
                return Ok(());
            }
        }
        Err(VmError::MemoryMapFailed)
    }

    fn write_phys(&mut self, addr: u64, buf: &[u8]) -> Result<(), VmError> {
        for slot in &self.memory_slots {
            if addr >= slot.guest_phys && addr + buf.len() as u64 <= slot.guest_phys + slot.size {
                let offset = (addr - slot.guest_phys) as usize;
                unsafe {
                    core::ptr::copy_nonoverlapping(buf.as_ptr(), slot.host_ptr.add(offset), buf.len());
                }
                return Ok(());
            }
        }
        Err(VmError::MemoryMapFailed)
    }

    fn create_vcpu(&mut self, id: u32) -> Result<(), VmError> {
        check(unsafe { (self.api.create_vp)(self.partition, id, 0) })?;

        // Smoke test: try reading a single register (RIP) to verify the VP works
        let mut val = WHV_REGISTER_VALUE::default();
        let name = REG_RIP;
        let hr = unsafe {
            (self.api.get_regs)(self.partition, id, &name, 1, &mut val)
        };
        if hr < 0 {
            return Err(VmError::BackendErrorCtx(hr, "WHvGetVirtualProcessorRegisters(RIP) after create"));
        }

        // Set IA32_APIC_BASE to standard xAPIC mode: base=0xFEE00000, BSP=1, Enable=1.
        // Without this, WHP may inherit the host's x2APIC-enabled value (bit 10 set),
        // causing the guest kernel to try x2APIC MSR access which our soft LAPIC can't handle.
        // bit 8 = BSP, bit 11 = APIC Global Enable, bit 10 must be CLEAR (no x2APIC)
        let apic_base_val = 0xFEE0_0000u64 | (1 << 8) | (1 << 11); // 0xFEE00900
        let _ = self.set_regs_raw(id, &[REG_APIC_BASE], &[WHV_REGISTER_VALUE::from_u64(apic_base_val)]);

        // Also log what APIC_BASE was before we set it, for debugging
        let mut apic_val = [WHV_REGISTER_VALUE::default()];
        if self.get_regs_raw(id, &[REG_APIC_BASE], &mut apic_val).is_ok() {
            whp_debug(format_args!("APIC_BASE after set: {:#x}", unsafe { apic_val[0].reg64 }));
        }

        Ok(())
    }

    fn destroy_vcpu(&mut self, id: u32) -> Result<(), VmError> {
        check(unsafe { (self.api.delete_vp)(self.partition, id) })
    }

    fn run_vcpu(&mut self, id: u32) -> Result<VmExitReason, VmError> {
        static FIRST: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(true);
        if FIRST.swap(false, core::sync::atomic::Ordering::Relaxed) {
            whp_debug(format_args!("run_vcpu entered for the first time"));
        }
        let mut inner_loops = 0u32;
        loop {
            // Break out of inner loop periodically so the vmmanager can advance
            // the PIT timer, poll IRQs, and process other events. Without this,
            // LAPIC/IOAPIC MMIO handled internally via `continue` can starve
            // the timer and cause the guest to hang polling LAPIC timer counts.
            inner_loops += 1;
            if inner_loops > 64 {
                return Ok(VmExitReason::InterruptWindow);
            }

            // Apply pending MMIO read response before re-entering the guest.
            // This sets the destination register with the device's read value.
            if let Some((value, dest_reg)) = self.pending_mmio_read.take() {
                let mut regs = self.get_vcpu_regs(id)?;
                // apply MMIO logging removed to save budget
                match dest_reg {
                    0 => regs.rax = value,
                    1 => regs.rcx = value,
                    2 => regs.rdx = value,
                    3 => regs.rbx = value,
                    4 => regs.rsp = value,
                    5 => regs.rbp = value,
                    6 => regs.rsi = value,
                    7 => regs.rdi = value,
                    _ => regs.rax = value,
                }
                if let Err(e) = self.set_vcpu_regs(id, &regs) {
                    whp_debug(format_args!("set_vcpu_regs FAILED: {:?}", e));
                    return Err(e);
                }
                // verify logging removed to save budget
            }

            let mut exit_ctx = core::mem::MaybeUninit::<WHV_RUN_VP_EXIT_CONTEXT>::uninit();
            check(unsafe {
                (self.api.run_vp)(
                    self.partition, id,
                    exit_ctx.as_mut_ptr() as *mut u8,
                    core::mem::size_of::<WHV_RUN_VP_EXIT_CONTEXT>() as u32,
                )
            })?;

            let ctx = unsafe { exit_ctx.assume_init() };
            // Instruction length is lower 4 bits of vp_context[2] (upper 4 bits = Cr8)
            let instr_len = (ctx.vp_context[2] & 0x0F) as u64;

            // Log ALL exits after IOAPIC init to trace the reboot cause
            {
                static IOAPIC_SEEN: core::sync::atomic::AtomicBool = core::sync::atomic::AtomicBool::new(false);
                static EXIT_LOG_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

                // Detect IOAPIC MMIO (GPA 0xFEC00000..0xFEC00FFF)
                if ctx.exit_reason == WHV_EXIT_REASON_MEMORY_ACCESS {
                    let gpa = u64::from_le_bytes([
                        ctx.exit_data[0x18], ctx.exit_data[0x19], ctx.exit_data[0x1A], ctx.exit_data[0x1B],
                        ctx.exit_data[0x1C], ctx.exit_data[0x1D], ctx.exit_data[0x1E], ctx.exit_data[0x1F],
                    ]);
                    if gpa >= 0xFEC0_0000 && gpa < 0xFEC0_1000 {
                        IOAPIC_SEEN.store(true, core::sync::atomic::Ordering::Relaxed);
                    }
                }

                if IOAPIC_SEEN.load(core::sync::atomic::Ordering::Relaxed) {
                    // Filter out noisy exits: VGA palette, debug port, IOAPIC MMIO, CPUID, CANCEL
                    let skip = match ctx.exit_reason {
                        WHV_EXIT_REASON_IO_PORT => {
                            let port = u16::from_le_bytes([ctx.exit_data[0x18], ctx.exit_data[0x19]]);
                            matches!(port, 0x03C8 | 0x03C9 | 0x03C6 | 0x03DA | 0x0402
                                | 0x01CE | 0x01CF | 0x01D0  // Bochs VBE
                                | 0x0070 | 0x0071            // CMOS
                                | 0x03D4 | 0x03D5            // VGA CRTC
                                | 0x00ED                     // Linux I/O delay port
                            )
                        }
                        WHV_EXIT_REASON_MEMORY_ACCESS => {
                            let gpa = u64::from_le_bytes([
                                ctx.exit_data[0x18], ctx.exit_data[0x19], ctx.exit_data[0x1A], ctx.exit_data[0x1B],
                                ctx.exit_data[0x1C], ctx.exit_data[0x1D], ctx.exit_data[0x1E], ctx.exit_data[0x1F],
                            ]);
                            // Skip IOAPIC and LAPIC MMIO, VGA framebuffer
                            (0xFEC0_0000..0xFEC0_1000).contains(&gpa)
                                || (0xFEE0_0000..0xFEE0_1000).contains(&gpa)
                                || gpa < 0xC0000 && gpa >= 0xA0000
                        }
                        0x2001 => true, // CANCEL
                        0x1001 => true, // CPUID
                        0x1000 => true, // MSR
                        _ => false,
                    };
                    if !skip {
                        let n = EXIT_LOG_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        if n < 500 {
                            let rip = u64::from_le_bytes([
                                ctx.vp_context[24], ctx.vp_context[25], ctx.vp_context[26], ctx.vp_context[27],
                                ctx.vp_context[28], ctx.vp_context[29], ctx.vp_context[30], ctx.vp_context[31],
                            ]);
                            let detail = match ctx.exit_reason {
                                WHV_EXIT_REASON_IO_PORT => {
                                    let port = u16::from_le_bytes([ctx.exit_data[0x18], ctx.exit_data[0x19]]);
                                    let access = u32::from_le_bytes([ctx.exit_data[0x14], ctx.exit_data[0x15], ctx.exit_data[0x16], ctx.exit_data[0x17]]);
                                    let wr = if access & 1 != 0 { "W" } else { "R" };
                                    let rax = u64::from_le_bytes([
                                        ctx.exit_data[0x20], ctx.exit_data[0x21], ctx.exit_data[0x22], ctx.exit_data[0x23],
                                        ctx.exit_data[0x24], ctx.exit_data[0x25], ctx.exit_data[0x26], ctx.exit_data[0x27],
                                    ]);
                                    std::format!("IO {}{:#06x} val={:#x}", wr, port, rax & 0xFF)
                                }
                                WHV_EXIT_REASON_MEMORY_ACCESS => {
                                    let gpa = u64::from_le_bytes([
                                        ctx.exit_data[0x18], ctx.exit_data[0x19], ctx.exit_data[0x1A], ctx.exit_data[0x1B],
                                        ctx.exit_data[0x1C], ctx.exit_data[0x1D], ctx.exit_data[0x1E], ctx.exit_data[0x1F],
                                    ]);
                                    std::format!("MMIO gpa={:#x}", gpa)
                                }
                                WHV_EXIT_REASON_HALT => std::format!("HALT"),
                                0x1000 => std::format!("MSR"),
                                WHV_EXIT_REASON_UNRECOVERABLE_EXCEPTION => std::format!("UNRECOVERABLE"),
                                WHV_EXIT_REASON_INVALID_VP_STATE => std::format!("INVALID_VP"),
                                0x7 => std::format!("IRQ_WINDOW"),
                                0x9 => std::format!("APIC_EOI"),
                                other => std::format!("exit={:#x}", other),
                            };
                            whp_debug(format_args!("[{}] rip={:#x} {}", n, rip, detail));
                        }
                    }
                }
            }

            // Debug: log when inner loop hits limit (late boot only)
            if inner_loops == 64 {
                static STUCK_TOTAL: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                let total = STUCK_TOTAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                // Skip early boot, log after 1000 batch-limits
                if total >= 1000 && total < 1010 {
                    let rip = self.get_vcpu_regs(id).map(|r| r.rip).unwrap_or(0);
                    let rflags = self.get_vcpu_regs(id).map(|r| r.rflags).unwrap_or(0);
                    whp_debug(format_args!("late_stuck: exit={} rip={:#x} rflags={:#x} ilen={}", ctx.exit_reason, rip, rflags, instr_len));
                }
            }

            match ctx.exit_reason {
                WHV_EXIT_REASON_HALT => {
                    return Ok(VmExitReason::Halted);
                }
                WHV_EXIT_REASON_IO_PORT => {
                    // WHV_X64_IO_PORT_ACCESS_CONTEXT layout (from Windows SDK):
                    // [0x00]     InstructionByteCount: u8
                    // [0x01..04] Reserved: [u8; 3]
                    // [0x04..14] InstructionBytes: [u8; 16]
                    // [0x14..18] AccessInfo: u32
                    //            bit 0 = IsWrite, bits 1-3 = AccessSize,
                    //            bit 4 = StringOp, bit 5 = RepPrefix
                    // [0x18..1A] PortNumber: u16
                    // [0x1A..20] Reserved
                    // [0x20..28] Rax: u64
                    // [0x28..30] Rcx: u64
                    // [0x30..38] Rsi: u64
                    // [0x38..40] Rdi: u64
                    // [0x40..50] Ds: WHV_X64_SEGMENT_REGISTER (16 bytes)
                    // [0x50..60] Es: WHV_X64_SEGMENT_REGISTER (16 bytes)
                    let data = &ctx.exit_data;
                    let access_info = u32::from_le_bytes([data[0x14], data[0x15], data[0x16], data[0x17]]);
                    let is_write = (access_info & 1) != 0;
                    let access_size = ((access_info >> 1) & 0x7) as u8;
                    let access_size = if access_size == 0 { 1 } else { access_size };
                    let string_op = (access_info & (1 << 4)) != 0;
                    let rep_prefix = (access_info & (1 << 5)) != 0;
                    let port = u16::from_le_bytes([data[0x18], data[0x19]]);
                    let rax = u64::from_le_bytes([
                        data[0x20], data[0x21], data[0x22], data[0x23],
                        data[0x24], data[0x25], data[0x26], data[0x27],
                    ]);

                    if string_op {
                        // REP INSB/OUTSB: bulk I/O to/from guest memory.
                        let rcx = u64::from_le_bytes([
                            data[0x28], data[0x29], data[0x2A], data[0x2B],
                            data[0x2C], data[0x2D], data[0x2E], data[0x2F],
                        ]);
                        let rsi = u64::from_le_bytes([
                            data[0x30], data[0x31], data[0x32], data[0x33],
                            data[0x34], data[0x35], data[0x36], data[0x37],
                        ]);
                        let rdi = u64::from_le_bytes([
                            data[0x38], data[0x39], data[0x3A], data[0x3B],
                            data[0x3C], data[0x3D], data[0x3E], data[0x3F],
                        ]);
                        // ES segment base (for INSB destination)
                        let es_base = u64::from_le_bytes([
                            data[0x50], data[0x51], data[0x52], data[0x53],
                            data[0x54], data[0x55], data[0x56], data[0x57],
                        ]);
                        // DS segment base (for OUTSB source)
                        let ds_base = u64::from_le_bytes([
                            data[0x40], data[0x41], data[0x42], data[0x43],
                            data[0x44], data[0x45], data[0x46], data[0x47],
                        ]);

                        // Determine address size from CS.D bit (in VP context)
                        // VP context CS is at vp_context[8..24], attributes at [8+14..8+16]
                        let cs_attr = u16::from_le_bytes([ctx.vp_context[22], ctx.vp_context[23]]);
                        let cs_l = (cs_attr >> 13) & 1; // Long mode
                        let cs_d = (cs_attr >> 14) & 1; // Default size
                        let addr_size: u8 = if cs_l == 1 { 8 } else if cs_d == 1 { 4 } else { 2 };

                        // Direction from RFLAGS (in VP context at offset [32..40])
                        let rflags = u64::from_le_bytes([
                            ctx.vp_context[32], ctx.vp_context[33], ctx.vp_context[34], ctx.vp_context[35],
                            ctx.vp_context[36], ctx.vp_context[37], ctx.vp_context[38], ctx.vp_context[39],
                        ]);
                        let df = (rflags >> 10) & 1;
                        let step: i64 = if df == 0 { access_size as i64 } else { -(access_size as i64) };

                        let count = if rep_prefix {
                            Self::read_counter(rcx, addr_size)
                        } else {
                            1 // Single INS/OUTS without REP
                        };

                        if count == 0 {
                            // REP with CX=0: just advance RIP, nothing to do
                            let mut regs = self.get_vcpu_regs(id)?;
                            regs.rip += instr_len;
                            self.set_vcpu_regs(id, &regs)?;
                            continue;
                        }

                        // Mask offset register to address size (16-bit in real mode)
                        let addr_mask: u64 = match addr_size {
                            2 => 0xFFFF,
                            4 => 0xFFFF_FFFF,
                            _ => u64::MAX,
                        };
                        let gpa = if is_write {
                            ds_base.wrapping_add(rsi & addr_mask)
                        } else {
                            es_base.wrapping_add(rdi & addr_mask)
                        };

                        return Ok(VmExitReason::StringIo {
                            port, is_write, count, gpa, step, instr_len, addr_size, access_size,
                        });
                    }

                    // Regular (non-string) I/O: advance RIP and return
                    let mut regs = self.get_vcpu_regs(id)?;
                    regs.rip += instr_len;
                    self.set_vcpu_regs(id, &regs)?;

                    return if is_write {
                        Ok(VmExitReason::IoOut { port, size: access_size, data: rax as u32, count: 1 })
                    } else {
                        Ok(VmExitReason::IoIn { port, size: access_size, count: 1 })
                    };
                }
                WHV_EXIT_REASON_MEMORY_ACCESS => {
                    // WHV_MEMORY_ACCESS_CONTEXT layout:
                    // [0x00]     InstructionByteCount: u8
                    // [0x01..04] Reserved: [u8; 3]
                    // [0x04..14] InstructionBytes: [u8; 16]
                    // [0x14..18] AccessInfo: u32 (bits 0-1=AccessType: 0=Read,1=Write,2=Execute)
                    // [0x18..20] Gpa: u64
                    // [0x20..28] Gva: u64
                    let data = &ctx.exit_data;
                    let instr_byte_count = data[0x00] as usize;
                    let instr_bytes = &data[0x04..0x04 + instr_byte_count.min(16)];
                    let access_info = u32::from_le_bytes([data[0x14], data[0x15], data[0x16], data[0x17]]);
                    let access_type = access_info & 0x3;
                    let is_write = access_type == 1;
                    let gpa = u64::from_le_bytes([
                        data[0x18], data[0x19], data[0x1A], data[0x1B],
                        data[0x1C], data[0x1D], data[0x1E], data[0x1F],
                    ]);

                    // Decode instruction to get access size and write data
                    let regs = self.get_vcpu_regs(id)?;
                    let (access_size, write_data) = decode_mmio_instruction(instr_bytes, &regs);

                    // VP context InstructionLength is 0 for MEMORY_ACCESS exits.
                    // Compute from instruction bytes instead.
                    let instr_len = if instr_len == 0 && instr_byte_count > 0 {
                        compute_instr_len(instr_bytes)
                    } else {
                        instr_len
                    };

                    // LAPIC MMIO (0xFEE00000) handled by WHP in XApic mode — no exits expected.
                    if gpa >= 0xFEE0_0000 && gpa < 0xFEE0_1000 {
                        whp_debug(format_args!("LAPIC MMIO in XApic mode: gpa={:#x} write={} data={:#x} rip={:#x}",
                            gpa, is_write, write_data, regs.rip));
                        let mut new_regs = regs;
                        new_regs.rip += instr_len;
                        self.set_vcpu_regs(id, &new_regs)?;
                        continue;
                    }

                    // Handle IOAPIC MMIO internally (0xFEC00000-0xFEC00FFF)
                    if gpa >= 0xFEC0_0000 && gpa < 0xFEC0_1000 {
                        let mmio_off = gpa - 0xFEC0_0000;
                        {
                            static IOAPIC_LOG_CTR: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                            let cnt = IOAPIC_LOG_CTR.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                            if is_write {
                                whp_debug(format_args!("IOAPIC WR [{:#05x}] = {:#x} rip={:#x} sel={:#x}",
                                    mmio_off, write_data, regs.rip, self.ioapic.ioregsel));
                            } else if cnt < 200 || cnt % 1000 == 0 {
                                let val = self.ioapic.read(mmio_off);
                                whp_debug(format_args!("IOAPIC RD [{:#05x}] = {:#x} rip={:#x} sel={:#x}",
                                    mmio_off, val, regs.rip, self.ioapic.ioregsel));
                            }
                        }
                        let mut new_regs = regs;
                        new_regs.rip += instr_len;
                        if is_write {
                            self.ioapic.write(mmio_off, write_data as u32);
                            self.deliver_ioapic_pending();
                        } else {
                            let val = self.ioapic.read(mmio_off) as u64;
                            // Decode dest register same as LAPIC
                            if instr_byte_count > 0 {
                                let mut pi = 0;
                                let mut io_rex = 0u8;
                                while pi < instr_bytes.len() {
                                    match instr_bytes[pi] {
                                        0x66 | 0x67 | 0xF0 | 0xF2 | 0xF3
                                        | 0x26 | 0x2E | 0x36 | 0x3E | 0x64 | 0x65 => { pi += 1; }
                                        b @ 0x40..=0x4F => { io_rex = b; pi += 1; }
                                        _ => break,
                                    }
                                }
                                if pi < instr_bytes.len() && (instr_bytes[pi] == 0x8B || instr_bytes[pi] == 0x8A) && pi + 1 < instr_bytes.len() {
                                    let mut dest = ((instr_bytes[pi + 1] >> 3) & 7) as usize;
                                    if (io_rex & 0x04) != 0 { dest += 8; }
                                    match dest {
                                        0 => new_regs.rax = val,
                                        1 => new_regs.rcx = val,
                                        2 => new_regs.rdx = val,
                                        3 => new_regs.rbx = val,
                                        4 => new_regs.rsp = val,
                                        5 => new_regs.rbp = val,
                                        6 => new_regs.rsi = val,
                                        7 => new_regs.rdi = val,
                                        8 => new_regs.r8 = val,
                                        9 => new_regs.r9 = val,
                                        10 => new_regs.r10 = val,
                                        11 => new_regs.r11 = val,
                                        12 => new_regs.r12 = val,
                                        13 => new_regs.r13 = val,
                                        14 => new_regs.r14 = val,
                                        15 => new_regs.r15 = val,
                                        _ => new_regs.rax = val,
                                    }
                                } else {
                                    new_regs.rax = val;
                                }
                            } else {
                                new_regs.rax = val;
                            }
                        }
                        self.set_vcpu_regs(id, &new_regs)?;
                        continue; // re-enter guest
                    }

                    // Advance RIP for both reads and writes — same pattern as I/O exits.
                    // For reads, the FFI handler will additionally set the dest register.
                    let mut new_regs = regs;
                    new_regs.rip += instr_len;
                    self.set_vcpu_regs(id, &new_regs)?;

                    if is_write {
                        return Ok(VmExitReason::MmioWrite { addr: gpa, size: access_size, data: write_data });
                    } else {
                        let dest_reg = decode_mmio_dest_reg(instr_bytes);
                        return Ok(VmExitReason::MmioRead {
                            addr: gpa, size: access_size, dest_reg,
                            instr_len: 0, // RIP already advanced
                        });
                    }
                }
                WHV_EXIT_REASON_MSR => {
                    // WHV_X64_MSR_ACCESS_CONTEXT layout:
                    // [0x00..04] AccessInfo: u32 (bit 0=IsWrite)
                    // [0x04..08] MsrNumber: u32
                    // [0x08..10] Rax: u64
                    // [0x10..18] Rdx: u64
                    let data = &ctx.exit_data;
                    let access_info = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                    let msr_num = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
                    let is_write = (access_info & 1) != 0;

                    if !is_write {
                        let mut regs = self.get_vcpu_regs(id)?;
                        // Read critical MSRs from WHP virtual registers; others return fixed values
                        let whp_msr_reg = match msr_num {
                            0x1B => Some(REG_APIC_BASE),
                            0xC000_0080 => Some(REG_EFER),
                            0x174 => Some(REG_SYSENTER_CS),
                            0x175 => Some(REG_SYSENTER_ESP),
                            0x176 => Some(REG_SYSENTER_EIP),
                            0x277 => Some(REG_PAT),
                            0xC000_0081 => Some(REG_STAR),
                            0xC000_0082 => Some(REG_LSTAR),
                            0xC000_0083 => Some(REG_CSTAR),
                            0xC000_0084 => Some(REG_SFMASK),
                            0xC000_0102 => Some(REG_KERNEL_GS_BASE),
                            _ => None,
                        };
                        let val: u64 = if let Some(reg_id) = whp_msr_reg {
                            let mut v = [WHV_REGISTER_VALUE::default()];
                            if self.get_regs_raw(id, &[reg_id], &mut v).is_ok() {
                                unsafe { v[0].reg64 }
                            } else {
                                0
                            }
                        } else { match msr_num {
                            0x17 => 0, // IA32_PLATFORM_ID
                            0x1A0 => (1 << 11), // IA32_MISC_ENABLE: BTS unavailable (bit 11)
                            0xFE => 0, // IA32_MTRRCAP
                            _ => {
                                static MSR_LOG_CTR: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                                let cnt = MSR_LOG_CTR.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                                if cnt < 100 || cnt % 1000 == 0 {
                                    whp_debug(format_args!("RDMSR {:#x} = 0 (unhandled) rip={:#x}", msr_num, regs.rip));
                                }
                                0
                            }
                        }};
                        regs.rax = val & 0xFFFF_FFFF;
                        regs.rdx = val >> 32;
                        regs.rip += instr_len;
                        self.set_vcpu_regs(id, &regs)?;
                    } else {
                        let rax = u64::from_le_bytes([data[8], data[9], data[10], data[11], data[12], data[13], data[14], data[15]]);
                        let rdx = u64::from_le_bytes([data[16], data[17], data[18], data[19], data[20], data[21], data[22], data[23]]);
                        let val = (rdx << 32) | (rax & 0xFFFF_FFFF);
                        {
                            static WRMSR_LOG_CTR: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
                            let cnt = WRMSR_LOG_CTR.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                            if cnt < 100 || cnt % 1000 == 0 {
                                let cur_rip = self.get_vcpu_regs(id).map(|r| r.rip).unwrap_or(0);
                                whp_debug(format_args!("WRMSR {:#x} = {:#x} rip={:#x}", msr_num, val, cur_rip));
                            }
                        }
                        // Forward critical MSR writes to WHP virtual registers
                        let whp_reg = match msr_num {
                            0x1B => Some(REG_APIC_BASE),
                            0xC000_0080 => Some(REG_EFER),
                            0x174 => Some(REG_SYSENTER_CS),
                            0x175 => Some(REG_SYSENTER_ESP),
                            0x176 => Some(REG_SYSENTER_EIP),
                            0x277 => Some(REG_PAT),
                            0xC000_0081 => Some(REG_STAR),
                            0xC000_0082 => Some(REG_LSTAR),
                            0xC000_0083 => Some(REG_CSTAR),
                            0xC000_0084 => Some(REG_SFMASK),
                            0xC000_0102 => Some(REG_KERNEL_GS_BASE),
                            _ => None,
                        };
                        if let Some(reg_id) = whp_reg {
                            let _ = self.set_regs_raw(id, &[reg_id], &[WHV_REGISTER_VALUE::from_u64(val)]);
                        }
                        let mut regs = self.get_vcpu_regs(id)?;
                        regs.rip += instr_len;
                        self.set_vcpu_regs(id, &regs)?;
                    }
                    // Re-enter guest
                    continue;
                }
                WHV_EXIT_REASON_CPUID => {
                    // Handle CPUID internally: execute native CPUID and return results.
                    // WHV_X64_CPUID_ACCESS_CONTEXT layout:
                    // [0x00..08] Rax (=leaf): u64
                    // [0x08..10] Rcx (=subleaf): u64
                    // [0x10..18] Rdx: u64
                    // [0x18..20] Rbx: u64
                    // [0x20..28] DefaultResultRax: u64
                    // [0x28..30] DefaultResultRcx: u64
                    // [0x30..38] DefaultResultRdx: u64
                    // [0x38..40] DefaultResultRbx: u64
                    let data = &ctx.exit_data;
                    let leaf = u64::from_le_bytes([
                        data[0x00], data[0x01], data[0x02], data[0x03],
                        data[0x04], data[0x05], data[0x06], data[0x07],
                    ]) as u32;
                    let subleaf = u64::from_le_bytes([
                        data[0x08], data[0x09], data[0x0A], data[0x0B],
                        data[0x0C], data[0x0D], data[0x0E], data[0x0F],
                    ]) as u32;

                    // Use WHP's DefaultResults — these are the sanitized values WHP
                    // would have returned if CPUID interception was disabled. Using
                    // native CPUID returns host values altered by Hyper-V which may
                    // expose features WHP doesn't virtualize.
                    let mut eax = u64::from_le_bytes([
                        data[0x20], data[0x21], data[0x22], data[0x23],
                        data[0x24], data[0x25], data[0x26], data[0x27],
                    ]) as u32;
                    let mut ecx = u64::from_le_bytes([
                        data[0x28], data[0x29], data[0x2A], data[0x2B],
                        data[0x2C], data[0x2D], data[0x2E], data[0x2F],
                    ]) as u32;
                    let mut edx = u64::from_le_bytes([
                        data[0x30], data[0x31], data[0x32], data[0x33],
                        data[0x34], data[0x35], data[0x36], data[0x37],
                    ]) as u32;
                    let mut ebx = u64::from_le_bytes([
                        data[0x38], data[0x39], data[0x3A], data[0x3B],
                        data[0x3C], data[0x3D], data[0x3E], data[0x3F],
                    ]) as u32;

                    // Additional filtering on top of WHP defaults
                    if leaf == 1 {
                        ecx &= !(1 << 5);  // Remove VMX
                        ecx &= !(1 << 21); // Remove x2APIC (we emulate xAPIC via MMIO only)
                        ecx &= !(1 << 24); // Remove TSC-Deadline (we don't handle MSR 0x6E0)
                        ecx &= !(1 << 31); // Remove hypervisor present (avoid PV code paths)
                    }

                    // Hide Hyper-V signature so Linux uses standard LAPIC timer
                    if leaf >= 0x40000000 && leaf <= 0x4000FFFF {
                        eax = 0; ebx = 0; ecx = 0; edx = 0;
                    }

                    let mut regs = self.get_vcpu_regs(id)?;
                    regs.rax = eax as u64;
                    regs.rbx = ebx as u64;
                    regs.rcx = ecx as u64;
                    regs.rdx = edx as u64;
                    regs.rip += instr_len;
                    self.set_vcpu_regs(id, &regs)?;
                    // Re-enter guest
                    continue;
                }
                WHV_EXIT_REASON_INTERRUPT_WINDOW => {
                    // Guest is now interruptible (IF=1). Disable the notification
                    // and deliver any re-queued IOAPIC interrupts before returning
                    // to vm.rs for PIC interrupt injection.
                    let _ = self.request_interrupt_window(id, false);
                    self.deliver_ioapic_pending();
                    return Ok(VmExitReason::InterruptWindow);
                }
                WHV_EXIT_REASON_UNSUPPORTED_FEATURE => {
                    let regs = self.get_vcpu_regs(id)?;
                    whp_debug(format_args!("UNSUPPORTED_FEATURE exit RIP={:#x}", regs.rip));
                    return Ok(VmExitReason::Error);
                }
                WHV_EXIT_REASON_EXCEPTION => {
                    // Exception exit: fired by ExceptionExitBitmap (#UD=6, #DF=8).
                    // WHV_VP_EXCEPTION_CONTEXT layout:
                    // [0x00] InstructionByteCount(u8), [0x01..04] Rsvd, [0x04..14] InstructionBytes
                    // [0x14] ExceptionInfo(u32), [0x18] ExceptionType(u8), [0x1C] ErrorCode(u32)
                    // [0x20] ExceptionParameter(u64)
                    let exc_type = ctx.exit_data[0x18];
                    let err_code = u32::from_le_bytes([
                        ctx.exit_data[0x1C], ctx.exit_data[0x1D],
                        ctx.exit_data[0x1E], ctx.exit_data[0x1F],
                    ]);
                    let regs = self.get_vcpu_regs(id)?;
                    let sregs = self.get_vcpu_sregs(id)?;
                    whp_debug(format_args!(
                        "EXCEPTION: vec=#{} err={:#x} RIP={:#x} RSP={:#x} RFLAGS={:#x} CR0={:#x} CR3={:#x} CS={:#x}",
                        exc_type, err_code, regs.rip, regs.rsp, regs.rflags,
                        sregs.cr0, sregs.cr3, sregs.cs.selector
                    ));
                    // #DF (8) is fatal — return Shutdown so the VM stops
                    if exc_type == 8 {
                        return Ok(VmExitReason::Shutdown);
                    }
                    // #UD (6) — log and return error
                    return Ok(VmExitReason::Error);
                }
                WHV_EXIT_REASON_APIC_EOI => {
                    // XApic mode: guest EOI'd. Extract vector from exit data
                    // and clear Remote IRR in IOAPIC for level-triggered entries.
                    let eoi_vector = u32::from_le_bytes([
                        ctx.exit_data[0], ctx.exit_data[1],
                        ctx.exit_data[2], ctx.exit_data[3],
                    ]) as u8;
                    self.ioapic.eoi_vector(eoi_vector);
                    // Deliver any re-triggered IOAPIC interrupts
                    self.deliver_ioapic_pending();
                    continue;
                }
                WHV_EXIT_REASON_UNRECOVERABLE_EXCEPTION | WHV_EXIT_REASON_INVALID_VP_STATE => {
                    let regs = self.get_vcpu_regs(id).unwrap_or_default();
                    let sregs = self.get_vcpu_sregs(id).unwrap_or_default();
                    // WHV_VP_EXCEPTION_CONTEXT layout:
                    // [0x00] InstructionByteCount(u8), [0x14] ExceptionInfo(u32),
                    // [0x18] ExceptionType(u8), [0x1C] ErrorCode(u32), [0x20] ExceptionParameter(u64)
                    let exc_type = ctx.exit_data[0x18];
                    let err_code = u32::from_le_bytes([
                        ctx.exit_data[0x1C], ctx.exit_data[0x1D],
                        ctx.exit_data[0x1E], ctx.exit_data[0x1F],
                    ]);
                    let exc_param = u64::from_le_bytes([
                        ctx.exit_data[0x20], ctx.exit_data[0x21], ctx.exit_data[0x22], ctx.exit_data[0x23],
                        ctx.exit_data[0x24], ctx.exit_data[0x25], ctx.exit_data[0x26], ctx.exit_data[0x27],
                    ]);
                    whp_debug(format_args!(
                        "EXCEPTION EXIT: exit={:#x} vec=#{} err_code={:#x} param={:#x} RIP={:#x} RSP={:#x} RFLAGS={:#x} CR0={:#x} CR3={:#x} CR4={:#x} CS={:#x} SS={:#x} EFER={:#x}",
                        ctx.exit_reason, exc_type, err_code, exc_param,
                        regs.rip, regs.rsp, regs.rflags,
                        sregs.cr0, sregs.cr3, sregs.cr4,
                        sregs.cs.selector, sregs.ss.selector, sregs.efer
                    ));
                    return Ok(VmExitReason::Shutdown);
                }
                WHV_EXIT_REASON_CANCELED => return Ok(VmExitReason::InterruptWindow),
                WHV_EXIT_REASON_NONE => {
                    whp_debug(format_args!("EXIT_REASON_NONE"));
                    return Ok(VmExitReason::Error);
                }
                unknown => {
                    let regs = self.get_vcpu_regs(id).unwrap_or_default();
                    whp_debug(format_args!("UNKNOWN exit reason={:#x} RIP={:#x} RSP={:#x} RFLAGS={:#x} CR0={:#x}",
                        unknown, regs.rip, regs.rsp, regs.rflags, 0));
                    return Ok(VmExitReason::Error);
                }
            }
        }
    }

    fn get_vcpu_regs(&self, id: u32) -> Result<VcpuRegs, VmError> {
        let mut vals = [WHV_REGISTER_VALUE::default(); 18];
        self.get_regs_raw(id, &GP_REG_NAMES, &mut vals)?;

        Ok(VcpuRegs {
            rax: unsafe { vals[0].reg64 },
            rbx: unsafe { vals[1].reg64 },
            rcx: unsafe { vals[2].reg64 },
            rdx: unsafe { vals[3].reg64 },
            rsi: unsafe { vals[4].reg64 },
            rdi: unsafe { vals[5].reg64 },
            rbp: unsafe { vals[6].reg64 },
            rsp: unsafe { vals[7].reg64 },
            r8:  unsafe { vals[8].reg64 },
            r9:  unsafe { vals[9].reg64 },
            r10: unsafe { vals[10].reg64 },
            r11: unsafe { vals[11].reg64 },
            r12: unsafe { vals[12].reg64 },
            r13: unsafe { vals[13].reg64 },
            r14: unsafe { vals[14].reg64 },
            r15: unsafe { vals[15].reg64 },
            rip: unsafe { vals[16].reg64 },
            rflags: unsafe { vals[17].reg64 },
        })
    }

    fn set_vcpu_regs(&mut self, id: u32, regs: &VcpuRegs) -> Result<(), VmError> {
        let vals = [
            WHV_REGISTER_VALUE::from_u64(regs.rax),
            WHV_REGISTER_VALUE::from_u64(regs.rbx),
            WHV_REGISTER_VALUE::from_u64(regs.rcx),
            WHV_REGISTER_VALUE::from_u64(regs.rdx),
            WHV_REGISTER_VALUE::from_u64(regs.rsi),
            WHV_REGISTER_VALUE::from_u64(regs.rdi),
            WHV_REGISTER_VALUE::from_u64(regs.rbp),
            WHV_REGISTER_VALUE::from_u64(regs.rsp),
            WHV_REGISTER_VALUE::from_u64(regs.r8),
            WHV_REGISTER_VALUE::from_u64(regs.r9),
            WHV_REGISTER_VALUE::from_u64(regs.r10),
            WHV_REGISTER_VALUE::from_u64(regs.r11),
            WHV_REGISTER_VALUE::from_u64(regs.r12),
            WHV_REGISTER_VALUE::from_u64(regs.r13),
            WHV_REGISTER_VALUE::from_u64(regs.r14),
            WHV_REGISTER_VALUE::from_u64(regs.r15),
            WHV_REGISTER_VALUE::from_u64(regs.rip),
            WHV_REGISTER_VALUE::from_u64(regs.rflags),
        ];
        self.set_regs_raw(id, &GP_REG_NAMES, &vals)
    }

    fn get_vcpu_sregs(&self, id: u32) -> Result<VcpuSregs, VmError> {
        let mut vals = [WHV_REGISTER_VALUE::default(); 13];
        self.get_regs_raw(id, &SREG_NAMES, &mut vals)?;

        let mut ext_vals = [WHV_REGISTER_VALUE::default(); 2];
        self.get_regs_raw(id, &SREG_NAMES_EXT, &mut ext_vals)?;

        Ok(VcpuSregs {
            cs:  whv_to_seg(unsafe { &vals[0].segment }),
            ds:  whv_to_seg(unsafe { &vals[1].segment }),
            es:  whv_to_seg(unsafe { &vals[2].segment }),
            fs:  whv_to_seg(unsafe { &vals[3].segment }),
            gs:  whv_to_seg(unsafe { &vals[4].segment }),
            ss:  whv_to_seg(unsafe { &vals[5].segment }),
            tr:  whv_to_seg(unsafe { &vals[6].segment }),
            ldt: whv_to_seg(unsafe { &vals[7].segment }),
            gdt: DescriptorTable {
                base: unsafe { vals[8].table.base },
                limit: unsafe { vals[8].table.limit },
            },
            idt: DescriptorTable {
                base: unsafe { vals[9].table.base },
                limit: unsafe { vals[9].table.limit },
            },
            cr0: unsafe { vals[10].reg64 },
            cr2: unsafe { vals[11].reg64 },
            cr3: unsafe { vals[12].reg64 },
            cr4: unsafe { ext_vals[0].reg64 },
            efer: unsafe { ext_vals[1].reg64 },
        })
    }

    fn set_vcpu_sregs(&mut self, id: u32, sregs: &VcpuSregs) -> Result<(), VmError> {
        let vals: [WHV_REGISTER_VALUE; 13] = [
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.cs)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.ds)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.es)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.fs)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.gs)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.ss)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.tr)),
            WHV_REGISTER_VALUE::from_seg(seg_to_whv(&sregs.ldt)),
            WHV_REGISTER_VALUE::from_table(WhvTable { _pad: [0; 3], limit: sregs.gdt.limit, base: sregs.gdt.base }),
            WHV_REGISTER_VALUE::from_table(WhvTable { _pad: [0; 3], limit: sregs.idt.limit, base: sregs.idt.base }),
            WHV_REGISTER_VALUE::from_u64(sregs.cr0),
            WHV_REGISTER_VALUE::from_u64(sregs.cr2),
            WHV_REGISTER_VALUE::from_u64(sregs.cr3),
        ];
        self.set_regs_raw(id, &SREG_NAMES, &vals)?;

        let ext_vals = [
            WHV_REGISTER_VALUE::from_u64(sregs.cr4),
            WHV_REGISTER_VALUE::from_u64(sregs.efer),
        ];
        self.set_regs_raw(id, &SREG_NAMES_EXT, &ext_vals)
    }

    fn inject_interrupt(&mut self, id: u32, vector: u8) -> Result<(), VmError> {
        // XApic mode: use WHvRegisterPendingEvent (0x80000002) with
        // WHV_X64_PENDING_EXT_INT_EVENT (EventType=5).
        //
        // WHV_X64_PENDING_EXT_INT_EVENT layout (128-bit register):
        //   bit 0:     EventPending = 1
        //   bits 1-3:  EventType = 5 (WHvX64PendingEventExtInt)
        //   bits 4-7:  Reserved = 0
        //   bits 8-15: Vector
        //   bits 16-63: Reserved = 0
        //   bits 64-127: Reserved = 0
        //
        // Clear HaltSuspend in InternalActivityState — if the vCPU is in HLT,
        // writing PendingEvent alone won't wake it.
        let mut activity = [WHV_REGISTER_VALUE::default()];
        if self.get_regs_raw(id, &[REG_INTERNAL_ACTIVITY_STATE], &mut activity).is_ok() {
            let state = unsafe { activity[0].reg64 };
            if state & 2 != 0 {
                let cleared = WHV_REGISTER_VALUE::from_u64(state & !2);
                let _ = self.set_regs_raw(id, &[REG_INTERNAL_ACTIVITY_STATE], &[cleared]);
            }
        }

        let lo: u64 = 1 | (5u64 << 1) | ((vector as u64) << 8);
        let val = WHV_REGISTER_VALUE { reg128: [lo, 0] };
        let result = self.set_regs_raw(id, &[REG_PENDING_EVENT], &[val]);
        if result.is_err() {
            static INJECT_FAIL_COUNT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
            let cnt = INJECT_FAIL_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if cnt < 10 || cnt % 1000 == 0 {
                whp_debug(format_args!("inject_ext_int failed #{} vec={} err={:?}", cnt, vector, result));
            }
        }
        result
    }

    fn inject_exception(&mut self, id: u32, vector: u8, error_code: Option<u32>) -> Result<(), VmError> {
        // InterruptionType 3 = Hardware exception
        let mut val: u64 = 1u64 | (3u64 << 1) | ((vector as u64) << 16);
        if let Some(ec) = error_code {
            val |= 1u64 << 4; // DeliverErrorCode
            val |= (ec as u64) << 32;
        }
        self.set_regs_raw(id, &[REG_PENDING_INTERRUPTION], &[WHV_REGISTER_VALUE::from_u64(val)])
    }

    fn inject_nmi(&mut self, id: u32) -> Result<(), VmError> {
        // InterruptionType 2 = NMI
        let val = 1u64 | (2u64 << 1);
        self.set_regs_raw(id, &[REG_PENDING_INTERRUPTION], &[WHV_REGISTER_VALUE::from_u64(val)])
    }

    fn request_interrupt_window(&mut self, id: u32, enable: bool) -> Result<(), VmError> {
        let val: u64 = if enable { 1 } else { 0 };
        self.set_regs_raw(id, &[REG_DELIVERABILITY_NOTIFICATIONS], &[WHV_REGISTER_VALUE::from_u64(val)])
    }

    fn set_cpuid(&mut self, _entries: &[CpuidEntry]) -> Result<(), VmError> {
        // WHP does not support custom CPUID configuration directly.
        // CPUID exits must be handled via WHvRunVpExitReasonX64Cpuid exit reason
        // after enabling CPUID exits in extended VM exits.
        Ok(())
    }
}

impl Drop for WhpBackend {
    fn drop(&mut self) {
        if !self.partition.is_null() {
            self.destroy();
        }
    }
}
