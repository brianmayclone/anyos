//! PCI MSI/MSI-X and PCIe ECAM support.
//!
//! Adds Message Signaled Interrupts (MSI/MSI-X) for PCI/PCIe devices and
//! Enhanced Configuration Access Mechanism (ECAM) for the full 4 KiB PCIe
//! config space (via MCFG ACPI table).

use crate::drivers::pci::{pci_config_read16, pci_config_read32, pci_config_write32, PciDevice};
use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

// ── PCI Capability IDs ──────────────────────────────────────────────────────

const PCI_CAP_MSI: u8 = 0x05;
const PCI_CAP_MSIX: u8 = 0x11;
const PCI_CAP_PCIE: u8 = 0x10;

// ── MSI Register Offsets (relative to capability pointer) ───────────────────

const MSI_CONTROL: u8 = 0x02; // Message Control (16-bit)
const MSI_ADDR_LO: u8 = 0x04; // Message Address (lower 32 bits)
const MSI_ADDR_HI: u8 = 0x08; // Message Address (upper 32 bits, 64-bit only)
const MSI_DATA_32: u8 = 0x08; // Message Data (32-bit addressing)
const MSI_DATA_64: u8 = 0x0C; // Message Data (64-bit addressing)

// MSI Control register bits
const MSI_CTRL_ENABLE: u16 = 1 << 0;
const MSI_CTRL_64BIT: u16 = 1 << 7;
const MSI_CTRL_PERVECTOR_MASK: u16 = 1 << 8;

// ── MSI-X Register Offsets ──────────────────────────────────────────────────

const MSIX_CONTROL: u8 = 0x02; // Message Control (16-bit)
const MSIX_TABLE_OFFSET: u8 = 0x04; // Table Offset + BIR
const MSIX_PBA_OFFSET: u8 = 0x08; // PBA Offset + BIR

// MSI-X Control bits
const MSIX_CTRL_ENABLE: u16 = 1 << 15;
const MSIX_CTRL_MASK_ALL: u16 = 1 << 14;

// MSI-X Table Entry (16 bytes each, in MMIO)
const MSIX_ENTRY_ADDR_LO: usize = 0x00;
const MSIX_ENTRY_ADDR_HI: usize = 0x04;
const MSIX_ENTRY_DATA: usize = 0x08;
const MSIX_ENTRY_CONTROL: usize = 0x0C;
const MSIX_ENTRY_MASKED: u32 = 1 << 0;

// ── MSI Message Address/Data format (x86 LAPIC) ────────────────────────────

/// MSI message address for x86: targets LAPIC at 0xFEE00000.
/// Bits [19:12] = Destination APIC ID, bit 3 = RH (redirection hint),
/// bit 2 = DM (destination mode: 0=physical, 1=logical).
fn msi_address(apic_id: u8) -> u32 {
    0xFEE0_0000 | ((apic_id as u32) << 12)
}

/// MSI message data: vector number in bits [7:0], delivery mode in [10:8].
/// Delivery mode 000 = Fixed, trigger mode edge (bit 15=0, bit 14=0).
fn msi_data(vector: u8) -> u32 {
    vector as u32 // Fixed delivery, edge triggered
}

// ── MSI Vector Allocator ────────────────────────────────────────────────────

/// MSI vectors start after the LAPIC/IPI block (48-55).
/// The upper bound matches the IDT gates + asm stubs defined in
/// `kernel/asm/x86/interrupts.asm` and `kernel/src/arch/x86/idt.rs`; vectors
/// beyond that range have no IDT entry and would fault with #GP on delivery.
/// Bump all three when adding more slots.
pub const MSI_VECTOR_BASE: u8 = 56;
pub const MSI_VECTOR_MAX: u8 = 87;
const MSI_VECTOR_COUNT: usize = (MSI_VECTOR_MAX - MSI_VECTOR_BASE + 1) as usize;

/// Bitmap tracking allocated MSI vectors (4 × 64 = 256 bits, more than enough).
static MSI_ALLOC_BITMAP: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

/// Handler table for MSI vectors. Index = vector - MSI_VECTOR_BASE.
static MSI_HANDLERS: [AtomicU64; MSI_VECTOR_COUNT] = {
    const NULL: AtomicU64 = AtomicU64::new(0);
    // SAFETY: MSI_VECTOR_COUNT is const, array init with const element
    // We need a workaround since Rust doesn't allow [AtomicU64::new(0); N] for large N
    // Use a simpler approach with a fixed-size array
    [NULL; MSI_VECTOR_COUNT]
};

/// Allocate a free MSI vector. Returns None if all vectors are exhausted.
pub fn alloc_vector() -> Option<u8> {
    for i in 0..MSI_VECTOR_COUNT {
        let word = i / 64;
        let bit = i % 64;
        let mask = 1u64 << bit;
        let old = MSI_ALLOC_BITMAP[word].fetch_or(mask, Ordering::AcqRel);
        if old & mask == 0 {
            // Successfully claimed this bit
            return Some(MSI_VECTOR_BASE + i as u8);
        }
    }
    None
}

/// Free a previously allocated MSI vector.
pub fn free_vector(vector: u8) {
    if vector < MSI_VECTOR_BASE || vector > MSI_VECTOR_MAX {
        return;
    }
    let i = (vector - MSI_VECTOR_BASE) as usize;
    let word = i / 64;
    let bit = i % 64;
    MSI_ALLOC_BITMAP[word].fetch_and(!(1u64 << bit), Ordering::Release);
    MSI_HANDLERS[i].store(0, Ordering::Release);
}

/// Register an IRQ handler for an MSI vector.
pub fn register_msi_handler(vector: u8, handler: fn(u8)) {
    if vector < MSI_VECTOR_BASE || vector > MSI_VECTOR_MAX {
        return;
    }
    let i = (vector - MSI_VECTOR_BASE) as usize;
    if i < MSI_HANDLERS.len() {
        MSI_HANDLERS[i].store(handler as u64, Ordering::Release);
    }
}

/// Dispatch an MSI interrupt. Called from the IDT handler for vectors >= MSI_VECTOR_BASE.
/// Returns true if a handler was found.
pub fn dispatch_msi(vector: u8) -> bool {
    if vector < MSI_VECTOR_BASE {
        return false;
    }
    let i = (vector - MSI_VECTOR_BASE) as usize;
    if i >= MSI_HANDLERS.len() {
        return false;
    }
    let handler_ptr = MSI_HANDLERS[i].load(Ordering::Acquire);
    if handler_ptr != 0 {
        let func: fn(u8) = unsafe { core::mem::transmute(handler_ptr) };
        func(vector);
        true
    } else {
        false
    }
}

// ── PCI Capability List Walking ─────────────────────────────────────────────

/// Information about a device's MSI capability.
#[derive(Debug, Clone, Copy)]
pub struct MsiCapability {
    /// Offset of the MSI capability in PCI config space.
    pub offset: u8,
    /// True if 64-bit addressing is supported.
    pub is_64bit: bool,
    /// True if per-vector masking is supported.
    pub per_vector_mask: bool,
    /// Maximum number of vectors the device supports (1, 2, 4, 8, 16, or 32).
    pub max_vectors: u8,
}

/// Information about a device's MSI-X capability.
#[derive(Debug, Clone, Copy)]
pub struct MsixCapability {
    /// Offset of the MSI-X capability in PCI config space.
    pub offset: u8,
    /// Number of table entries (1-2048).
    pub table_size: u16,
    /// BAR index for the MSI-X table.
    pub table_bir: u8,
    /// Offset within the BAR for the MSI-X table.
    pub table_offset: u32,
    /// BAR index for the Pending Bit Array.
    pub pba_bir: u8,
    /// Offset within the BAR for the PBA.
    pub pba_offset: u32,
}

/// Walk the PCI capability list and find MSI/MSI-X capabilities.
pub fn find_msi_cap(dev: &PciDevice) -> Option<MsiCapability> {
    let status = pci_config_read16(dev.bus, dev.device, dev.function, 0x06);
    if status & (1 << 4) == 0 {
        return None; // No capabilities list
    }

    let mut cap_ptr = pci_config_read8(dev.bus, dev.device, dev.function, 0x34) & 0xFC;
    while cap_ptr != 0 {
        let cap_id = pci_config_read8(dev.bus, dev.device, dev.function, cap_ptr);
        if cap_id == PCI_CAP_MSI {
            let control =
                pci_config_read16(dev.bus, dev.device, dev.function, cap_ptr + MSI_CONTROL);
            let is_64bit = control & MSI_CTRL_64BIT != 0;
            let per_vector_mask = control & MSI_CTRL_PERVECTOR_MASK != 0;
            let mmc = ((control >> 1) & 0x07) as u8; // Multiple Message Capable
            let max_vectors = 1u8 << mmc;

            return Some(MsiCapability {
                offset: cap_ptr,
                is_64bit,
                per_vector_mask,
                max_vectors,
            });
        }
        cap_ptr = pci_config_read8(dev.bus, dev.device, dev.function, cap_ptr + 1) & 0xFC;
    }
    None
}

/// Read 8-bit PCI config register (convenience wrapper).
fn pci_config_read8(bus: u8, device: u8, function: u8, offset: u8) -> u8 {
    crate::drivers::pci::pci_config_read8(bus, device, function, offset)
}

/// Walk the PCI capability list and find MSI-X capability.
pub fn find_msix_cap(dev: &PciDevice) -> Option<MsixCapability> {
    let status = pci_config_read16(dev.bus, dev.device, dev.function, 0x06);
    if status & (1 << 4) == 0 {
        return None;
    }

    let mut cap_ptr = pci_config_read8(dev.bus, dev.device, dev.function, 0x34) & 0xFC;
    while cap_ptr != 0 {
        let cap_id = pci_config_read8(dev.bus, dev.device, dev.function, cap_ptr);
        if cap_id == PCI_CAP_MSIX {
            let control =
                pci_config_read16(dev.bus, dev.device, dev.function, cap_ptr + MSIX_CONTROL);
            let table_size = (control & 0x07FF) + 1;

            let table_raw = pci_config_read32(
                dev.bus,
                dev.device,
                dev.function,
                cap_ptr + MSIX_TABLE_OFFSET,
            );
            let table_bir = (table_raw & 0x07) as u8;
            let table_offset = table_raw & !0x07;

            let pba_raw =
                pci_config_read32(dev.bus, dev.device, dev.function, cap_ptr + MSIX_PBA_OFFSET);
            let pba_bir = (pba_raw & 0x07) as u8;
            let pba_offset = pba_raw & !0x07;

            return Some(MsixCapability {
                offset: cap_ptr,
                table_size,
                table_bir,
                table_offset,
                pba_bir,
                pba_offset,
            });
        }
        cap_ptr = pci_config_read8(dev.bus, dev.device, dev.function, cap_ptr + 1) & 0xFC;
    }
    None
}

// ── MSI Configuration ───────────────────────────────────────────────────────

/// Enable MSI for a device. Allocates a vector and programs the MSI registers.
/// Returns the allocated vector number, or None on failure.
pub fn enable_msi(dev: &PciDevice) -> Option<u8> {
    let cap = find_msi_cap(dev)?;
    let vector = alloc_vector()?;
    let apic_id = crate::arch::x86::apic::lapic_id();

    let addr = msi_address(apic_id);
    let data = msi_data(vector);

    // Write message address
    pci_config_write32(
        dev.bus,
        dev.device,
        dev.function,
        cap.offset + MSI_ADDR_LO,
        addr,
    );

    if cap.is_64bit {
        // Upper address = 0 (LAPIC is in low 4 GiB)
        pci_config_write32(
            dev.bus,
            dev.device,
            dev.function,
            cap.offset + MSI_ADDR_HI,
            0,
        );
        // Write message data
        pci_config_write32(
            dev.bus,
            dev.device,
            dev.function,
            cap.offset + MSI_DATA_64,
            data,
        );
    } else {
        pci_config_write32(
            dev.bus,
            dev.device,
            dev.function,
            cap.offset + MSI_DATA_32,
            data,
        );
    }

    // Enable MSI: set enable bit, request 1 vector (MME = 000)
    let mut control =
        pci_config_read16(dev.bus, dev.device, dev.function, cap.offset + MSI_CONTROL);
    control |= MSI_CTRL_ENABLE;
    control &= !(0x07 << 4); // Clear MME (Multiple Message Enable) = 1 vector
                             // Write control as part of a 32-bit write (cap_id + next + control)
    let cap_header = pci_config_read32(dev.bus, dev.device, dev.function, cap.offset);
    let new_header = (cap_header & 0x0000_FFFF) | ((control as u32) << 16);
    pci_config_write32(dev.bus, dev.device, dev.function, cap.offset, new_header);

    // Disable legacy INTx (PCI Command register bit 10)
    let cmd = pci_config_read16(dev.bus, dev.device, dev.function, 0x04);
    let new_cmd = cmd | (1 << 10); // Set Interrupt Disable bit
    pci_config_write32(
        dev.bus,
        dev.device,
        dev.function,
        0x04,
        (pci_config_read32(dev.bus, dev.device, dev.function, 0x04) & 0xFFFF_0000) | new_cmd as u32,
    );

    crate::serial_println!(
        "  PCI MSI: {}:{}.{} → vector {} (addr={:#010x} data={:#06x})",
        dev.bus,
        dev.device,
        dev.function,
        vector,
        addr,
        data
    );

    Some(vector)
}

/// Disable MSI for a device. Frees the allocated vector.
pub fn disable_msi(dev: &PciDevice, vector: u8) {
    if let Some(cap) = find_msi_cap(dev) {
        let cap_header = pci_config_read32(dev.bus, dev.device, dev.function, cap.offset);
        let control = (cap_header >> 16) as u16;
        let new_control = control & !MSI_CTRL_ENABLE;
        let new_header = (cap_header & 0x0000_FFFF) | ((new_control as u32) << 16);
        pci_config_write32(dev.bus, dev.device, dev.function, cap.offset, new_header);
    }
    free_vector(vector);
}

// ── PCIe ECAM (Extended Configuration Access Mechanism) ─────────────────────

/// ECAM base address discovered from the MCFG ACPI table.
static ECAM_BASE: AtomicU64 = AtomicU64::new(0);
/// Bus range covered by ECAM (start_bus in low 8 bits, end_bus in high 8 bits).
static ECAM_BUS_RANGE: AtomicU64 = AtomicU64::new(0);
/// Virtual base address where ECAM is mapped.
static ECAM_VIRT_BASE: AtomicU64 = AtomicU64::new(0);

/// ECAM virtual mapping area (after other MMIO regions).
const ECAM_MAP_VIRT: u64 = 0xFFFF_FFFF_E000_0000;

/// Set the ECAM base address (called from ACPI MCFG parsing).
pub fn set_ecam_base(phys_base: u64, start_bus: u8, end_bus: u8) {
    ECAM_BASE.store(phys_base, Ordering::Release);
    ECAM_BUS_RANGE.store(
        (start_bus as u64) | ((end_bus as u64) << 8),
        Ordering::Release,
    );

    // Map the ECAM region: each bus needs 256 KiB (32 devices × 8 functions × 4 KiB)
    let bus_count = (end_bus as u64 - start_bus as u64 + 1) as usize;
    let total_bytes = bus_count * 256 * 1024; // 256 KiB per bus
    let total_pages = (total_bytes + 0xFFF) / 0x1000;

    // Limit mapping to avoid excessive page table usage
    let max_pages = 4096; // 16 MiB max (covers 64 buses)
    let pages_to_map = total_pages.min(max_pages);

    use crate::memory::address::{PhysAddr, VirtAddr};
    use crate::memory::virtual_mem;
    for i in 0..pages_to_map {
        let virt = ECAM_MAP_VIRT + (i as u64) * 0x1000;
        let phys = phys_base + (i as u64) * 0x1000;
        virtual_mem::map_page(
            VirtAddr::new(virt),
            PhysAddr::new(phys),
            0x03, // PAGE_PRESENT | PAGE_WRITE
        );
    }

    ECAM_VIRT_BASE.store(ECAM_MAP_VIRT, Ordering::Release);
    crate::serial_println!(
        "  PCIe ECAM: phys={:#014x} buses {}-{} ({} pages mapped)",
        phys_base,
        start_bus,
        end_bus,
        pages_to_map
    );
}

/// Read from PCIe extended config space (offsets 0-4095) via ECAM.
/// Falls back to legacy I/O port access for offsets 0-255 if ECAM is not available.
pub fn pcie_config_read32(bus: u8, device: u8, function: u8, offset: u16) -> u32 {
    if offset < 256 {
        // Legacy access works for the first 256 bytes
        return pci_config_read32(bus, device, function, offset as u8);
    }

    let virt_base = ECAM_VIRT_BASE.load(Ordering::Acquire);
    if virt_base == 0 {
        return 0xFFFFFFFF; // ECAM not available
    }

    let bus_range = ECAM_BUS_RANGE.load(Ordering::Acquire);
    let start_bus = (bus_range & 0xFF) as u8;

    let addr = virt_base
        + ((bus - start_bus) as u64) * 256 * 1024  // 256 KiB per bus
        + (device as u64) * 8 * 4096               // 4 KiB per function, 8 functions
        + (function as u64) * 4096                  // 4 KiB per function
        + (offset as u64);

    unsafe { core::ptr::read_volatile(addr as *const u32) }
}

/// Write to PCIe extended config space via ECAM.
pub fn pcie_config_write32(bus: u8, device: u8, function: u8, offset: u16, value: u32) {
    if offset < 256 {
        pci_config_write32(bus, device, function, offset as u8, value);
        return;
    }

    let virt_base = ECAM_VIRT_BASE.load(Ordering::Acquire);
    if virt_base == 0 {
        return;
    }

    let bus_range = ECAM_BUS_RANGE.load(Ordering::Acquire);
    let start_bus = (bus_range & 0xFF) as u8;

    let addr = virt_base
        + ((bus - start_bus) as u64) * 256 * 1024
        + (device as u64) * 8 * 4096
        + (function as u64) * 4096
        + (offset as u64);

    unsafe {
        core::ptr::write_volatile(addr as *mut u32, value);
    }
}

/// Check if ECAM is available.
pub fn ecam_available() -> bool {
    ECAM_VIRT_BASE.load(Ordering::Acquire) != 0
}
