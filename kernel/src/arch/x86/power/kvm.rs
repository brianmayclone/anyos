//! KVM / hypervisor CPU power backend.
//!
//! Hypervisors normally expose host CPU identity and frequency counters, but
//! often trap or hide active P-state MSRs. This backend records the virtualized
//! host surface and leaves active control to Intel/AMD backends only when the
//! hypervisor exposes those MSRs safely.

use super::{set_driver_kind, set_frequency_limits};

const DRIVER_KVM_HOST: u32 = 4;

pub(super) fn init_host_fallback() {
    set_driver_kind(DRIVER_KVM_HOST);
    let tsc_mhz = (crate::arch::x86::pit::tsc_hz() / 1_000_000) as u32;
    if tsc_mhz > 0 {
        set_frequency_limits(tsc_mhz, tsc_mhz);
    }
    crate::serial_verbose_println!("  KVM/Hypervisor: host CPU fallback={}MHz", tsc_mhz);
}
