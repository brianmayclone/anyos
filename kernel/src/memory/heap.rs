//! Kernel heap allocator using a linked-list free list with demand paging.
//!
//! The heap reserves a 512 MiB virtual address range but only maps a small initial
//! region at boot. Growth just advances a committed watermark — physical frames are
//! allocated lazily by the page fault handler when memory is first accessed.
//!
//! The lock is IRQ-safe: interrupts are disabled while the heap lock is held.
//! This prevents deadlock when `reap_terminated()` frees a kernel stack from
//! within the timer ISR while the preempted thread was holding the heap lock.

use crate::memory::address::VirtAddr;
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::memory::FRAME_SIZE;
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Virtual address where the kernel heap begins.
///
/// Must be ABOVE the kernel's higher-half mapping (which covers physical 0-16 MiB
/// at virtual KERNEL_VIRT_BASE..KERNEL_VIRT_BASE+16MiB). Placed at 32 MiB offset
/// from KERNEL_VIRT_BASE to leave room for kernel code, data, BSS, and stack growth.
const HEAP_START: u64 = 0xFFFF_FFFF_8200_0000;
/// Size of the region pre-mapped at boot (4 MiB — enough for early init).
const HEAP_INITIAL_MAPPED: usize = 4 * 1024 * 1024;
/// Initial committed size (32 MiB — rest is demand-paged on first access).
const HEAP_INITIAL_SIZE: usize = 32 * 1024 * 1024;
/// Maximum heap size (512 MiB) — fits within the 1 GiB PML4[511]/PDPT[510] window.
const HEAP_MAX_SIZE: usize = 512 * 1024 * 1024;
/// Minimum growth increment when expanding the heap (4 MiB).
const GROW_CHUNK: usize = 4 * 1024 * 1024;

/// Committed heap size in bytes. Readable by the page fault handler without
/// acquiring the heap lock. Pages in [HEAP_START, HEAP_START + HEAP_COMMITTED)
/// are valid heap addresses — if not yet mapped, the page fault handler
/// allocates a frame on demand.
pub static HEAP_COMMITTED: AtomicUsize = AtomicUsize::new(0);

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap = LockedHeap::new();

/// Global kernel heap allocator protected by an IRQ-safe atomic spinlock.
///
/// Interrupts are disabled while the lock is held to prevent deadlock:
/// if a timer ISR fires while the heap lock is held, `reap_terminated()` could
/// try to free a kernel stack, re-entering the allocator and deadlocking.
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

impl LockedHeap {
    const fn new() -> Self {
        LockedHeap {
            lock: core::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Acquire the heap lock with interrupts disabled.
    /// Returns the saved RFLAGS so `release` can restore the interrupt state.
    fn acquire(&self) -> u64 {
        let flags: u64;
        unsafe {
            core::arch::asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags));
            core::arch::asm!("cli", options(nomem, nostack));
        }

        let mut spin_count: u32 = 0;
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
            spin_count += 1;
            if spin_count == 10_000_000 {
                // Probable deadlock — print via direct UART (bypasses all locks)
                unsafe {
                    use crate::arch::x86::port::{inb, outb};
                    let msg = b"\r\n!!! HEAP_LOCK TIMEOUT\r\n";
                    for &c in msg { while inb(0x3FD) & 0x20 == 0 {} outb(0x3F8, c); }
                }
            }
        }

        flags
    }

    /// Release the heap lock and restore the saved interrupt state.
    fn release(&self, flags: u64) {
        self.lock
            .store(false, core::sync::atomic::Ordering::Release);

        // Restore caller's interrupt state
        if flags & 0x200 != 0 {
            unsafe { core::arch::asm!("sti", options(nomem, nostack)); }
        }
    }
}

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !HEAP_INITIALIZED {
            return core::ptr::null_mut();
        }

        let flags = self.acquire();
        let mut result = alloc_inner(layout);

        // If allocation failed, try growing the heap and retry
        if result.is_null() {
            let needed = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), layout.align().max(16));
            if grow_heap(needed) {
                result = alloc_inner(layout);
            }
        }

        self.release(flags);
        result
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let flags = self.acquire();
        dealloc_inner(ptr, layout);
        self.release(flags);
    }
}

/// Check if the heap lock is currently held (lock-free diagnostic).
/// Used by the timer heartbeat to detect if the heap is part of a deadlock chain.
#[inline]
pub fn is_heap_locked() -> bool {
    HEAP_ALLOCATOR.lock.load(core::sync::atomic::Ordering::Relaxed)
}

/// Check if an address is within the committed heap range.
#[inline]
fn is_in_heap(addr: usize) -> bool {
    let heap_start = HEAP_START as usize;
    let heap_end = heap_start + HEAP_COMMITTED.load(Ordering::Relaxed);
    addr >= heap_start && addr < heap_end
}

unsafe fn alloc_inner(layout: Layout) -> *mut u8 {
    let size = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), layout.align().max(16));

    // First-fit search with cycle detection (max iteration guard).
    // A corrupted free list with cycles would loop forever under the heap lock
    // with IF=0, causing ALL CPUs to deadlock when they need allocations.
    const MAX_ITER: usize = 100_000;
    let mut prev: *mut FreeBlock = core::ptr::null_mut();
    let mut current = HEAP_FREE_LIST;
    let mut iter = 0usize;

    while !current.is_null() {
        iter += 1;
        if iter > MAX_ITER {
            return core::ptr::null_mut(); // Probable cycle — bail out
        }

        // Validate current pointer is within heap bounds
        if !is_in_heap(current as usize) {
            return core::ptr::null_mut();
        }

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

/// Grow the heap by advancing the committed watermark.
/// Physical frames are NOT allocated here — they are demand-paged on first access.
/// Called while the heap lock is held. Returns true if growth succeeded.
unsafe fn grow_heap(min_bytes: usize) -> bool {
    // Compute growth amount: at least min_bytes, rounded up to GROW_CHUNK
    let growth = align_up(min_bytes.max(GROW_CHUNK), FRAME_SIZE);

    let current_committed = HEAP_COMMITTED.load(Ordering::Acquire);

    // Check limits
    let new_committed = current_committed + growth;
    if new_committed > HEAP_MAX_SIZE {
        // Try to grow as much as we can
        let remaining = HEAP_MAX_SIZE.saturating_sub(current_committed);
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

/// Advance the committed watermark by `growth` bytes and add a free block.
/// No physical frame allocation happens here — pages are demand-faulted on access.
unsafe fn grow_heap_exact(growth: usize) -> bool {
    let growth = align_up(growth, FRAME_SIZE);
    if growth == 0 {
        return false;
    }

    let old_committed = HEAP_COMMITTED.load(Ordering::Acquire);
    let new_committed = old_committed + growth;

    // Advance the committed watermark (makes these addresses valid for demand paging)
    HEAP_COMMITTED.store(new_committed, Ordering::Release);

    let base = HEAP_START as usize + old_committed;

    // Insert the new region as a free block, inserted into the sorted free list.
    // Writing to `base` will trigger a demand page fault if the page isn't mapped yet —
    // the page fault handler (ISR 14) allocates a frame and maps it transparently.
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

    true
}

unsafe fn dealloc_inner(ptr: *mut u8, layout: Layout) {
    let size = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), layout.align().max(16));

    // Validate: pointer must be within heap bounds
    if !is_in_heap(ptr as usize) {
        return; // Leak instead of corrupting
    }

    let block = ptr as *mut FreeBlock;
    (*block).size = size;

    // Insert sorted by address for coalescing.
    // Max iteration guard prevents infinite loop on corrupted free list.
    const MAX_ITER: usize = 100_000;
    let mut prev: *mut FreeBlock = core::ptr::null_mut();
    let mut current = HEAP_FREE_LIST;
    let mut iter = 0usize;

    while !current.is_null() && (current as usize) < (block as usize) {
        iter += 1;
        if iter > MAX_ITER {
            // Probable cycle — insert at head to avoid infinite loop
            (*block).next = HEAP_FREE_LIST;
            HEAP_FREE_LIST = block;
            return;
        }

        // Validate current pointer
        if !is_in_heap(current as usize) {
            // Corruption detected — insert block at head to avoid walking further
            (*block).next = HEAP_FREE_LIST;
            HEAP_FREE_LIST = block;
            return;
        }

        // Double-free guard: check if block is already in the free list
        if current == block {
            return; // Skip the free entirely
        }

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
///
/// Acquires the heap lock to read the free list consistently.
pub fn heap_stats() -> (usize, usize) {
    unsafe {
        let flags = HEAP_ALLOCATOR.acquire();
        let committed = HEAP_COMMITTED.load(Ordering::Acquire);
        let mut total_free = 0usize;
        let mut current = HEAP_FREE_LIST;
        while !current.is_null() {
            if !is_in_heap(current as usize) {
                break; // Corrupt — stop walking
            }
            total_free += (*current).size;
            current = (*current).next;
        }
        HEAP_ALLOCATOR.release(flags);
        (committed.saturating_sub(total_free), committed)
    }
}

/// Walk the free list and validate heap integrity. Prints results to serial.
pub fn validate_heap() {
    unsafe {
        let flags = HEAP_ALLOCATOR.acquire();
        let mut current = HEAP_FREE_LIST;
        let mut prev_end: usize = 0;
        let mut total_free = 0usize;
        let mut count = 0usize;
        let heap_start = HEAP_START as usize;
        let heap_end = heap_start + HEAP_COMMITTED.load(Ordering::Acquire);

        while !current.is_null() {
            let addr = current as usize;
            let size = (*current).size;

            if addr < heap_start || addr >= heap_end {
                crate::serial_println!("HEAP CORRUPT: block #{} at {:#x} outside heap bounds [{:#x}..{:#x}]",
                    count, addr, heap_start, heap_end);
                HEAP_ALLOCATOR.release(flags);
                return;
            }
            if size == 0 || addr + size > heap_end {
                crate::serial_println!("HEAP CORRUPT: block #{} at {:#x} size {:#x} extends past heap end {:#x}",
                    count, addr, size, heap_end);
                HEAP_ALLOCATOR.release(flags);
                return;
            }
            if addr < prev_end {
                crate::serial_println!("HEAP CORRUPT: block #{} at {:#x} overlaps previous ending at {:#x}",
                    count, addr, prev_end);
                HEAP_ALLOCATOR.release(flags);
                return;
            }

            total_free += size;
            prev_end = addr + size;
            count += 1;
            current = (*current).next;

            if count > 10000 {
                crate::serial_println!("HEAP CORRUPT: free list has >10000 entries (loop?)");
                HEAP_ALLOCATOR.release(flags);
                return;
            }
        }

        crate::serial_println!("  Heap check: {} free block(s), {} KiB free / {} KiB committed",
            count, total_free / 1024, HEAP_COMMITTED.load(Ordering::Acquire) / 1024);
        HEAP_ALLOCATOR.release(flags);
    }
}

/// Initialize the kernel heap with demand paging.
///
/// Maps a small initial region (4 MiB) and commits a larger virtual range (32 MiB).
/// Pages beyond the initial mapped region are demand-faulted by the page fault handler.
/// Must be called after physical and virtual memory are initialized.
pub fn init() {
    // Map only the initial region (4 MiB = 1024 pages)
    let mapped_pages = HEAP_INITIAL_MAPPED / FRAME_SIZE;
    for i in 0..mapped_pages {
        let virt = VirtAddr::new(HEAP_START + (i * FRAME_SIZE) as u64);
        let phys = physical::alloc_frame().expect("Failed to allocate heap frame");
        virtual_mem::map_page(virt, phys, 0x03); // Present + Writable
    }

    // Commit the full initial size (rest will be demand-paged on access)
    HEAP_COMMITTED.store(HEAP_INITIAL_SIZE, Ordering::Release);

    // Initialize free list with one big block spanning HEAP_INITIAL_SIZE.
    // Only the first 4 MiB of this block is pre-mapped; the rest will be
    // demand-faulted when the allocator splits or traverses the block.
    unsafe {
        let block = HEAP_START as *mut FreeBlock;
        (*block).size = HEAP_INITIAL_SIZE;
        (*block).next = core::ptr::null_mut();
        HEAP_FREE_LIST = block;
        HEAP_INITIALIZED = true;
    }

    crate::serial_println!(
        "Kernel heap initialized: {:#018x} - {:#018x} ({} KiB mapped, {} KiB committed, max {} MiB)",
        HEAP_START,
        HEAP_START + HEAP_INITIAL_SIZE as u64,
        HEAP_INITIAL_MAPPED / 1024,
        HEAP_INITIAL_SIZE / 1024,
        HEAP_MAX_SIZE / (1024 * 1024)
    );
}
