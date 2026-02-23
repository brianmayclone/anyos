//! DLL free-list allocator using per-process `.bss` statics.
//!
//! DLL allocators share the sbrk address space with stdlib. We call
//! sbrk(0) + sbrk(n) for each new allocation to get fresh addresses
//! that don't overlap with stdlib. Freed blocks go into a free list
//! for reuse with coalescing.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use crate::syscall;

#[global_allocator]
static ALLOCATOR: DllFreeListAlloc = DllFreeListAlloc;

struct DllFreeListAlloc;

#[repr(C)]
struct FreeBlock {
    size: usize,
    next: *mut FreeBlock,
}

static mut FREE_LIST: *mut FreeBlock = ptr::null_mut();

const MIN_BLOCK: usize = 16;

#[inline]
fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

#[inline]
fn block_size(layout: Layout) -> usize {
    align_up(layout.size().max(MIN_BLOCK), layout.align().max(16))
}

unsafe impl GlobalAlloc for DllFreeListAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = block_size(layout);

        // 1) Search free list for first fit (reuse freed memory)
        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut curr = FREE_LIST;
        while !curr.is_null() {
            if (*curr).size >= size {
                let remaining = (*curr).size - size;
                if remaining >= MIN_BLOCK + 8 {
                    let new_free = (curr as *mut u8).add(size) as *mut FreeBlock;
                    (*new_free).size = remaining;
                    (*new_free).next = (*curr).next;
                    if prev.is_null() { FREE_LIST = new_free; } else { (*prev).next = new_free; }
                } else {
                    if prev.is_null() { FREE_LIST = (*curr).next; } else { (*prev).next = (*curr).next; }
                }
                return curr as *mut u8;
            }
            prev = curr;
            curr = (*curr).next;
        }

        // 2) No free block — get fresh memory from sbrk.
        //    Must call sbrk(0) each time to get the CURRENT break,
        //    since stdlib's allocator may have moved it.
        let brk = syscall::sbrk(0) as u64;
        if brk == u64::MAX { return ptr::null_mut(); }
        let align = layout.align().max(16) as u64;
        let aligned = (brk + align - 1) & !(align - 1);
        let needed = (aligned - brk + size as u64) as i32;
        let result = syscall::sbrk(needed);
        if result == usize::MAX { return ptr::null_mut(); }
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() { return; }
        let size = block_size(layout);

        let block = ptr as *mut FreeBlock;
        (*block).size = size;

        // Insert sorted by address for coalescing
        let mut prev: *mut FreeBlock = ptr::null_mut();
        let mut curr = FREE_LIST;
        while !curr.is_null() && (curr as usize) < (block as usize) {
            prev = curr;
            curr = (*curr).next;
        }

        (*block).next = curr;
        if prev.is_null() { FREE_LIST = block; } else { (*prev).next = block; }

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
}
