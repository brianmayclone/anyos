//! Shared free-list heap primitives for anyOS user-space allocators.
//!
//! Provides the core free-list search (first-fit with splitting) and
//! dealloc (sorted insert with coalescing) logic used by all user-space
//! allocators: stdlib (custom impl with mmap fallback) and all DLLs
//! (via the [`dll_allocator!`] macro).
//!
//! # DLL Allocator Macro
//!
//! DLLs should use the [`dll_allocator!`] macro to define their
//! `#[global_allocator]`. It generates a free-list allocator with mmap
//! fallback. The syscall module must export:
//! - `sbrk(u32) -> u64` (returns `u64::MAX` on failure)
//! - `mmap(u32) -> u64` (returns `u64::MAX` on failure)
//! - `munmap(u64, u32) -> u64` (returns 0 on success)
//!
//! ```ignore
//! libheap::dll_allocator!(crate::syscall::sbrk, crate::syscall::mmap, crate::syscall::munmap);
//! ```

#![no_std]

use core::alloc::Layout;
use core::ptr;

/// Free block node — stored in-place in freed memory.
/// Blocks are sorted by address to enable coalescing.
#[repr(C)]
pub struct FreeBlock {
    pub size: usize,
    pub next: *mut FreeBlock,
}

/// Minimum block size (must fit a FreeBlock node = 16 bytes on 64-bit).
pub const MIN_BLOCK: usize = 16;

/// Round `value` up to the next multiple of `align`.
#[inline]
pub fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

/// Compute the actual block size for a layout (same formula for alloc and dealloc).
#[inline]
pub fn block_size(layout: Layout) -> usize {
    align_up(layout.size().max(MIN_BLOCK), layout.align().max(16))
}

/// Search the free list for a first-fit block of at least `size` bytes.
///
/// If a suitable block is found, it is removed from the free list (with optional
/// splitting if the remainder is large enough) and returned as a pointer.
/// Returns null if no suitable block exists.
///
/// # Safety
/// `free_list` must point to a valid free list head pointer.
#[inline]
pub unsafe fn free_list_alloc(free_list: *mut *mut FreeBlock, size: usize) -> *mut u8 {
    let mut prev: *mut FreeBlock = ptr::null_mut();
    let mut curr = *free_list;
    while !curr.is_null() {
        if (*curr).size >= size {
            let remaining = (*curr).size - size;
            if remaining >= MIN_BLOCK + 8 {
                // Split: allocate from front, new free block after
                let new_free = (curr as *mut u8).add(size) as *mut FreeBlock;
                (*new_free).size = remaining;
                (*new_free).next = (*curr).next;
                if prev.is_null() { *free_list = new_free; } else { (*prev).next = new_free; }
            } else {
                // Use entire block
                if prev.is_null() { *free_list = (*curr).next; } else { (*prev).next = (*curr).next; }
            }
            return curr as *mut u8;
        }
        prev = curr;
        curr = (*curr).next;
    }
    ptr::null_mut()
}

/// Insert a freed block into the sorted free list with coalescing.
///
/// The block is inserted at its address-sorted position. Adjacent blocks
/// are merged (coalesced) to reduce fragmentation.
///
/// # Safety
/// `free_list` must point to a valid free list head pointer.
/// `ptr` must point to a previously allocated block of `size` bytes.
#[inline]
pub unsafe fn free_list_dealloc(free_list: *mut *mut FreeBlock, ptr: *mut u8, size: usize) {
    if ptr.is_null() { return; }

    let block = ptr as *mut FreeBlock;
    (*block).size = size;

    // Insert sorted by address for coalescing
    let mut prev: *mut FreeBlock = ptr::null_mut();
    let mut curr = *free_list;
    while !curr.is_null() && (curr as usize) < (block as usize) {
        prev = curr;
        curr = (*curr).next;
    }

    (*block).next = curr;
    if prev.is_null() { *free_list = block; } else { (*prev).next = block; }

    // Coalesce with next
    if !curr.is_null() && (block as *mut u8).add((*block).size) == curr as *mut u8 {
        (*block).size += (*curr).size;
        (*block).next = (*curr).next;
    }
    // Coalesce with prev
    if !prev.is_null() && (prev as *mut u8).add((*prev).size) == block as *mut u8 {
        (*prev).size += (*block).size;
        (*prev).next = (*block).next;
    }
}

/// Define a `#[global_allocator]` for a DLL with sbrk + mmap fallback.
///
/// Generates a private module with a `DllFreeListAlloc` struct implementing
/// `GlobalAlloc`. Small allocations use sbrk with a free list; large
/// allocations (>= 64 KiB) go directly through mmap. When sbrk fails,
/// any allocation transparently falls back to mmap.
///
/// # Arguments
///
/// - `$sbrk` — `fn(u32) -> u64`, returns `u64::MAX` on failure
/// - `$mmap` — `fn(u32) -> u64`, returns `u64::MAX` on failure
/// - `$munmap` — `fn(u64, u32) -> u64`, returns 0 on success
///
/// # Example
///
/// ```ignore
/// libheap::dll_allocator!(crate::syscall::sbrk, crate::syscall::mmap, crate::syscall::munmap);
/// ```
#[macro_export]
macro_rules! dll_allocator {
    ($sbrk:path, $mmap:path, $munmap:path) => {
        mod _dll_heap {
            use core::alloc::{GlobalAlloc, Layout};
            use core::ptr;

            struct DllFreeListAlloc;

            static mut FREE_LIST: *mut $crate::FreeBlock = ptr::null_mut();

            /// Allocations >= this size bypass sbrk and go directly through mmap.
            const MMAP_THRESHOLD: usize = 64 * 1024;

            /// Start of the mmap virtual address region.
            const MMAP_REGION_START: u64 = 0x7000_0000;
            /// End of the mmap virtual address region.
            const MMAP_REGION_END: u64 = 0xBF00_0000;

            /// Round `size` up to the next page boundary (4 KiB).
            #[inline]
            fn page_align(size: usize) -> usize {
                (size + 4095) & !4095
            }

            /// Check whether a pointer falls into the mmap region.
            #[inline]
            fn is_mmap_ptr(ptr: *mut u8) -> bool {
                let addr = ptr as u64;
                addr >= MMAP_REGION_START && addr < MMAP_REGION_END
            }

            /// Allocate via mmap. Returns null on failure.
            #[inline]
            fn mmap_alloc(size: usize) -> *mut u8 {
                let mapped_size = page_align(size) as u32;
                let mmap_fn: fn(u32) -> u64 = $mmap;
                let addr = mmap_fn(mapped_size);
                if addr == u64::MAX { return ptr::null_mut(); }
                if addr < MMAP_REGION_START || addr >= MMAP_REGION_END {
                    let munmap_fn: fn(u64, u32) -> u64 = $munmap;
                    munmap_fn(addr, mapped_size);
                    return ptr::null_mut();
                }
                addr as *mut u8
            }

            unsafe impl GlobalAlloc for DllFreeListAlloc {
                unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                    let size = $crate::block_size(layout);

                    // Large allocations go directly through mmap.
                    if size >= MMAP_THRESHOLD {
                        return mmap_alloc(size);
                    }

                    // 1) Search free list for first fit.
                    let ptr = $crate::free_list_alloc(&mut FREE_LIST, size);
                    if !ptr.is_null() { return ptr; }

                    // 2) Try sbrk.
                    let sbrk_fn: fn(u32) -> u64 = $sbrk;
                    let brk = sbrk_fn(0);
                    if brk != u64::MAX {
                        let align = layout.align().max(16) as u64;
                        let aligned = (brk + align - 1) & !(align - 1);
                        let needed = (aligned - brk + size as u64) as u32;
                        let result = sbrk_fn(needed);
                        if result != u64::MAX {
                            return aligned as *mut u8;
                        }
                    }

                    // 3) sbrk failed — fall back to mmap.
                    mmap_alloc(size)
                }

                unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
                    if ptr.is_null() { return; }

                    let size = $crate::block_size(layout);

                    // mmap allocations are returned directly to the OS.
                    if is_mmap_ptr(ptr) {
                        let mapped_size = page_align(size) as u32;
                        let munmap_fn: fn(u64, u32) -> u64 = $munmap;
                        munmap_fn(ptr as u64, mapped_size);
                        return;
                    }

                    // sbrk allocations go back to the free list.
                    $crate::free_list_dealloc(
                        &mut FREE_LIST,
                        ptr,
                        size,
                    );
                }
            }

            #[global_allocator]
            static ALLOCATOR: DllFreeListAlloc = DllFreeListAlloc;
        }
    };
}
