use crate::memory::{address::PhysAddr, physical, physmap};

const PAGE_SIZE: u64 = 0x1000;

pub(super) fn alloc_filled_pages(count: usize, value: u8) -> Option<u64> {
    let phys = physical::alloc_contiguous(count)?;
    let ptr = match physmap::phys_to_virt(phys) {
        Some(ptr) => ptr,
        None => {
            free_pages(phys.as_u64(), count);
            return None;
        }
    };
    unsafe {
        core::ptr::write_bytes(ptr, value, count * PAGE_SIZE as usize);
    }
    Some(phys.as_u64())
}

pub(super) fn free_pages(base: u64, count: usize) {
    for i in 0..count {
        physical::free_frame(PhysAddr::new(base + (i as u64) * PAGE_SIZE));
    }
}
