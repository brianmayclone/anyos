//! DLL bump allocator using per-process `.bss` statics.
//!
//! With DLIB v3, each process gets its own `.bss` section (zeroed on demand
//! by the kernel). The heap position and end pointers live here as normal
//! statics — no DLL state page needed.
//!
//! Memory is obtained from the process heap via `SYS_SBRK`. This coexists
//! with stdlib's allocator — when sbrk returns non-contiguous memory (because
//! the other allocator moved the break), we detect the gap and start fresh
//! from the sbrk return value.

use core::alloc::{GlobalAlloc, Layout};

use crate::syscall;

/// Heap state — lives in per-process .bss (zero-initialized per process).
static mut HEAP_POS: u64 = 0;
static mut HEAP_END: u64 = 0;

#[global_allocator]
static ALLOCATOR: DllBumpAlloc = DllBumpAlloc;

/// Zero-sized bump allocator — state lives in per-process .bss statics.
struct DllBumpAlloc;

unsafe impl GlobalAlloc for DllBumpAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let mut pos = HEAP_POS;

        // First allocation: initialize from current sbrk position
        if pos == 0 {
            let brk = syscall::sbrk(0) as u64;
            if brk == u64::MAX {
                return core::ptr::null_mut();
            }
            HEAP_POS = brk;
            HEAP_END = brk;
            pos = brk;
        }

        let align = layout.align() as u64;
        let size = layout.size() as u64;

        // Align current position
        let aligned = (pos + align - 1) & !(align - 1);
        let new_pos = aligned + size;

        // Grow the heap via sbrk if needed
        let end = HEAP_END;
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
                HEAP_END = end + grow;
            } else {
                // Non-contiguous: another allocator owns [end, result)
                // Start fresh from the sbrk return value
                HEAP_END = result + grow;
                let aligned = (result + align - 1) & !(align - 1);
                let new_pos = aligned + size;
                HEAP_POS = new_pos;
                return aligned as *mut u8;
            }
        }

        HEAP_POS = new_pos;
        aligned as *mut u8
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {
        // Bump allocator: no deallocation. Memory reclaimed on process exit.
    }
}
