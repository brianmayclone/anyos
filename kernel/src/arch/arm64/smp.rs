//! ARM64 SMP bring-up via PSCI (Power State Coordination Interface).
//!
//! Uses PSCI CPU_ON to start secondary processors on QEMU virt.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Maximum number of CPUs supported.
pub const MAX_CPUS: usize = 16;

/// Number of online CPUs (starts at 1 for BSP).
static ONLINE_CPUS: AtomicUsize = AtomicUsize::new(1);

/// PSCI function IDs (SMCCC calling convention).
const PSCI_CPU_ON_64: u64 = 0xC400_0003;

/// Get the number of online CPUs.
#[inline]
pub fn cpu_count() -> usize {
    ONLINE_CPUS.load(Ordering::Relaxed)
}

/// Get the current CPU ID from MPIDR_EL1.
#[inline]
pub fn current_cpu_id() -> usize {
    let mpidr: u64;
    unsafe {
        core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr, options(nomem, nostack));
    }
    (mpidr & 0xFF) as usize
}

/// Initialize BSP SMP state.
pub fn init_bsp() {
    ONLINE_CPUS.store(1, Ordering::Relaxed);
    crate::serial_println!("[OK] SMP: BSP CPU {} online", current_cpu_id());
}

/// Start Application Processors using PSCI CPU_ON.
///
/// `num_cpus` is the total number of CPUs to bring up (including BSP).
/// The entry point for APs is `_ap_entry` defined in `asm_arm64/ap_startup.S`.
pub fn start_aps(num_cpus: usize) {
    extern "C" {
        fn _ap_entry();
    }

    let entry_addr = _ap_entry as *const () as u64;
    let bsp_id = current_cpu_id();

    for cpu in 0..num_cpus {
        if cpu == bsp_id {
            continue;
        }

        crate::serial_println!("  Starting CPU {}...", cpu);

        // PSCI CPU_ON: SMC #0 with x0=fn_id, x1=target_cpu, x2=entry_point, x3=context_id
        let result: i64;
        unsafe {
            core::arch::asm!(
                "mov x0, {fn_id}",
                "mov x1, {target}",
                "mov x2, {entry}",
                "mov x3, {ctx}",
                "hvc #0",
                "mov {result}, x0",
                fn_id = in(reg) PSCI_CPU_ON_64,
                target = in(reg) cpu as u64,
                entry = in(reg) entry_addr,
                ctx = in(reg) cpu as u64,
                result = out(reg) result,
                out("x0") _, out("x1") _, out("x2") _, out("x3") _,
                options(nostack),
            );
        }

        if result == 0 {
            ONLINE_CPUS.fetch_add(1, Ordering::Relaxed);
            crate::serial_println!("  CPU {} started successfully", cpu);
        } else {
            crate::serial_println!("  CPU {} failed to start: PSCI error {}", cpu, result);
        }
    }

    crate::serial_println!("[OK] SMP: {} CPUs online", ONLINE_CPUS.load(Ordering::Relaxed));
}

/// Register the current AP as online (called from ap_startup.S → Rust AP entry).
pub fn register_ap() {
    // AP-specific init will be done here
    let cpu = current_cpu_id();
    crate::serial_println!("  AP CPU {} entered Rust code", cpu);
}
