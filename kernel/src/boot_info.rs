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
    pub _padding: [u8; 2],
    pub kernel_phys_start: u32,
    pub kernel_phys_end: u32,
}

pub const BOOT_INFO_MAGIC: u32 = 0x414E594F; // "ANYO"

#[repr(C, packed)]
#[derive(Copy, Clone)]
pub struct E820Entry {
    pub base_addr: u64,
    pub length: u64,
    pub entry_type: u32,
    pub acpi_extended: u32,
}

pub const E820_TYPE_USABLE: u32 = 1;
pub const E820_TYPE_RESERVED: u32 = 2;
pub const E820_TYPE_ACPI_RECLAIMABLE: u32 = 3;

impl BootInfo {
    pub fn validate(&self) -> bool {
        self.magic == BOOT_INFO_MAGIC
    }

    pub unsafe fn memory_map(&self) -> &[E820Entry] {
        let addr = self.memory_map_addr;
        let count = self.memory_map_count;
        core::slice::from_raw_parts(addr as *const E820Entry, count as usize)
    }
}
