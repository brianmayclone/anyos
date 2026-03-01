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
//! `#[global_allocator]`. It generates a complete sbrk-based free-list
//! allocator. The syscall module must export `sbrk(u32) -> u64`.
//!
//! ```ignore
//! libheap::dll_allocator!(crate::syscall::sbrk);
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

/// Define a `#[global_allocator]` for a DLL using sbrk-based free-list allocation.
///
/// Generates a private module with a `DllFreeListAlloc` struct implementing
/// `GlobalAlloc`. Freed blocks are reused via a sorted free list with coalescing.
/// Fresh memory is obtained via `sbrk(0)` + `sbrk(n)`.
///
/// # Arguments
///
/// `$sbrk` — path to a function with signature `fn(u32) -> u64`.
/// Must return `u64::MAX` on failure. Typically `crate::syscall::sbrk`.
///
/// # Example
///
/// ```ignore
/// libheap::dll_allocator!(crate::syscall::sbrk);
/// ```
#[macro_export]
macro_rules! dll_allocator {
    ($sbrk:path) => {
        mod _dll_heap {
            use core::alloc::{GlobalAlloc, Layout};
            use core::ptr;

            struct DllFreeListAlloc;

            static mut FREE_LIST: *mut $crate::FreeBlock = ptr::null_mut();

            unsafe impl GlobalAlloc for DllFreeListAlloc {
                unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                    let size = $crate::block_size(layout);

                    let ptr = $crate::free_list_alloc(&mut FREE_LIST, size);
                    if !ptr.is_null() { return ptr; }

                    let sbrk_fn: fn(u32) -> u64 = $sbrk;
                    let brk = sbrk_fn(0);
                    if brk == u64::MAX { return ptr::null_mut(); }
                    let align = layout.align().max(16) as u64;
                    let aligned = (brk + align - 1) & !(align - 1);
                    let needed = (aligned - brk + size as u64) as u32;
                    let result = sbrk_fn(needed);
                    if result == u64::MAX { return ptr::null_mut(); }
                    aligned as *mut u8
                }

                unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
                    $crate::free_list_dealloc(
                        &mut FREE_LIST,
                        ptr,
                        $crate::block_size(layout),
                    );
                }
            }

            #[global_allocator]
            static ALLOCATOR: DllFreeListAlloc = DllFreeListAlloc;
        }
    };
}
