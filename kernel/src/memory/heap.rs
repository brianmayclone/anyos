//! Kernel heap allocator using a linked-list free list with demand paging.
//!
//! The heap reserves a bounded virtual address range and maps its initial
//! committed region at boot. Growth adds committed heap pages as long as the
//! reserved VA window and the physical frame allocator have enough space.
//!
//! The lock is IRQ-safe: interrupts are disabled while the heap lock is held.
//! This prevents deadlock when `reap_terminated()` frees a kernel stack from
//! within the timer ISR while the preempted thread was holding the heap lock.

use crate::memory::address::PhysAddr;
#[cfg(target_arch = "x86_64")]
use crate::memory::address::VirtAddr;
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::memory::FRAME_SIZE;
use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicUsize, Ordering};

/// Virtual address where the kernel heap begins.
///
/// Placed above the static higher-half kernel mapping to leave room for kernel
/// code, data, BSS, and boot-time page tables without overlapping heap pages.
///
/// x86_64: PML4[511]/PDPT[510] window at 0xFFFF_FFFF_8000_0000.
/// ARM64:  1 GiB block mapping at 0xFFFF_0000_8000_0000 (boot.S TTBR1).
#[cfg(target_arch = "x86_64")]
pub const HEAP_START: u64 = 0xFFFF_FFFF_8500_0000;
#[cfg(target_arch = "aarch64")]
pub const HEAP_START: u64 = 0xFFFF_0000_8200_0000;
/// Initial committed size (32 MiB).
const HEAP_INITIAL_SIZE: usize = 32 * 1024 * 1024;
/// Maximum heap size.
///
/// x86_64: keep the heap below the KDRV load window at
/// 0xFFFF_FFFF_B000_0000, with a 32 MiB guard gap for fixed temporary
/// mappings and future kernel-reserved VA.
#[cfg(target_arch = "x86_64")]
pub const HEAP_MAX_SIZE: usize = 656 * 1024 * 1024;
/// ARM64: the early TTBR1 block gives this region a fixed 1 GiB physical
/// backing relationship; keep the historical cap until the ARM64 heap is moved
/// onto explicit page-table mappings like x86_64.
#[cfg(target_arch = "aarch64")]
pub const HEAP_MAX_SIZE: usize = 512 * 1024 * 1024;
/// Minimum growth increment when expanding the heap (4 MiB).
const GROW_CHUNK: usize = 4 * 1024 * 1024;

/// Committed heap size in bytes. Readable by the page fault handler without
/// acquiring the heap lock. Pages in [HEAP_START, HEAP_START + HEAP_COMMITTED)
/// are valid heap addresses.
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

// --- Per-CPU free list cache ---
// Each CPU maintains a local free list that can be accessed without the global
// lock (interrupts are disabled). This eliminates contention on the global
// heap lock for the common case where a CPU's local cache has a suitable block.

const PERCPU_MAX_CPUS: usize = 16;
/// Maximum bytes cached per CPU before flushing half back to the global list.
const PERCPU_CACHE_MAX: usize = 256 * 1024; // 256 KiB

// --- Size-class bucket allocator ---
// Provides O(1) alloc/dealloc with no global lock for sizes up to 128 KiB.
// Each bucket is a simple LIFO stack of fixed-size blocks. Requests are
// rounded UP to the next size class, so freed blocks always return to a
// matching bucket and can be reused without external fragmentation.
//
// Power-of-two classes from 32 B to 128 KiB. The 73 KiB allocation that
// previously panicked the kernel after 10 h uptime now lands in the 128 KiB
// bucket and is satisfied without walking the fragmented global free list.

const SIZE_CLASSES: [usize; 13] = [
    32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072,
];
const NUM_SIZE_CLASSES: usize = 13;

/// Per-class cap on cached blocks per CPU. Larger classes cap lower so
/// total per-CPU bucket residency stays bounded (~2 MiB per CPU across
/// all classes when fully populated × 16 CPUs ≈ 32 MiB on a 442 MiB heap).
const BUCKET_CAPS: [usize; 13] = [
    128, 128, 128, 128, 128, 128, // 32 B .. 1 KiB
    64, 32, 32, 16, // 2 KiB .. 16 KiB
    8, 4, 4, // 32 KiB .. 128 KiB
];

/// A single node in a bucket's free stack (stored in-place in the freed block).
#[repr(C)]
struct BucketNode {
    next: *mut BucketNode,
}

/// Per-CPU size-class bucket.
struct SizeClassBucket {
    head: *mut BucketNode,
    count: usize,
}

impl SizeClassBucket {
    const fn new() -> Self {
        SizeClassBucket {
            head: core::ptr::null_mut(),
            count: 0,
        }
    }
}

/// Per-CPU buckets for each size class.
struct PerCpuBuckets {
    buckets: [SizeClassBucket; NUM_SIZE_CLASSES],
}

impl PerCpuBuckets {
    const fn new() -> Self {
        const INIT: SizeClassBucket = SizeClassBucket::new();
        PerCpuBuckets {
            buckets: [INIT; NUM_SIZE_CLASSES],
        }
    }
}

static mut PERCPU_BUCKETS: [PerCpuBuckets; PERCPU_MAX_CPUS] = {
    const INIT: PerCpuBuckets = PerCpuBuckets::new();
    [INIT; PERCPU_MAX_CPUS]
};

/// Round `size` up to the next size class. Returns the index, or None
/// if the request exceeds the largest class (then the request goes
/// directly to the per-CPU cache / global free list).
#[inline(always)]
fn size_class_index(size: usize) -> Option<usize> {
    let mut i = 0;
    while i < NUM_SIZE_CLASSES {
        if SIZE_CLASSES[i] >= size {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Block size for a given size-class index. Always >= the original request
/// that produced this index — alloc returns blocks of this exact size, and
/// dealloc routes them back to the same bucket.
#[inline(always)]
fn size_class_size(idx: usize) -> usize {
    SIZE_CLASSES[idx]
}

/// Try to allocate from a per-CPU size-class bucket. O(1), lock-free (IF=0).
///
/// Defends against a corrupted free-list head by validating that the head
/// pointer — and the `next` pointer we're about to install — both land inside
/// the committed heap window. A use-after-free that wrote e.g. `0xFFFF_FFFF`
/// into a bucket block's first 8 bytes would otherwise fault the kernel on
/// the next bucket alloc (observed crash: RIP inside LockedHeap::alloc,
/// CR2=0xFFFFFFFF). With the checks we return null and let the slow path
/// handle it; the bucket is then reset so the corrupted chain stops spreading.
unsafe fn bucket_alloc(cpu: usize, sci: usize) -> *mut u8 {
    if cpu >= PERCPU_MAX_CPUS || sci >= NUM_SIZE_CLASSES {
        return core::ptr::null_mut();
    }
    let bucket = &mut PERCPU_BUCKETS[cpu].buckets[sci];
    if bucket.head.is_null() {
        return core::ptr::null_mut();
    }
    if !is_in_heap(bucket.head as usize) {
        // Corrupted head — drop the whole bucket rather than dereference.
        bucket.head = core::ptr::null_mut();
        bucket.count = 0;
        return core::ptr::null_mut();
    }
    let node = bucket.head;
    let next = (*node).next;
    if !next.is_null() && !is_in_heap(next as usize) {
        // Corrupted link inside the bucket — drop the chain.
        bucket.head = core::ptr::null_mut();
        bucket.count = 0;
        return core::ptr::null_mut();
    }
    bucket.head = next;
    if bucket.count > 0 {
        bucket.count -= 1;
    }
    node as *mut u8
}

/// Return a block to a per-CPU size-class bucket. O(1), lock-free (IF=0).
/// Returns true if successfully cached, false if bucket is full.
unsafe fn bucket_dealloc(cpu: usize, ptr: *mut u8, sci: usize) -> bool {
    if cpu >= PERCPU_MAX_CPUS || sci >= NUM_SIZE_CLASSES {
        return false;
    }
    let bucket = &mut PERCPU_BUCKETS[cpu].buckets[sci];
    if bucket.count >= BUCKET_CAPS[sci] {
        return false; // bucket full, use regular dealloc path
    }
    let node = ptr as *mut BucketNode;
    (*node).next = bucket.head;
    bucket.head = node;
    bucket.count += 1;
    true
}

/// Per-CPU free list head and cached byte count.
/// Accessed ONLY with interrupts disabled (no lock needed — single CPU).
struct PerCpuCache {
    free_list: *mut FreeBlock,
    cached_bytes: usize,
}

impl PerCpuCache {
    const fn new() -> Self {
        PerCpuCache {
            free_list: core::ptr::null_mut(),
            cached_bytes: 0,
        }
    }
}

static mut PERCPU_CACHES: [PerCpuCache; PERCPU_MAX_CPUS] = {
    const INIT: PerCpuCache = PerCpuCache::new();
    [INIT; PERCPU_MAX_CPUS]
};

/// Try to allocate from the per-CPU local free list (lock-free, IF=0).
/// Returns null if no suitable block found locally.
///
/// Walks the per-CPU free list with the same cycle + bounds safeguards as
/// `alloc_inner`: a corrupted link (use-after-free that wrote garbage into a
/// cached block's header) drops the whole local cache instead of faulting.
unsafe fn percpu_alloc(cpu: usize, size: usize) -> *mut u8 {
    if cpu >= PERCPU_MAX_CPUS {
        return core::ptr::null_mut();
    }
    let cache = &mut PERCPU_CACHES[cpu];
    let mut prev: *mut FreeBlock = core::ptr::null_mut();
    let mut current = cache.free_list;

    const MAX_ITER: usize = 100_000;
    let mut iter = 0usize;
    while !current.is_null() {
        iter += 1;
        if iter > MAX_ITER || !is_in_heap(current as usize) {
            // Corrupted or cyclic free list — discard everything we've seen
            // so the next alloc starts from a clean slate via the global
            // path. We can't coalesce safely once the chain is broken.
            cache.free_list = core::ptr::null_mut();
            cache.cached_bytes = 0;
            return core::ptr::null_mut();
        }
        let block_size = (*current).size;
        if block_size >= size {
            if block_size >= size + core::mem::size_of::<FreeBlock>() + 8 {
                // Split
                let new_block = (current as *mut u8).add(size) as *mut FreeBlock;
                (*new_block).size = block_size - size;
                (*new_block).next = (*current).next;
                if prev.is_null() {
                    cache.free_list = new_block;
                } else {
                    (*prev).next = new_block;
                }
                cache.cached_bytes -= size;
            } else {
                // Use entire block
                if prev.is_null() {
                    cache.free_list = (*current).next;
                } else {
                    (*prev).next = (*current).next;
                }
                cache.cached_bytes -= block_size;
            }
            return current as *mut u8;
        }
        prev = current;
        current = (*current).next;
    }
    core::ptr::null_mut()
}

/// Add a freed block to the per-CPU local cache. If the cache exceeds
/// PERCPU_CACHE_MAX, flush half of it back to the global free list.
/// Caller must hold IF=0 (interrupts disabled).
unsafe fn percpu_dealloc(cpu: usize, ptr: *mut u8, size: usize) {
    if cpu >= PERCPU_MAX_CPUS {
        // Fallback: insert directly into global list (caller holds global lock)
        dealloc_inner(
            ptr,
            core::alloc::Layout::from_size_align_unchecked(size, 16),
        );
        return;
    }
    let cache = &mut PERCPU_CACHES[cpu];
    let block = ptr as *mut FreeBlock;
    (*block).size = size;
    (*block).next = cache.free_list;
    cache.free_list = block;
    cache.cached_bytes += size;

    // Flush half back to global if over threshold
    if cache.cached_bytes > PERCPU_CACHE_MAX {
        percpu_flush_half(cpu);
    }
}

/// Move roughly half the per-CPU cache blocks to the global free list.
/// MUST be called with the global heap lock held.
unsafe fn percpu_flush_half(cpu: usize) {
    let cache = &mut PERCPU_CACHES[cpu];
    let target = cache.cached_bytes / 2;
    let mut flushed = 0usize;

    while !cache.free_list.is_null() && flushed < target {
        let block = cache.free_list;
        cache.free_list = (*block).next;
        let bsize = (*block).size;
        cache.cached_bytes -= bsize;
        flushed += bsize;

        // Insert into global free list (sorted by address)
        const MAX_WALK: usize = 100_000;
        let mut prev: *mut FreeBlock = core::ptr::null_mut();
        let mut cur = HEAP_FREE_LIST;
        let mut walk = 0usize;
        while !cur.is_null() && (cur as usize) < (block as usize) {
            walk += 1;
            if walk > MAX_WALK {
                // Probable cycle — insert at head to avoid infinite loop
                (*block).next = HEAP_FREE_LIST;
                HEAP_FREE_LIST = block;
                break;
            }
            prev = cur;
            cur = (*cur).next;
        }
        if walk > MAX_WALK {
            continue; // skip coalescing, block already inserted at head
        }
        (*block).next = cur;
        if prev.is_null() {
            HEAP_FREE_LIST = block;
        } else {
            (*prev).next = block;
        }
        // Coalesce with neighbors
        if !prev.is_null() && (prev as *mut u8).add((*prev).size) == block as *mut u8 {
            (*prev).size += (*block).size;
            (*prev).next = (*block).next;
            // Check prev+next
            if !(*prev).next.is_null()
                && (prev as *mut u8).add((*prev).size) == (*prev).next as *mut u8
            {
                let next = (*prev).next;
                (*prev).size += (*next).size;
                (*prev).next = (*next).next;
            }
        } else {
            if !(*block).next.is_null()
                && (block as *mut u8).add((*block).size) == (*block).next as *mut u8
            {
                let next = (*block).next;
                (*block).size += (*next).size;
                (*block).next = (*next).next;
            }
        }
    }
}

impl LockedHeap {
    const fn new() -> Self {
        LockedHeap {
            lock: core::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Acquire the heap lock with interrupts disabled.
    /// Returns the saved interrupt state so `release` can restore it.
    fn acquire(&self) -> u64 {
        let flags = crate::arch::hal::save_and_disable_interrupts();

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
                #[cfg(target_arch = "x86_64")]
                unsafe {
                    use crate::arch::x86::port::{inb, outb};
                    let msg = b"\r\n!!! HEAP_LOCK TIMEOUT\r\n";
                    for &c in msg {
                        while inb(0x3FD) & 0x20 == 0 {}
                        outb(0x3F8, c);
                    }
                }
                #[cfg(target_arch = "aarch64")]
                {
                    let msg = b"\r\n!!! HEAP_LOCK TIMEOUT\r\n";
                    for &c in msg {
                        crate::arch::arm64::serial::write_byte(c);
                    }
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
        crate::arch::hal::restore_interrupt_state(flags);
    }
}

unsafe impl GlobalAlloc for LockedHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if !HEAP_INITIALIZED {
            return core::ptr::null_mut();
        }

        let raw_size = align_up(
            layout.size().max(core::mem::size_of::<FreeBlock>()),
            layout.align().max(16),
        );

        // Round up to the next size class. Bucket-eligible allocations are
        // ALWAYS satisfied with a class-sized block — even when the bucket
        // is empty and we fall through to the global free list — so dealloc
        // can route the same block back to the matching bucket without
        // size mismatch. Sizes > 128 KiB go straight to the global path.
        let (effective_size, sci) = match size_class_index(raw_size) {
            Some(idx) => (size_class_size(idx), Some(idx)),
            None => (raw_size, None),
        };

        // Ultra-fast path: per-CPU bucket (O(1), no lock, IF=0).
        let flags = crate::arch::hal::save_and_disable_interrupts();
        let cpu = crate::arch::hal::cpu_id();
        if let Some(idx) = sci {
            let result = bucket_alloc(cpu, idx);
            if !result.is_null() {
                crate::arch::hal::restore_interrupt_state(flags);
                return result;
            }
        }

        // Fast path: per-CPU free-list cache (only useful for sizes >128 KiB
        // now that everything smaller has its own bucket).
        if sci.is_none() {
            let result = percpu_alloc(cpu, effective_size);
            if !result.is_null() {
                crate::arch::hal::restore_interrupt_state(flags);
                return result;
            }
        }
        crate::arch::hal::restore_interrupt_state(flags);

        // Slow path: global lock. Build a layout that requests the rounded
        // class size so the free-list returns a block large enough for
        // bucket recycling.
        let alloc_layout =
            Layout::from_size_align_unchecked(effective_size, layout.align().max(16));
        let flags = self.acquire();
        let mut result = alloc_inner(alloc_layout);
        if result.is_null() {
            if grow_heap(effective_size) {
                result = alloc_inner(alloc_layout);
            }
        }
        self.release(flags);
        result
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        let raw_size = align_up(
            layout.size().max(core::mem::size_of::<FreeBlock>()),
            layout.align().max(16),
        );
        // Symmetry with alloc: round to the same size class so the block
        // lands in the bucket it was allocated from.
        let (effective_size, sci) = match size_class_index(raw_size) {
            Some(idx) => (size_class_size(idx), Some(idx)),
            None => (raw_size, None),
        };

        // Validate: pointer must be within heap bounds
        if !is_in_heap(ptr as usize) {
            return; // Leak instead of corrupting
        }

        // Ultra-fast path: return to size-class bucket (O(1), no lock, IF=0)
        let flags = crate::arch::hal::save_and_disable_interrupts();
        let cpu = crate::arch::hal::cpu_id();
        if let Some(idx) = sci {
            if bucket_dealloc(cpu, ptr, idx) {
                crate::arch::hal::restore_interrupt_state(flags);
                return;
            }
        }

        // Fast path: add to per-CPU cache (no global lock). Used when the
        // bucket is full (overflow) or for sizes >128 KiB (no bucket).
        if cpu < PERCPU_MAX_CPUS {
            let cache = &mut PERCPU_CACHES[cpu];
            let block = ptr as *mut FreeBlock;
            (*block).size = effective_size;
            (*block).next = cache.free_list;
            cache.free_list = block;
            cache.cached_bytes += effective_size;

            if cache.cached_bytes > PERCPU_CACHE_MAX {
                // Need global lock to flush back
                crate::arch::hal::restore_interrupt_state(flags);
                let flags = self.acquire();
                let cpu = crate::arch::hal::cpu_id();
                if cpu < PERCPU_MAX_CPUS {
                    percpu_flush_half(cpu);
                }
                self.release(flags);
            } else {
                crate::arch::hal::restore_interrupt_state(flags);
            }
            return;
        }
        crate::arch::hal::restore_interrupt_state(flags);

        // Fallback: global dealloc with the rounded size so the freed
        // block stays consistent with what alloc handed out.
        let dealloc_layout =
            Layout::from_size_align_unchecked(effective_size, layout.align().max(16));
        let flags = self.acquire();
        dealloc_inner(ptr, dealloc_layout);
        self.release(flags);
    }
}

/// Check if the heap lock is currently held (lock-free diagnostic).
/// Used by the timer heartbeat to detect if the heap is part of a deadlock chain.
#[inline]
pub fn is_heap_locked() -> bool {
    HEAP_ALLOCATOR
        .lock
        .load(core::sync::atomic::Ordering::Relaxed)
}

/// Check if an address is within the committed heap range.
#[inline]
fn is_in_heap(addr: usize) -> bool {
    let heap_start = HEAP_START as usize;
    let heap_end = heap_start + HEAP_COMMITTED.load(Ordering::Relaxed);
    addr >= heap_start && addr < heap_end
}

unsafe fn alloc_inner(layout: Layout) -> *mut u8 {
    let size = align_up(
        layout.size().max(core::mem::size_of::<FreeBlock>()),
        layout.align().max(16),
    );

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

/// Grow the heap by adding a committed free-list block.
///
/// x86_64 pre-maps the newly committed pages immediately so allocation failure
/// is reported here instead of later from a page fault while holding heap data.
/// ARM64's early 1 GiB block already maps the VA range, so growth only reserves
/// the corresponding backing frames.
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
unsafe fn grow_heap_exact(growth: usize) -> bool {
    let growth = align_up(growth, FRAME_SIZE);
    if growth == 0 {
        return false;
    }

    let old_committed = HEAP_COMMITTED.load(Ordering::Acquire);
    let new_committed = old_committed + growth;

    // Advance the committed watermark (makes these addresses valid for demand paging)
    HEAP_COMMITTED.store(new_committed, Ordering::Release);

    // ARM64: the 1 GiB block already maps the new region, but we must reserve
    // the backing physical frames so the frame allocator doesn't hand them out.
    #[cfg(target_arch = "aarch64")]
    {
        let phys_to_virt = virtual_mem::PHYS_TO_VIRT_OFFSET;
        for offset in (0..growth).step_by(FRAME_SIZE) {
            let va = HEAP_START + (old_committed + offset) as u64;
            let pa = va - phys_to_virt;
            physical::reserve_frame(PhysAddr::new(pa));
        }
    }

    let base = HEAP_START as usize + old_committed;

    // Pre-map pages for the new heap region to avoid demand page faults.
    // Previously we relied on ISR 14 demand paging, but if the page fault handler
    // fails (OOM), the write below would cause a double/triple fault.
    #[cfg(target_arch = "x86_64")]
    {
        for offset in (0..growth).step_by(FRAME_SIZE) {
            let va = (base + offset) as u64;
            let page_va = crate::memory::address::VirtAddr::new(va & !0xFFF);
            if !virtual_mem::is_page_mapped(page_va) {
                let frame = match physical::alloc_frame_with(physical::FrameAllocPolicy::Any) {
                    Some(f) => f,
                    None => return false,
                };
                virtual_mem::map_page(page_va, frame, 0x03);
                // Zero the page immediately
                core::ptr::write_bytes(va as *mut u8, 0, FRAME_SIZE);
            }
        }
    }

    // Insert the new region as a free block, inserted into the sorted free list.
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
    let size = align_up(
        layout.size().max(core::mem::size_of::<FreeBlock>()),
        layout.align().max(16),
    );

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
                crate::serial_verbose_println!(
                    "HEAP CORRUPT: block #{} at {:#x} outside heap bounds [{:#x}..{:#x}]",
                    count,
                    addr,
                    heap_start,
                    heap_end
                );
                HEAP_ALLOCATOR.release(flags);
                return;
            }
            if size == 0 || addr + size > heap_end {
                crate::serial_verbose_println!(
                    "HEAP CORRUPT: block #{} at {:#x} size {:#x} extends past heap end {:#x}",
                    count,
                    addr,
                    size,
                    heap_end
                );
                HEAP_ALLOCATOR.release(flags);
                return;
            }
            if addr < prev_end {
                crate::serial_verbose_println!(
                    "HEAP CORRUPT: block #{} at {:#x} overlaps previous ending at {:#x}",
                    count,
                    addr,
                    prev_end
                );
                HEAP_ALLOCATOR.release(flags);
                return;
            }

            total_free += size;
            prev_end = addr + size;
            count += 1;
            current = (*current).next;

            if count > 10000 {
                crate::serial_verbose_println!(
                    "HEAP CORRUPT: free list has >10000 entries (loop?)"
                );
                HEAP_ALLOCATOR.release(flags);
                return;
            }
        }

        crate::serial_verbose_println!(
            "  Heap check: {} free block(s), {} KiB free / {} KiB committed",
            count,
            total_free / 1024,
            HEAP_COMMITTED.load(Ordering::Acquire) / 1024
        );
        HEAP_ALLOCATOR.release(flags);
    }
}

/// Initialize the kernel heap.
///
/// **x86_64**: Maps the full initial committed virtual range (32 MiB). Keeping
/// committed heap pages backed from the start prevents them from aliasing broad
/// boot-time kernel mappings or later buddy allocations.
///
/// **ARM64**: The heap VA range is already mapped by the 1 GiB block in TTBR1.
/// We just reserve the backing physical frames so the allocator won't reuse them.
///
/// Must be called after physical and virtual memory are initialized.
pub fn init() {
    #[cfg(target_arch = "x86_64")]
    {
        let mapped_pages = HEAP_INITIAL_SIZE / FRAME_SIZE;
        for i in 0..mapped_pages {
            let virt = VirtAddr::new(HEAP_START + (i * FRAME_SIZE) as u64);
            let phys = physical::alloc_frame_with(physical::FrameAllocPolicy::Any)
                .expect("Failed to allocate heap frame");
            virtual_mem::map_page(virt, phys, 0x03); // Present + Writable
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // 1 GiB block already maps the entire heap VA range.
        // Reserve the backing physical frames for the initial committed region.
        let phys_to_virt = virtual_mem::PHYS_TO_VIRT_OFFSET;
        let committed_pages = HEAP_INITIAL_SIZE / FRAME_SIZE;
        for i in 0..committed_pages {
            let va = HEAP_START + (i * FRAME_SIZE) as u64;
            let pa = va - phys_to_virt;
            physical::reserve_frame(PhysAddr::new(pa));
        }
    }

    // Commit the full initial size.
    HEAP_COMMITTED.store(HEAP_INITIAL_SIZE, Ordering::Release);

    // Initialize free list with one big block spanning HEAP_INITIAL_SIZE.
    unsafe {
        let block = HEAP_START as *mut FreeBlock;
        (*block).size = HEAP_INITIAL_SIZE;
        (*block).next = core::ptr::null_mut();
        HEAP_FREE_LIST = block;
        HEAP_INITIALIZED = true;
    }

    crate::serial_verbose_println!(
        "Kernel heap initialized: {:#018x} - {:#018x} ({} KiB committed, max {} MiB)",
        HEAP_START,
        HEAP_START + HEAP_INITIAL_SIZE as u64,
        HEAP_INITIAL_SIZE / 1024,
        HEAP_MAX_SIZE / (1024 * 1024)
    );
}
