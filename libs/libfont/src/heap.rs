//! DLL free-list allocator using per-process `.bss` statics.
//!
//! DLL allocators share the sbrk address space with stdlib. We call
//! sbrk(0) + sbrk(n) for each new allocation to get fresh addresses
//! that don't overlap with stdlib. Freed blocks go into a free list
//! for reuse with coalescing (via libheap).

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use crate::syscall;
use libheap::{FreeBlock, block_size, free_list_alloc, free_list_dealloc};

#[global_allocator]
static ALLOCATOR: DllFreeListAlloc = DllFreeListAlloc;

struct DllFreeListAlloc;

static mut FREE_LIST: *mut FreeBlock = ptr::null_mut();

unsafe impl GlobalAlloc for DllFreeListAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = block_size(layout);

        // 1) Search free list for first fit (reuse freed memory)
        let ptr = free_list_alloc(&mut FREE_LIST, size);
        if !ptr.is_null() { return ptr; }

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
        free_list_dealloc(&mut FREE_LIST, ptr, block_size(layout));
    }
}
