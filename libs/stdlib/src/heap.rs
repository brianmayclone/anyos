//! User-space heap allocator with free-list and coalescing.
//!
//! Uses a sorted linked-list free list (via libheap) with the same algorithm
//! as the kernel heap. Memory is obtained from the kernel via `sbrk()`.
//! Freed blocks are inserted back into the free list sorted by address
//! and coalesced with neighbors.
//!
//! The `GlobalAlloc` trait provides the `Layout` on dealloc, so no per-block
//! header is needed — the block size is recomputed from the layout.

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, Ordering};
use core::ptr;

use libheap::{FreeBlock, block_size, free_list_alloc, free_list_dealloc};

#[global_allocator]
static ALLOCATOR: FreeListAlloc = FreeListAlloc;

struct FreeListAlloc;

/// Spinlock protecting all heap state.
static HEAP_LOCK: AtomicBool = AtomicBool::new(false);

/// Next sbrk allocation position (grows upward).
static mut HEAP_POS: u64 = 0;
/// Current end of mapped heap pages (kernel break).
static mut HEAP_END: u64 = 0;

/// Head of the sorted free list.
static mut FREE_LIST: *mut FreeBlock = ptr::null_mut();

/// Initialize the heap allocator. Must be called before any allocation.
pub fn init() {
    let brk = crate::process::sbrk(0) as u64;
    unsafe {
        HEAP_POS = brk;
        HEAP_END = brk;
    }
}

#[inline]
fn lock() {
    while HEAP_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
}

#[inline]
fn unlock() {
    HEAP_LOCK.store(false, Ordering::Release);
}

unsafe impl GlobalAlloc for FreeListAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = block_size(layout);

        lock();

        // 1) Search free list for first fit
        let ptr = free_list_alloc(&mut FREE_LIST, size);
        if !ptr.is_null() {
            unlock();
            return ptr;
        }

        // 2) No free block found — allocate from sbrk
        let align = layout.align().max(16) as u64;
        let aligned = (HEAP_POS + align - 1) & !(align - 1);
        let new_pos = aligned + size as u64;

        if new_pos > HEAP_END {
            let grow = ((new_pos - HEAP_END + 4095) & !4095) as usize;
            let result = crate::process::sbrk(grow as i32);
            if result == u32::MAX as usize {
                unlock();
                return ptr::null_mut();
            }
            let result = result as u64;
            let grow = grow as u64;

            if result == HEAP_END {
                // Contiguous extension
                HEAP_END += grow;
            } else {
                // Non-contiguous: another allocator moved the break
                HEAP_END = result + grow;
                let aligned = (result + align - 1) & !(align - 1);
                let new_pos = aligned + size as u64;
                HEAP_POS = new_pos;
                unlock();
                return aligned as *mut u8;
            }
        }

        HEAP_POS = new_pos;
        unlock();
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() {
            return;
        }

        lock();
        free_list_dealloc(&mut FREE_LIST, ptr, block_size(layout));
        unlock();
    }
}
