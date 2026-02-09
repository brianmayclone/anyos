//! Kernel heap allocator using a linked-list free list.
//!
//! Provides a `GlobalAlloc` implementation backed by demand-paged virtual memory.
//! Starts with 16 MiB and grows on demand up to 512 MiB, allocating physical
//! frames and mapping them into the kernel's virtual address space.

use crate::memory::address::VirtAddr;
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::memory::FRAME_SIZE;
use core::alloc::{GlobalAlloc, Layout};

/// Virtual address where the kernel heap begins (higher-half, after kernel code area).
const HEAP_START: u64 = 0xFFFF_FFFF_8040_0000;
/// Initial heap size mapped at boot (16 MiB).
const HEAP_INITIAL_SIZE: usize = 16 * 1024 * 1024;
/// Maximum heap size (512 MiB) — fits within the 1 GiB PML4[511]/PDPT[510] window.
const HEAP_MAX_SIZE: usize = 512 * 1024 * 1024;
/// Minimum growth increment when expanding the heap (1 MiB).
const GROW_CHUNK: usize = 1024 * 1024;

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap = LockedHeap::new();

/// Global kernel heap allocator protected by an atomic spinlock.
struct LockedHeap {
    lock: core::sync::atomic::AtomicBool,
}

/// Header for a free block in the linked-list free list.
///
/// Stored in-place at the start of each free region. Blocks are kept
/// sorted by address to enable coalescing on deallocation.
#[repr(C)]
struct FreeBlock {
    /// Total size of this free block in bytes (including the header).
    size: usize,
    /// Pointer to the next free block, or null if this is the last.
    next: *mut FreeBlock,
}

static mut HEAP_FREE_LIST: *mut FreeBlock = core::ptr::null_mut();
static mut HEAP_INITIALIZED: bool = false;
static mut HEAP_COMMITTED: usize = 0; // Bytes actually mapped

impl LockedHeap {
    const fn new() -> Self {
        LockedHeap {
            lock: core::sync::atomic::AtomicBool::new(false),
        }
    }

    fn acquire(&self) {
        while self
            .lock
            .compare_exchange_weak(
                false,
                true,
                core::sync::atomic::Ordering::Acquire,
                core::sync::atomic::Ordering::Relaxed,
            )
            .is_err()
        {
            core::hint::spin_loop();
        }
    }

    fn release(&self) {
        self.lock
            .store(false, core::sync::atomic::Ordering::Release);
    }
}

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !HEAP_INITIALIZED {
            return core::ptr::null_mut();
        }

        self.acquire();
        let mut result = alloc_inner(layout);

        // If allocation failed, try growing the heap and retry
        if result.is_null() {
            let needed = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), layout.align().max(16));
            if grow_heap(needed) {
                result = alloc_inner(layout);
            }
        }

        self.release();
        result
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.acquire();
        dealloc_inner(ptr, layout);
        self.release();
    }
}

unsafe fn alloc_inner(layout: Layout) -> *mut u8 {
    let size = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), layout.align().max(16));

    // First-fit search
    let mut prev: *mut FreeBlock = core::ptr::null_mut();
    let mut current = HEAP_FREE_LIST;

    while !current.is_null() {
        let block_size = (*current).size;

        if block_size >= size {
            // Found a fitting block
            if block_size >= size + core::mem::size_of::<FreeBlock>() + 8 {
                // Split the block
                let new_block = (current as *mut u8).add(size) as *mut FreeBlock;
                (*new_block).size = block_size - size;
                (*new_block).next = (*current).next;

                if prev.is_null() {
                    HEAP_FREE_LIST = new_block;
                } else {
                    (*prev).next = new_block;
                }
            } else {
                // Use the entire block
                if prev.is_null() {
                    HEAP_FREE_LIST = (*current).next;
                } else {
                    (*prev).next = (*current).next;
                }
            }

            return current as *mut u8;
        }

        prev = current;
        current = (*current).next;
    }

    core::ptr::null_mut()
}

/// Grow the heap by mapping additional physical pages.
/// Called while the heap lock is held. Returns true if growth succeeded.
unsafe fn grow_heap(min_bytes: usize) -> bool {
    // Compute growth amount: at least min_bytes, rounded up to GROW_CHUNK
    let growth = align_up(min_bytes.max(GROW_CHUNK), FRAME_SIZE);

    // Check limits
    let new_committed = HEAP_COMMITTED + growth;
    if new_committed > HEAP_MAX_SIZE {
        // Try to grow as much as we can
        let remaining = HEAP_MAX_SIZE.saturating_sub(HEAP_COMMITTED);
        if remaining < min_bytes {
            return false; // Can't grow enough
        }
        return grow_heap_exact(remaining);
    }

    // Check physical memory availability (keep 256 frames = 1 MiB reserve)
    let pages_needed = growth / FRAME_SIZE;
    if physical::free_frames() < pages_needed + 256 {
        // Try smaller growth
        let available = physical::free_frames().saturating_sub(256);
        if available * FRAME_SIZE < min_bytes {
            return false;
        }
        return grow_heap_exact(available * FRAME_SIZE);
    }

    grow_heap_exact(growth)
}

/// Map exactly `growth` bytes of new heap pages and add to free list.
unsafe fn grow_heap_exact(growth: usize) -> bool {
    let growth = align_up(growth, FRAME_SIZE);
    if growth == 0 {
        return false;
    }

    let pages = growth / FRAME_SIZE;
    let base = HEAP_START as usize + HEAP_COMMITTED;

    // Map new pages
    for i in 0..pages {
        let virt = VirtAddr::new((base + i * FRAME_SIZE) as u64);
        match physical::alloc_frame() {
            Some(phys) => virtual_mem::map_page(virt, phys, 0x03), // Present + Writable
            None => {
                // Out of physical memory — unmap what we already mapped
                // (in practice this shouldn't happen since we checked free_frames)
                return false;
            }
        }
    }

    // Add the new region as a free block, inserted into the sorted free list
    let new_block = base as *mut FreeBlock;
    (*new_block).size = growth;

    // Insert at correct position in sorted free list (by address)
    let mut prev: *mut FreeBlock = core::ptr::null_mut();
    let mut current = HEAP_FREE_LIST;
    while !current.is_null() && (current as usize) < base {
        prev = current;
        current = (*current).next;
    }

    (*new_block).next = current;
    if prev.is_null() {
        HEAP_FREE_LIST = new_block;
    } else {
        (*prev).next = new_block;
    }

    // Coalesce with previous block if adjacent
    if !prev.is_null() {
        if (prev as *mut u8).add((*prev).size) == new_block as *mut u8 {
            (*prev).size += (*new_block).size;
            (*prev).next = (*new_block).next;
            // new_block is now part of prev; check if we can also coalesce with next
            if !(*prev).next.is_null() {
                let next = (*prev).next;
                if (prev as *mut u8).add((*prev).size) == next as *mut u8 {
                    (*prev).size += (*next).size;
                    (*prev).next = (*next).next;
                }
            }
        } else {
            // Try coalesce new_block with next
            if !(*new_block).next.is_null() {
                let next = (*new_block).next;
                if (new_block as *mut u8).add((*new_block).size) == next as *mut u8 {
                    (*new_block).size += (*next).size;
                    (*new_block).next = (*next).next;
                }
            }
        }
    } else {
        // new_block is the head; try coalesce with next
        if !(*new_block).next.is_null() {
            let next = (*new_block).next;
            if (new_block as *mut u8).add((*new_block).size) == next as *mut u8 {
                (*new_block).size += (*next).size;
                (*new_block).next = (*next).next;
            }
        }
    }

    HEAP_COMMITTED = HEAP_COMMITTED + growth;

    crate::serial_println!(
        "  Heap grew: {} KiB committed ({} MiB max)",
        HEAP_COMMITTED / 1024,
        HEAP_MAX_SIZE / (1024 * 1024)
    );

    true
}

unsafe fn dealloc_inner(ptr: *mut u8, layout: Layout) {
    let size = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), layout.align().max(16));

    let block = ptr as *mut FreeBlock;
    (*block).size = size;

    // Insert sorted by address for coalescing
    let mut prev: *mut FreeBlock = core::ptr::null_mut();
    let mut current = HEAP_FREE_LIST;

    while !current.is_null() && (current as usize) < (block as usize) {
        prev = current;
        current = (*current).next;
    }

    (*block).next = current;

    if prev.is_null() {
        HEAP_FREE_LIST = block;
    } else {
        (*prev).next = block;
    }

    // Try to coalesce with next block
    if !(*block).next.is_null() {
        let next = (*block).next;
        if (block as *mut u8).add((*block).size) == next as *mut u8 {
            (*block).size += (*next).size;
            (*block).next = (*next).next;
        }
    }

    // Try to coalesce with previous block
    if !prev.is_null() {
        if (prev as *mut u8).add((*prev).size) == block as *mut u8 {
            (*prev).size += (*block).size;
            (*prev).next = (*block).next;
        }
    }
}

fn align_up(value: usize, align: usize) -> usize {
    (value + align - 1) & !(align - 1)
}

/// Returns (used_bytes, total_committed_bytes) for the kernel heap.
pub fn heap_stats() -> (usize, usize) {
    unsafe {
        let committed = HEAP_COMMITTED;
        let mut total_free = 0usize;
        let mut current = HEAP_FREE_LIST;
        while !current.is_null() {
            total_free += (*current).size;
            current = (*current).next;
        }
        (committed.saturating_sub(total_free), committed)
    }
}

/// Walk the free list and validate heap integrity. Prints results to serial.
pub fn validate_heap() {
    unsafe {
        let mut current = HEAP_FREE_LIST;
        let mut prev_end: usize = 0;
        let mut total_free = 0usize;
        let mut count = 0usize;
        let heap_start = HEAP_START as usize;
        let heap_end = heap_start + HEAP_COMMITTED;

        while !current.is_null() {
            let addr = current as usize;
            let size = (*current).size;

            if addr < heap_start || addr >= heap_end {
                crate::serial_println!("HEAP CORRUPT: block #{} at {:#x} outside heap bounds [{:#x}..{:#x}]",
                    count, addr, heap_start, heap_end);
                return;
            }
            if size == 0 || addr + size > heap_end {
                crate::serial_println!("HEAP CORRUPT: block #{} at {:#x} size {:#x} extends past heap end {:#x}",
                    count, addr, size, heap_end);
                return;
            }
            if addr < prev_end {
                crate::serial_println!("HEAP CORRUPT: block #{} at {:#x} overlaps previous ending at {:#x}",
                    count, addr, prev_end);
                return;
            }

            total_free += size;
            prev_end = addr + size;
            count += 1;
            current = (*current).next;

            if count > 10000 {
                crate::serial_println!("HEAP CORRUPT: free list has >10000 entries (loop?)");
                return;
            }
        }

        crate::serial_println!("  Heap check: {} free block(s), {} KiB free / {} KiB committed",
            count, total_free / 1024, HEAP_COMMITTED / 1024);
    }
}

/// Initialize the kernel heap by mapping physical frames and creating the initial free list.
///
/// Must be called after physical and virtual memory are initialized.
pub fn init() {
    let pages = HEAP_INITIAL_SIZE / FRAME_SIZE;

    // Map heap pages
    for i in 0..pages {
        let virt = VirtAddr::new(HEAP_START + (i * FRAME_SIZE) as u64);
        let phys = physical::alloc_frame().expect("Failed to allocate heap frame");
        virtual_mem::map_page(virt, phys, 0x03); // Present + Writable
    }

    // Initialize free list with one big block
    unsafe {
        let block = HEAP_START as *mut FreeBlock;
        (*block).size = HEAP_INITIAL_SIZE;
        (*block).next = core::ptr::null_mut();
        HEAP_FREE_LIST = block;
        HEAP_COMMITTED = HEAP_INITIAL_SIZE;
        HEAP_INITIALIZED = true;
    }

    crate::serial_println!(
        "Kernel heap initialized: {:#018x} - {:#018x} ({} KiB, max {} MiB)",
        HEAP_START,
        HEAP_START + HEAP_INITIAL_SIZE as u64,
        HEAP_INITIAL_SIZE / 1024,
        HEAP_MAX_SIZE / (1024 * 1024)
    );
}
