//! ARM64 SMP bring-up via PSCI (Power State Coordination Interface).
//!
//! Uses PSCI CPU_ON to start secondary processors on QEMU virt.

use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Maximum number of CPUs supported.
pub const MAX_CPUS: usize = 16;

/// Number of online CPUs (starts at 1 for BSP).
static ONLINE_CPUS: AtomicUsize = AtomicUsize::new(1);

/// Per-CPU kernel stack top pointers (SP_EL1 value for each CPU).
static KERNEL_STACKS: [AtomicU64; MAX_CPUS] = {
    const INIT: AtomicU64 = AtomicU64::new(0);
    [INIT; MAX_CPUS]
};

/// Set the kernel stack top for a CPU (called during task setup and AP init).
#[inline]
pub fn set_kernel_stack(cpu: usize, stack_top: u64) {
    if cpu < MAX_CPUS {
        KERNEL_STACKS[cpu].store(stack_top, Ordering::Relaxed);
    }
}

/// Get the kernel stack top for a CPU.
#[inline]
pub fn get_kernel_stack(cpu: usize) -> u64 {
    if cpu < MAX_CPUS {
        KERNEL_STACKS[cpu].load(Ordering::Relaxed)
    } else {
        0
    }
}

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
    crate::serial_verbose_println!("[OK] SMP: BSP CPU {} online", current_cpu_id());
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

        crate::serial_verbose_println!("  Starting CPU {}...", cpu);

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
            crate::serial_verbose_println!("  CPU {} started successfully", cpu);
        } else {
            crate::serial_verbose_println!("  CPU {} failed to start: PSCI error {}", cpu, result);
        }
    }

    crate::serial_verbose_println!("[OK] SMP: {} CPUs online", ONLINE_CPUS.load(Ordering::Relaxed));
}

/// Full AP initialization — called from ap_startup.S as `arm64_ap_init(cpu_id)`.
///
/// Performs the same per-CPU setup that `kernel_main` does for the BSP:
/// - Installs exception vectors (VBAR_EL1)
/// - Initializes the GIC CPU interface
/// - Enables the generic timer
/// - Registers the AP in the scheduler run-queue
/// - Enables interrupts and enters the scheduler idle loop
#[no_mangle]
pub extern "C" fn arm64_ap_init(cpu_id: usize) {
    // 1. Exception vectors
    super::exceptions::init();

    // 2. GIC CPU interface
    super::gic::init_cpu(cpu_id);

    // 3. Generic timer (enables PPI 30 on this CPU)
    super::generic_timer::init();

    // 4. Register IRQ handler for the timer
    super::exceptions::register_irq(
        30,
        super::generic_timer::irq_handler_with_schedule,
    );

    // 5. Syscall support for this AP
    super::syscall::init_cpu(cpu_id);

    crate::serial_verbose_println!("[OK] AP CPU {} online", cpu_id);

    // 6. Enable interrupts and enter scheduler
    crate::arch::hal::enable_interrupts();
    crate::task::scheduler::run();
}
