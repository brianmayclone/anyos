/// SMP (Symmetric Multi-Processing) — AP startup and per-CPU management.
///
/// Starts Application Processors (APs) using the INIT-SIPI-SIPI sequence.
/// Each AP gets its own stack, GDT, TSS, and enters the scheduler loop.

use core::sync::atomic::{AtomicU8, AtomicU32, AtomicBool, Ordering};
use crate::arch::x86::acpi::ProcessorInfo;
use alloc::vec::Vec;

/// Maximum number of CPUs supported
pub const MAX_CPUS: usize = 16;

/// Per-CPU metadata: CPU index, LAPIC ID, BSP flag, and initialization state.
#[repr(C)]
pub struct PerCpu {
    /// Logical CPU index (0 = BSP, 1+ = APs).
    pub cpu_id: u8,
    /// Hardware LAPIC ID for this CPU.
    pub lapic_id: u8,
    /// `true` for the Bootstrap Processor.
    pub is_bsp: bool,
    /// `true` once this CPU has completed initialization.
    pub initialized: bool,
}

/// Global SMP state
static mut CPU_DATA: [PerCpu; MAX_CPUS] = {
    const INIT: PerCpu = PerCpu {
        cpu_id: 0,
        lapic_id: 0,
        is_bsp: false,
        initialized: false,
    };
    [INIT; MAX_CPUS]
};

static CPU_COUNT: AtomicU8 = AtomicU8::new(0);
static AP_STARTED: AtomicU32 = AtomicU32::new(0);
static BSP_LAPIC_ID: AtomicU8 = AtomicU8::new(0);

/// Physical address of the AP trampoline code (must be < 1MB, page-aligned)
const AP_TRAMPOLINE_PHYS: u32 = 0x8000;

/// Communication area between BSP and AP (below trampoline)
/// Layout at 0x7F00 (64-bit):
///   0x7F00: u64 — stack pointer for the AP (virtual address)
///   0x7F08: u64 — CR3 (PML4 physical address)
///   0x7F10: u64 — entry point (Rust function pointer, virtual address)
///   0x7F18: u8  — AP ready flag (set by AP when initialized)
///   0x7F1C: u32 — cpu_id assigned to the AP
const AP_COMM_BASE: u64 = 0x7F00;
const AP_COMM_STACK: u64 = AP_COMM_BASE;
const AP_COMM_CR3: u64   = AP_COMM_BASE + 8;
const AP_COMM_ENTRY: u64 = AP_COMM_BASE + 16;
const AP_COMM_READY: u64 = AP_COMM_BASE + 24;
const AP_COMM_CPUID: u64 = AP_COMM_BASE + 28;

/// Initialize BSP's per-CPU data.
pub fn init_bsp() {
    let bsp_id = crate::arch::x86::apic::lapic_id();
    BSP_LAPIC_ID.store(bsp_id, Ordering::SeqCst);

    unsafe {
        CPU_DATA[0] = PerCpu {
            cpu_id: 0,
            lapic_id: bsp_id,
            is_bsp: true,
            initialized: true,
        };
    }
    CPU_COUNT.store(1, Ordering::SeqCst);
}

/// Start all Application Processors.
pub fn start_aps(processors: &[ProcessorInfo]) {
    let bsp_id = BSP_LAPIC_ID.load(Ordering::SeqCst);

    // Copy AP trampoline to physical address 0x8000
    install_trampoline();

    let cr3 = crate::memory::virtual_mem::kernel_cr3();

    let mut cpu_id: u8 = 1; // BSP is 0

    for proc_info in processors {
        if !proc_info.enabled {
            continue;
        }
        if proc_info.apic_id == bsp_id {
            continue; // Skip BSP
        }

        crate::serial_println!("  SMP: Starting AP (APIC_ID={})...", proc_info.apic_id);

        // Allocate stack for this AP (16 KiB) — returns virtual stack top
        let stack_top = alloc_ap_stack_top();
        if stack_top == 0 {
            crate::serial_println!("  SMP: Failed to allocate AP stack");
            continue;
        }

        // Set up communication area (64-bit values)
        unsafe {
            core::ptr::write_volatile(AP_COMM_STACK as *mut u64, stack_top);
            core::ptr::write_volatile(AP_COMM_CR3 as *mut u64, cr3);
            core::ptr::write_volatile(AP_COMM_ENTRY as *mut u64, ap_entry as u64);
            core::ptr::write_volatile(AP_COMM_READY as *mut u8, 0);
            core::ptr::write_volatile(AP_COMM_CPUID as *mut u32, cpu_id as u32);
        }

        crate::serial_println!("  SMP: stack_top={:#018x}, CR3={:#018x}", stack_top, cr3);

        // Send INIT IPI
        crate::arch::x86::apic::send_init(proc_info.apic_id);

        // Wait 10ms
        delay_ms(10);

        // Send SIPI (twice, as per Intel spec)
        let vector_page = (AP_TRAMPOLINE_PHYS >> 12) as u8;
        crate::serial_println!("  SMP: Sending SIPI (vector_page={:#x})", vector_page);
        crate::arch::x86::apic::send_sipi(proc_info.apic_id, vector_page);
        delay_ms(1);
        crate::arch::x86::apic::send_sipi(proc_info.apic_id, vector_page);

        // Wait for AP to signal ready (up to 500ms)
        let start = crate::arch::x86::pit::get_ticks();
        let ready = loop {
            let flag = unsafe { core::ptr::read_volatile(AP_COMM_READY as *const u8) };
            if flag != 0 { break true; }
            let elapsed = crate::arch::x86::pit::get_ticks().wrapping_sub(start);
            if elapsed > 50 {
                crate::serial_println!("  SMP: Timeout after {} ticks waiting for AP", elapsed);
                break false;
            }
            core::hint::spin_loop();
        };

        if ready {
            // Register this CPU
            unsafe {
                CPU_DATA[cpu_id as usize] = PerCpu {
                    cpu_id,
                    lapic_id: proc_info.apic_id,
                    is_bsp: false,
                    initialized: true,
                };
            }
            AP_STARTED.fetch_add(1, Ordering::SeqCst);
            CPU_COUNT.store(cpu_id + 1, Ordering::SeqCst);
            crate::serial_println!("  SMP: AP (APIC_ID={}) started as CPU#{}", proc_info.apic_id, cpu_id);
            cpu_id += 1;
        } else {
            crate::serial_println!("  SMP: AP (APIC_ID={}) failed to start", proc_info.apic_id);
        }
    }

    crate::serial_println!("  SMP: {} CPU(s) online ({} APs)",
        cpu_count(), AP_STARTED.load(Ordering::SeqCst));
}

/// AP entry point — called by trampoline after switching to long mode.
/// Runs on the AP's own stack. Must never return.
extern "C" fn ap_entry() -> ! {
    // Load the kernel's GDT (replace trampoline's minimal GDT)
    crate::arch::x86::gdt::reload();

    // Load the kernel's IDT (AP starts with no valid IDT)
    crate::arch::x86::idt::reload();

    // Load TSS (shared with BSP, needed for Ring 3 → Ring 0 transitions)
    crate::arch::x86::tss::reload_tr();

    // Initialize this AP's LAPIC
    crate::arch::x86::apic::init_ap();

    let cpu_id = unsafe { core::ptr::read_volatile(AP_COMM_CPUID as *const u32) } as u8;
    crate::serial_println!("  SMP: AP#{} entry point reached, LAPIC initialized", cpu_id);

    // Configure SYSCALL/SYSRET MSRs for this AP
    crate::arch::x86::syscall_msr::init_ap(cpu_id as usize);

    // Signal BSP that we're ready
    unsafe {
        core::ptr::write_volatile(AP_COMM_READY as *mut u8, 1);
    }

    // Enter idle loop — the LAPIC timer will trigger scheduling
    unsafe { core::arch::asm!("sti"); }
    loop {
        unsafe { core::arch::asm!("hlt"); }
    }
}

/// Copy the AP trampoline code to physical address 0x8000.
fn install_trampoline() {
    // Include the pre-assembled trampoline binary (NASM flat binary)
    let trampoline: &[u8] = include_bytes!(env!("ANYOS_AP_TRAMPOLINE"));

    crate::serial_println!("  SMP: Trampoline size = {} bytes", trampoline.len());

    // Copy to physical address 0x8000 (identity-mapped)
    let dest = AP_TRAMPOLINE_PHYS as *mut u8;
    unsafe {
        core::ptr::copy_nonoverlapping(trampoline.as_ptr(), dest, trampoline.len());
    }
}

/// Allocate a 16 KiB stack for an AP. Returns the virtual address of the stack TOP.
/// Uses the kernel heap so the returned address is a valid higher-half virtual address,
/// accessible in any context that uses the kernel page directory.
fn alloc_ap_stack_top() -> u64 {
    let stack = alloc::vec![0u8; 16 * 1024];
    let top = stack.as_ptr() as u64 + stack.len() as u64;
    core::mem::forget(stack); // intentional leak — AP stack lives forever
    top
}

/// Get the number of online CPUs.
pub fn cpu_count() -> u8 {
    CPU_COUNT.load(Ordering::SeqCst)
}

/// Get the current CPU's index (0 = BSP).
pub fn current_cpu_id() -> u8 {
    if !crate::arch::x86::apic::is_initialized() {
        return 0; // Before APIC init, always BSP
    }
    let lapic_id = crate::arch::x86::apic::lapic_id();
    let count = cpu_count();
    for i in 0..count as usize {
        if unsafe { CPU_DATA[i].lapic_id } == lapic_id {
            return i as u8;
        }
    }
    0 // fallback
}

/// Check if the current CPU is the BSP.
pub fn is_bsp() -> bool {
    current_cpu_id() == 0
}

fn delay_ms(ms: u32) {
    let pit_hz = crate::arch::x86::pit::TICK_HZ;
    let ms_per_tick = 1000 / pit_hz;
    let ticks = ms / ms_per_tick;
    let ticks = if ticks == 0 { 1 } else { ticks };
    let start = crate::arch::x86::pit::get_ticks();
    while crate::arch::x86::pit::get_ticks().wrapping_sub(start) < ticks {
        core::hint::spin_loop();
    }
}
