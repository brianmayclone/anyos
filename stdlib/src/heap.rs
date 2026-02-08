use core::alloc::{GlobalAlloc, Layout};

#[global_allocator]
static ALLOCATOR: BumpAlloc = BumpAlloc;

struct BumpAlloc;

/// Next allocation position (grows upward).
static mut HEAP_POS: u32 = 0;
/// Current end of mapped heap pages (kernel break).
static mut HEAP_END: u32 = 0;

/// Initialize the heap allocator. Must be called before any allocation.
pub fn init() {
    let brk = crate::process::sbrk(0);
    unsafe {
        HEAP_POS = brk;
        HEAP_END = brk;
    }
}

unsafe impl GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align() as u32;
        let size = layout.size() as u32;

        // Align current position
        let aligned = (HEAP_POS + align - 1) & !(align - 1);
        let new_pos = aligned + size;

        // Grow the heap via sbrk if needed
        if new_pos > HEAP_END {
            let needed = new_pos - HEAP_END;
            // Round up to page size (4 KiB) for efficiency
            let grow = (needed + 4095) & !4095;
            let result = crate::process::sbrk(grow as i32);
            if result == u32::MAX {
                return core::ptr::null_mut();
            }
            HEAP_END += grow;
        }

        HEAP_POS = new_pos;
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator: individual frees are no-ops.
        // All memory is reclaimed when the process exits.
    }
}
