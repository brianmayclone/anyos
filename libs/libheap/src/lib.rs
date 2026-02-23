//! Shared free-list heap primitives for anyOS user-space allocators.
//!
//! Provides the core free-list search (first-fit with splitting) and
//! dealloc (sorted insert with coalescing) logic used by all three
//! user-space allocators: stdlib, libanyui, and libfont.
//!
//! Each consumer owns its own `FREE_LIST` static and `GlobalAlloc` impl.
//! This crate only provides the shared algorithm — no statics, no sbrk calls.

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
