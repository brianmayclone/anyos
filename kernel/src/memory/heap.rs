use crate::memory::address::VirtAddr;
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::memory::FRAME_SIZE;
use core::alloc::{GlobalAlloc, Layout};

// Kernel heap: starting at virtual 0xC0400000
const HEAP_START: u32 = 0xC040_0000;
const HEAP_INITIAL_SIZE: usize = 16 * 1024 * 1024; // 16 MiB
const HEAP_MAX_SIZE: usize = 32 * 1024 * 1024;     // 32 MiB max

#[global_allocator]
static HEAP_ALLOCATOR: LockedHeap = LockedHeap::new();

struct LockedHeap {
    // Simple spinlock around the heap state
    lock: core::sync::atomic::AtomicBool,
}

// Heap free block header
#[repr(C)]
struct FreeBlock {
    size: usize,
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
        let result = alloc_inner(layout);
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
    let size = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), layout.align().max(8));

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

unsafe fn dealloc_inner(ptr: *mut u8, layout: Layout) {
    let size = align_up(layout.size().max(core::mem::size_of::<FreeBlock>()), layout.align().max(8));

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

/// Walk the free list and validate heap integrity. Prints results to serial.
pub fn validate_heap() {
    unsafe {
        let mut current = HEAP_FREE_LIST;
        let mut prev_end: usize = 0;
        let mut total_free = 0usize;
        let mut count = 0usize;
        let heap_start = HEAP_START as usize;
        let heap_end = heap_start + HEAP_INITIAL_SIZE;

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

        crate::serial_println!("  Heap check: {} free block(s), {} KiB free / {} KiB total",
            count, total_free / 1024, HEAP_INITIAL_SIZE / 1024);
    }
}

pub fn init() {
    let pages = HEAP_INITIAL_SIZE / FRAME_SIZE;

    // Map heap pages
    for i in 0..pages {
        let virt = VirtAddr::new(HEAP_START + (i * FRAME_SIZE) as u32);
        let phys = physical::alloc_frame().expect("Failed to allocate heap frame");
        virtual_mem::map_page(virt, phys, 0x03); // Present + Writable
    }

    // Initialize free list with one big block
    unsafe {
        let block = HEAP_START as *mut FreeBlock;
        (*block).size = HEAP_INITIAL_SIZE;
        (*block).next = core::ptr::null_mut();
        HEAP_FREE_LIST = block;
        HEAP_INITIALIZED = true;
    }

    crate::serial_println!(
        "Kernel heap initialized: {:#010x} - {:#010x} ({} KiB)",
        HEAP_START,
        HEAP_START + HEAP_INITIAL_SIZE as u32,
        HEAP_INITIAL_SIZE / 1024
    );
}
