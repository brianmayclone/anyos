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
            // Request enough for a full allocation in case sbrk returns
            // non-contiguous memory (another allocator moved the break)
            let grow = ((size + align + 4095) & !4095) as usize;
            let result = crate::process::sbrk(grow as i32);
            if result == u32::MAX as usize {
                HEAP_LOCK.store(false, Ordering::Release);
                return core::ptr::null_mut();
            }
            let result = result as u64;
            let grow = grow as u64;

            if result == HEAP_END {
                // Contiguous extension — original aligned/new_pos are valid
                HEAP_END += grow;
            } else {
                // Non-contiguous: another allocator (e.g. DLL) owns [HEAP_END, result)
                // Start fresh from the sbrk return value
                HEAP_END = result + grow;
                let aligned = (result + align - 1) & !(align - 1);
                let new_pos = aligned + size;
                HEAP_POS = new_pos;
                HEAP_LOCK.store(false, Ordering::Release);
                return aligned as *mut u8;
            }
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
