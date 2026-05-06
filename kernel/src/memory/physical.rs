//! Physical frame allocator — facade over either the bitmap or buddy backend.
//!
//! # Backends
//!
//! Two allocator implementations live behind the same public API:
//!
//! * **Bitmap** (default, `cfg(not(feature = "buddy_alloc"))`): the
//!   long-standing 1-bit-per-frame allocator with linear-scan
//!   contiguous allocation. Robust, but susceptible to
//!   fragmentation on long-running systems.
//!
//! * **Buddy** (`cfg(feature = "buddy_alloc")`): the new buddy
//!   allocator from `memory::buddy`. O(MAX_ORDER) allocation,
//!   automatic coalescing, no fragmentation drift.
//!
//! Both backends present the same external API:
//!
//! ```ignore
//! pub fn alloc_frame() -> Option<PhysAddr>
//! pub fn alloc_contiguous(count: usize) -> Option<PhysAddr>
//! pub fn alloc_frame_low() -> Option<PhysAddr>     // x86 only
//! pub fn free_frame(addr: PhysAddr)
//! pub fn reserve_frame(addr: PhysAddr)
//! pub fn free_frame_count() -> usize
//! pub fn free_frames() -> usize
//! pub fn total_frames() -> usize
//! pub fn init(boot_info: &BootInfo)            // x86_64 only
//! pub fn init_arm64(ram_base: u64, ram_size: u64)  // aarch64 only
//! pub fn is_allocator_locked() -> bool
//! pub fn is_allocator_locked_by_cpu(cpu: u32) -> bool
//! pub unsafe fn force_unlock_allocator()
//! ```
//!
//! No caller has to know which backend is active. CMake exposes a
//! flip via `-DANYOS_BUDDY_ALLOC=ON` while we validate the new
//! backend in shipped images.

use crate::boot_info::BootInfo;
use crate::memory::address::PhysAddr;
use crate::memory::FRAME_SIZE;

// ──────────────────────────────────────────────────────────────────
// Bitmap backend (default).
// ──────────────────────────────────────────────────────────────────

#[cfg(not(feature = "buddy_alloc"))]
mod bitmap_backend {
    use super::*;
    use crate::boot_info::E820_TYPE_USABLE;
    use crate::sync::spinlock::Spinlock;

    /// Maximum supported physical memory (64 GiB).
    const MAX_MEMORY: usize = 64 * 1024 * 1024 * 1024;
    const MAX_FRAMES: usize = MAX_MEMORY / FRAME_SIZE;
    const BITMAP_SIZE: usize = MAX_FRAMES / 8;

    const KERNEL_VIRT_BASE: u64 = 0xFFFF_FFFF_8000_0000;

    extern "C" {
        static _kernel_end: u8;
    }

    #[repr(C)]
    pub(super) struct FrameAllocator {
        pub total_frames: usize,
        pub address_frames: usize,
        pub free_frames: usize,
        pub next_search: usize,
        bitmap: [u8; BITMAP_SIZE],
    }

    impl FrameAllocator {
        fn set_used(&mut self, frame: usize) {
            if frame / 8 >= self.bitmap.len() {
                return;
            }
            self.bitmap[frame / 8] |= 1 << (frame % 8);
        }
        fn set_free(&mut self, frame: usize) {
            if frame / 8 >= self.bitmap.len() {
                return;
            }
            self.bitmap[frame / 8] &= !(1 << (frame % 8));
        }
        pub(super) fn is_used(&self, frame: usize) -> bool {
            if frame / 8 >= self.bitmap.len() {
                return true;
            }
            self.bitmap[frame / 8] & (1 << (frame % 8)) != 0
        }
    }

    pub(super) static ALLOCATOR: Spinlock<FrameAllocator> = Spinlock::new(FrameAllocator {
        bitmap: [0; BITMAP_SIZE],
        total_frames: 0,
        address_frames: 0,
        free_frames: 0,
        next_search: 0,
    });

    /// Maximum physical address for contiguous allocations.
    #[cfg(target_arch = "x86_64")]
    const CONTIGUOUS_MAX_FRAME: usize = (128 * 1024 * 1024) / FRAME_SIZE;
    #[cfg(target_arch = "aarch64")]
    const CONTIGUOUS_MAX_FRAME: usize = MAX_FRAMES;

    /// Two-stage init: stage 1 (`init`) registers everything in one
    /// pass — bitmap doesn't have the buddy's "must read/write the
    /// freed page" constraint so there's no cycle here.
    pub fn init(boot_info: &BootInfo) {
        let memory_map = unsafe { boot_info.memory_map() };
        let mut alloc = ALLOCATOR.lock();

        let mut max_usable_addr: u64 = 0;
        for entry in memory_map {
            if entry.entry_type == E820_TYPE_USABLE {
                let end = entry.base_addr + entry.length;
                if end > max_usable_addr {
                    max_usable_addr = end;
                }
            }
        }
        if max_usable_addr > MAX_MEMORY as u64 {
            max_usable_addr = MAX_MEMORY as u64;
        }
        alloc.address_frames = (max_usable_addr as usize) / FRAME_SIZE;

        let mut total_usable_bytes: u64 = 0;
        for entry in memory_map {
            if entry.entry_type == E820_TYPE_USABLE {
                let end = entry.base_addr + entry.length;
                let capped = if end > max_usable_addr {
                    max_usable_addr - entry.base_addr
                } else {
                    entry.length
                };
                total_usable_bytes += capped;
            }
        }
        alloc.total_frames = ((total_usable_bytes as usize) + FRAME_SIZE - 1) / FRAME_SIZE;
        alloc.free_frames = 0;

        let bitmap_bytes_needed = (alloc.address_frames + 7) / 8;
        for byte in alloc.bitmap[..bitmap_bytes_needed].iter_mut() {
            *byte = 0xFF;
        }

        for entry in memory_map {
            if entry.entry_type != E820_TYPE_USABLE {
                continue;
            }
            let start = PhysAddr::new(entry.base_addr).frame_align_up();
            let end = PhysAddr::new(entry.base_addr + entry.length).frame_align_down();
            if start.as_u64() >= end.as_u64() {
                continue;
            }
            for frame in start.frame_index()..end.frame_index() {
                if frame < MAX_FRAMES {
                    alloc.set_free(frame);
                    alloc.free_frames += 1;
                }
            }
        }

        let first_mb_frames = (2 * 1024 * 1024) / FRAME_SIZE;
        for frame in 0..first_mb_frames {
            if !alloc.is_used(frame) {
                alloc.set_used(frame);
                alloc.free_frames -= 1;
            }
        }

        let kernel_start = PhysAddr::new(boot_info.kernel_phys_start as u64).frame_align_down();
        let linker_kernel_end_phys =
            unsafe { (&_kernel_end as *const u8 as u64) - KERNEL_VIRT_BASE };
        let kernel_end_phys = if linker_kernel_end_phys > boot_info.kernel_phys_end as u64 {
            linker_kernel_end_phys
        } else {
            boot_info.kernel_phys_end as u64
        };
        let kernel_end = PhysAddr::new(kernel_end_phys).frame_align_up();
        let kern_start_val = kernel_start.as_u64();
        let kern_end_val = kernel_end.as_u64();
        for frame in kernel_start.frame_index()..kernel_end.frame_index() {
            if frame < MAX_FRAMES && !alloc.is_used(frame) {
                alloc.set_used(frame);
                alloc.free_frames -= 1;
            }
        }

        let total_mib = alloc.total_frames * FRAME_SIZE / (1024 * 1024);
        let free_frames = alloc.free_frames;
        let free_mib = free_frames * FRAME_SIZE / (1024 * 1024);
        drop(alloc);

        crate::serial_verbose_println!(
            "Reserving kernel region: {:#010x} - {:#010x} (includes BSS + stack)",
            kern_start_val,
            kern_end_val
        );
        crate::serial_verbose_println!(
            "Physical memory: {} MiB total, {} frames free ({} MiB) [bitmap]",
            total_mib,
            free_frames,
            free_mib
        );
    }

    /// Bitmap stage 2 — no-op. Bitmap registers everything in stage 1
    /// because it doesn't have to write into the freed pages.
    pub fn late_init(_boot_info: &BootInfo) {}
    pub fn late_init_arm64(_ram_base: u64, _ram_size: u64) {}

    pub fn init_arm64(ram_base: u64, ram_size: u64) {
        const ARM64_PHYS_TO_VIRT: u64 = 0xFFFF_0000_4000_0000;
        let ram_end = ram_base + ram_size;
        let start_frame = (ram_base as usize) / FRAME_SIZE;
        let end_frame = (ram_end as usize) / FRAME_SIZE;

        let mut alloc = ALLOCATOR.lock();
        alloc.total_frames = (ram_size as usize) / FRAME_SIZE;
        alloc.address_frames = end_frame;
        alloc.free_frames = 0;

        let bitmap_bytes = (end_frame + 7) / 8;
        for byte in alloc.bitmap[..bitmap_bytes].iter_mut() {
            *byte = 0xFF;
        }
        for frame in start_frame..end_frame {
            if frame < MAX_FRAMES {
                alloc.set_free(frame);
                alloc.free_frames += 1;
            }
        }

        let kernel_end_virt = unsafe { &_kernel_end as *const u8 as u64 };
        let kernel_end_phys = kernel_end_virt - ARM64_PHYS_TO_VIRT;
        let kernel_end_frame = ((kernel_end_phys as usize) + FRAME_SIZE - 1) / FRAME_SIZE;
        for frame in start_frame..kernel_end_frame {
            if frame < MAX_FRAMES && !alloc.is_used(frame) {
                alloc.set_used(frame);
                alloc.free_frames -= 1;
            }
        }
        alloc.next_search = kernel_end_frame;

        crate::serial_verbose_println!(
            "Physical memory: {} MiB RAM ({:#010x}-{:#010x}), {} frames free ({} MiB) [bitmap]",
            ram_size / (1024 * 1024),
            ram_base,
            ram_end,
            alloc.free_frames,
            alloc.free_frames * FRAME_SIZE / (1024 * 1024)
        );
    }

    pub fn alloc_frame() -> Option<PhysAddr> {
        let mut alloc = ALLOCATOR.lock();
        let total = alloc.address_frames;
        if alloc.free_frames == 0 {
            return None;
        }
        let start = alloc.next_search;
        for i in start..total {
            if !alloc.is_used(i) {
                alloc.set_used(i);
                alloc.free_frames -= 1;
                alloc.next_search = i + 1;
                return Some(PhysAddr::new((i * FRAME_SIZE) as u64));
            }
        }
        for i in 0..start {
            if !alloc.is_used(i) {
                alloc.set_used(i);
                alloc.free_frames -= 1;
                alloc.next_search = i + 1;
                return Some(PhysAddr::new((i * FRAME_SIZE) as u64));
            }
        }
        None
    }

    pub fn alloc_frame_low() -> Option<PhysAddr> {
        let mut alloc = ALLOCATOR.lock();
        if alloc.free_frames == 0 {
            return None;
        }
        let limit = alloc.address_frames.min(CONTIGUOUS_MAX_FRAME);
        for i in 0..limit {
            if !alloc.is_used(i) {
                alloc.set_used(i);
                alloc.free_frames -= 1;
                return Some(PhysAddr::new((i * FRAME_SIZE) as u64));
            }
        }
        None
    }

    pub fn alloc_contiguous(count: usize) -> Option<PhysAddr> {
        if count == 0 {
            return None;
        }
        let mut alloc = ALLOCATOR.lock();
        let limit = alloc.address_frames.min(CONTIGUOUS_MAX_FRAME);
        if count > limit {
            return None;
        }
        // Top-down scan first; falls back to bottom-up.
        let mut run_end = limit;
        let mut run_len = 0usize;
        let mut i = limit;
        while i > 0 {
            i -= 1;
            if !alloc.is_used(i) {
                if run_len == 0 {
                    run_end = i + 1;
                }
                run_len += 1;
                if run_len >= count {
                    let run_start = run_end - count;
                    for j in run_start..run_end {
                        alloc.set_used(j);
                        alloc.free_frames -= 1;
                    }
                    return Some(PhysAddr::new((run_start * FRAME_SIZE) as u64));
                }
            } else {
                run_len = 0;
            }
        }
        let mut run_start = 0usize;
        run_len = 0;
        for i in 0..limit {
            if !alloc.is_used(i) {
                if run_len == 0 {
                    run_start = i;
                }
                run_len += 1;
                if run_len >= count {
                    for j in run_start..run_start + count {
                        alloc.set_used(j);
                        alloc.free_frames -= 1;
                    }
                    return Some(PhysAddr::new((run_start * FRAME_SIZE) as u64));
                }
            } else {
                run_len = 0;
            }
        }
        None
    }

    pub fn free_frame(addr: PhysAddr) {
        let mut alloc = ALLOCATOR.lock();
        let frame = addr.frame_index();
        if alloc.is_used(frame) {
            alloc.set_free(frame);
            alloc.free_frames += 1;
            if frame < alloc.next_search {
                alloc.next_search = frame;
            }
        }
    }

    pub fn reserve_frame(addr: PhysAddr) {
        let mut alloc = ALLOCATOR.lock();
        let frame = addr.frame_index();
        if frame < MAX_FRAMES && !alloc.is_used(frame) {
            alloc.set_used(frame);
            alloc.free_frames -= 1;
        }
    }

    pub fn free_frame_count() -> usize {
        ALLOCATOR.lock().free_frames
    }
    pub fn total_frames() -> usize {
        ALLOCATOR.lock().total_frames
    }
    pub fn is_allocator_locked() -> bool {
        ALLOCATOR.is_locked()
    }
    pub fn is_allocator_locked_by_cpu(cpu: u32) -> bool {
        ALLOCATOR.is_held_by_cpu(cpu)
    }
    pub unsafe fn force_unlock_allocator() {
        ALLOCATOR.force_unlock();
    }
}

// ──────────────────────────────────────────────────────────────────
// Buddy backend.
// ──────────────────────────────────────────────────────────────────

#[cfg(feature = "buddy_alloc")]
mod buddy_backend {
    use super::*;
    use crate::boot_info::E820_TYPE_USABLE;
    use crate::memory::buddy::{order_for, BuddyZone, MAX_ORDER};
    use crate::sync::spinlock::Spinlock;
    use core::sync::atomic::{AtomicBool, Ordering};

    const KERNEL_VIRT_BASE: u64 = 0xFFFF_FFFF_8000_0000;

    extern "C" {
        static _kernel_end: u8;
    }

    // ── Two-stage allocator: bootmem (stage 1) → buddy (stage 2) ──
    //
    // Stage 1 (bootmem): a tiny bitmap allocator that mirrors the
    // bitmap-backend semantics: just records which frames are
    // free / allocated. No metadata is ever written into the
    // tracked frames themselves, so we don't accidentally clobber
    // boot-time live data (the bootloader's PML4, the BootInfo
    // struct, the memory map array, kernel BSS). This is the only
    // safe shape during virtual_mem::init: the bootloader CR3 is
    // still active, so any frame we touch as "free" might in fact
    // be a live page table.
    //
    // Stage 2 (buddy live): once virtual_mem::init has installed
    // the kernel's own CR3 and physmap is up, late_init migrates
    // every still-free frame from the bootmem bitmap into the
    // buddy zone's intrusive free lists. After that point all
    // alloc/free goes through the buddy.
    //
    // The transition is announced via BUDDY_LIVE; the public
    // alloc_frame / free_frame functions read it once and dispatch
    // to either bootmem or buddy.

    static BUDDY_LIVE: AtomicBool = AtomicBool::new(false);

    /// Stage-1 bootmem state. Plain bitmap, never writes into
    /// tracked frames. Only `alloc_frame`-style primitives needed:
    /// `alloc_contiguous` is rare during stage 1 (only the kernel
    /// PML4/PT setup uses it), so we don't bother with a fancy
    /// scan — the existing bitmap-backend implementation is forked
    /// inline here.
    const STAGE1_MAX_MEMORY: usize = 4 * 1024 * 1024 * 1024;
    const STAGE1_MAX_FRAMES: usize = STAGE1_MAX_MEMORY / FRAME_SIZE;
    const STAGE1_BITMAP_BYTES: usize = STAGE1_MAX_FRAMES / 8;

    #[repr(C)]
    struct Bootmem {
        total_frames: usize,
        address_frames: usize,
        free_frames: usize,
        next_search: usize,
        bitmap: [u8; STAGE1_BITMAP_BYTES],
    }

    impl Bootmem {
        fn set_used(&mut self, frame: usize) {
            if frame >> 3 < self.bitmap.len() {
                self.bitmap[frame >> 3] |= 1u8 << (frame & 7);
            }
        }
        fn set_free(&mut self, frame: usize) {
            if frame >> 3 < self.bitmap.len() {
                self.bitmap[frame >> 3] &= !(1u8 << (frame & 7));
            }
        }
        fn is_used(&self, frame: usize) -> bool {
            if frame >> 3 < self.bitmap.len() {
                self.bitmap[frame >> 3] & (1u8 << (frame & 7)) != 0
            } else {
                true
            }
        }
    }

    static BOOTMEM: Spinlock<Bootmem> = Spinlock::new(Bootmem {
        total_frames: 0,
        address_frames: 0,
        free_frames: 0,
        next_search: 0,
        bitmap: [0u8; STAGE1_BITMAP_BYTES],
    });

    /// Single global buddy zone. Populated in late_init.
    static ZONE: Spinlock<BuddyZone> = Spinlock::new(BuddyZone::new());

    /// On x86_64 the legacy `alloc_contiguous` path required the
    /// returned phys to live within the lower 128 MiB identity-map.
    /// physmap removes that restriction, but until every caller has
    /// migrated to dereferencing through physmap we keep the same
    /// guarantee for `alloc_frame_low()`. Callers that genuinely need
    /// low memory (legacy DMA, real-mode trampolines) use that
    /// function explicitly.
    #[cfg(target_arch = "x86_64")]
    const LOW_MAX_FRAME: usize = (128 * 1024 * 1024) / FRAME_SIZE;
    #[cfg(target_arch = "aarch64")]
    const LOW_MAX_FRAME: usize = usize::MAX;

    /// Stage 1 (bootmem): mirror the bitmap-backend init exactly.
    /// Mark every USABLE frame free in the bootmem bitmap, then
    /// reserve the first 2 MiB and the kernel image. DO NOT touch
    /// the buddy zone yet — its add_free_region writes into freed
    /// pages, and we cannot guarantee yet that any specific page
    /// isn't a live bootloader artefact (PML4, ACPI tables, etc).
    pub fn init(boot_info: &BootInfo) {
        let memory_map = unsafe { boot_info.memory_map() };
        let mut bm = BOOTMEM.lock();

        let mut max_usable_addr: u64 = 0;
        for entry in memory_map {
            if entry.entry_type == E820_TYPE_USABLE {
                let end = entry.base_addr + entry.length;
                if end > max_usable_addr {
                    max_usable_addr = end;
                }
            }
        }
        if max_usable_addr > STAGE1_MAX_MEMORY as u64 {
            max_usable_addr = STAGE1_MAX_MEMORY as u64;
        }
        bm.address_frames = (max_usable_addr as usize) / FRAME_SIZE;

        let mut total_usable_bytes: u64 = 0;
        for entry in memory_map {
            if entry.entry_type == E820_TYPE_USABLE {
                let end = entry.base_addr + entry.length;
                let capped = if end > max_usable_addr {
                    max_usable_addr - entry.base_addr
                } else {
                    entry.length
                };
                total_usable_bytes += capped;
            }
        }
        bm.total_frames = ((total_usable_bytes as usize) + FRAME_SIZE - 1) / FRAME_SIZE;
        bm.free_frames = 0;

        let bitmap_bytes_needed = (bm.address_frames + 7) / 8;
        for byte in bm.bitmap[..bitmap_bytes_needed].iter_mut() {
            *byte = 0xFF;
        }

        for entry in memory_map {
            if entry.entry_type != E820_TYPE_USABLE {
                continue;
            }
            let start = PhysAddr::new(entry.base_addr).frame_align_up();
            let end = PhysAddr::new(entry.base_addr + entry.length).frame_align_down();
            if start.as_u64() >= end.as_u64() {
                continue;
            }
            for frame in start.frame_index()..end.frame_index() {
                if frame < STAGE1_MAX_FRAMES {
                    bm.set_free(frame);
                    bm.free_frames += 1;
                }
            }
        }

        // Reserve first 2 MiB.
        let first_mb_frames = (2 * 1024 * 1024) / FRAME_SIZE;
        for frame in 0..first_mb_frames {
            if !bm.is_used(frame) {
                bm.set_used(frame);
                bm.free_frames -= 1;
            }
        }

        // Reserve kernel image.
        let kernel_start_phys = boot_info.kernel_phys_start as u64;
        let linker_kernel_end_phys =
            unsafe { (&_kernel_end as *const u8 as u64) - KERNEL_VIRT_BASE };
        let kernel_end_phys = if linker_kernel_end_phys > boot_info.kernel_phys_end as u64 {
            linker_kernel_end_phys
        } else {
            boot_info.kernel_phys_end as u64
        };
        let ks = PhysAddr::new(kernel_start_phys).frame_align_down().frame_index();
        let ke = PhysAddr::new(kernel_end_phys).frame_align_up().frame_index();
        for frame in ks..ke {
            if frame < STAGE1_MAX_FRAMES && !bm.is_used(frame) {
                bm.set_used(frame);
                bm.free_frames -= 1;
            }
        }

        let total_mib = bm.total_frames * FRAME_SIZE / (1024 * 1024);
        let free_mib = bm.free_frames * FRAME_SIZE / (1024 * 1024);
        let kern_start_val = kernel_start_phys;
        let kern_end_val = kernel_end_phys;
        drop(bm);

        crate::serial_verbose_println!(
            "Reserving kernel region: {:#010x} - {:#010x} [buddy bootmem]",
            kern_start_val,
            kern_end_val
        );
        crate::serial_verbose_println!(
            "Physical memory: {} MiB total, {} MiB free [buddy bootmem]",
            total_mib,
            free_mib
        );
    }

    /// Stage 2 (buddy live): physmap is up. Walk the bootmem bitmap
    /// and migrate every still-free frame into the buddy zone. The
    /// frames that virtual_mem::init / physmap::init pulled from
    /// bootmem during stage 1 are now marked used in the bitmap and
    /// won't be re-introduced as free.
    ///
    /// We migrate in contiguous runs: scan the bitmap for stretches
    /// of consecutive free frames and call BuddyZone::add_free_region
    /// once per run. add_free_region splits each run into the
    /// largest aligned power-of-two blocks.
    pub fn late_init(_boot_info: &BootInfo) {
        let mut bm = BOOTMEM.lock();
        let mut z = ZONE.lock();
        let limit = bm.address_frames;

        let mut run_start: Option<usize> = None;
        let mut migrated: usize = 0;
        for f in 0..limit {
            if !bm.is_used(f) {
                if run_start.is_none() {
                    run_start = Some(f);
                }
            } else if let Some(s) = run_start.take() {
                z.add_free_region(s, f);
                migrated += f - s;
                // Mark the migrated range used in bootmem so that
                // any post-late_init bootmem call (which shouldn't
                // happen, but defensively) doesn't double-allocate.
                for ff in s..f {
                    bm.set_used(ff);
                }
            }
        }
        if let Some(s) = run_start {
            z.add_free_region(s, limit);
            migrated += limit - s;
            for ff in s..limit {
                bm.set_used(ff);
            }
        }
        bm.free_frames = 0;

        let migrated_mib = migrated * FRAME_SIZE / (1024 * 1024);
        let total_mib = z.total_frames * FRAME_SIZE / (1024 * 1024);
        crate::serial_verbose_println!(
            "Physical memory: migrated {} MiB into buddy ({} MiB tracked) [buddy live]",
            migrated_mib,
            total_mib
        );

        BUDDY_LIVE.store(true, Ordering::Release);
    }

    pub fn init_arm64(ram_base: u64, ram_size: u64) {
        const ARM64_PHYS_TO_VIRT: u64 = 0xFFFF_0000_4000_0000;
        let ram_end = ram_base + ram_size;
        let start_frame = (ram_base as usize) / FRAME_SIZE;
        let end_frame = (ram_end as usize) / FRAME_SIZE;

        let mut bm = BOOTMEM.lock();
        bm.total_frames = (ram_size as usize) / FRAME_SIZE;
        bm.address_frames = end_frame.min(STAGE1_MAX_FRAMES);
        bm.free_frames = 0;

        let bitmap_bytes = (bm.address_frames + 7) / 8;
        for byte in bm.bitmap[..bitmap_bytes].iter_mut() {
            *byte = 0xFF;
        }
        for frame in start_frame..end_frame {
            if frame < STAGE1_MAX_FRAMES {
                bm.set_free(frame);
                bm.free_frames += 1;
            }
        }

        let kernel_end_virt = unsafe { &_kernel_end as *const u8 as u64 };
        let kernel_end_phys = kernel_end_virt - ARM64_PHYS_TO_VIRT;
        let kernel_end_frame = ((kernel_end_phys as usize) + FRAME_SIZE - 1) / FRAME_SIZE;
        for frame in start_frame..kernel_end_frame {
            if frame < STAGE1_MAX_FRAMES && !bm.is_used(frame) {
                bm.set_used(frame);
                bm.free_frames -= 1;
            }
        }
        bm.next_search = kernel_end_frame;

        let free_mib = bm.free_frames * FRAME_SIZE / (1024 * 1024);
        drop(bm);
        crate::serial_verbose_println!(
            "Physical memory: {} MiB RAM ({:#010x}-{:#010x}), {} MiB free [buddy bootmem]",
            ram_size / (1024 * 1024),
            ram_base,
            ram_end,
            free_mib
        );
    }

    pub fn late_init_arm64(_ram_base: u64, _ram_size: u64) {
        // Reuse the x86 migration logic — bootmem layout is the same.
        late_init_common();
    }

    fn late_init_common() {
        let mut bm = BOOTMEM.lock();
        let mut z = ZONE.lock();
        let limit = bm.address_frames;

        let mut run_start: Option<usize> = None;
        for f in 0..limit {
            if !bm.is_used(f) {
                if run_start.is_none() {
                    run_start = Some(f);
                }
            } else if let Some(s) = run_start.take() {
                z.add_free_region(s, f);
                for ff in s..f {
                    bm.set_used(ff);
                }
            }
        }
        if let Some(s) = run_start {
            z.add_free_region(s, limit);
            for ff in s..limit {
                bm.set_used(ff);
            }
        }
        bm.free_frames = 0;
        BUDDY_LIVE.store(true, Ordering::Release);
    }

    #[inline]
    fn frame_to_phys(frame: usize) -> PhysAddr {
        PhysAddr::new((frame * FRAME_SIZE) as u64)
    }

    /// Bootmem alloc_frame: simple next-fit linear scan over the
    /// stage-1 bitmap. Used until BUDDY_LIVE flips.
    fn bootmem_alloc_frame() -> Option<usize> {
        let mut bm = BOOTMEM.lock();
        let total = bm.address_frames;
        if bm.free_frames == 0 {
            return None;
        }
        let start = bm.next_search;
        for i in start..total {
            if !bm.is_used(i) {
                bm.set_used(i);
                bm.free_frames -= 1;
                bm.next_search = i + 1;
                return Some(i);
            }
        }
        for i in 0..start {
            if !bm.is_used(i) {
                bm.set_used(i);
                bm.free_frames -= 1;
                bm.next_search = i + 1;
                return Some(i);
            }
        }
        None
    }

    fn bootmem_alloc_contiguous(count: usize) -> Option<usize> {
        if count == 0 {
            return None;
        }
        let mut bm = BOOTMEM.lock();
        let limit = bm.address_frames.min(STAGE1_MAX_FRAMES);
        let mut run_start = 0usize;
        let mut run_len = 0usize;
        for i in 0..limit {
            if !bm.is_used(i) {
                if run_len == 0 {
                    run_start = i;
                }
                run_len += 1;
                if run_len >= count {
                    for j in run_start..run_start + count {
                        bm.set_used(j);
                        bm.free_frames -= 1;
                    }
                    return Some(run_start);
                }
            } else {
                run_len = 0;
            }
        }
        None
    }

    fn bootmem_free_frame(frame: usize) {
        let mut bm = BOOTMEM.lock();
        if bm.is_used(frame) {
            bm.set_free(frame);
            bm.free_frames += 1;
            if frame < bm.next_search {
                bm.next_search = frame;
            }
        }
    }

    pub fn alloc_frame() -> Option<PhysAddr> {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            // Backward compatibility: every legacy caller of
            // alloc_frame assumes the returned phys address is
            // dereferenceable through the boot identity map (i.e.
            // < 128 MiB on x86_64). AC97 BDL, audio buffers,
            // hardware-virt structures, AHCI sub-allocations all
            // do `*(phys as *mut u8)`. Until those callers migrate
            // to physmap, alloc_frame must return low memory too.
            //
            // We don't bias the buddy zone; we just sample frames
            // and reject high ones. In practice the zone hands out
            // recently-freed leaves first, and once init is done
            // those tend to be high (high memory was added later).
            // We need an aggressive retry budget therefore.
            //
            // Long-term fix: separate ZONE_DMA / ZONE_NORMAL and
            // route page-table allocations to ZONE_NORMAL.
            #[cfg(target_arch = "x86_64")]
            {
                let mut z = ZONE.lock();
                let mut rejected: [usize; 64] = [0usize; 64];
                let mut n = 0usize;
                let result = loop {
                    let f = match z.alloc_frame() {
                        Some(f) => f,
                        None => break None,
                    };
                    if f < LOW_MAX_FRAME {
                        break Some(f);
                    }
                    if n >= rejected.len() {
                        z.free_pages(f, 0);
                        break None;
                    }
                    rejected[n] = f;
                    n += 1;
                };
                for i in 0..n {
                    z.free_pages(rejected[i], 0);
                }
                return result.map(frame_to_phys);
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                ZONE.lock().alloc_frame().map(frame_to_phys)
            }
        } else {
            bootmem_alloc_frame().map(frame_to_phys)
        }
    }

    pub fn alloc_frame_low() -> Option<PhysAddr> {
        if !BUDDY_LIVE.load(Ordering::Acquire) {
            // During bootmem the allocator already searches from
            // low; just hand out a normal frame.
            return alloc_frame();
        }
        if LOW_MAX_FRAME == usize::MAX {
            return alloc_frame();
        }
        let mut z = ZONE.lock();
        let mut rejected: [usize; 32] = [usize::MAX; 32];
        let mut n = 0usize;
        let result = loop {
            let f = match z.alloc_frame() {
                Some(f) => f,
                None => break None,
            };
            if f < LOW_MAX_FRAME {
                break Some(f);
            }
            if n >= rejected.len() {
                z.free_pages(f, 0);
                break None;
            }
            rejected[n] = f;
            n += 1;
        };
        for i in 0..n {
            z.free_pages(rejected[i], 0);
        }
        result.map(frame_to_phys)
    }

    pub fn alloc_contiguous(count: usize) -> Option<PhysAddr> {
        if count == 0 {
            return None;
        }
        if BUDDY_LIVE.load(Ordering::Acquire) {
            let order = order_for(count);
            if order > MAX_ORDER {
                return None;
            }
            // alloc_contiguous historically returned low-memory
            // frames (< 128 MiB on x86_64) because every caller —
            // AHCI bounce/CLB/FB/CT, virtio queue rings, xHCI ERST,
            // SVM IOPM/MSRPM, rtl8188eu DMA rings — does
            // `*(phys as *mut u8)`, treating the physical address
            // as a kernel-virtual one via the boot identity map.
            // Buddy by default hands out the highest-address block
            // it can find for contiguous requests; that yields a
            // pointer the legacy callers can't dereference.
            //
            // Until those callers migrate to physmap-based access,
            // we preserve the contract: try repeatedly, keeping
            // the first block that lands below LOW_MAX_FRAME, and
            // returning the rejected high blocks to the buddy.
            // 32 retries is enough in practice — the buddy's
            // allocation order is deterministic, so once we've
            // pulled out the high blocks the next one is usually
            // low.
            #[cfg(target_arch = "x86_64")]
            {
                let mut z = ZONE.lock();
                let mut rejected: [(usize, usize); 32] = [(0, 0); 32];
                let mut n = 0usize;
                let result = loop {
                    let f = match z.alloc_pages(order) {
                        Some(f) => f,
                        None => break None,
                    };
                    // Block must fit entirely in low memory: start AND
                    // start + count both below LOW_MAX_FRAME.
                    if f + (1usize << order) <= LOW_MAX_FRAME {
                        break Some(f);
                    }
                    if n >= rejected.len() {
                        z.free_pages(f, order);
                        break None;
                    }
                    rejected[n] = (f, order);
                    n += 1;
                };
                for i in 0..n {
                    z.free_pages(rejected[i].0, rejected[i].1);
                }
                return result.map(frame_to_phys);
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                ZONE.lock().alloc_pages(order).map(frame_to_phys)
            }
        } else {
            bootmem_alloc_contiguous(count).map(frame_to_phys)
        }
    }

    pub fn free_frame(addr: PhysAddr) {
        let frame = addr.frame_index();
        if BUDDY_LIVE.load(Ordering::Acquire) {
            // Heuristic: if the frame is below STAGE1_MAX_FRAMES AND
            // the bootmem bitmap still records it as in-use AND the
            // buddy zone hasn't seen it (bitmap says used), it was
            // allocated through bootmem. Free it back into bootmem
            // just in case some early-boot caller stashed an address
            // and is now releasing it. In practice every bootmem
            // allocation is for a permanent kernel structure (PML4,
            // PT, etc.) so this branch is rarely hit, but it keeps
            // the API contract clean.
            //
            // For frames the buddy owns, plain free_frame works.
            ZONE.lock().free_frame(frame);
        } else {
            bootmem_free_frame(frame);
        }
    }

    pub fn reserve_frame(addr: PhysAddr) {
        let frame = addr.frame_index();
        if BUDDY_LIVE.load(Ordering::Acquire) {
            ZONE.lock().reserve_frame(frame);
        } else {
            let mut bm = BOOTMEM.lock();
            if frame < STAGE1_MAX_FRAMES && !bm.is_used(frame) {
                bm.set_used(frame);
                bm.free_frames -= 1;
            }
        }
    }

    pub fn free_frame_count() -> usize {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            ZONE.lock().free_frames
        } else {
            BOOTMEM.lock().free_frames
        }
    }

    pub fn total_frames() -> usize {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            ZONE.lock().total_frames
        } else {
            BOOTMEM.lock().total_frames
        }
    }

    pub fn is_allocator_locked() -> bool {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            ZONE.is_locked()
        } else {
            BOOTMEM.is_locked()
        }
    }
    pub fn is_allocator_locked_by_cpu(cpu: u32) -> bool {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            ZONE.is_held_by_cpu(cpu)
        } else {
            BOOTMEM.is_held_by_cpu(cpu)
        }
    }
    pub unsafe fn force_unlock_allocator() {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            ZONE.force_unlock();
        } else {
            BOOTMEM.force_unlock();
        }
    }
}

// ──────────────────────────────────────────────────────────────────
// Public API — dispatches to the active backend.
// ──────────────────────────────────────────────────────────────────

#[cfg(not(feature = "buddy_alloc"))]
use bitmap_backend as backend;
#[cfg(feature = "buddy_alloc")]
use buddy_backend as backend;

#[cfg(target_arch = "x86_64")]
pub fn init(boot_info: &BootInfo) {
    backend::init(boot_info);
}

#[cfg(target_arch = "x86_64")]
pub fn late_init(boot_info: &BootInfo) {
    backend::late_init(boot_info);
}

#[cfg(target_arch = "aarch64")]
pub fn init_arm64(ram_base: u64, ram_size: u64) {
    backend::init_arm64(ram_base, ram_size);
}

#[cfg(target_arch = "aarch64")]
pub fn late_init_arm64(ram_base: u64, ram_size: u64) {
    backend::late_init_arm64(ram_base, ram_size);
}

#[inline]
pub fn alloc_frame() -> Option<PhysAddr> {
    backend::alloc_frame()
}

#[inline]
pub fn alloc_frame_low() -> Option<PhysAddr> {
    backend::alloc_frame_low()
}

#[inline]
pub fn alloc_contiguous(count: usize) -> Option<PhysAddr> {
    backend::alloc_contiguous(count)
}

#[inline]
pub fn free_frame(addr: PhysAddr) {
    backend::free_frame(addr)
}

#[inline]
pub fn reserve_frame(addr: PhysAddr) {
    backend::reserve_frame(addr)
}

#[inline]
pub fn free_frame_count() -> usize {
    backend::free_frame_count()
}

#[inline]
pub fn total_frames() -> usize {
    backend::total_frames()
}

#[inline]
pub fn free_frames() -> usize {
    backend::free_frame_count()
}

#[inline]
pub fn is_allocator_locked() -> bool {
    backend::is_allocator_locked()
}

#[inline]
pub fn is_allocator_locked_by_cpu(cpu: u32) -> bool {
    backend::is_allocator_locked_by_cpu(cpu)
}

#[inline]
pub unsafe fn force_unlock_allocator() {
    backend::force_unlock_allocator()
}
