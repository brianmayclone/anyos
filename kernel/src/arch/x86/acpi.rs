/// ACPI table parsing — finds LAPIC, I/O APIC, and processor information.
///
/// Searches for the RSDP signature, parses RSDT, and extracts the MADT
/// (Multiple APIC Description Table) to enumerate processors and APICs.

use alloc::vec::Vec;

/// Information about a processor (Local APIC entry from MADT)
#[derive(Debug, Clone, Copy)]
pub struct ProcessorInfo {
    pub acpi_id: u8,
    pub apic_id: u8,
    pub enabled: bool,
}

/// Information about an I/O APIC (from MADT)
#[derive(Debug, Clone, Copy)]
pub struct IoApicInfo {
    pub id: u8,
    pub address: u32,
    pub gsi_base: u32,
}

/// Interrupt Source Override (from MADT)
#[derive(Debug, Clone, Copy)]
pub struct IsoInfo {
    pub bus: u8,
    pub source: u8,       // ISA IRQ
    pub gsi: u32,          // Global System Interrupt
    pub flags: u16,
}

/// Parsed ACPI information
pub struct AcpiInfo {
    pub lapic_address: u32,
    pub processors: Vec<ProcessorInfo>,
    pub io_apics: Vec<IoApicInfo>,
    pub isos: Vec<IsoInfo>,
}

// RSDP signature: "RSD PTR "
const RSDP_SIGNATURE: [u8; 8] = *b"RSD PTR ";

#[repr(C, packed)]
struct Rsdp {
    signature: [u8; 8],
    checksum: u8,
    oem_id: [u8; 6],
    revision: u8,
    rsdt_address: u32,
}

#[repr(C, packed)]
struct AcpiSdtHeader {
    signature: [u8; 4],
    length: u32,
    revision: u8,
    checksum: u8,
    oem_id: [u8; 6],
    oem_table_id: [u8; 8],
    oem_revision: u32,
    creator_id: u32,
    creator_revision: u32,
}

// MADT entry types
const MADT_LAPIC: u8 = 0;
const MADT_IOAPIC: u8 = 1;
const MADT_ISO: u8 = 2;   // Interrupt Source Override
const MADT_LAPIC_NMI: u8 = 4;

/// Virtual address window for temporarily mapping ACPI tables.
const ACPI_MAP_BASE: u32 = 0xD020_0000;
/// Maximum ACPI table region we map (256 KiB = 64 pages)
const ACPI_MAP_PAGES: usize = 64;

/// Map a physical address range into the ACPI virtual window.
/// Returns the virtual address corresponding to `phys_addr`.
fn acpi_map(phys_addr: u32, size: u32) -> u32 {
    use crate::memory::address::{PhysAddr, VirtAddr};
    use crate::memory::virtual_mem;

    let page_start = phys_addr & !0xFFF;
    let page_end = (phys_addr + size + 0xFFF) & !0xFFF;
    let num_pages = ((page_end - page_start) / 0x1000) as usize;

    for i in 0..core::cmp::min(num_pages, ACPI_MAP_PAGES) {
        let virt = ACPI_MAP_BASE + (i as u32) * 0x1000;
        let phys = page_start + (i as u32) * 0x1000;
        virtual_mem::map_page(
            VirtAddr::new(virt),
            PhysAddr::new(phys),
            0x01, // PAGE_PRESENT (read-only is fine)
        );
    }

    // Return virtual address with proper offset within page
    ACPI_MAP_BASE + (phys_addr - page_start)
}

/// Unmap the ACPI virtual window.
fn acpi_unmap(num_pages: usize) {
    use crate::memory::address::VirtAddr;
    use crate::memory::virtual_mem;

    for i in 0..core::cmp::min(num_pages, ACPI_MAP_PAGES) {
        let virt = ACPI_MAP_BASE + (i as u32) * 0x1000;
        virtual_mem::unmap_page(VirtAddr::new(virt));
    }
}

/// Discover ACPI tables and parse MADT for SMP information.
pub fn init() -> Option<AcpiInfo> {
    let rsdp = find_rsdp()?;

    crate::serial_println!("  ACPI: RSDP found at {:#010x}, RSDT at {:#010x}",
        rsdp as *const _ as u32, unsafe { core::ptr::addr_of!((*rsdp).rsdt_address).read_unaligned() });

    let rsdt_phys = unsafe { core::ptr::addr_of!((*rsdp).rsdt_address).read_unaligned() };

    // Map the RSDT region (map 16 pages = 64 KiB to cover RSDT + nearby tables)
    let rsdt_virt = acpi_map(rsdt_phys, 0x10000);
    let rsdt = rsdt_virt as *const AcpiSdtHeader;

    // Verify RSDT signature
    let sig = unsafe { core::ptr::addr_of!((*rsdt).signature).read_unaligned() };
    if &sig != b"RSDT" {
        crate::serial_println!("  ACPI: RSDT signature mismatch");
        acpi_unmap(16);
        return None;
    }

    let rsdt_len = unsafe { core::ptr::addr_of!((*rsdt).length).read_unaligned() };
    let header_size = core::mem::size_of::<AcpiSdtHeader>() as u32;
    let num_entries = (rsdt_len - header_size) / 4;

    crate::serial_println!("  ACPI: RSDT has {} table entries", num_entries);

    // Walk RSDT entries to find MADT (signature "APIC")
    let entries_base = (rsdt_virt + header_size) as *const u32;

    for i in 0..num_entries {
        let table_phys = unsafe { entries_base.add(i as usize).read_unaligned() };

        // Check if this table is within our current mapping
        // If not, map it separately
        let table_page = table_phys & !0xFFF;
        let rsdt_page = rsdt_phys & !0xFFF;
        let table_virt = if table_page >= rsdt_page && table_page < rsdt_page + 0x10000 {
            // Table is within the RSDT mapping window
            ACPI_MAP_BASE + (table_phys - rsdt_page)
        } else {
            // Table is elsewhere — remap
            acpi_map(table_phys, 0x1000)
        };

        let table = table_virt as *const AcpiSdtHeader;
        let table_sig = unsafe { core::ptr::addr_of!((*table).signature).read_unaligned() };

        if &table_sig == b"APIC" {
            let table_len = unsafe { core::ptr::addr_of!((*table).length).read_unaligned() };
            crate::serial_println!("  ACPI: MADT found at phys {:#010x} (len={})", table_phys, table_len);

            // Remap to ensure the full MADT is accessible
            let madt_virt = acpi_map(table_phys, table_len);
            let result = parse_madt(madt_virt, table_len);
            acpi_unmap(ACPI_MAP_PAGES);
            return result;
        }
    }

    acpi_unmap(16);
    crate::serial_println!("  ACPI: MADT not found");
    None
}

/// Search for the RSDP structure.
/// Searches EBDA and the 0xE0000-0xFFFFF BIOS ROM area.
fn find_rsdp() -> Option<*const Rsdp> {
    // Search the main BIOS area (0x000E0000 - 0x000FFFFF)
    let start = 0x000E0000usize;
    let end = 0x00100000usize;

    let mut addr = start;
    while addr < end {
        let ptr = addr as *const [u8; 8];
        let sig = unsafe { ptr.read() };
        if sig == RSDP_SIGNATURE {
            let rsdp = addr as *const Rsdp;
            if validate_rsdp_checksum(rsdp) {
                return Some(rsdp);
            }
        }
        addr += 16; // RSDP is always 16-byte aligned
    }

    // Search EBDA (Extended BIOS Data Area)
    // EBDA segment is stored at 0x040E (real-mode segment)
    let ebda_seg = unsafe { (0x040E as *const u16).read() };
    let ebda_start = (ebda_seg as usize) << 4;
    if ebda_start > 0 && ebda_start < 0xA0000 {
        let ebda_end = core::cmp::min(ebda_start + 1024, 0xA0000);
        let mut addr = ebda_start;
        while addr < ebda_end {
            let ptr = addr as *const [u8; 8];
            let sig = unsafe { ptr.read() };
            if sig == RSDP_SIGNATURE {
                let rsdp = addr as *const Rsdp;
                if validate_rsdp_checksum(rsdp) {
                    return Some(rsdp);
                }
            }
            addr += 16;
        }
    }

    None
}

fn validate_rsdp_checksum(rsdp: *const Rsdp) -> bool {
    let bytes = rsdp as *const u8;
    let mut sum: u8 = 0;
    for i in 0..20 {
        sum = sum.wrapping_add(unsafe { bytes.add(i).read() });
    }
    sum == 0
}

/// Parse the MADT (Multiple APIC Description Table).
fn parse_madt(madt_virt: u32, table_len: u32) -> Option<AcpiInfo> {
    let header_size = core::mem::size_of::<AcpiSdtHeader>() as u32;

    // MADT has LAPIC address at offset 36 (header_size = 36 for SDT header)
    // Then flags at offset 40
    let lapic_address = unsafe { ((madt_virt + header_size) as *const u32).read_unaligned() };
    let _flags = unsafe { ((madt_virt + header_size + 4) as *const u32).read_unaligned() };

    crate::serial_println!("  ACPI: LAPIC address = {:#010x}", lapic_address);

    let mut processors = Vec::new();
    let mut io_apics = Vec::new();
    let mut isos = Vec::new();

    // Parse MADT entries starting at offset header_size + 8
    let entries_start = madt_virt + header_size + 8;
    let entries_end = madt_virt + table_len;
    let mut off = entries_start;

    while off + 2 <= entries_end {
        let entry_type = unsafe { (off as *const u8).read() };
        let entry_len = unsafe { ((off + 1) as *const u8).read() } as u32;

        if entry_len < 2 { break; }

        match entry_type {
            MADT_LAPIC => {
                if entry_len >= 8 {
                    let acpi_id = unsafe { ((off + 2) as *const u8).read() };
                    let apic_id = unsafe { ((off + 3) as *const u8).read() };
                    let flags = unsafe { ((off + 4) as *const u32).read_unaligned() };
                    let enabled = flags & 1 != 0;

                    processors.push(ProcessorInfo { acpi_id, apic_id, enabled });
                    crate::serial_println!("  ACPI: Processor #{} APIC_ID={} enabled={}",
                        acpi_id, apic_id, enabled);
                }
            }
            MADT_IOAPIC => {
                if entry_len >= 12 {
                    let id = unsafe { ((off + 2) as *const u8).read() };
                    let address = unsafe { ((off + 4) as *const u32).read_unaligned() };
                    let gsi_base = unsafe { ((off + 8) as *const u32).read_unaligned() };

                    io_apics.push(IoApicInfo { id, address, gsi_base });
                    crate::serial_println!("  ACPI: I/O APIC #{} at {:#010x} GSI_base={}",
                        id, address, gsi_base);
                }
            }
            MADT_ISO => {
                if entry_len >= 10 {
                    let bus = unsafe { ((off + 2) as *const u8).read() };
                    let source = unsafe { ((off + 3) as *const u8).read() };
                    let gsi = unsafe { ((off + 4) as *const u32).read_unaligned() };
                    let flags = unsafe { ((off + 8) as *const u16).read_unaligned() };

                    isos.push(IsoInfo { bus, source, gsi, flags });
                    crate::serial_println!("  ACPI: ISO IRQ{} -> GSI{} flags={:#x}",
                        source, gsi, flags);
                }
            }
            MADT_LAPIC_NMI => {
                // NMI routing — acknowledge but don't need to act on it for basic SMP
            }
            _ => {}
        }

        off += entry_len;
    }

    Some(AcpiInfo {
        lapic_address,
        processors,
        io_apics,
        isos,
    })
}
