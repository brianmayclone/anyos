/// Local APIC driver — per-CPU interrupt controller.
///
/// Each CPU has its own Local APIC mapped at the same physical address
/// (default 0xFEE00000). The LAPIC handles:
/// - Local timer interrupts (replaces PIT for preemptive scheduling)
/// - Inter-Processor Interrupts (IPI) for SMP coordination
/// - EOI (End of Interrupt) signaling

use core::sync::atomic::{AtomicU32, AtomicBool, Ordering};

// LAPIC register offsets
const LAPIC_ID: u32        = 0x020;
const LAPIC_VERSION: u32   = 0x030;
const LAPIC_TPR: u32       = 0x080;  // Task Priority Register
const LAPIC_EOI: u32       = 0x0B0;  // End of Interrupt
const LAPIC_SVR: u32       = 0x0F0;  // Spurious Interrupt Vector Register
const LAPIC_ICR_LOW: u32   = 0x300;  // Interrupt Command Register (low)
const LAPIC_ICR_HIGH: u32  = 0x310;  // Interrupt Command Register (high)
const LAPIC_TIMER: u32     = 0x320;  // LVT Timer Register
const LAPIC_LINT0: u32     = 0x350;  // LVT LINT0
const LAPIC_LINT1: u32     = 0x360;  // LVT LINT1
const LAPIC_TIMER_INIT: u32 = 0x380; // Timer Initial Count
const LAPIC_TIMER_CURRENT: u32 = 0x390; // Timer Current Count
const LAPIC_TIMER_DIV: u32 = 0x3E0;  // Timer Divide Configuration

// SVR flags
const SVR_ENABLE: u32 = 1 << 8;

// Timer modes
const TIMER_PERIODIC: u32 = 1 << 17;
const TIMER_MASKED: u32   = 1 << 16;

// ICR delivery modes
const ICR_INIT: u32    = 5 << 8;
const ICR_STARTUP: u32 = 6 << 8;
const ICR_LEVEL: u32   = 1 << 14;
const ICR_ASSERT: u32  = 1 << 15;
const ICR_DEASSERT: u32 = 0;

/// Interrupt vector for the LAPIC periodic timer (INT 48).
pub const VECTOR_TIMER: u8    = 48;
/// Interrupt vector for spurious interrupts (INT 255).
pub const VECTOR_SPURIOUS: u8 = 255;
/// IPI vector used to halt a remote processor (INT 53 = IRQ 21).
pub const VECTOR_IPI_HALT: u8 = 53;
/// IPI vector used for TLB shootdown across cores (INT 52 = IRQ 20).
pub const VECTOR_IPI_TLB: u8  = 52;

/// Virtual address where LAPIC MMIO is mapped
const LAPIC_VIRT_BASE: u64 = 0xFFFF_FFFF_D010_0000;

static LAPIC_PHYS: AtomicU32 = AtomicU32::new(0);
static LAPIC_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Map and initialize the BSP's Local APIC.
pub fn init_bsp(lapic_phys: u32) {
    LAPIC_PHYS.store(lapic_phys, Ordering::SeqCst);

    // Map LAPIC MMIO page (4K is sufficient, LAPIC register space is 1 page)
    use crate::memory::address::{PhysAddr, VirtAddr};
    use crate::memory::virtual_mem;

    virtual_mem::map_page(
        VirtAddr::new(LAPIC_VIRT_BASE),
        PhysAddr::new(lapic_phys as u64),
        0x03, // PAGE_PRESENT | PAGE_WRITABLE
    );

    // Set up the LAPIC
    unsafe {
        // Enable LAPIC via SVR — set spurious vector and enable bit
        let svr = read(LAPIC_SVR);
        write(LAPIC_SVR, svr | SVR_ENABLE | VECTOR_SPURIOUS as u32);

        // Set Task Priority to 0 (accept all interrupts)
        write(LAPIC_TPR, 0);

        // Disable LVT LINT0/LINT1 (we'll use IOAPIC for external interrupts)
        write(LAPIC_LINT0, TIMER_MASKED);
        write(LAPIC_LINT1, TIMER_MASKED);

        // Disable timer initially
        write(LAPIC_TIMER, TIMER_MASKED);
    }

    let id = lapic_id();
    let version = unsafe { read(LAPIC_VERSION) } & 0xFF;
    crate::serial_println!("  LAPIC: BSP id={} version={:#x} at virt {:#010x}",
        id, version, LAPIC_VIRT_BASE);

    LAPIC_INITIALIZED.store(true, Ordering::SeqCst);
}

/// Initialize the LAPIC on an Application Processor.
pub fn init_ap() {
    unsafe {
        // Enable LAPIC
        let svr = read(LAPIC_SVR);
        write(LAPIC_SVR, svr | SVR_ENABLE | VECTOR_SPURIOUS as u32);

        // Accept all interrupts
        write(LAPIC_TPR, 0);

        // Disable LINTs
        write(LAPIC_LINT0, TIMER_MASKED);
        write(LAPIC_LINT1, TIMER_MASKED);

        // Start LAPIC timer using calibrated count from BSP
        let count = timer_initial_count();
        if count > 0 {
            start_timer_with_count(count);
            crate::serial_println!("  LAPIC: AP id={} timer started (count={})", lapic_id(), count);
        } else {
            write(LAPIC_TIMER, TIMER_MASKED);
            crate::serial_println!("  LAPIC: AP id={} timer masked (BSP not yet calibrated)", lapic_id());
        }
    }

    let id = lapic_id();
    crate::serial_println!("  LAPIC: AP id={} initialized", id);
}

/// Calibrate and start the LAPIC timer for periodic scheduling.
/// Uses the PIT to measure the LAPIC timer frequency, then sets up
/// periodic mode at approximately `target_hz` interrupts per second.
pub fn calibrate_timer(target_hz: u32) {
    // Use PIT channel 2 for calibration: count down from max over a known period
    // Alternatively, use the PIT tick counter we already have.
    //
    // Simple approach: measure LAPIC ticks over 1 PIT tick, then scale.
    let pit_hz = crate::arch::x86::pit::TICK_HZ;
    unsafe {
        // Set timer divider to 16
        write(LAPIC_TIMER_DIV, 0x03); // divide by 16

        // Set initial count to max
        write(LAPIC_TIMER_INIT, 0xFFFFFFFF);
        write(LAPIC_TIMER, TIMER_MASKED | VECTOR_TIMER as u32); // one-shot, masked

        // Wait 1 PIT tick (1000/pit_hz ms)
        let start = crate::arch::x86::pit::get_ticks();
        while crate::arch::x86::pit::get_ticks().wrapping_sub(start) < 1 {
            core::hint::spin_loop();
        }

        // Read how many ticks elapsed
        let elapsed = 0xFFFFFFFF - read(LAPIC_TIMER_CURRENT);

        // Stop timer
        write(LAPIC_TIMER, TIMER_MASKED);

        // Calculate initial count for desired frequency
        // elapsed ticks in 1 PIT tick → ticks_per_second = elapsed * pit_hz
        // initial_count = ticks_per_second / target_hz
        let ticks_per_second = elapsed as u64 * pit_hz as u64;
        let initial_count = (ticks_per_second / target_hz as u64) as u32;

        crate::serial_println!("  LAPIC timer: {} ticks/{}ms, initial_count={} for {}Hz",
            elapsed, 1000 / pit_hz, initial_count, target_hz);

        // Store calibrated value for APs
        TIMER_INITIAL_COUNT.store(initial_count, Ordering::SeqCst);

        // Start periodic timer
        start_timer_with_count(initial_count);
    }
}

static TIMER_INITIAL_COUNT: AtomicU32 = AtomicU32::new(0);

fn timer_initial_count() -> u32 {
    TIMER_INITIAL_COUNT.load(Ordering::SeqCst)
}

unsafe fn start_timer_with_count(initial_count: u32) {
    // Divider = 16
    write(LAPIC_TIMER_DIV, 0x03);
    // Periodic mode, vector = VECTOR_TIMER
    write(LAPIC_TIMER, TIMER_PERIODIC | VECTOR_TIMER as u32);
    // Set initial count (starts counting)
    write(LAPIC_TIMER_INIT, initial_count);
}

/// Get this CPU's LAPIC ID.
pub fn lapic_id() -> u8 {
    unsafe { ((read(LAPIC_ID) >> 24) & 0xFF) as u8 }
}

/// Send End of Interrupt (EOI) to the Local APIC.
pub fn eoi() {
    if LAPIC_INITIALIZED.load(Ordering::Relaxed) {
        unsafe { write(LAPIC_EOI, 0); }
    }
}

/// Send an INIT IPI to a target processor.
pub fn send_init(target_apic_id: u8) {
    unsafe {
        // Set target in ICR high
        write(LAPIC_ICR_HIGH, (target_apic_id as u32) << 24);
        // Send INIT, level-triggered, assert
        write(LAPIC_ICR_LOW, ICR_INIT | ICR_LEVEL | ICR_ASSERT);

        // Wait for delivery (poll delivery status bit)
        wait_icr_idle();

        // De-assert
        write(LAPIC_ICR_HIGH, (target_apic_id as u32) << 24);
        write(LAPIC_ICR_LOW, ICR_INIT | ICR_LEVEL | ICR_DEASSERT);
        wait_icr_idle();
    }
}

/// Send a STARTUP IPI (SIPI) to a target processor.
/// `vector_page` is the physical page number (0-255) of the AP trampoline.
/// E.g., if trampoline is at 0x8000, vector_page = 0x08.
pub fn send_sipi(target_apic_id: u8, vector_page: u8) {
    unsafe {
        write(LAPIC_ICR_HIGH, (target_apic_id as u32) << 24);
        write(LAPIC_ICR_LOW, ICR_STARTUP | vector_page as u32);
        wait_icr_idle();
    }
}

/// Send a generic IPI to a target processor.
pub fn send_ipi(target_apic_id: u8, vector: u8) {
    unsafe {
        write(LAPIC_ICR_HIGH, (target_apic_id as u32) << 24);
        write(LAPIC_ICR_LOW, vector as u32); // fixed delivery
        wait_icr_idle();
    }
}

unsafe fn wait_icr_idle() {
    // Bit 12 of ICR_LOW is delivery status (0 = idle)
    for _ in 0..10000 {
        if read(LAPIC_ICR_LOW) & (1 << 12) == 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

/// Read a LAPIC register.
#[inline(always)]
unsafe fn read(reg: u32) -> u32 {
    let addr = LAPIC_VIRT_BASE + reg as u64;
    core::ptr::read_volatile(addr as *const u32)
}

/// Write a LAPIC register.
#[inline(always)]
unsafe fn write(reg: u32, value: u32) {
    let addr = LAPIC_VIRT_BASE + reg as u64;
    core::ptr::write_volatile(addr as *mut u32, value);
}

/// Check if LAPIC is initialized.
pub fn is_initialized() -> bool {
    LAPIC_INITIALIZED.load(Ordering::Relaxed)
}
