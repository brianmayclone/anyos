//! DLL bump allocator using the per-process DLL state page.
//!
//! The kernel maps a zeroed physical frame at `DLL_STATE_PAGE` (0x0BFE_0000)
//! for each user process. This allocator stores its heap_pos and heap_end
//! at fixed offsets in that page, avoiding the need for `.data`/`.bss` sections.
//!
//! Memory is obtained from the process heap via `SYS_SBRK`. This coexists
//! with stdlib's allocator — when sbrk returns non-contiguous memory (because
//! the other allocator moved the break), we detect the gap and start fresh
//! from the sbrk return value.

use core::alloc::{GlobalAlloc, Layout};

use crate::syscall;

/// Per-process DLL state page virtual address (must match kernel constant).
const DLL_STATE_PAGE: usize = 0x0BFE_0000;

// Layout within the state page (libfont uses bytes 0-23):
//   [0x00] u64: FontManager pointer (0 = not initialized)
//   [0x08] u64: heap_pos (next allocation address)
//   [0x10] u64: heap_end (current mapped heap boundary)
const HEAP_POS_OFFSET: usize = 0x08;
const HEAP_END_OFFSET: usize = 0x10;

#[global_allocator]
static ALLOCATOR: DllBumpAlloc = DllBumpAlloc;

/// Zero-sized bump allocator — all state lives in the DLL state page.
struct DllBumpAlloc;

unsafe impl GlobalAlloc for DllBumpAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pos_ptr = (DLL_STATE_PAGE + HEAP_POS_OFFSET) as *mut u64;
        let end_ptr = (DLL_STATE_PAGE + HEAP_END_OFFSET) as *mut u64;

        let mut pos = *pos_ptr;

        // First allocation: initialize from current sbrk position
        if pos == 0 {
            let brk = syscall::sbrk(0) as u64;
            if brk == u64::MAX {
                return core::ptr::null_mut();
            }
            *pos_ptr = brk;
            *end_ptr = brk;
            pos = brk;
        }

        let align = layout.align() as u64;
        let size = layout.size() as u64;

        // Align current position
        let aligned = (pos + align - 1) & !(align - 1);
        let new_pos = aligned + size;

        // Grow the heap via sbrk if needed
        let end = *end_ptr;
        if new_pos > end {
            // Request enough for a full allocation in case sbrk returns
            // non-contiguous memory (another allocator moved the break)
            let grow = ((size + align + 4095) & !4095) as usize;
            let result = syscall::sbrk(grow as i32);
            if result == usize::MAX {
                return core::ptr::null_mut();
            }
            let result = result as u64;
            let grow = grow as u64;

            if result == end {
                // Contiguous extension — original aligned/new_pos are valid
                *end_ptr = end + grow;
            } else {
                // Non-contiguous: another allocator owns [end, result)
                // Start fresh from the sbrk return value
                *end_ptr = result + grow;
                let aligned = (result + align - 1) & !(align - 1);
                let new_pos = aligned + size;
                *pos_ptr = new_pos;
                return aligned as *mut u8;
            }
        }

        *pos_ptr = new_pos;
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator: no deallocation. Memory reclaimed on process exit.
    }
}
