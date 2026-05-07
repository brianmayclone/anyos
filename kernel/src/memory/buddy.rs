//! Buddy allocator for physical memory frames.
//!
//! Replaces the linear-scan bitmap allocator that has historically
//! sat behind `physical::alloc_frame` / `alloc_contiguous`. The
//! buddy algorithm gives us:
//!
//! * O(MAX_ORDER) worst-case allocation regardless of fragmentation.
//! * Automatic coalescing on free — adjacent buddies merge upward
//!   without an explicit defrag pass.
//! * Built-in support for large physically-contiguous requests (DMA
//!   buffers, GPU framebuffers).
//!
//! # Design choices
//!
//! ## Single global zone (for now)
//!
//! Linux's mm/page_alloc.c uses per-CPU page lists (PCP) for the
//! lock-free hot path. That's a follow-up. This module provides the
//! correct, well-tested foundation; PCP magazines go on top later.
//!
//! ## Intrusive free lists
//!
//! Each free block is at frame `f` and spans frames `[f, f + 2^order)`.
//! Its first 4 bytes hold the frame index of the next free block at
//! the same order, or `LINK_NIL` for the tail. We do NOT keep a
//! separate `Vec<u32>` per order because:
//!
//!   * The buddy IS the physical-memory allocator. It cannot call
//!     into the heap (which would need it). Intrusive lists live in
//!     the freed pages themselves — zero allocation overhead.
//!   * The link word is in the cache line we'll touch anyway when
//!     the page gets allocated, so cache locality is excellent.
//!
//! Reading and writing the link word goes through `physmap::phys_to_virt`
//! so the allocator works for every RAM page, not just identity-mapped
//! low memory.
//!
//! ## Per-frame allocation order
//!
//! `order_of_alloc[frame]` records the order at which `frame` was
//! allocated, set on `alloc_pages` and consulted by `free_frame()`
//! so callers can free without remembering their order. 1 byte per
//! frame — 4 MiB BSS at the 16 GiB ceiling, demand-faulted.
//!
//! ## Used bitmap
//!
//! 1 bit per frame indicating "buddy considers this frame allocated".
//! Used by:
//!
//!   1. The merge step on free: read the buddy's bit without touching
//!      the page itself (which may be allocated and contain real data).
//!   2. `is_used()` queries from outside the allocator (deferred
//!      reaper, audit code).
//!   3. The audit walker: confirms the free-list lengths agree with
//!      the bitmap.
//!
//! 2 MiB BSS at the 16 GiB ceiling.
//!
//! ## Robustness
//!
//! Every public entry point validates inputs and no-ops on garbage.
//! Double-free is detected and dropped. Out-of-range frames are
//! silently rejected. The internal split/merge code uses
//! `debug_assert!` for invariants in debug builds and short-circuits
//! defensively in release. An `audit()` helper walks every order and
//! verifies the data structure's invariants.

use crate::memory::address::PhysAddr;
use crate::memory::physmap;
use crate::memory::FRAME_SIZE;

/// Maximum supported physical memory. Two BuddyZone instances live
/// in `static` storage (ZONE_DMA + ZONE_NORMAL in physical.rs);
/// each carries a `[u8; MAX_FRAMES]` order map (16 MiB at the
/// 16 GiB cap), a `[u8; MAX_FRAMES/8]` used bitmap (2 MiB), and a
/// matching managed-frame bitmap (2 MiB).
/// Two zones total ~40 MiB of `.data`, accommodated by the 64 MiB
/// higher-half kernel mapping in `virtual_mem::init`.
///
/// Lifting this further (to 64 GiB or unbounded) requires moving
/// the order map and bitmap off the static `.data` segment into
/// heap-allocated arrays sized at runtime from the actual E820 /
/// DTB extents. See `todos/buddy-no-cap.md` for the full plan.
pub const MAX_MEMORY: usize = 16 * 1024 * 1024 * 1024;
pub const MAX_FRAMES: usize = MAX_MEMORY / FRAME_SIZE;
const BITMAP_BYTES: usize = MAX_FRAMES / 8;

/// Largest order the allocator can produce.
///
/// Block size at order `o` is `2^o` frames. order 12 = 16 MiB —
/// covers any real contiguous request the kernel makes (largest
/// historic case is ~8 MiB for 1080p ARGB framebuffer; order 11).
pub const MAX_ORDER: usize = 16;
const NUM_ORDERS: usize = MAX_ORDER + 1;

/// Sentinel for "no next link" in the intrusive free list. u32::MAX
/// is reserved because no valid frame index can reach it (MAX_FRAMES
/// is far below 2^32 for our 16 GiB cap).
const LINK_NIL: u32 = u32::MAX;

/// Compute the smallest order whose block (2^order frames) holds at
/// least `n` frames.
///
/// `order_for(0)` is 0 (degenerate, callers reject n == 0).
/// `order_for(1)` is 0.
/// `order_for(2)` is 1.
/// `order_for(5)` is 3.
/// Saturates at MAX_ORDER for inputs > `1 << MAX_ORDER`.
#[inline]
pub const fn order_for(n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let bits = (usize::BITS - (n - 1).leading_zeros()) as usize;
    if bits > MAX_ORDER {
        MAX_ORDER
    } else {
        bits
    }
}

/// One zone of physical memory.
///
/// All state lives in statically-sized arrays (BSS), so this struct
/// can be embedded in a `Spinlock<BuddyZone>` static and used before
/// the heap allocator is up.
pub struct BuddyZone {
    /// Highest frame index ever exposed via `add_free_region`. Used
    /// to bound merge attempts: a buddy at or past `address_frames`
    /// is invalid and stops coalescing.
    pub address_frames: usize,
    /// Currently free frame count, summed over all orders. Maintained
    /// incrementally on alloc/free, never recomputed.
    pub free_frames: usize,
    /// Sum of frames added via `add_free_region` since init. Stays
    /// constant after init and is used by `physical::total_frames`.
    pub total_frames: usize,
    /// 1 bit per frame: `1` = currently allocated/reserved, `0` =
    /// free or never registered.
    used_bitmap: [u8; BITMAP_BYTES],
    /// 1 bit per frame: `1` = this zone owns/manages the frame.
    /// Needed because a zone may start at an arbitrary physical frame;
    /// unregistered gaps must not be treated as free RAM by audit or
    /// defensive free paths.
    managed_bitmap: [u8; BITMAP_BYTES],
    /// `order_of_alloc[frame]` is the order at which `frame` was
    /// last returned from `alloc_pages` — only valid for the head
    /// frame of a current allocation, garbage otherwise.
    order_of_alloc: [u8; MAX_FRAMES],
    /// Head of the intrusive free list per order. The list itself
    /// lives in the freed pages: the first 4 bytes of a free block
    /// are a `u32` (LINK_NIL for None) pointing at the next free
    /// block of the same order.
    free_list_heads: [u32; NUM_ORDERS],
    /// Tail of the intrusive free list per order. Maintained so
    /// `push_free_to_tail` is O(1); used during boot-time
    /// region registration to preserve ascending-address order in
    /// the free list. Runtime `free_pages` still pushes to the
    /// head (cache-friendly LIFO for recently-freed blocks).
    free_list_tails: [u32; NUM_ORDERS],
    /// Number of free blocks per order. Kept in sync with the
    /// intrusive lists for O(1) `audit` and diagnostic dumps without
    /// walking the lists.
    free_count: [u32; NUM_ORDERS],
}

impl BuddyZone {
    /// Construct an empty zone. Use `add_free_region` to populate it
    /// from E820 entries (or the ARM RAM extents), then `reserve_range`
    /// for the kernel image / BIOS / framebuffer / DMA pre-allocations.
    pub const fn new() -> Self {
        Self {
            address_frames: 0,
            free_frames: 0,
            total_frames: 0,
            used_bitmap: [0u8; BITMAP_BYTES],
            managed_bitmap: [0u8; BITMAP_BYTES],
            order_of_alloc: [0u8; MAX_FRAMES],
            free_list_heads: [LINK_NIL; NUM_ORDERS],
            free_list_tails: [LINK_NIL; NUM_ORDERS],
            free_count: [0u32; NUM_ORDERS],
        }
    }

    // ── Bitmap ──────────────────────────────────────────────────────

    #[inline]
    fn mark_used_one(&mut self, frame: usize) {
        if frame >= MAX_FRAMES {
            return;
        }
        self.used_bitmap[frame >> 3] |= 1u8 << (frame & 7);
    }

    #[inline]
    fn mark_free_one(&mut self, frame: usize) {
        if frame >= MAX_FRAMES {
            return;
        }
        self.used_bitmap[frame >> 3] &= !(1u8 << (frame & 7));
    }

    /// Check the bitmap. Out-of-range frames are reported as used
    /// so callers (e.g. the deferred reaper walking corrupted PTEs)
    /// skip them rather than panic.
    #[inline]
    pub fn is_used(&self, frame: usize) -> bool {
        if frame >= MAX_FRAMES {
            return true;
        }
        self.used_bitmap[frame >> 3] & (1u8 << (frame & 7)) != 0
    }

    #[inline]
    fn mark_managed_one(&mut self, frame: usize) {
        if frame >= MAX_FRAMES {
            return;
        }
        self.managed_bitmap[frame >> 3] |= 1u8 << (frame & 7);
    }

    #[inline]
    fn is_managed(&self, frame: usize) -> bool {
        if frame >= MAX_FRAMES {
            return false;
        }
        self.managed_bitmap[frame >> 3] & (1u8 << (frame & 7)) != 0
    }

    fn mark_range_managed(&mut self, frame: usize, count: usize) {
        let end = frame + count;
        if end > MAX_FRAMES {
            return;
        }
        let mut f = frame;
        while f < end && (f & 7) != 0 {
            self.mark_managed_one(f);
            f += 1;
        }
        while f + 8 <= end {
            self.managed_bitmap[f >> 3] = 0xFF;
            f += 8;
        }
        while f < end {
            self.mark_managed_one(f);
            f += 1;
        }
    }

    /// Bulk mark a range used. Whole-byte writes when possible — the
    /// per-frame bit ops would otherwise dominate large block alloc
    /// (one alloc of 2^16 frames = 65536 separate bit ops).
    fn mark_range_used(&mut self, frame: usize, count: usize) {
        let end = frame + count;
        if end > MAX_FRAMES {
            return;
        }
        let mut f = frame;
        while f < end && (f & 7) != 0 {
            self.used_bitmap[f >> 3] |= 1u8 << (f & 7);
            f += 1;
        }
        while f + 8 <= end {
            self.used_bitmap[f >> 3] = 0xFF;
            f += 8;
        }
        while f < end {
            self.used_bitmap[f >> 3] |= 1u8 << (f & 7);
            f += 1;
        }
    }

    fn mark_range_free(&mut self, frame: usize, count: usize) {
        let end = frame + count;
        if end > MAX_FRAMES {
            return;
        }
        let mut f = frame;
        while f < end && (f & 7) != 0 {
            self.used_bitmap[f >> 3] &= !(1u8 << (f & 7));
            f += 1;
        }
        while f + 8 <= end {
            self.used_bitmap[f >> 3] = 0;
            f += 8;
        }
        while f < end {
            self.used_bitmap[f >> 3] &= !(1u8 << (f & 7));
            f += 1;
        }
    }

    // ── Intrusive link accessors ────────────────────────────────────
    //
    // Free blocks are not heap allocations — they're physical pages
    // we're keeping track of. Their first 4 bytes hold a u32 link
    // word. Reading and writing it goes through physmap so we work
    // for every RAM page, not just the low identity-map window.
    //
    // SAFETY notes for callers:
    //   - frame must be currently free (sole owner of the page).
    //   - physmap must be ready (asserted at first use; before
    //     physmap is up, the buddy is not initialised either).

    #[inline]
    fn link_ptr(&self, frame: usize) -> *mut u32 {
        let phys = PhysAddr::new((frame * FRAME_SIZE) as u64);
        // physmap_or_identity falls back to identity-map if physmap
        // isn't ready yet; during normal operation physmap is up
        // before any buddy use, so this returns the physmap virt.
        physmap::phys_to_virt_or_identity(phys) as *mut u32
    }

    #[inline]
    fn read_link(&self, frame: usize) -> u32 {
        unsafe { core::ptr::read_volatile(self.link_ptr(frame)) }
    }

    #[inline]
    fn write_link(&mut self, frame: usize, next: u32) {
        unsafe { core::ptr::write_volatile(self.link_ptr(frame), next) }
    }

    // ── Free-list operations ────────────────────────────────────────

    /// Push `frame` as a free block of `order` onto its list.
    /// Marks the bitmap, writes the link, updates head + counts.
    fn push_free(&mut self, frame: usize, order: usize) {
        debug_assert!(order < NUM_ORDERS);
        let count = 1usize << order;
        let prev_head = self.free_list_heads[order];
        // Order matters: clear bitmap first, write link, then
        // publish via head store. A reader that observes a head
        // value always sees the link already written.
        self.mark_range_free(frame, count);
        self.write_link(frame, prev_head);
        self.free_list_heads[order] = frame as u32;
        if self.free_list_tails[order] == LINK_NIL {
            // List was empty — new block is also the tail.
            self.free_list_tails[order] = frame as u32;
        }
        self.free_count[order] = self.free_count[order].saturating_add(1);
        self.free_frames += count;
    }

    /// Push a free block at the TAIL of its order's list. Used by
    /// `add_free_region` so that consecutive low→high inserts produce
    /// an ascending-by-address free list — the head is the lowest
    /// block, so subsequent allocations consistently return low
    /// memory until the list drains. Low-identity callers are handled at the
    /// physical allocator's zone-policy layer; keeping each zone internally
    /// address-ordered makes that behaviour deterministic.
    fn push_free_to_tail(&mut self, frame: usize, order: usize) {
        debug_assert!(order < NUM_ORDERS);
        let count = 1usize << order;
        self.mark_range_free(frame, count);
        // The new block has no successor — it's the new tail.
        self.write_link(frame, LINK_NIL);
        let prev_tail = self.free_list_tails[order];
        if prev_tail == LINK_NIL {
            // Empty list: new block is both head and tail.
            self.free_list_heads[order] = frame as u32;
        } else {
            // Non-empty: update old tail's link to point at the new
            // block.
            self.write_link(prev_tail as usize, frame as u32);
        }
        self.free_list_tails[order] = frame as u32;
        self.free_count[order] = self.free_count[order].saturating_add(1);
        self.free_frames += count;
    }

    /// Pop one block from the free list at `order`. Returns the
    /// frame index, or `None` if the list is empty.
    fn pop_free(&mut self, order: usize) -> Option<usize> {
        debug_assert!(order < NUM_ORDERS);
        let head = self.free_list_heads[order];
        if head == LINK_NIL {
            return None;
        }
        // Defense in depth: if the head got clobbered by an outside
        // writer (the buddy zone's static occasionally takes a stray
        // write — see todos for the open chase), refuse to walk the
        // bogus pointer. Returning None here turns into a clean
        // alloc failure at the caller instead of a page fault deep
        // inside read_link's deref.
        let frame_idx = head as usize;
        if frame_idx >= MAX_FRAMES {
            crate::serial_println!(
                "[buddy] pop_free order={} head={:#x} out of range — dropping list",
                order,
                head
            );
            self.free_list_heads[order] = LINK_NIL;
            self.free_list_tails[order] = LINK_NIL;
            self.free_count[order] = 0;
            return None;
        }
        let frame = frame_idx;
        let next = self.read_link(frame);
        self.free_list_heads[order] = next;
        if next == LINK_NIL {
            // List is now empty — clear the tail too.
            self.free_list_tails[order] = LINK_NIL;
        }
        self.free_count[order] = self.free_count[order].saturating_sub(1);
        let count = 1usize << order;
        self.mark_range_used(frame, count);
        self.free_frames -= count;
        Some(frame)
    }

    /// Remove a *specific* frame from the free list at `order`. O(N)
    /// in the list length; called from the merge path. In the
    /// common case the buddy is at or near the head of its list
    /// (it was just freed by an adjacent free), so the lookup is
    /// effectively O(1).
    ///
    /// Bounded by `free_count[order]` to defend against a corrupted
    /// link forming a cycle.
    fn remove_specific(&mut self, frame: usize, order: usize) -> bool {
        debug_assert!(order < NUM_ORDERS);
        let target = frame as u32;
        let head = self.free_list_heads[order];
        if head == LINK_NIL {
            return false;
        }
        if head == target {
            let next = self.read_link(frame);
            self.free_list_heads[order] = next;
            if next == LINK_NIL {
                self.free_list_tails[order] = LINK_NIL;
            }
            self.free_count[order] = self.free_count[order].saturating_sub(1);
            let count = 1usize << order;
            self.mark_range_used(frame, count);
            self.free_frames -= count;
            return true;
        }
        let mut prev = head;
        let mut steps_left = self.free_count[order] as u32;
        while steps_left > 0 {
            steps_left -= 1;
            let cur = self.read_link(prev as usize);
            if cur == LINK_NIL {
                return false;
            }
            if cur == target {
                let next = self.read_link(frame);
                self.write_link(prev as usize, next);
                if next == LINK_NIL {
                    // We removed the tail. prev is now the new tail.
                    self.free_list_tails[order] = prev;
                }
                self.free_count[order] = self.free_count[order].saturating_sub(1);
                let count = 1usize << order;
                self.mark_range_used(frame, count);
                self.free_frames -= count;
                return true;
            }
            prev = cur;
        }
        false
    }

    /// True iff `frame` is currently the head of a free block of
    /// exactly `order`. Walks that order's list.
    fn is_free_head_at_order(&self, frame: usize, order: usize) -> bool {
        if frame >= MAX_FRAMES {
            return false;
        }
        if self.is_used(frame) {
            return false;
        }
        let mut cur = self.free_list_heads[order];
        let mut steps_left = self.free_count[order] as u32;
        while cur != LINK_NIL && steps_left > 0 {
            if cur as usize == frame {
                return true;
            }
            cur = self.read_link(cur as usize);
            steps_left -= 1;
        }
        false
    }

    // ── Initialisation ──────────────────────────────────────────────

    /// Add a usable region `[start, end)` of frames. Splits it into
    /// the largest power-of-two-aligned blocks that fit and pushes
    /// each onto its order's free list.
    pub fn add_free_region(&mut self, start_frame: usize, end_frame: usize) {
        if end_frame <= start_frame || end_frame > MAX_FRAMES {
            return;
        }
        if end_frame > self.address_frames {
            self.address_frames = end_frame;
        }
        self.total_frames += end_frame - start_frame;
        self.mark_range_managed(start_frame, end_frame - start_frame);
        // Walk the region low→high and append each natural
        // power-of-two block to the TAIL of its order's free list.
        // Tail-append preserves ascending-address order, so the
        // head of every order's list is the lowest block in that
        // order — and `pop_free` (which always pops the head) hands
        // out low addresses until the list drains.
        //
        // Keep allocation order deterministic inside each zone. The physical
        // allocator decides whether a caller needs low identity memory or may
        // use any RAM frame; the buddy should then hand out the lowest suitable
        // block in that zone.
        let mut f = start_frame;
        while f < end_frame {
            let mut order = MAX_ORDER;
            loop {
                let size = 1usize << order;
                if (f & (size - 1)) == 0 && end_frame - f >= size {
                    break;
                }
                if order == 0 {
                    break;
                }
                order -= 1;
            }
            self.push_free_to_tail(f, order);
            f += 1usize << order;
        }
    }

    /// Reserve a contiguous range of frames as already-allocated
    /// (kernel image, firmware tables, framebuffer ROM). Walks
    /// frame-by-frame; for each, locates the smallest free block
    /// that contains it, removes that block from its free list,
    /// re-frees the other halves, and leaves the target frame
    /// marked used.
    pub fn reserve_range(&mut self, start_frame: usize, end_frame: usize) {
        for f in start_frame..end_frame {
            self.reserve_frame(f);
        }
    }

    /// Reserve a single frame.
    pub fn reserve_frame(&mut self, frame: usize) {
        if frame >= MAX_FRAMES || !self.is_managed(frame) || self.is_used(frame) {
            return;
        }
        for o in 0..NUM_ORDERS {
            let head = frame & !((1usize << o) - 1);
            if !self.is_free_head_at_order(head, o) {
                continue;
            }
            if !self.remove_specific(head, o) {
                continue;
            }
            // Split downward, freeing every half not containing `frame`.
            let mut block = head;
            let mut bo = o;
            while bo > 0 {
                bo -= 1;
                let half = 1usize << bo;
                let upper = block + half;
                if frame < upper {
                    self.push_free(upper, bo);
                } else {
                    self.push_free(block, bo);
                    block = upper;
                }
            }
            debug_assert_eq!(block, frame);
            return;
        }
        // Frame wasn't in any free list — already reserved by some
        // earlier path. Mark used defensively.
        self.mark_used_one(frame);
    }

    // ── Allocation ──────────────────────────────────────────────────

    /// Allocate `2^want_order` contiguous frames. Returns the
    /// starting frame, or `None` on OOM.
    pub fn alloc_pages(&mut self, want_order: usize) -> Option<usize> {
        if want_order >= NUM_ORDERS {
            return None;
        }
        let mut o = want_order;
        while o < NUM_ORDERS && self.free_list_heads[o] == LINK_NIL {
            o += 1;
        }
        if o >= NUM_ORDERS {
            return None;
        }
        let frame = self.pop_free(o)?;
        while o > want_order {
            o -= 1;
            let half = 1usize << o;
            let upper = frame + half;
            self.push_free(upper, o);
        }
        // Record the order so a single-arg free can recover it.
        self.order_of_alloc[frame] = want_order as u8;
        Some(frame)
    }

    #[inline]
    pub fn alloc_frame(&mut self) -> Option<usize> {
        self.alloc_pages(0)
    }

    /// Allocate exactly `count` physically-contiguous frames.
    ///
    /// Internally the buddy must grab a power-of-two block, but the public
    /// physical allocator has historically allowed callers to release a
    /// contiguous allocation one frame at a time via `free_frame()`. Preserve
    /// that contract by returning unused tail frames immediately and recording
    /// each returned frame as an order-0 allocation.
    pub fn alloc_contiguous(&mut self, count: usize) -> Option<usize> {
        if count == 0 {
            return None;
        }
        if count > (1usize << MAX_ORDER) {
            return None;
        }
        let order = order_for(count);
        let block_count = 1usize << order;
        let frame = self.alloc_pages(order)?;

        for i in 0..count {
            self.order_of_alloc[frame + i] = 0;
        }
        for i in count..block_count {
            self.free_pages(frame + i, 0);
        }

        Some(frame)
    }

    // ── Free ────────────────────────────────────────────────────────

    /// Free a block of `order` previously returned by `alloc_pages(order)`.
    /// Coalesces upward as far as both buddies remain free at the
    /// same order.
    pub fn free_pages(&mut self, frame: usize, order: usize) {
        if order >= NUM_ORDERS || frame >= MAX_FRAMES {
            return;
        }
        let count = 1usize << order;
        if frame + count > MAX_FRAMES {
            return;
        }
        for f in frame..(frame + count) {
            if !self.is_managed(f) {
                return;
            }
        }
        if !self.is_used(frame) {
            // Defensive: double-free or never-allocated frame.
            // Drop instead of corrupting the lists.
            return;
        }

        let mut block = frame;
        let mut bo = order;
        while bo < MAX_ORDER {
            let buddy = block ^ (1usize << bo);
            if buddy + (1usize << bo) > self.address_frames {
                break;
            }
            if self.is_used(buddy) {
                break;
            }
            if !self.is_free_head_at_order(buddy, bo) {
                break;
            }
            if !self.remove_specific(buddy, bo) {
                break;
            }
            if buddy < block {
                block = buddy;
            }
            bo += 1;
        }
        self.push_free(block, bo);
    }

    /// Free a frame, recovering the order from `order_of_alloc`.
    /// Wraps `free_pages`. Drops silently for double-free or
    /// out-of-range — same as the legacy bitmap allocator.
    pub fn free_frame(&mut self, frame: usize) {
        if frame >= MAX_FRAMES || !self.is_managed(frame) || !self.is_used(frame) {
            return;
        }
        let order = self.order_of_alloc[frame] as usize;
        let order = if order >= NUM_ORDERS { 0 } else { order };
        self.free_pages(frame, order);
    }

    // ── Diagnostics ─────────────────────────────────────────────────

    /// Walk every free list and verify invariants. Returns Err with
    /// a static reason string on the first violation, Ok if every
    /// list is consistent.
    ///
    /// Checks:
    ///   - Recorded `free_count[o]` matches actual list length.
    ///   - Each listed frame is marked free in the bitmap.
    ///   - Each listed frame is properly aligned for its order.
    ///   - No list extends past `address_frames`.
    ///   - No cycles (bounded by `free_count` per list).
    ///   - `sum(free_count[o] * 2^o) == free_frames`.
    pub fn audit(&self) -> Result<(), &'static str> {
        let mut total_free_frames = 0usize;
        for o in 0..NUM_ORDERS {
            let mut count = 0u32;
            let mut cur = self.free_list_heads[o];
            let mut steps_left = (self.address_frames as u32).saturating_add(1);
            while cur != LINK_NIL && steps_left > 0 {
                let frame = cur as usize;
                if frame >= self.address_frames {
                    return Err("free list contains out-of-range frame");
                }
                let block_size = 1usize << o;
                if (frame & (block_size - 1)) != 0 {
                    return Err("free list contains misaligned frame");
                }
                if self.is_used(frame) {
                    return Err("free list contains frame marked used");
                }
                if frame + block_size > self.address_frames {
                    return Err("free list block extends past end of memory");
                }
                count += 1;
                steps_left -= 1;
                cur = self.read_link(frame);
            }
            if cur != LINK_NIL {
                return Err("free list cycle detected");
            }
            if count != self.free_count[o] {
                return Err("free_count[order] disagrees with list length");
            }
            total_free_frames += (count as usize) * (1usize << o);
        }
        if total_free_frames != self.free_frames {
            return Err("sum of free-list block sizes != free_frames");
        }
        let mut bitmap_free_frames = 0usize;
        for frame in 0..self.address_frames {
            if self.is_managed(frame) && !self.is_used(frame) {
                bitmap_free_frames += 1;
            }
        }
        if bitmap_free_frames != self.free_frames {
            return Err("bitmap free count != free_frames");
        }
        Ok(())
    }

    /// Return per-order free-block counts. For diagnostic dumps.
    pub fn free_counts(&self) -> [u32; NUM_ORDERS] {
        self.free_count
    }
}
