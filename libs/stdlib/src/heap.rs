use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, Ordering};

#[global_allocator]
static ALLOCATOR: BumpAlloc = BumpAlloc;

struct BumpAlloc;

/// Spinlock protecting HEAP_POS and HEAP_END for thread safety.
static HEAP_LOCK: AtomicBool = AtomicBool::new(false);

/// Next allocation position (grows upward). Protected by HEAP_LOCK.
static mut HEAP_POS: u64 = 0;
/// Current end of mapped heap pages (kernel break). Protected by HEAP_LOCK.
static mut HEAP_END: u64 = 0;

/// Initialize the heap allocator. Must be called before any allocation.
pub fn init() {
    let brk = crate::process::sbrk(0) as u64;
    unsafe {
        HEAP_POS = brk;
        HEAP_END = brk;
    }
}

unsafe impl GlobalAlloc for BumpAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Acquire spinlock
        while HEAP_LOCK
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }

        let align = layout.align() as u64;
        let size = layout.size() as u64;

        // Align current position
        let aligned = (HEAP_POS + align - 1) & !(align - 1);
        let new_pos = aligned + size;

        // Grow the heap via sbrk if needed
        if new_pos > HEAP_END {
            let needed = new_pos - HEAP_END;
            // Round up to page size (4 KiB) for efficiency
            let grow = (needed + 4095) & !4095;
            let result = crate::process::sbrk(grow as i32);
            if result == u32::MAX as usize {
                HEAP_LOCK.store(false, Ordering::Release);
                return core::ptr::null_mut();
            }
            HEAP_END += grow;
        }

        HEAP_POS = new_pos;
        HEAP_LOCK.store(false, Ordering::Release);
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator: individual frees are no-ops.
        // All memory is reclaimed when the process exits.
    }
}
