//! User-space heap allocator with free-list, coalescing, and mmap fallback.
//!
//! Small allocations use `sbrk()` with a sorted linked-list free list (via
//! libheap).  When the sbrk region is exhausted, the allocator transparently
//! falls back to `mmap()` which draws from a separate 1.25 GiB virtual region
//! (0x70000000–0xBF000000).
//!
//! Large allocations (≥ MMAP_THRESHOLD) go directly through `mmap()` so they
//! can be returned to the OS on dealloc via `munmap()` without fragmenting
//! the sbrk heap.
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

/// Spinlock protecting all heap state (sbrk + free list).
static HEAP_LOCK: AtomicBool = AtomicBool::new(false);

/// Next sbrk allocation position (grows upward).
static mut HEAP_POS: u64 = 0;
/// Current end of mapped heap pages (kernel break).
static mut HEAP_END: u64 = 0;

/// Head of the sorted free list.
static mut FREE_LIST: *mut FreeBlock = ptr::null_mut();

/// Allocations ≥ this size bypass sbrk and go directly through mmap/munmap.
/// 64 KiB — matches typical OS large-allocation thresholds.
const MMAP_THRESHOLD: usize = 64 * 1024;

/// Start of the mmap virtual address region.
const MMAP_REGION_START: u64 = 0x7000_0000;
/// End of the mmap virtual address region.
const MMAP_REGION_END: u64 = 0xBF00_0000;

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

/// Round `size` up to the next page boundary (4 KiB).
#[inline]
fn page_align(size: usize) -> usize {
    (size + 4095) & !4095
}

/// Allocate via `mmap()`.  Returns null on failure.
///
/// The kernel returns zeroed, page-aligned memory from the mmap region.
#[inline]
fn mmap_alloc(size: usize) -> *mut u8 {
    let mapped_size = page_align(size);
    let ptr = crate::process::mmap(mapped_size);
    if ptr.is_null() {
        return ptr::null_mut();
    }
    // Verify the returned address is within the mmap region.
    let addr = ptr as u64;
    if addr < MMAP_REGION_START || addr >= MMAP_REGION_END {
        // Unexpected — free it and fall through.
        crate::process::munmap(ptr, mapped_size);
        return ptr::null_mut();
    }
    ptr
}

/// Check whether a pointer falls into the mmap region.
#[inline]
fn is_mmap_ptr(ptr: *mut u8) -> bool {
    let addr = ptr as u64;
    addr >= MMAP_REGION_START && addr < MMAP_REGION_END
}

unsafe impl GlobalAlloc for FreeListAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = block_size(layout);

        // Large allocations go directly through mmap (no free-list overhead,
        // and the pages are returned to the OS on dealloc).
        if size >= MMAP_THRESHOLD {
            return mmap_alloc(size);
        }

        lock();

        // 1) Search free list for first fit.
        let ptr = free_list_alloc(&mut FREE_LIST, size);
        if !ptr.is_null() {
            unlock();
            return ptr;
        }

        // 2) No free block found — try to grow via sbrk.
        let align = layout.align().max(16) as u64;
        let aligned = (HEAP_POS + align - 1) & !(align - 1);
        let new_pos = aligned + size as u64;

        if new_pos > HEAP_END {
            let grow = ((new_pos - HEAP_END + 4095) & !4095) as usize;
            let result = crate::process::sbrk(grow as i32);

            if result == u32::MAX as usize {
                // sbrk failed — fall back to mmap for this allocation.
                unlock();
                return mmap_alloc(size);
            }

            let result = result as u64;
            let grow = grow as u64;

            if result == HEAP_END {
                // Contiguous extension.
                HEAP_END += grow;
            } else {
                // Non-contiguous: another allocator (DLL) moved the break.
                // Relocate the allocation to start at `result`.
                let aligned = (result + align - 1) & !(align - 1);
                let new_pos = aligned + size as u64;
                let mapped_end = result + grow;

                // The original `grow` may not cover the full allocation
                // from the new position — request extra pages if needed.
                if new_pos > mapped_end {
                    let extra = ((new_pos - mapped_end + 4095) & !4095) as usize;
                    let r2 = crate::process::sbrk(extra as i32);
                    if r2 == u32::MAX as usize {
                        // sbrk ran out even for the extra — mmap fallback.
                        unlock();
                        return mmap_alloc(size);
                    }
                    HEAP_END = mapped_end + extra as u64;
                } else {
                    HEAP_END = mapped_end;
                }

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

        // Large allocations from mmap are returned directly to the OS.
        let size = block_size(layout);
        if is_mmap_ptr(ptr) {
            let mapped_size = page_align(size);
            crate::process::munmap(ptr, mapped_size);
            return;
        }

        // Small sbrk allocations go back to the free list.
        lock();
        free_list_dealloc(&mut FREE_LIST, ptr, size);
        unlock();
    }
}
