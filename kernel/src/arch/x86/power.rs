//! CPU power management: P-states, C-states, and frequency monitoring.
//!
//! Detects and initializes CPU power features:
//! - **Intel HWP** (Hardware P-States): automatic frequency scaling
//! - **Legacy P-States**: MSR-based frequency control
//! - **AMD P-States**: AMD-specific frequency registers
//! - **APERF/MPERF**: actual frequency measurement
//! - **C-States**: MWAIT idle power states

use crate::arch::x86::smp::MAX_CPUS;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

mod amd;
mod intel;
mod kvm;

// ── MSR Constants ───────────────────────────────────────────────────────────

const MSR_MPERF: u32 = 0xE7;
const MSR_APERF: u32 = 0xE8;

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
static IS_HYPERVISOR: AtomicBool = AtomicBool::new(false);
static ACTIVE_CONTROL_OK: AtomicBool = AtomicBool::new(false);
static MAX_FREQ_MHZ: AtomicU32 = AtomicU32::new(0);
static BASE_FREQ_MHZ: AtomicU32 = AtomicU32::new(0);
static MAX_CSTATE: AtomicU32 = AtomicU32::new(1); // default C1
static ACTIVE_PROFILE: AtomicU32 = AtomicU32::new(PowerProfile::Balanced as u32);
static DRIVER_KIND: AtomicU32 = AtomicU32::new(0);
static PROFILE_GENERATION: AtomicU32 = AtomicU32::new(1);
static PER_CPU_PROFILE_GENERATION: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};
static PER_CPU_FREQ_MHZ: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};
static PER_CPU_PREV_APERF: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_CPUS]
};
static PER_CPU_PREV_MPERF: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_CPUS]
};
static PER_CPU_LAST_FREQ_SAMPLE_TICK: [AtomicU32; MAX_CPUS] = {
    const INIT: AtomicU32 = AtomicU32::new(0);
    [INIT; MAX_CPUS]
};

use core::sync::atomic::AtomicU64;

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerProfile {
    PowerSaver = 0,
    Balanced = 1,
    Performance = 2,
}

impl PowerProfile {
    fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::PowerSaver),
            1 => Some(Self::Balanced),
            2 => Some(Self::Performance),
            _ => None,
        }
    }
}

// ── Public Query API ────────────────────────────────────────────────────────

pub fn has_hwp() -> bool {
    HAS_HWP.load(Ordering::Relaxed)
}
pub fn has_turbo() -> bool {
    HAS_TURBO.load(Ordering::Relaxed)
}
pub fn has_aperf() -> bool {
    HAS_APERF_MPERF.load(Ordering::Relaxed)
}
pub fn has_hypervisor() -> bool {
    IS_HYPERVISOR.load(Ordering::Relaxed)
}
pub fn has_active_control() -> bool {
    ACTIVE_CONTROL_OK.load(Ordering::Relaxed)
}
pub fn max_frequency_mhz() -> u32 {
    MAX_FREQ_MHZ.load(Ordering::Relaxed)
}
pub fn max_cstate() -> u32 {
    MAX_CSTATE.load(Ordering::Relaxed)
}
pub fn active_profile() -> u32 {
    ACTIVE_PROFILE.load(Ordering::Relaxed)
}
pub fn driver_kind() -> u32 {
    DRIVER_KIND.load(Ordering::Relaxed)
}

pub(super) fn set_frequency_limits(max_mhz: u32, base_mhz: u32) {
    MAX_FREQ_MHZ.store(max_mhz, Ordering::Relaxed);
    BASE_FREQ_MHZ.store(base_mhz, Ordering::Relaxed);
}

pub(super) fn set_active_control(enabled: bool) {
    ACTIVE_CONTROL_OK.store(enabled, Ordering::Relaxed);
}

pub(super) fn set_driver_kind(kind: u32) {
    DRIVER_KIND.store(kind, Ordering::Relaxed);
}

/// Read current CPU frequency in MHz.
/// Uses APERF/MPERF if available, otherwise reads P-state ratio.
pub fn current_frequency_mhz() -> u32 {
    sample_current_cpu_frequency_mhz()
}

/// Return the last sampled frequency for a CPU.
pub fn per_cpu_frequency_mhz(cpu: usize) -> u32 {
    if cpu >= MAX_CPUS {
        return 0;
    }
    PER_CPU_FREQ_MHZ[cpu].load(Ordering::Relaxed)
}

/// Average current frequency across sampled CPUs.
pub fn average_frequency_mhz() -> u32 {
    let ncpu = (crate::arch::x86::smp::cpu_count() as usize)
        .max(1)
        .min(MAX_CPUS);
    let mut total = 0u64;
    let mut count = 0u64;
    for cpu in 0..ncpu {
        let freq = per_cpu_frequency_mhz(cpu);
        if freq > 0 {
            total += freq as u64;
            count += 1;
        }
    }
    if count > 0 {
        (total / count) as u32
    } else {
        BASE_FREQ_MHZ.load(Ordering::Relaxed)
    }
}

/// Sum of current frequencies across sampled CPUs.
pub fn total_frequency_mhz() -> u32 {
    let ncpu = (crate::arch::x86::smp::cpu_count() as usize)
        .max(1)
        .min(MAX_CPUS);
    let mut total = 0u64;
    let mut count = 0u64;
    for cpu in 0..ncpu {
        let freq = per_cpu_frequency_mhz(cpu);
        if freq > 0 {
            total += freq as u64;
            count += 1;
        }
    }
    if count > 0 {
        total.min(u32::MAX as u64) as u32
    } else {
        BASE_FREQ_MHZ
            .load(Ordering::Relaxed)
            .saturating_mul(ncpu as u32)
    }
}

/// Sample the executing CPU and publish its current MHz for sysinfo consumers.
pub fn sample_current_cpu_frequency_mhz() -> u32 {
    let cpu = crate::arch::x86::smp::current_cpu_id() as usize;
    let freq = measure_current_cpu_frequency_mhz(cpu);
    if cpu < MAX_CPUS && freq > 0 {
        PER_CPU_FREQ_MHZ[cpu].store(freq, Ordering::Relaxed);
    }
    freq
}

/// Sample the executing CPU at most roughly ten times per second.
pub fn sample_current_cpu_frequency_mhz_if_due() {
    let cpu = crate::arch::x86::smp::current_cpu_id() as usize;
    if cpu >= MAX_CPUS {
        return;
    }
    let now = crate::arch::hal::timer_current_ticks();
    let hz = crate::arch::hal::timer_frequency_hz().max(1) as u32;
    let interval = (hz / 10).max(1);
    let last = PER_CPU_LAST_FREQ_SAMPLE_TICK[cpu].load(Ordering::Relaxed);
    if now.wrapping_sub(last) < interval {
        return;
    }
    PER_CPU_LAST_FREQ_SAMPLE_TICK[cpu].store(now, Ordering::Relaxed);
    let _ = sample_current_cpu_frequency_mhz();
}

fn measure_current_cpu_frequency_mhz(cpu: usize) -> u32 {
    if HAS_APERF_MPERF.load(Ordering::Relaxed) {
        return aperf_mperf_frequency(cpu);
    }

    if IS_AMD.load(Ordering::Relaxed) {
        return amd::current_frequency_mhz(
            BASE_FREQ_MHZ.load(Ordering::Relaxed),
            IS_HYPERVISOR.load(Ordering::Relaxed),
        );
    }

    // Intel legacy: read current ratio from PERF_STATUS (only if MSR is available)
    if intel::legacy_available() {
        return intel::current_legacy_frequency_mhz(BASE_FREQ_MHZ.load(Ordering::Relaxed));
    }

    BASE_FREQ_MHZ.load(Ordering::Relaxed)
}

/// Power features as a bitfield for sysinfo.
/// Bit 0 = HWP, bit 1 = Turbo, bit 2 = APERF/MPERF,
/// bit 3 = hypervisor, bit 4 = active frequency control.
pub fn features_bitfield() -> u32 {
    let mut bits = 0u32;
    if has_hwp() {
        bits |= 1;
    }
    if has_turbo() {
        bits |= 2;
    }
    if has_aperf() {
        bits |= 4;
    }
    if has_hypervisor() {
        bits |= 8;
    }
    if has_active_control() {
        bits |= 16;
    }
    bits
}

pub fn set_profile_id(profile_id: u32) -> bool {
    let Some(profile) = PowerProfile::from_u32(profile_id) else {
        return false;
    };
    ACTIVE_PROFILE.store(profile as u32, Ordering::Relaxed);
    PROFILE_GENERATION.fetch_add(1, Ordering::Release);
    apply_profile_on_current_cpu()
}

pub fn apply_profile_on_current_cpu() -> bool {
    let profile = PowerProfile::from_u32(ACTIVE_PROFILE.load(Ordering::Relaxed))
        .unwrap_or(PowerProfile::Balanced);

    if HAS_HWP.load(Ordering::Relaxed) && ACTIVE_CONTROL_OK.load(Ordering::Relaxed) {
        intel::apply_hwp_profile(profile);
        mark_current_cpu_profile_synced();
        return true;
    }
    if IS_AMD.load(Ordering::Relaxed) && !IS_HYPERVISOR.load(Ordering::Relaxed) {
        let applied = amd::apply_profile(
            profile,
            IS_HYPERVISOR.load(Ordering::Relaxed),
            ACTIVE_CONTROL_OK.load(Ordering::Relaxed),
        );
        if applied {
            mark_current_cpu_profile_synced();
        }
        return applied;
    }
    if intel::legacy_available() {
        intel::apply_legacy_profile(profile, MAX_FREQ_MHZ.load(Ordering::Relaxed));
        mark_current_cpu_profile_synced();
        return true;
    }
    false
}

pub fn sync_profile_on_current_cpu() {
    let cpu = crate::arch::x86::smp::current_cpu_id() as usize;
    if cpu >= MAX_CPUS {
        return;
    }
    let generation = PROFILE_GENERATION.load(Ordering::Acquire);
    if PER_CPU_PROFILE_GENERATION[cpu].load(Ordering::Relaxed) != generation {
        let _ = apply_profile_on_current_cpu();
    }
}

fn mark_current_cpu_profile_synced() {
    let cpu = crate::arch::x86::smp::current_cpu_id() as usize;
    if cpu < MAX_CPUS {
        let generation = PROFILE_GENERATION.load(Ordering::Acquire);
        PER_CPU_PROFILE_GENERATION[cpu].store(generation, Ordering::Release);
    }
}

// ── Initialization (BSP) ───────────────────────────────────────────────────

/// Detect power features and initialize P-states on the bootstrap processor.
/// Must be called after `cpuid::detect()`.
pub fn init() {
    let vendor = crate::arch::x86::cpuid::vendor();
    let is_amd = &vendor[0..12] == b"AuthenticAMD";
    IS_AMD.store(is_amd, Ordering::Relaxed);
    let (_, _, ecx1, _) = crate::arch::x86::cpuid::cpuid(1, 0);
    let hypervisor = ecx1 & (1 << 31) != 0;
    IS_HYPERVISOR.store(hypervisor, Ordering::Relaxed);
    if hypervisor {
        kvm::init_host_fallback();
    }

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
            if sub_states > 0 {
                max_cs = cs;
            }
        }
        MAX_CSTATE.store(max_cs, Ordering::Relaxed);
    }

    // ── P-State initialization ──
    if hypervisor {
        // KVM exposes host CPU identity/counters, but active host-frequency
        // control is not generally writable from the guest. Keep this path
        // read-only unless a backend later proves safe synthetic controls.
    } else if is_amd {
        amd::init_pstates(hypervisor);
    } else if HAS_HWP.load(Ordering::Relaxed) {
        intel::init_hwp(hypervisor);
    } else {
        intel::init_legacy_pstate(hypervisor);
    }
    let _ = apply_profile_on_current_cpu();

    // ── Initialize APERF/MPERF baseline ──
    if HAS_APERF_MPERF.load(Ordering::Relaxed) {
        unsafe {
            let aperf = rdmsr(MSR_APERF);
            let mperf = rdmsr(MSR_MPERF);
            PER_CPU_PREV_APERF[0].store(aperf, Ordering::Relaxed);
            PER_CPU_PREV_MPERF[0].store(mperf, Ordering::Relaxed);
        }
    }
    let _ = sample_current_cpu_frequency_mhz();

    // Log results
    let max_mhz = MAX_FREQ_MHZ.load(Ordering::Relaxed);
    let base_mhz = BASE_FREQ_MHZ.load(Ordering::Relaxed);
    crate::serial_verbose_println!(
        "[OK] CPU Power: HWP={} Turbo={} APERF={} AMD={} HV={} driver={} profile={} max={}MHz base={}MHz C-states=C0..C{}",
        has_hwp(),
        has_turbo(),
        has_aperf(),
        is_amd,
        hypervisor,
        DRIVER_KIND.load(Ordering::Relaxed),
        active_profile(),
        max_mhz,
        base_mhz,
        MAX_CSTATE.load(Ordering::Relaxed)
    );
}

/// Per-AP power initialization. Enables HWP and sets P-state on each AP.
pub fn init_ap() {
    if IS_AMD.load(Ordering::Relaxed) {
        // AMD P-states are per-core on systems that expose the control MSR.
    }
    let _ = apply_profile_on_current_cpu();
    let _ = sample_current_cpu_frequency_mhz();
}

// ── APERF/MPERF Frequency Calculation ───────────────────────────────────────

fn aperf_mperf_frequency(cpu: usize) -> u32 {
    let base = BASE_FREQ_MHZ.load(Ordering::Relaxed) as u64;
    if base == 0 {
        return MAX_FREQ_MHZ.load(Ordering::Relaxed);
    }
    if cpu >= MAX_CPUS {
        return base as u32;
    }

    unsafe {
        let aperf = rdmsr(MSR_APERF);
        let mperf = rdmsr(MSR_MPERF);

        let prev_a = PER_CPU_PREV_APERF[cpu].swap(aperf, Ordering::Relaxed);
        let prev_m = PER_CPU_PREV_MPERF[cpu].swap(mperf, Ordering::Relaxed);

        let da = aperf.wrapping_sub(prev_a);
        let dm = mperf.wrapping_sub(prev_m);

        if dm == 0 || prev_m == 0 {
            return base as u32;
        }

        // actual_freq = base_freq * (aperf_delta / mperf_delta)
        let freq = base * da / dm;
        freq as u32
    }
}
