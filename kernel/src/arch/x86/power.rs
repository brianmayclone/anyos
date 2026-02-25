//! CPU power management: P-states, C-states, and frequency monitoring.
//!
//! Detects and initializes CPU power features:
//! - **Intel HWP** (Hardware P-States): automatic frequency scaling
//! - **Legacy P-States**: MSR-based frequency control
//! - **AMD P-States**: AMD-specific frequency registers
//! - **APERF/MPERF**: actual frequency measurement
//! - **C-States**: MWAIT idle power states

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

// ── MSR Constants ───────────────────────────────────────────────────────────

// Intel P-state MSRs
const MSR_PLATFORM_INFO: u32     = 0xCE;
const MSR_PERF_STATUS: u32       = 0x198;
const MSR_PERF_CTL: u32          = 0x199;
const MSR_MPERF: u32             = 0xE7;
const MSR_APERF: u32             = 0xE8;

// Intel HWP MSRs
const MSR_PM_ENABLE: u32         = 0x770;
const MSR_HWP_CAPABILITIES: u32  = 0x771;
const MSR_HWP_REQUEST: u32       = 0x774;

// AMD P-state MSRs
const MSR_AMD_PSTATE_STATUS: u32 = 0xC001_0063;
const MSR_AMD_PSTATE_DEF_BASE: u32 = 0xC001_0064;

// ── Public MSR Helpers ──────────────────────────────────────────────────────

/// Read a Model-Specific Register.
#[inline(always)]
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let lo: u32;
    let hi: u32;
    core::arch::asm!(
        "rdmsr",
        in("ecx") msr,
        out("eax") lo,
        out("edx") hi,
        options(nostack, preserves_flags),
    );
    ((hi as u64) << 32) | (lo as u64)
}

/// Write a Model-Specific Register.
#[inline(always)]
pub unsafe fn wrmsr(msr: u32, value: u64) {
    let lo = value as u32;
    let hi = (value >> 32) as u32;
    core::arch::asm!(
        "wrmsr",
        in("ecx") msr,
        in("eax") lo,
        in("edx") hi,
        options(nostack, preserves_flags),
    );
}

// ── Power Feature Flags ─────────────────────────────────────────────────────

static HAS_HWP: AtomicBool = AtomicBool::new(false);
static HAS_TURBO: AtomicBool = AtomicBool::new(false);
static HAS_APERF_MPERF: AtomicBool = AtomicBool::new(false);
static IS_AMD: AtomicBool = AtomicBool::new(false);
static MAX_FREQ_MHZ: AtomicU32 = AtomicU32::new(0);
static BASE_FREQ_MHZ: AtomicU32 = AtomicU32::new(0);
static MAX_CSTATE: AtomicU32 = AtomicU32::new(1); // default C1

// APERF/MPERF previous values for delta calculation
static PREV_APERF: AtomicU64 = AtomicU64::new(0);
static PREV_MPERF: AtomicU64 = AtomicU64::new(0);

use core::sync::atomic::AtomicU64;

// ── Public Query API ────────────────────────────────────────────────────────

pub fn has_hwp() -> bool { HAS_HWP.load(Ordering::Relaxed) }
pub fn has_turbo() -> bool { HAS_TURBO.load(Ordering::Relaxed) }
pub fn has_aperf() -> bool { HAS_APERF_MPERF.load(Ordering::Relaxed) }
pub fn max_frequency_mhz() -> u32 { MAX_FREQ_MHZ.load(Ordering::Relaxed) }
pub fn max_cstate() -> u32 { MAX_CSTATE.load(Ordering::Relaxed) }

/// Read current CPU frequency in MHz.
/// Uses APERF/MPERF if available, otherwise reads P-state ratio.
pub fn current_frequency_mhz() -> u32 {
    if HAS_APERF_MPERF.load(Ordering::Relaxed) {
        return aperf_mperf_frequency();
    }

    if IS_AMD.load(Ordering::Relaxed) {
        return amd_current_frequency();
    }

    // Intel legacy: read current ratio from PERF_STATUS
    let status = unsafe { rdmsr(MSR_PERF_STATUS) };
    let ratio = ((status >> 8) & 0xFF) as u32;
    if ratio > 0 {
        ratio * 100 // assume 100 MHz bus clock
    } else {
        BASE_FREQ_MHZ.load(Ordering::Relaxed)
    }
}

/// Power features as a bitfield for sysinfo.
/// Bit 0 = HWP, bit 1 = Turbo, bit 2 = APERF/MPERF.
pub fn features_bitfield() -> u32 {
    let mut bits = 0u32;
    if has_hwp() { bits |= 1; }
    if has_turbo() { bits |= 2; }
    if has_aperf() { bits |= 4; }
    bits
}

// ── Initialization (BSP) ───────────────────────────────────────────────────

/// Detect power features and initialize P-states on the bootstrap processor.
/// Must be called after `cpuid::detect()`.
pub fn init() {
    let vendor = crate::arch::x86::cpuid::vendor();
    let is_amd = &vendor[0..12] == b"AuthenticAMD";
    IS_AMD.store(is_amd, Ordering::Relaxed);

    // ── CPUID leaf 6: Thermal & Power Management ──
    let max_leaf = crate::arch::x86::cpuid::cpuid(0, 0).0;
    if max_leaf >= 6 {
        let (eax6, _, ecx6, _) = crate::arch::x86::cpuid::cpuid(6, 0);
        let turbo = eax6 & (1 << 1) != 0;
        let hwp = eax6 & (1 << 7) != 0;
        let aperf = ecx6 & (1 << 0) != 0;

        HAS_TURBO.store(turbo, Ordering::Relaxed);
        HAS_HWP.store(hwp, Ordering::Relaxed);
        HAS_APERF_MPERF.store(aperf, Ordering::Relaxed);
    }

    // ── CPUID leaf 5: MONITOR/MWAIT C-state support ──
    if max_leaf >= 5 {
        let (_, _, _, edx5) = crate::arch::x86::cpuid::cpuid(5, 0);
        // EDX bits [3:0] = C0 sub-states, [7:4] = C1 sub-states, etc.
        // Count highest supported C-state
        let mut max_cs = 0u32;
        for cs in 0..8 {
            let sub_states = (edx5 >> (cs * 4)) & 0xF;
            if sub_states > 0 { max_cs = cs; }
        }
        MAX_CSTATE.store(max_cs, Ordering::Relaxed);
    }

    // ── P-State initialization ──
    if is_amd {
        init_amd_pstates();
    } else if HAS_HWP.load(Ordering::Relaxed) {
        init_intel_hwp();
    } else {
        init_intel_legacy_pstate();
    }

    // ── Initialize APERF/MPERF baseline ──
    if HAS_APERF_MPERF.load(Ordering::Relaxed) {
        unsafe {
            let aperf = rdmsr(MSR_APERF);
            let mperf = rdmsr(MSR_MPERF);
            PREV_APERF.store(aperf, Ordering::Relaxed);
            PREV_MPERF.store(mperf, Ordering::Relaxed);
        }
    }

    // Log results
    let max_mhz = MAX_FREQ_MHZ.load(Ordering::Relaxed);
    let base_mhz = BASE_FREQ_MHZ.load(Ordering::Relaxed);
    crate::serial_println!(
        "[OK] CPU Power: HWP={} Turbo={} APERF={} AMD={} max={}MHz base={}MHz C-states=C0..C{}",
        has_hwp(), has_turbo(), has_aperf(), is_amd, max_mhz, base_mhz,
        MAX_CSTATE.load(Ordering::Relaxed)
    );
}

/// Per-AP power initialization. Enables HWP and sets P-state on each AP.
pub fn init_ap() {
    if IS_AMD.load(Ordering::Relaxed) {
        // AMD: P-states are per-core, nothing extra needed
        return;
    }

    if HAS_HWP.load(Ordering::Relaxed) {
        unsafe {
            // Enable HWP on this core
            wrmsr(MSR_PM_ENABLE, 1);
            // Request max performance
            let caps = rdmsr(MSR_HWP_CAPABILITIES);
            let highest = caps & 0xFF;
            let request = highest | (highest << 8); // min=max=highest
            wrmsr(MSR_HWP_REQUEST, request);
        }
    } else {
        // Legacy: set max P-state ratio
        let max_ratio = MAX_FREQ_MHZ.load(Ordering::Relaxed) as u64 / 100;
        if max_ratio > 0 {
            unsafe { wrmsr(MSR_PERF_CTL, max_ratio << 8); }
        }
    }
}

// ── Intel HWP Initialization ────────────────────────────────────────────────

fn init_intel_hwp() {
    unsafe {
        // Enable HWP
        wrmsr(MSR_PM_ENABLE, 1);

        // Read capabilities
        let caps = rdmsr(MSR_HWP_CAPABILITIES);
        let highest = (caps & 0xFF) as u32;           // bits 7:0
        let _lowest = ((caps >> 8) & 0xFF) as u32;    // bits 15:8
        let efficient = ((caps >> 16) & 0xFF) as u32;  // bits 23:16

        // Set request: min=efficient, max=highest (let HW decide)
        let request = (efficient as u64) | ((highest as u64) << 8);
        wrmsr(MSR_HWP_REQUEST, request);

        let max_mhz = highest * 100;
        MAX_FREQ_MHZ.store(max_mhz, Ordering::Relaxed);
        BASE_FREQ_MHZ.store(efficient * 100, Ordering::Relaxed);

        crate::serial_println!(
            "  HWP: highest={} efficient={} → max={}MHz",
            highest, efficient, max_mhz
        );
    }
}

// ── Intel Legacy P-State Initialization ─────────────────────────────────────

fn init_intel_legacy_pstate() {
    unsafe {
        // Read PLATFORM_INFO for max non-turbo ratio
        let platform_info = rdmsr(MSR_PLATFORM_INFO);
        let max_ratio = ((platform_info >> 8) & 0xFF) as u32;

        if max_ratio > 0 {
            let max_mhz = max_ratio * 100;
            MAX_FREQ_MHZ.store(max_mhz, Ordering::Relaxed);
            BASE_FREQ_MHZ.store(max_mhz, Ordering::Relaxed);

            // Request max performance
            wrmsr(MSR_PERF_CTL, (max_ratio as u64) << 8);

            crate::serial_println!("  Legacy P-state: ratio={} → max={}MHz", max_ratio, max_mhz);
        } else {
            // Fallback: use TSC frequency
            let tsc_mhz = (crate::arch::x86::pit::tsc_hz() / 1_000_000) as u32;
            MAX_FREQ_MHZ.store(tsc_mhz, Ordering::Relaxed);
            BASE_FREQ_MHZ.store(tsc_mhz, Ordering::Relaxed);
            crate::serial_println!("  Legacy P-state: using TSC={}MHz (no PLATFORM_INFO)", tsc_mhz);
        }
    }
}

// ── AMD P-State Initialization ──────────────────────────────────────────────

fn init_amd_pstates() {
    unsafe {
        // Read P-state 0 definition (highest performance)
        let pstate0 = rdmsr(MSR_AMD_PSTATE_DEF_BASE);
        if pstate0 & (1 << 63) != 0 {
            // P-state is valid (bit 63 = PstateEn)
            let freq = amd_pstate_frequency(pstate0);
            MAX_FREQ_MHZ.store(freq, Ordering::Relaxed);
            BASE_FREQ_MHZ.store(freq, Ordering::Relaxed);
            crate::serial_println!("  AMD P-state 0: {}MHz", freq);
        } else {
            // Fallback to TSC
            let tsc_mhz = (crate::arch::x86::pit::tsc_hz() / 1_000_000) as u32;
            MAX_FREQ_MHZ.store(tsc_mhz, Ordering::Relaxed);
            BASE_FREQ_MHZ.store(tsc_mhz, Ordering::Relaxed);
            crate::serial_println!("  AMD: using TSC={}MHz (no P-state info)", tsc_mhz);
        }
    }
}

/// Decode AMD P-state register to frequency in MHz.
/// Family 17h+: freq = 200 * CpuFid / CpuDid
fn amd_pstate_frequency(pstate: u64) -> u32 {
    let fid = (pstate & 0xFF) as u32;           // bits 7:0
    let did = ((pstate >> 8) & 0x3F) as u32;    // bits 13:8
    if did == 0 { return 0; }
    (200 * fid) / did
}

fn amd_current_frequency() -> u32 {
    unsafe {
        let status = rdmsr(MSR_AMD_PSTATE_STATUS);
        let cur_pstate = (status & 0x7) as u32;
        let pstate_def = rdmsr(MSR_AMD_PSTATE_DEF_BASE + cur_pstate);
        if pstate_def & (1 << 63) != 0 {
            amd_pstate_frequency(pstate_def)
        } else {
            BASE_FREQ_MHZ.load(Ordering::Relaxed)
        }
    }
}

// ── APERF/MPERF Frequency Calculation ───────────────────────────────────────

fn aperf_mperf_frequency() -> u32 {
    let base = BASE_FREQ_MHZ.load(Ordering::Relaxed) as u64;
    if base == 0 {
        return MAX_FREQ_MHZ.load(Ordering::Relaxed);
    }

    unsafe {
        let aperf = rdmsr(MSR_APERF);
        let mperf = rdmsr(MSR_MPERF);

        let prev_a = PREV_APERF.swap(aperf, Ordering::Relaxed);
        let prev_m = PREV_MPERF.swap(mperf, Ordering::Relaxed);

        let da = aperf.wrapping_sub(prev_a);
        let dm = mperf.wrapping_sub(prev_m);

        if dm == 0 {
            return base as u32;
        }

        // actual_freq = base_freq * (aperf_delta / mperf_delta)
        let freq = base * da / dm;
        freq as u32
    }
}
