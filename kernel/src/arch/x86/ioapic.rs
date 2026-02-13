/// I/O APIC driver — routes external hardware interrupts to Local APICs.
///
/// The I/O APIC replaces the legacy 8259 PIC for interrupt routing.
/// It has a redirection table with one entry per interrupt input pin,
/// mapping each to a destination LAPIC and vector.

use alloc::vec::Vec;
use crate::arch::x86::acpi::{IoApicInfo, IsoInfo};

/// Virtual address where I/O APIC MMIO is mapped
const IOAPIC_VIRT_BASE: u64 = 0xFFFF_FFFF_D011_0000;

// I/O APIC registers (accessed indirectly via IOREGSEL/IOWIN)
const IOAPIC_REGSEL: u32 = 0x00;  // Register select
const IOAPIC_IOWIN: u32  = 0x10;  // I/O window (data)

// Register indices
const IOAPICID: u32   = 0x00;
const IOAPICVER: u32  = 0x01;
const IOREDTBL: u32   = 0x10;  // Redirection table base (entries at 0x10 + 2*n)

// Redirection entry flags
const REDIR_MASKED: u64    = 1 << 16;
const REDIR_LEVEL: u64     = 1 << 15;   // Level-triggered
const REDIR_ACTIVELOW: u64 = 1 << 13;   // Active-low polarity

/// Maximum number of redirection entries reported by the I/O APIC.
static mut IOAPIC_MAX_ENTRIES: u32 = 0;

/// ISO overrides: for each ISA IRQ (0-15), store the actual GSI it maps to.
/// Default: GSI = IRQ (no override).
static mut IRQ_TO_GSI: [u32; 16] = [0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15];

/// Initialize the I/O APIC.
pub fn init(io_apic_info: &[IoApicInfo], isos: &[IsoInfo]) {
    if io_apic_info.is_empty() {
        crate::serial_println!("  IOAPIC: No I/O APICs found");
        return;
    }

    let info = &io_apic_info[0]; // Use first I/O APIC

    // Map I/O APIC MMIO
    use crate::memory::address::{PhysAddr, VirtAddr};
    use crate::memory::virtual_mem;

    virtual_mem::map_page(
        VirtAddr::new(IOAPIC_VIRT_BASE),
        PhysAddr::new(info.address as u64),
        0x03, // PAGE_PRESENT | PAGE_WRITABLE
    );

    // Read version and max redirection entries
    let version = read_reg(IOAPICVER);
    let max_entries = ((version >> 16) & 0xFF) + 1;
    unsafe { IOAPIC_MAX_ENTRIES = max_entries; }

    crate::serial_println!("  IOAPIC: id={} at {:#010x} (virt {:#010x}), {} entries",
        info.id, info.address, IOAPIC_VIRT_BASE, max_entries);

    // Mask all entries initially
    for i in 0..max_entries {
        write_redir(i, REDIR_MASKED);
    }

    // Store ISO overrides in the lookup table
    for iso in isos {
        if (iso.source as usize) < 16 {
            unsafe { IRQ_TO_GSI[iso.source as usize] = iso.gsi; }
        }
    }

    // Set up ISA IRQ → APIC vector mappings
    // Default: IRQ N → GSI N → vector (32 + N), unless overridden by ISOs
    setup_irq_routing(isos);
}

/// Configure interrupt routing for standard ISA IRQs.
fn setup_irq_routing(isos: &[IsoInfo]) {
    // Build ISO lookup: source IRQ → (GSI, flags)
    let mut overrides = [(0u32, 0u16); 16];
    let mut has_override = [false; 16];

    for iso in isos {
        if (iso.source as usize) < 16 {
            overrides[iso.source as usize] = (iso.gsi, iso.flags);
            has_override[iso.source as usize] = true;
        }
    }

    // Map each ISA IRQ
    for irq in 0..16u8 {
        if irq == 2 { continue; } // Skip cascade (doesn't exist in APIC mode)

        let (gsi, flags) = if has_override[irq as usize] {
            overrides[irq as usize]
        } else {
            (irq as u32, 0u16) // Default: GSI = IRQ, edge-triggered, active-high
        };

        let max = unsafe { IOAPIC_MAX_ENTRIES };
        if gsi >= max {
            continue;
        }

        // Vector = 32 + IRQ (same as PIC mapping for compatibility)
        let vector = 32 + irq;

        // Determine trigger mode and polarity from ISO flags
        let mut entry: u64 = vector as u64;

        // Polarity (bits 0-1 of flags): 00=bus default, 01=active high, 11=active low
        let polarity = flags & 0x03;
        if polarity == 3 {
            entry |= REDIR_ACTIVELOW;
        }

        // Trigger mode (bits 2-3 of flags): 00=bus default, 01=edge, 11=level
        let trigger = (flags >> 2) & 0x03;
        if trigger == 3 {
            entry |= REDIR_LEVEL;
        }

        // Destination: BSP (LAPIC ID 0) for now — fixed delivery, physical mode
        // High 32 bits contain destination APIC ID in bits 56-63 (relative to entry start)
        entry |= 0; // destination = LAPIC 0

        write_redir(gsi, entry);
    }
}

/// Enable (unmask) an ISA IRQ on the I/O APIC, respecting ISO overrides.
/// E.g., `unmask_irq(0)` unmasks the PIT, which may be on GSI 2 per the ISO.
pub fn unmask_irq(irq: u8) {
    let gsi = if (irq as usize) < 16 {
        unsafe { IRQ_TO_GSI[irq as usize] }
    } else {
        irq as u32
    };
    unmask(gsi);
}

/// Enable (unmask) a specific GSI on the I/O APIC.
pub fn unmask(gsi: u32) {
    let max = unsafe { IOAPIC_MAX_ENTRIES };
    if gsi >= max { return; }

    let entry = read_redir(gsi);
    write_redir(gsi, entry & !REDIR_MASKED);
}

/// Disable (mask) an ISA IRQ on the I/O APIC, respecting ISO overrides.
pub fn mask_irq(irq: u8) {
    let gsi = if (irq as usize) < 16 {
        unsafe { IRQ_TO_GSI[irq as usize] }
    } else {
        irq as u32
    };
    mask(gsi);
}

/// Disable (mask) a specific GSI/IRQ on the I/O APIC.
pub fn mask(gsi: u32) {
    let max = unsafe { IOAPIC_MAX_ENTRIES };
    if gsi >= max { return; }

    let entry = read_redir(gsi);
    write_redir(gsi, entry | REDIR_MASKED);
}

/// Set the destination LAPIC for a GSI.
pub fn set_destination(gsi: u32, lapic_id: u8) {
    let max = unsafe { IOAPIC_MAX_ENTRIES };
    if gsi >= max { return; }

    let entry = read_redir(gsi);
    // Clear destination bits (56-63) and set new destination
    let entry = (entry & 0x00FFFFFF_FFFFFFFF) | ((lapic_id as u64) << 56);
    write_redir(gsi, entry);
}

// Low-level I/O APIC register access (indirect via IOREGSEL/IOWIN)

fn read_reg(reg: u32) -> u32 {
    unsafe {
        let regsel = (IOAPIC_VIRT_BASE + IOAPIC_REGSEL as u64) as *mut u32;
        let iowin = (IOAPIC_VIRT_BASE + IOAPIC_IOWIN as u64) as *mut u32;
        core::ptr::write_volatile(regsel, reg);
        core::ptr::read_volatile(iowin)
    }
}

fn write_reg(reg: u32, value: u32) {
    unsafe {
        let regsel = (IOAPIC_VIRT_BASE + IOAPIC_REGSEL as u64) as *mut u32;
        let iowin = (IOAPIC_VIRT_BASE + IOAPIC_IOWIN as u64) as *mut u32;
        core::ptr::write_volatile(regsel, reg);
        core::ptr::write_volatile(iowin, value);
    }
}

fn read_redir(index: u32) -> u64 {
    let reg = IOREDTBL + index * 2;
    let low = read_reg(reg) as u64;
    let high = read_reg(reg + 1) as u64;
    low | (high << 32)
}

fn write_redir(index: u32, value: u64) {
    let reg = IOREDTBL + index * 2;
    write_reg(reg, value as u32);
    write_reg(reg + 1, (value >> 32) as u32);
}

/// Disable the legacy 8259 PIC by masking all interrupts.
/// This should be called after I/O APIC is initialized.
pub fn disable_legacy_pic() {
    unsafe {
        // Mask all IRQs on both PICs
        crate::arch::x86::port::outb(0x21, 0xFF);
        crate::arch::x86::port::outb(0xA1, 0xFF);
    }
    crate::serial_println!("  Legacy 8259 PIC disabled");
}
