use crate::memory::FRAME_SIZE;

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PhysAddr(pub u32);

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct VirtAddr(pub u32);

impl PhysAddr {
    pub const fn new(addr: u32) -> Self {
        PhysAddr(addr)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }

    pub fn frame_index(self) -> usize {
        (self.0 as usize) / FRAME_SIZE
    }

    pub fn is_frame_aligned(self) -> bool {
        self.0 % FRAME_SIZE as u32 == 0
    }

    pub fn frame_align_down(self) -> Self {
        PhysAddr(self.0 & !(FRAME_SIZE as u32 - 1))
    }

    pub fn frame_align_up(self) -> Self {
        PhysAddr((self.0 + FRAME_SIZE as u32 - 1) & !(FRAME_SIZE as u32 - 1))
    }
}

impl VirtAddr {
    pub const fn new(addr: u32) -> Self {
        VirtAddr(addr)
    }

    pub const fn as_u32(self) -> u32 {
        self.0
    }

    pub fn page_directory_index(self) -> usize {
        ((self.0 >> 22) & 0x3FF) as usize
    }

    pub fn page_table_index(self) -> usize {
        ((self.0 >> 12) & 0x3FF) as usize
    }

    pub fn page_offset(self) -> usize {
        (self.0 & 0xFFF) as usize
    }

    pub fn is_page_aligned(self) -> bool {
        self.0 % FRAME_SIZE as u32 == 0
    }
}
