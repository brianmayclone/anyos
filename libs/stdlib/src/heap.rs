//! User-space heap allocator with free-list and coalescing.
//!
//! Uses a sorted linked-list free list (same algorithm as the kernel heap).
//! Memory is obtained from the kernel via `sbrk()`. Freed blocks are inserted
//! back into the free list sorted by address and coalesced with neighbors.
//!
//! The `GlobalAlloc` trait provides the `Layout` on dealloc, so no per-block
//! header is needed — the block size is recomputed from the layout.

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicBool, Ordering};
use core::ptr;

#[global_allocator]
static ALLOCATOR: FreeListAlloc = FreeListAlloc;

struct FreeListAlloc;

/// Spinlock protecting all heap state.
static HEAP_LOCK: AtomicBool = AtomicBool::new(false);

/// Next sbrk allocation position (grows upward).
static mut HEAP_POS: u64 = 0;
/// Current end of mapped heap pages (kernel break).
static mut HEAP_END: u64 = 0;

/// Free block node — stored in-place in freed memory.
/// Blocks are sorted by address to enable coalescing.
#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

/// Head of the sorted free list.
static mut FREE_LIST: *mut FreeBlock = ptr::null_mut();

/// Minimum block size (must fit a FreeBlock node).
const MIN_BLOCK: usize = 16; // size_of::<FreeBlock>() on 64-bit

/// Round `value` up to the next multiple of `align`.
#[inline]
fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

/// Compute the actual block size for a layout (same formula for alloc and dealloc).
#[inline]
fn block_size(layout: Layout) -> usize {
    align_up(layout.size().max(MIN_BLOCK), layout.align().max(16))
}

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
        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut curr = FREE_LIST;

        while !curr.is_null() {
            if (*curr).size >= size {
                let remaining = (*curr).size - size;

                if remaining >= MIN_BLOCK + 8 {
                    // Split: allocate from the front, free block moves after
                    let new_free = (curr as *mut u8).add(size) as *mut FreeBlock;
                    (*new_free).size = remaining;
                    (*new_free).next = (*curr).next;
                    if prev.is_null() {
                        FREE_LIST = new_free;
                    } else {
                        (*prev).next = new_free;
                    }
                } else {
                    // Use entire block
                    if prev.is_null() {
                        FREE_LIST = (*curr).next;
                    } else {
                        (*prev).next = (*curr).next;
                    }
                }

                unlock();
                return curr as *mut u8;
            }
            prev = curr;
            curr = (*curr).next;
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

        let size = block_size(layout);

        lock();

        let block = ptr as *mut FreeBlock;
        (*block).size = size;

        // Insert into sorted free list (sorted by address)
        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut curr = FREE_LIST;
        while !curr.is_null() && (curr as usize) < (block as usize) {
            prev = curr;
            curr = (*curr).next;
        }

        (*block).next = curr;
        if prev.is_null() {
            FREE_LIST = block;
        } else {
            (*prev).next = block;
        }

        // Coalesce with next block
        if !curr.is_null()
            && (block as *mut u8).add((*block).size) == curr as *mut u8
        {
            (*block).size += (*curr).size;
            (*block).next = (*curr).next;
        }

        // Coalesce with previous block
        if !prev.is_null()
            && (prev as *mut u8).add((*prev).size) == block as *mut u8
        {
            (*prev).size += (*block).size;
            (*prev).next = (*block).next;
        }

        unlock();
    }
}
