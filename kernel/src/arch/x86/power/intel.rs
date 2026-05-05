//! Intel CPU frequency control backends.
//!
//! Handles Intel HWP and legacy PERF_CTL based P-state control.

use super::{
    rdmsr, set_active_control, set_driver_kind, set_frequency_limits, wrmsr, PowerProfile,
};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

const MSR_PLATFORM_INFO: u32 = 0xCE;
const MSR_PERF_STATUS: u32 = 0x198;
const MSR_PERF_CTL: u32 = 0x199;

const MSR_PM_ENABLE: u32 = 0x770;
const MSR_HWP_CAPABILITIES: u32 = 0x771;
const MSR_HWP_REQUEST: u32 = 0x774;

const DRIVER_INTEL_HWP: u32 = 1;
const DRIVER_INTEL_LEGACY: u32 = 2;

static LEGACY_PSTATE_OK: AtomicBool = AtomicBool::new(false);
static HWP_LOWEST_RATIO: AtomicU32 = AtomicU32::new(0);
static HWP_EFFICIENT_RATIO: AtomicU32 = AtomicU32::new(0);
static HWP_HIGHEST_RATIO: AtomicU32 = AtomicU32::new(0);

pub(super) fn legacy_available() -> bool {
    LEGACY_PSTATE_OK.load(Ordering::Relaxed)
}

pub(super) fn init_hwp(hypervisor: bool) {
    unsafe {
        wrmsr(MSR_PM_ENABLE, 1);

        let caps = rdmsr(MSR_HWP_CAPABILITIES);
        let highest = (caps & 0xFF) as u32;
        let lowest = ((caps >> 8) & 0xFF) as u32;
        let efficient = ((caps >> 16) & 0xFF) as u32;

        set_frequency_limits(highest * 100, efficient * 100);
        HWP_LOWEST_RATIO.store(lowest, Ordering::Relaxed);
        HWP_EFFICIENT_RATIO.store(efficient, Ordering::Relaxed);
        HWP_HIGHEST_RATIO.store(highest, Ordering::Relaxed);
        set_active_control(true);
        if !hypervisor {
            set_driver_kind(DRIVER_INTEL_HWP);
        }

        crate::serial_verbose_println!(
            "  HWP: lowest={} efficient={} highest={} -> max={}MHz",
            lowest,
            efficient,
            highest,
            highest * 100
        );
    }
}

pub(super) fn apply_hwp_profile(profile: PowerProfile) {
    unsafe {
        wrmsr(MSR_PM_ENABLE, 1);
        let mut lowest = HWP_LOWEST_RATIO.load(Ordering::Relaxed) as u64;
        let mut efficient = HWP_EFFICIENT_RATIO.load(Ordering::Relaxed) as u64;
        let mut highest = HWP_HIGHEST_RATIO.load(Ordering::Relaxed) as u64;
        if highest == 0 {
            let caps = rdmsr(MSR_HWP_CAPABILITIES);
            highest = caps & 0xFF;
            lowest = (caps >> 8) & 0xFF;
            efficient = (caps >> 16) & 0xFF;
        }
        if lowest == 0 {
            lowest = efficient.max(1);
        }
        if efficient == 0 {
            efficient = highest.max(1);
        }

        let (min, max, epp) = match profile {
            PowerProfile::PowerSaver => (lowest, efficient.max(lowest), 0xC0u64),
            PowerProfile::Balanced => (lowest, highest.max(efficient), 0x80u64),
            PowerProfile::Performance => (highest, highest, 0x00u64),
        };
        wrmsr(MSR_HWP_REQUEST, min | (max << 8) | (epp << 24));
    }
}

pub(super) fn init_legacy_pstate(hypervisor: bool) {
    unsafe {
        let platform_info = rdmsr(MSR_PLATFORM_INFO);
        let max_ratio = ((platform_info >> 8) & 0xFF) as u32;

        if max_ratio > 0 {
            set_frequency_limits(max_ratio * 100, max_ratio * 100);
            LEGACY_PSTATE_OK.store(true, Ordering::Relaxed);
            set_active_control(true);
            if !hypervisor {
                set_driver_kind(DRIVER_INTEL_LEGACY);
            }

            crate::serial_verbose_println!(
                "  Legacy P-state: ratio={} -> max={}MHz",
                max_ratio,
                max_ratio * 100
            );
        } else {
            let tsc_mhz = (crate::arch::x86::pit::tsc_hz() / 1_000_000) as u32;
            set_frequency_limits(tsc_mhz, tsc_mhz);
            crate::serial_verbose_println!(
                "  Legacy P-state: using TSC={}MHz (no PLATFORM_INFO)",
                tsc_mhz
            );
        }
    }
}

pub(super) fn apply_legacy_profile(profile: PowerProfile, max_mhz: u32) {
    let max_ratio = max_mhz / 100;
    if max_ratio == 0 || !LEGACY_PSTATE_OK.load(Ordering::Relaxed) {
        return;
    }
    let ratio = match profile {
        PowerProfile::PowerSaver => (max_ratio / 2).max(1),
        PowerProfile::Balanced => ((max_ratio * 3) / 4).max(1),
        PowerProfile::Performance => max_ratio,
    };
    unsafe {
        wrmsr(MSR_PERF_CTL, (ratio as u64) << 8);
    }
}

pub(super) fn current_legacy_frequency_mhz(base_mhz: u32) -> u32 {
    if LEGACY_PSTATE_OK.load(Ordering::Relaxed) {
        let status = unsafe { rdmsr(MSR_PERF_STATUS) };
        let ratio = ((status >> 8) & 0xFF) as u32;
        if ratio > 0 {
            return ratio * 100;
        }
    }
    base_mhz
}
