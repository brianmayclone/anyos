//! AMD CPU frequency control backend.

use super::{
    rdmsr, set_active_control, set_driver_kind, set_frequency_limits, wrmsr, PowerProfile,
};

const MSR_AMD_PSTATE_CTRL: u32 = 0xC001_0062;
const MSR_AMD_PSTATE_STATUS: u32 = 0xC001_0063;
const MSR_AMD_PSTATE_DEF_BASE: u32 = 0xC001_0064;

const DRIVER_AMD_PSTATE: u32 = 3;

pub(super) fn init_pstates(hypervisor: bool) {
    unsafe {
        if hypervisor {
            let tsc_mhz = (crate::arch::x86::pit::tsc_hz() / 1_000_000) as u32;
            set_frequency_limits(tsc_mhz, tsc_mhz);
            crate::serial_verbose_println!("  AMD: hypervisor detected, using TSC={}MHz", tsc_mhz);
            return;
        }

        let pstate0 = rdmsr(MSR_AMD_PSTATE_DEF_BASE);
        if pstate0 & (1 << 63) != 0 {
            let freq = pstate_frequency_mhz(pstate0);
            set_frequency_limits(freq, freq);
            set_active_control(true);
            set_driver_kind(DRIVER_AMD_PSTATE);
            crate::serial_verbose_println!("  AMD P-state 0: {}MHz", freq);
        } else {
            let tsc_mhz = (crate::arch::x86::pit::tsc_hz() / 1_000_000) as u32;
            set_frequency_limits(tsc_mhz, tsc_mhz);
            crate::serial_verbose_println!("  AMD: using TSC={}MHz (no P-state info)", tsc_mhz);
        }
    }
}

pub(super) fn apply_profile(profile: PowerProfile, hypervisor: bool, active_control: bool) -> bool {
    if hypervisor || !active_control {
        return false;
    }
    let preferred = match profile {
        PowerProfile::Performance => 0u32,
        PowerProfile::Balanced => 1u32,
        PowerProfile::PowerSaver => 2u32,
    };

    unsafe {
        for offset in 0..=preferred {
            let idx = preferred - offset;
            let pstate = rdmsr(MSR_AMD_PSTATE_DEF_BASE + idx);
            if pstate & (1 << 63) != 0 {
                wrmsr(MSR_AMD_PSTATE_CTRL, idx as u64);
                return true;
            }
        }
    }
    false
}

pub(super) fn current_frequency_mhz(base_mhz: u32, hypervisor: bool) -> u32 {
    unsafe {
        if hypervisor {
            return base_mhz;
        }
        let status = rdmsr(MSR_AMD_PSTATE_STATUS);
        let cur_pstate = (status & 0x7) as u32;
        let pstate_def = rdmsr(MSR_AMD_PSTATE_DEF_BASE + cur_pstate);
        if pstate_def & (1 << 63) != 0 {
            pstate_frequency_mhz(pstate_def)
        } else {
            base_mhz
        }
    }
}

fn pstate_frequency_mhz(pstate: u64) -> u32 {
    let fid = (pstate & 0xFF) as u32;
    let did = ((pstate >> 8) & 0x3F) as u32;
    if did == 0 {
        return 0;
    }
    (200 * fid) / did
}
