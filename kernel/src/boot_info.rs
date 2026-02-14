//! Boot information passed from the stage 2 bootloader.
//!
//! Contains the E820 memory map, framebuffer parameters, and kernel
//! physical address range populated by the bootloader before entering
//! protected mode.

/// Information block passed from the stage 2 bootloader at a known physical address.
///
/// All fields are populated before the jump to `kernel_main`. The struct
/// is validated via a magic signature ([`BOOT_INFO_MAGIC`]).
#[repr(C, packed)]
pub struct BootInfo {
    pub magic: u32,
    pub memory_map_addr: u32,
    pub memory_map_count: u32,
    pub framebuffer_addr: u32,
    pub framebuffer_pitch: u32,
    pub framebuffer_width: u32,
    pub framebuffer_height: u32,
    pub framebuffer_bpp: u8,
    pub boot_drive: u8,
    /// Boot mode: 0 = Legacy BIOS, 1 = UEFI.
    pub boot_mode: u8,
    pub _padding: u8,
    pub kernel_phys_start: u32,
    pub kernel_phys_end: u32,
    /// Physical address of the ACPI RSDP (set by UEFI bootloader, 0 for BIOS).
    pub rsdp_addr: u32,
}

/// Magic value (`"ANYO"` in ASCII) used to validate the boot info struct.
pub const BOOT_INFO_MAGIC: u32 = 0x414E594F; // "ANYO"

/// A single entry from the BIOS INT 15h, AX=E820h memory map.
#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct E820Entry {
    pub base_addr: u64,
    pub length: u64,
    pub entry_type: u32,
    pub acpi_extended: u32,
}

/// E820 memory type: usable RAM.
pub const E820_TYPE_USABLE: u32 = 1;
/// E820 memory type: reserved by firmware.
pub const E820_TYPE_RESERVED: u32 = 2;
/// E820 memory type: ACPI reclaimable (can be freed after parsing ACPI tables).
pub const E820_TYPE_ACPI_RECLAIMABLE: u32 = 3;

impl BootInfo {
    /// Returns `true` if the magic field matches [`BOOT_INFO_MAGIC`].
    pub fn validate(&self) -> bool {
        self.magic == BOOT_INFO_MAGIC
    }

    /// Returns the E820 memory map as a slice.
    ///
    /// # Safety
    /// The caller must ensure `memory_map_addr` points to valid, mapped memory
    /// containing `memory_map_count` entries.
    pub unsafe fn memory_map(&self) -> &[E820Entry] {
        let addr = self.memory_map_addr;
        let count = self.memory_map_count;
        core::slice::from_raw_parts(addr as *const E820Entry, count as usize)
    }
}
