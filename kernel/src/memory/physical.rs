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

    const KERNEL_VIRT_BASE: u64 = 0xFFFF_FFFF_8000_0000;

    extern "C" {
        static _kernel_end: u8;
    }

    /// Single global buddy zone. Per-CPU caches are a follow-up commit.
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

    /// Stage 1: register only frames reachable through the boot
    /// identity map (lower 128 MiB on x86_64). The buddy needs to
    /// write intrusive link words into every freed page, and
    /// physmap (which would let us reach high frames) hasn't been
    /// initialised yet — `physmap::phys_to_virt_or_identity` falls
    /// back to identity-map and would page-fault on > 128 MiB.
    ///
    /// `late_init` runs after physmap is up and brings in the rest.
    /// Until then, virtual_mem::init has the lower 128 MiB to play
    /// with, which is more than enough for its PML4/PT setup.
    pub fn init(boot_info: &BootInfo) {
        let memory_map = unsafe { boot_info.memory_map() };
        let mut z = ZONE.lock();

        // Register USABLE regions, but TRUNCATE each to the lower
        // 128 MiB. The high portions are deferred to late_init.
        let stage1_max = LOW_MAX_FRAME;
        for entry in memory_map {
            if entry.entry_type != E820_TYPE_USABLE {
                continue;
            }
            let start = PhysAddr::new(entry.base_addr).frame_align_up();
            let end = PhysAddr::new(entry.base_addr + entry.length).frame_align_down();
            if start.as_u64() >= end.as_u64() {
                continue;
            }
            let s = start.frame_index();
            let e = end.frame_index().min(stage1_max);
            if s >= e {
                continue;
            }
            z.add_free_region(s, e);
        }

        // Reserve first 2 MiB (BIOS, real-mode trampoline, bootloader).
        let first_mb_frames = (2 * 1024 * 1024) / FRAME_SIZE;
        z.reserve_range(0, first_mb_frames);

        // Reserve kernel image (text + data + bss + .boot_stack).
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
        z.reserve_range(ks, ke);

        crate::serial_verbose_println!(
            "Reserving kernel region: {:#010x} - {:#010x} [buddy stage1]",
            kernel_start_phys,
            kernel_end_phys
        );
        let stage1_free_mib = z.free_frames * FRAME_SIZE / (1024 * 1024);
        crate::serial_verbose_println!(
            "Physical memory stage 1: {} MiB free below 128 MiB [buddy]",
            stage1_free_mib
        );
    }

    /// Stage 2: now that physmap is live, register the high portion
    /// of every USABLE region (frames ≥ LOW_MAX_FRAME). Buddy can
    /// write link words into those pages because physmap maps them
    /// at PHYSMAP_BASE + phys.
    pub fn late_init(boot_info: &BootInfo) {
        let memory_map = unsafe { boot_info.memory_map() };
        let mut z = ZONE.lock();
        let stage1_max = LOW_MAX_FRAME;

        let before = z.free_frames;
        for entry in memory_map {
            if entry.entry_type != E820_TYPE_USABLE {
                continue;
            }
            let start = PhysAddr::new(entry.base_addr).frame_align_up();
            let end = PhysAddr::new(entry.base_addr + entry.length).frame_align_down();
            if start.as_u64() >= end.as_u64() {
                continue;
            }
            let s = start.frame_index().max(stage1_max);
            let e = end.frame_index();
            if s >= e {
                continue;
            }
            z.add_free_region(s, e);
        }
        let added_mib = (z.free_frames - before) * FRAME_SIZE / (1024 * 1024);
        let total_mib = z.total_frames * FRAME_SIZE / (1024 * 1024);
        let free_mib = z.free_frames * FRAME_SIZE / (1024 * 1024);
        crate::serial_verbose_println!(
            "Physical memory stage 2: +{} MiB above 128 MiB; total {} MiB ({} MiB free) [buddy]",
            added_mib,
            total_mib,
            free_mib
        );
    }

    pub fn init_arm64(ram_base: u64, ram_size: u64) {
        const ARM64_PHYS_TO_VIRT: u64 = 0xFFFF_0000_4000_0000;
        let ram_end = ram_base + ram_size;
        let start_frame = (ram_base as usize) / FRAME_SIZE;
        let end_frame = (ram_end as usize) / FRAME_SIZE;

        // ARM64 already has TTBR1 mapping all RAM, so physmap is
        // effectively live from the start. We can register everything
        // in stage 1.
        let mut z = ZONE.lock();
        z.add_free_region(start_frame, end_frame);

        let kernel_end_virt = unsafe { &_kernel_end as *const u8 as u64 };
        let kernel_end_phys = kernel_end_virt - ARM64_PHYS_TO_VIRT;
        let kernel_end_frame =
            ((kernel_end_phys as usize) + FRAME_SIZE - 1) / FRAME_SIZE;
        z.reserve_range(start_frame, kernel_end_frame);

        let free_mib = z.free_frames * FRAME_SIZE / (1024 * 1024);
        crate::serial_verbose_println!(
            "Physical memory: {} MiB RAM ({:#010x}-{:#010x}), {} MiB free [buddy]",
            ram_size / (1024 * 1024),
            ram_base,
            ram_end,
            free_mib
        );
    }

    pub fn late_init_arm64(_ram_base: u64, _ram_size: u64) {}

    #[inline]
    fn frame_to_phys(frame: usize) -> PhysAddr {
        PhysAddr::new((frame * FRAME_SIZE) as u64)
    }

    pub fn alloc_frame() -> Option<PhysAddr> {
        ZONE.lock().alloc_frame().map(frame_to_phys)
    }

    pub fn alloc_frame_low() -> Option<PhysAddr> {
        // Buddy doesn't have a built-in "below address X" knob (Linux
        // would call this a separate ZONE_DMA). For our use cases —
        // x86 hardware-virt structures, real-mode trampoline frames —
        // we walk the order-0 free list looking for a frame below
        // LOW_MAX_FRAME. Fallback to a small linear retry budget so
        // we don't spin if low memory is exhausted.
        if LOW_MAX_FRAME == usize::MAX {
            return alloc_frame();
        }
        let mut z = ZONE.lock();
        // Try up to 32 order-0 allocations; keep any low-memory
        // frame, push back the others as free at order 0. If the
        // first try is below LOW_MAX_FRAME we exit immediately —
        // common case once buddy returns a young free leaf.
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
                // Give back the last sample too and bail; better to
                // fail than to drain the entire allocator looking
                // for a low frame.
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
        let order = order_for(count);
        if order > MAX_ORDER {
            return None;
        }
        ZONE.lock().alloc_pages(order).map(frame_to_phys)
    }

    pub fn free_frame(addr: PhysAddr) {
        let frame = addr.frame_index();
        ZONE.lock().free_frame(frame);
    }

    pub fn reserve_frame(addr: PhysAddr) {
        let frame = addr.frame_index();
        ZONE.lock().reserve_frame(frame);
    }

    pub fn free_frame_count() -> usize {
        ZONE.lock().free_frames
    }

    pub fn total_frames() -> usize {
        ZONE.lock().total_frames
    }

    pub fn is_allocator_locked() -> bool {
        ZONE.is_locked()
    }
    pub fn is_allocator_locked_by_cpu(cpu: u32) -> bool {
        ZONE.is_held_by_cpu(cpu)
    }
    pub unsafe fn force_unlock_allocator() {
        ZONE.force_unlock();
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
