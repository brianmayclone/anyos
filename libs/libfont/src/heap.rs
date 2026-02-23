//! DLL free-list allocator using per-process `.bss` statics.
//!
//! With DLIB v3, each process gets its own `.bss` section (zeroed on demand
//! by the kernel). The heap position and end pointers live here as normal
//! statics — no DLL state page needed.
//!
//! Memory is obtained from the process heap via `SYS_SBRK`. This coexists
//! with stdlib's allocator — when sbrk returns non-contiguous memory (because
//! the other allocator moved the break), we detect the gap and start fresh
//! from the sbrk return value.
//!
//! Freed blocks are inserted into a sorted free list with coalescing.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use crate::syscall;

/// Heap state — lives in per-process .bss (zero-initialized per process).
static mut HEAP_POS: u64 = 0;
static mut HEAP_END: u64 = 0;

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
        let mut pos = HEAP_POS;

        // First allocation: initialize from current sbrk position
        if pos == 0 {
            let brk = syscall::sbrk(0) as u64;
            if brk == u64::MAX {
                return ptr::null_mut();
            }
            HEAP_POS = brk;
            HEAP_END = brk;
            pos = brk;
        }

        // 1) Search free list for first fit
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

        // 2) Allocate from sbrk
        let align = layout.align().max(16) as u64;
        let aligned = (pos + align - 1) & !(align - 1);
        let new_pos = aligned + size as u64;

        let end = HEAP_END;
        if new_pos > end {
            let grow = ((size as u64 + align + 4095) & !4095) as usize;
            let result = syscall::sbrk(grow as i32);
            if result == usize::MAX {
                return ptr::null_mut();
            }
            let result = result as u64;
            let grow = grow as u64;

            if result == end {
                HEAP_END = end + grow;
            } else {
                // Non-contiguous: another allocator owns [end, result)
                HEAP_END = result + grow;
                let aligned = (result + align - 1) & !(align - 1);
                let new_pos = aligned + size as u64;
                HEAP_POS = new_pos;
                return aligned as *mut u8;
            }
        }

        HEAP_POS = new_pos;
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if ptr.is_null() { return; }
        let size = block_size(layout);

        let block = ptr as *mut FreeBlock;
        (*block).size = size;

        // Insert sorted by address
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
