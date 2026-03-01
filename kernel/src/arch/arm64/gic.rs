//! GICv3 (Generic Interrupt Controller v3) driver for QEMU virt machine.
//!
//! QEMU virt GICv3 base addresses:
//! - GICD: 0x0800_0000 (Distributor)
//! - GICR: 0x080A_0000 (Redistributor, per-CPU)

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// GICD (Distributor) base address on QEMU virt.
const GICD_BASE: usize = 0x0800_0000;

/// GICR (Redistributor) base address on QEMU virt.
const GICR_BASE: usize = 0x080A_0000;

/// GICR stride per CPU (128 KiB: 64 KiB RD_base + 64 KiB SGI_base).
const GICR_STRIDE: usize = 0x20000;

// GICD register offsets
const GICD_CTLR: usize = 0x000;
const GICD_TYPER: usize = 0x004;
const GICD_ISENABLER: usize = 0x100;
const GICD_ICENABLER: usize = 0x180;
const GICD_IPRIORITYR: usize = 0x400;
const GICD_ITARGETSR: usize = 0x800;
const GICD_ICFGR: usize = 0xC00;

// GICR register offsets (SGI_base, offset 0x10000 from RD_base)
const GICR_SGI_ISENABLER0: usize = 0x10100;
const GICR_SGI_ICENABLER0: usize = 0x10180;
const GICR_SGI_IPRIORITYR: usize = 0x10400;

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Last acknowledged interrupt ID (per-CPU in single-core; for SMP needs per-CPU storage).
static LAST_IAR: AtomicU32 = AtomicU32::new(1023);

/// Write to a GICD register.
#[inline]
unsafe fn gicd_write(offset: usize, val: u32) {
    core::ptr::write_volatile((GICD_BASE + offset) as *mut u32, val);
}

/// Read from a GICD register.
#[inline]
unsafe fn gicd_read(offset: usize) -> u32 {
    core::ptr::read_volatile((GICD_BASE + offset) as *const u32)
}

/// Write to a GICR register for the given CPU.
#[inline]
unsafe fn gicr_write(cpu: usize, offset: usize, val: u32) {
    let addr = GICR_BASE + cpu * GICR_STRIDE + offset;
    core::ptr::write_volatile(addr as *mut u32, val);
}

/// Read from a GICR register for the given CPU.
#[inline]
unsafe fn gicr_read(cpu: usize, offset: usize) -> u32 {
    let addr = GICR_BASE + cpu * GICR_STRIDE + offset;
    core::ptr::read_volatile(addr as *const u32)
}

/// Initialize the GICv3 distributor (BSP only, called once).
pub fn init_distributor() {
    unsafe {
        // Disable distributor while configuring
        gicd_write(GICD_CTLR, 0);

        // Read number of interrupt lines
        let typer = gicd_read(GICD_TYPER);
        let max_irqs = ((typer & 0x1F) + 1) * 32;

        // Disable all SPIs
        let mut i = 1; // Skip SGIs (bank 0)
        while i < max_irqs / 32 {
            gicd_write(GICD_ICENABLER + (i as usize) * 4, 0xFFFF_FFFF);
            i += 1;
        }

        // Set all SPIs to lowest priority
        i = 8; // Skip SGIs+PPIs (first 8 registers = 32 interrupts)
        while i < max_irqs as u32 {
            gicd_write(GICD_IPRIORITYR + (i as usize), 0xA0A0_A0A0);
            i += 4;
        }

        // Enable distributor with affinity routing (ARE_NS)
        gicd_write(GICD_CTLR, (1 << 0) | (1 << 4)); // EnableGrp1NS | ARE_NS
    }
    INITIALIZED.store(true, Ordering::Relaxed);
}

/// Initialize the GICv3 redistributor + CPU interface for the current CPU.
pub fn init_cpu(cpu: usize) {
    unsafe {
        // Wake up redistributor
        let waker_offset = 0x14; // GICR_WAKER
        let waker = core::ptr::read_volatile(
            (GICR_BASE + cpu * GICR_STRIDE + waker_offset) as *const u32
        );
        core::ptr::write_volatile(
            (GICR_BASE + cpu * GICR_STRIDE + waker_offset) as *mut u32,
            waker & !(1 << 1) // Clear ProcessorSleep
        );
        // Wait for ChildrenAsleep to clear
        while core::ptr::read_volatile(
            (GICR_BASE + cpu * GICR_STRIDE + waker_offset) as *const u32
        ) & (1 << 2) != 0 {
            core::hint::spin_loop();
        }

        // Enable all SGIs (0-15), disable all PPIs (16-31)
        gicr_write(cpu, GICR_SGI_ISENABLER0, 0x0000_FFFF);

        // Set SGI/PPI priorities
        for i in 0..8 {
            gicr_write(cpu, GICR_SGI_IPRIORITYR + i * 4, 0xA0A0_A0A0);
        }

        // Configure CPU interface via system registers
        // ICC_SRE_EL1: enable system register interface
        core::arch::asm!("msr icc_sre_el1, {}", in(reg) 0x7u64, options(nostack));
        core::arch::asm!("isb", options(nostack));

        // ICC_PMR_EL1: set priority mask to allow all priorities
        core::arch::asm!("msr icc_pmr_el1, {}", in(reg) 0xFFu64, options(nostack));

        // ICC_BPR1_EL1: no preemption grouping
        core::arch::asm!("msr icc_bpr1_el1, {}", in(reg) 0u64, options(nostack));

        // ICC_CTLR_EL1: EOImode = 0 (combined priority drop + deactivate)
        core::arch::asm!("msr icc_ctlr_el1, {}", in(reg) 0u64, options(nostack));

        // ICC_IGRPEN1_EL1: enable Group 1 interrupts
        core::arch::asm!("msr icc_igrpen1_el1, {}", in(reg) 1u64, options(nostack));

        core::arch::asm!("isb", options(nostack));
    }
}

/// Send End-Of-Interrupt for the given interrupt ID.
pub fn eoi(intid: u32) {
    unsafe {
        core::arch::asm!(
            "msr icc_eoir1_el1, {}",
            in(reg) intid as u64,
            options(nostack),
        );
    }
}

/// Send EOI for the last acknowledged interrupt.
pub fn eoi_current() {
    // Read IAR to get the interrupt ID, then write EOIR
    let intid: u64;
    unsafe {
        core::arch::asm!("mrs {}, icc_iar1_el1", out(reg) intid, options(nostack));
    }
    if intid < 1020 { // Valid interrupt (not spurious)
        eoi(intid as u32);
    }
}

/// Acknowledge and return the pending interrupt ID (or 1023 if spurious).
pub fn acknowledge() -> u32 {
    let intid: u64;
    unsafe {
        core::arch::asm!("mrs {}, icc_iar1_el1", out(reg) intid, options(nostack));
    }
    intid as u32
}

/// Enable a specific SPI interrupt at the distributor.
pub fn enable_irq(irq: u32) {
    let reg = (irq / 32) as usize;
    let bit = irq % 32;
    unsafe {
        gicd_write(GICD_ISENABLER + reg * 4, 1 << bit);
    }
}

/// Disable a specific SPI interrupt at the distributor.
pub fn disable_irq(irq: u32) {
    let reg = (irq / 32) as usize;
    let bit = irq % 32;
    unsafe {
        gicd_write(GICD_ICENABLER + reg * 4, 1 << bit);
    }
}

/// Send a Software Generated Interrupt (SGI) to a specific CPU.
/// Used for IPIs on ARM64.
pub fn send_sgi(target_cpu: usize, sgi_id: u8) {
    // ICC_SGI1R_EL1 format for GICv3:
    // Bits [23:16] = TargetList (1 << target_cpu within affinity)
    // Bits [3:0]   = INTID (SGI number 0-15)
    // Bits [55:48] = Aff3, [39:32] = Aff2, [31:24] = Aff1
    // For QEMU virt with flat topology: Aff0 = cpu_id
    let val: u64 = ((1u64 << target_cpu) << 16) | (sgi_id as u64);
    unsafe {
        core::arch::asm!(
            "msr icc_sgi1r_el1, {}",
            in(reg) val,
            options(nostack),
        );
        core::arch::asm!("isb", options(nostack));
    }
}

/// Acknowledge the pending interrupt and save its ID for later EOI.
pub fn acknowledge_and_save() -> u32 {
    let intid = acknowledge();
    LAST_IAR.store(intid, Ordering::Relaxed);
    intid
}

/// Send EOI for the last acknowledged interrupt (used by HAL irq_eoi()).
pub fn eoi_last() {
    let intid = LAST_IAR.load(Ordering::Relaxed);
    if intid < 1020 {
        eoi(intid);
    }
}

/// Check if GIC is initialized.
pub fn is_initialized() -> bool {
    INITIALIZED.load(Ordering::Relaxed)
}
