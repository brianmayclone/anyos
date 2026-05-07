//! Physical frame allocator.
//!
//! Two-stage init followed by a buddy allocator with two zones:
//!
//! * **Stage 1 — bootmem**: a tiny per-frame bitmap that comes up
//!   immediately after kernel entry. Used by `virtual_mem::init`,
//!   `physmap::init`, ACPI parsing, etc. — anything that runs before
//!   the buddy can write its intrusive free-list link words into
//!   freed pages safely. Only ~512 KiB BSS at the 16 GiB cap, no
//!   per-page side effects.
//!
//! * **Stage 2 — buddy live**: after physmap is up, `late_init`
//!   migrates every still-free bootmem frame into one of two
//!   `BuddyZone`s. ZONE_DMA covers `[0, 64 MiB)` (the identity-map
//!   region present in both kernel and user CR3s); ZONE_NORMAL
//!   covers everything above. Every subsequent alloc/free goes
//!   through the buddy: O(MAX_ORDER), auto-coalescing, no
//!   fragmentation drift.
//!
//! Public API:
//!
//! ```ignore
//! pub enum FrameAllocPolicy { LowIdentity, Any }
//! pub fn alloc_frame() -> Option<PhysAddr>
//! pub fn alloc_frame_with(policy: FrameAllocPolicy) -> Option<PhysAddr>
//! pub fn alloc_contiguous(count: usize) -> Option<PhysAddr>
//! pub fn alloc_contiguous_with(count: usize, policy: FrameAllocPolicy) -> Option<PhysAddr>
//! pub fn alloc_frame_low() -> Option<PhysAddr>     // x86 only
//! pub fn free_frame(addr: PhysAddr)
//! pub fn reserve_frame(addr: PhysAddr)
//! pub fn free_frame_count() -> usize
//! pub fn free_frames() -> usize
//! pub fn total_frames() -> usize
//! pub fn init(boot_info: &BootInfo)            // x86_64 only
//! pub fn late_init(boot_info: &BootInfo)       // x86_64 only
//! pub fn init_arm64(ram_base: u64, ram_size: u64)
//! pub fn late_init_arm64(ram_base: u64, ram_size: u64)
//! pub fn is_allocator_locked() -> bool
//! pub fn is_allocator_locked_by_cpu(cpu: u32) -> bool
//! pub unsafe fn force_unlock_allocator()
//! ```

use crate::boot_info::BootInfo;
use crate::memory::address::PhysAddr;
use crate::memory::FRAME_SIZE;

/// Contract requested from the physical allocator.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FrameAllocPolicy {
    /// Return a frame that legacy code may safely touch as `phys as *mut`.
    ///
    /// On x86_64 this means the frame must live below the identity range that
    /// every CR3 carries. Kernel CR3 maps more, but user page directories keep
    /// only the first 64 MiB identity-mapped because 64-128 MiB is user/DLL
    /// virtual address space.
    LowIdentity,
    /// Return any RAM frame. Callers using this must access it through an
    /// explicit mapping, physmap, or a temporary alias.
    Any,
}

// ──────────────────────────────────────────────────────────────────
// Backend implementation
// ──────────────────────────────────────────────────────────────────

mod backend {
    use super::*;
    use crate::boot_info::E820_TYPE_USABLE;
    use crate::memory::buddy::{BuddyZone, MAX_ORDER};
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
    /// Bootmem stage-1 covers the same physmem ceiling as the buddy.
    /// Lifting one without the other would mean the bitmap can't
    /// represent every frame the buddy will eventually own — late_init
    /// would silently drop frames above the bootmem cap.
    const STAGE1_MAX_MEMORY: usize = 16 * 1024 * 1024 * 1024;
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

    /// Two buddy zones, split by physical address. ZONE_DMA covers
    /// the lower 64 MiB on x86_64 (the identity-map range present in every
    /// CR3 — required by legacy "phys as *mut" callers). ZONE_NORMAL
    /// covers everything above. On ARM64 there's only ZONE_NORMAL
    /// because TTBR1 maps all RAM uniformly.
    ///
    /// `FrameAllocPolicy::LowIdentity` allocates only from ZONE_DMA and fails
    /// when that low window is exhausted. `FrameAllocPolicy::Any` prefers
    /// ZONE_NORMAL and falls back to ZONE_DMA, which preserves low memory for
    /// device buffers and legacy raw physical pointer users.
    static ZONE_DMA: Spinlock<BuddyZone> = Spinlock::new(BuddyZone::new());
    static ZONE_NORMAL: Spinlock<BuddyZone> = Spinlock::new(BuddyZone::new());

    /// Legacy phys-as-virt callers must stay below the identity range that every
    /// CR3 carries. Kernel CR3 maps 128 MiB, but user page directories copy only
    /// the first 64 MiB because 64-128 MiB is reserved for DLL/user mappings.
    /// Syscalls and IRQs can run under a user CR3, so raw physical pointers are
    /// only universally safe below 64 MiB.
    #[cfg(target_arch = "x86_64")]
    const LOW_MAX_FRAME: usize = (64 * 1024 * 1024) / FRAME_SIZE;
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
        let ks = PhysAddr::new(kernel_start_phys)
            .frame_align_down()
            .frame_index();
        let ke = PhysAddr::new(kernel_end_phys)
            .frame_align_up()
            .frame_index();
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
        let mut zd = ZONE_DMA.lock();
        let mut zn = ZONE_NORMAL.lock();
        let limit = bm.address_frames;

        // Find every free run in the bootmem bitmap. Each run is then
        // split at LOW_MAX_FRAME and the parts handed to the
        // appropriate zone: ZONE_DMA gets the portion below the
        // identity-map boundary, ZONE_NORMAL gets the rest.
        let mut migrated: usize = 0;
        let mut run_start: Option<usize> = None;
        let dma_max = LOW_MAX_FRAME;
        for f in 0..limit {
            if !bm.is_used(f) {
                if run_start.is_none() {
                    run_start = Some(f);
                }
            } else if let Some(s) = run_start.take() {
                migrate_run(&mut zd, &mut zn, &mut bm, s, f, dma_max);
                migrated += f - s;
            }
        }
        if let Some(s) = run_start {
            migrate_run(&mut zd, &mut zn, &mut bm, s, limit, dma_max);
            migrated += limit - s;
        }
        bm.free_frames = 0;

        let migrated_mib = migrated * FRAME_SIZE / (1024 * 1024);
        let dma_mib = zd.free_frames * FRAME_SIZE / (1024 * 1024);
        let normal_mib = zn.free_frames * FRAME_SIZE / (1024 * 1024);
        crate::serial_verbose_println!(
            "Physical memory: migrated {} MiB into buddy (DMA {} MiB / NORMAL {} MiB) [buddy live]",
            migrated_mib,
            dma_mib,
            normal_mib
        );

        BUDDY_LIVE.store(true, Ordering::Release);
    }

    /// Push a contiguous bootmem run into the appropriate buddy
    /// zone(s), splitting at the DMA/NORMAL boundary if necessary.
    /// Marks the migrated frames as used in the bootmem bitmap so
    /// subsequent passes don't double-count.
    fn migrate_run(
        zd: &mut BuddyZone,
        zn: &mut BuddyZone,
        bm: &mut Bootmem,
        s: usize,
        e: usize,
        dma_max: usize,
    ) {
        if e <= s {
            return;
        }
        if dma_max == usize::MAX {
            // ARM64: no zone split.
            zd.add_free_region(s, e);
            for ff in s..e {
                bm.set_used(ff);
            }
            return;
        }
        if e <= dma_max {
            zd.add_free_region(s, e);
        } else if s >= dma_max {
            zn.add_free_region(s, e);
        } else {
            zd.add_free_region(s, dma_max);
            zn.add_free_region(dma_max, e);
        }
        for ff in s..e {
            bm.set_used(ff);
        }
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
        let mut zd = ZONE_DMA.lock();
        let mut zn = ZONE_NORMAL.lock();
        let limit = bm.address_frames;
        let dma_max = LOW_MAX_FRAME;

        let mut run_start: Option<usize> = None;
        for f in 0..limit {
            if !bm.is_used(f) {
                if run_start.is_none() {
                    run_start = Some(f);
                }
            } else if let Some(s) = run_start.take() {
                migrate_run(&mut zd, &mut zn, &mut bm, s, f, dma_max);
            }
        }
        if let Some(s) = run_start {
            migrate_run(&mut zd, &mut zn, &mut bm, s, limit, dma_max);
        }
        bm.free_frames = 0;
        BUDDY_LIVE.store(true, Ordering::Release);
    }

    #[inline]
    fn frame_to_phys(frame: usize) -> PhysAddr {
        PhysAddr::new((frame * FRAME_SIZE) as u64)
    }

    #[inline]
    fn bootmem_limit_for(policy: FrameAllocPolicy, address_frames: usize) -> usize {
        match policy {
            FrameAllocPolicy::LowIdentity => address_frames.min(LOW_MAX_FRAME),
            FrameAllocPolicy::Any => address_frames,
        }
        .min(STAGE1_MAX_FRAMES)
    }

    /// Bootmem alloc_frame: simple next-fit linear scan over the stage-1
    /// bitmap. Used until BUDDY_LIVE flips. The policy is still honoured here
    /// so the public low-memory contract does not change during boot.
    fn bootmem_alloc_frame_with(policy: FrameAllocPolicy) -> Option<usize> {
        let mut bm = BOOTMEM.lock();
        let limit = bootmem_limit_for(policy, bm.address_frames);
        if bm.free_frames == 0 {
            return None;
        }
        let start = bm.next_search.min(limit);
        for i in start..limit {
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

    fn bootmem_alloc_contiguous_with(count: usize, policy: FrameAllocPolicy) -> Option<usize> {
        if count == 0 {
            return None;
        }
        let mut bm = BOOTMEM.lock();
        let limit = bootmem_limit_for(policy, bm.address_frames);
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

    pub fn alloc_frame_with(policy: FrameAllocPolicy) -> Option<PhysAddr> {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            match policy {
                FrameAllocPolicy::LowIdentity => {
                    // Legacy default: callers may dereference `phys as *mut`
                    // under a user CR3, so never silently fall back above
                    // LOW_MAX_FRAME.
                    ZONE_DMA.lock().alloc_frame().map(frame_to_phys)
                }
                FrameAllocPolicy::Any => {
                    let normal = ZONE_NORMAL.lock().alloc_frame().map(frame_to_phys);
                    if normal.is_some() {
                        return normal;
                    }
                    ZONE_DMA.lock().alloc_frame().map(frame_to_phys)
                }
            }
        } else {
            bootmem_alloc_frame_with(policy).map(frame_to_phys)
        }
    }

    pub fn alloc_frame() -> Option<PhysAddr> {
        alloc_frame_with(FrameAllocPolicy::LowIdentity)
    }

    pub fn alloc_frame_low() -> Option<PhysAddr> {
        alloc_frame_with(FrameAllocPolicy::LowIdentity)
    }

    pub fn alloc_contiguous_with(count: usize, policy: FrameAllocPolicy) -> Option<PhysAddr> {
        if count == 0 {
            return None;
        }
        if BUDDY_LIVE.load(Ordering::Acquire) {
            if count > (1usize << MAX_ORDER) {
                return None;
            }
            match policy {
                FrameAllocPolicy::LowIdentity => {
                    // Legacy default: contiguous callers are mostly DMA rings,
                    // bounce buffers, and raw phys pointers.
                    ZONE_DMA.lock().alloc_contiguous(count).map(frame_to_phys)
                }
                FrameAllocPolicy::Any => {
                    let normal = ZONE_NORMAL
                        .lock()
                        .alloc_contiguous(count)
                        .map(frame_to_phys);
                    if normal.is_some() {
                        return normal;
                    }
                    ZONE_DMA.lock().alloc_contiguous(count).map(frame_to_phys)
                }
            }
        } else {
            bootmem_alloc_contiguous_with(count, policy).map(frame_to_phys)
        }
    }

    pub fn alloc_contiguous(count: usize) -> Option<PhysAddr> {
        alloc_contiguous_with(count, FrameAllocPolicy::LowIdentity)
    }

    pub fn free_frame(addr: PhysAddr) {
        if !addr.is_frame_aligned() {
            crate::serial_verbose_println!(
                "  WARN: free_frame ignored unaligned phys={:#x}",
                addr.as_u64()
            );
            return;
        }
        let frame = addr.frame_index();
        if BUDDY_LIVE.load(Ordering::Acquire) {
            // Route to the zone that owns this frame. ZONE_DMA owns
            // [0, LOW_MAX_FRAME); ZONE_NORMAL owns the rest.
            if frame < LOW_MAX_FRAME {
                ZONE_DMA.lock().free_frame(frame);
            } else {
                ZONE_NORMAL.lock().free_frame(frame);
            }
        } else {
            bootmem_free_frame(frame);
        }
    }

    pub fn reserve_frame(addr: PhysAddr) {
        if !addr.is_frame_aligned() {
            crate::serial_verbose_println!(
                "  WARN: reserve_frame ignored unaligned phys={:#x}",
                addr.as_u64()
            );
            return;
        }
        let frame = addr.frame_index();
        if BUDDY_LIVE.load(Ordering::Acquire) {
            if frame < LOW_MAX_FRAME {
                ZONE_DMA.lock().reserve_frame(frame);
            } else {
                ZONE_NORMAL.lock().reserve_frame(frame);
            }
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
            ZONE_DMA.lock().free_frames + ZONE_NORMAL.lock().free_frames
        } else {
            BOOTMEM.lock().free_frames
        }
    }

    pub fn total_frames() -> usize {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            ZONE_DMA.lock().total_frames + ZONE_NORMAL.lock().total_frames
        } else {
            BOOTMEM.lock().total_frames
        }
    }

    pub fn is_allocator_locked() -> bool {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            ZONE_DMA.is_locked() || ZONE_NORMAL.is_locked()
        } else {
            BOOTMEM.is_locked()
        }
    }
    pub fn is_allocator_locked_by_cpu(cpu: u32) -> bool {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            ZONE_DMA.is_held_by_cpu(cpu) || ZONE_NORMAL.is_held_by_cpu(cpu)
        } else {
            BOOTMEM.is_held_by_cpu(cpu)
        }
    }
    pub unsafe fn force_unlock_allocator() {
        if BUDDY_LIVE.load(Ordering::Acquire) {
            ZONE_DMA.force_unlock();
            ZONE_NORMAL.force_unlock();
        } else {
            BOOTMEM.force_unlock();
        }
    }
}

// ──────────────────────────────────────────────────────────────────
// Public API
// ──────────────────────────────────────────────────────────────────

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
pub fn alloc_frame_with(policy: FrameAllocPolicy) -> Option<PhysAddr> {
    backend::alloc_frame_with(policy)
}

#[inline]
pub fn alloc_frame_low() -> Option<PhysAddr> {
    alloc_frame_with(FrameAllocPolicy::LowIdentity)
}

#[inline]
pub fn alloc_contiguous(count: usize) -> Option<PhysAddr> {
    backend::alloc_contiguous(count)
}

#[inline]
pub fn alloc_contiguous_with(count: usize, policy: FrameAllocPolicy) -> Option<PhysAddr> {
    backend::alloc_contiguous_with(count, policy)
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
