//! Physical frame allocator using a bitmap.
//!
//! Manages 4 KiB physical frames up to 64 GiB of RAM. The bitmap tracks
//! free/used state for each frame, initialized from the BIOS E820 memory map.
//! All operations are protected by an IRQ-safe spinlock for SMP safety.
//!
//! The bitmap is a static array in BSS (2 MiB), so it costs nothing until
//! the pages containing it are demand-faulted. With QEMU `-m 1024M`, only
//! the first 32 KiB of the bitmap is actually used.

use crate::boot_info::{BootInfo, E820_TYPE_USABLE};
use crate::memory::address::PhysAddr;
use crate::memory::FRAME_SIZE;
use crate::sync::spinlock::Spinlock;

/// Maximum supported physical memory (64 GiB).
const MAX_MEMORY: usize = 64 * 1024 * 1024 * 1024;
/// Total number of frames that can be tracked in the bitmap.
const MAX_FRAMES: usize = MAX_MEMORY / FRAME_SIZE;
/// Size of the bitmap in bytes (1 bit per frame, 2 MiB for 64 GiB).
const BITMAP_SIZE: usize = MAX_FRAMES / 8;

// Kernel virtual base (must match link.ld and boot.asm)
const KERNEL_VIRT_BASE: u64 = 0xFFFF_FFFF_8000_0000;

// _kernel_end from link.ld already includes the dedicated .boot_stack section,
// so no separate KERNEL_STACK_SIZE constant is needed here.

extern "C" {
    static _kernel_end: u8;
}

/// Physical frame allocator state.
///
/// Field order matters: `total_frames`/`free_frames` are placed BEFORE the
/// 2 MiB bitmap so they sit at the start of BSS.  The boot stack now lives in
/// a dedicated .boot_stack linker section above BSS, so stack overflow cannot
/// silently corrupt BSS statics.  The field ordering remains as an extra
/// safety margin in case of unexpected memory pressure.
#[repr(C)]
struct FrameAllocator {
    total_frames: usize,
    free_frames: usize,
    next_search: usize,
    bitmap: [u8; BITMAP_SIZE],
}

impl FrameAllocator {
    fn set_used(&mut self, frame: usize) {
        self.bitmap[frame / 8] |= 1 << (frame % 8);
    }

    fn set_free(&mut self, frame: usize) {
        self.bitmap[frame / 8] &= !(1 << (frame % 8));
    }

    fn is_used(&self, frame: usize) -> bool {
        self.bitmap[frame / 8] & (1 << (frame % 8)) != 0
    }
}

static ALLOCATOR: Spinlock<FrameAllocator> = Spinlock::new(FrameAllocator {
    bitmap:    [0; BITMAP_SIZE], // Zero-init → lives in BSS
    total_frames: 0,
    free_frames: 0,
    next_search: 0,
});

/// Initialize the physical frame allocator from the E820 memory map.
///
/// Marks usable regions as free, then reserves the first 2 MiB (legacy/bootloader area)
/// and the kernel's code, BSS, and stack regions.
pub fn init(boot_info: &BootInfo) {
    let memory_map = unsafe { boot_info.memory_map() };

    let mut alloc = ALLOCATOR.lock();

    // First pass: find highest usable address to size the bitmap.
    // Only consider USABLE entries — reserved MMIO regions (IOAPIC, LAPIC, etc.)
    // can have addresses far above actual RAM and would bloat total_frames.
    let mut max_usable_addr: u64 = 0;
    for entry in memory_map {
        if entry.entry_type == E820_TYPE_USABLE {
            let end = entry.base_addr + entry.length;
            if end > max_usable_addr {
                max_usable_addr = end;
            }
        }
    }

    // Cap at MAX_MEMORY
    if max_usable_addr > MAX_MEMORY as u64 {
        max_usable_addr = MAX_MEMORY as u64;
    }

    alloc.total_frames = (max_usable_addr as usize) / FRAME_SIZE;
    alloc.free_frames = 0;

    // Fill bitmap with 0xFF (all used) for the actual RAM range only.
    // The bitmap is zero-initialized in BSS — we only touch the bytes
    // corresponding to real physical memory, not the full 2 MiB.
    let bitmap_bytes_needed = (alloc.total_frames + 7) / 8;
    for byte in alloc.bitmap[..bitmap_bytes_needed].iter_mut() {
        *byte = 0xFF;
    }

    // Mark usable regions as free
    for entry in memory_map {
        if entry.entry_type != E820_TYPE_USABLE {
            continue;
        }

        let start = PhysAddr::new(entry.base_addr).frame_align_up();
        let end = PhysAddr::new(entry.base_addr + entry.length).frame_align_down();

        if start.as_u64() >= end.as_u64() {
            continue;
        }

        let start_frame = start.frame_index();
        let end_frame = end.frame_index();

        for frame in start_frame..end_frame {
            if frame < MAX_FRAMES {
                alloc.set_free(frame);
                alloc.free_frames += 1;
            }
        }
    }

    // Re-mark reserved regions as used:
    // - First 2 MiB (legacy area, bootloader, boot info, memory map,
    //   kernel at 1MB, and kernel stack growing down from physical 0x200000)
    let first_mb_frames = (2 * 1024 * 1024) / FRAME_SIZE;
    for frame in 0..first_mb_frames {
        if !alloc.is_used(frame) {
            alloc.set_used(frame);
            alloc.free_frames -= 1;
        }
    }

    // - Kernel region (including BSS which is not in the flat binary)
    //   _kernel_end from linker script is a virtual address; convert to physical
    let kernel_start = PhysAddr::new(boot_info.kernel_phys_start as u64).frame_align_down();
    let linker_kernel_end_phys = unsafe {
        (&_kernel_end as *const u8 as u64) - KERNEL_VIRT_BASE
    };
    // Use the larger of BootInfo's kernel_phys_end and the linker's _kernel_end
    let kernel_end_phys = if linker_kernel_end_phys > boot_info.kernel_phys_end as u64 {
        linker_kernel_end_phys
    } else {
        boot_info.kernel_phys_end as u64
    };
    // _kernel_end already covers BSS + the 512 KiB .boot_stack section.
    let kernel_end = PhysAddr::new(kernel_end_phys).frame_align_up();
    crate::serial_println!(
        "Reserving kernel region: {:#010x} - {:#010x} (includes BSS + stack)",
        kernel_start.as_u64(), kernel_end.as_u64()
    );
    for frame in kernel_start.frame_index()..kernel_end.frame_index() {
        if frame < MAX_FRAMES && !alloc.is_used(frame) {
            alloc.set_used(frame);
            alloc.free_frames -= 1;
        }
    }

    crate::serial_println!(
        "Physical memory: {} MiB total, {} frames free ({} MiB)",
        alloc.total_frames * FRAME_SIZE / (1024 * 1024),
        alloc.free_frames,
        alloc.free_frames * FRAME_SIZE / (1024 * 1024)
    );
}

/// Allocate a single 4 KiB physical frame, returning its physical address.
///
/// Uses next-fit: resumes scanning from where the last allocation left off.
/// This makes sequential allocations O(1) amortized instead of O(n).
pub fn alloc_frame() -> Option<PhysAddr> {
    let mut alloc = ALLOCATOR.lock();
    let total = alloc.total_frames;
    if alloc.free_frames == 0 {
        return None;
    }
    let start = alloc.next_search;
    // Search from next_search to end
    for i in start..total {
        if !alloc.is_used(i) {
            alloc.set_used(i);
            alloc.free_frames -= 1;
            alloc.next_search = i + 1;
            return Some(PhysAddr::new((i * FRAME_SIZE) as u64));
        }
    }
    // Wrap around: search from 0 to start
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

/// Free a physical frame, returning it to the allocator pool.
///
/// Has no effect if the frame is already free.
pub fn free_frame(addr: PhysAddr) {
    let mut alloc = ALLOCATOR.lock();
    let frame = addr.frame_index();
    if alloc.is_used(frame) {
        alloc.set_free(frame);
        alloc.free_frames += 1;
        // Hint allocator to reuse freed region sooner
        if frame < alloc.next_search {
            alloc.next_search = frame;
        }
    }
}

/// Returns the number of free physical frames currently available.
pub fn free_frame_count() -> usize {
    ALLOCATOR.lock().free_frames
}

/// Lock-free check if the physical frame allocator lock is currently held.
pub fn is_allocator_locked() -> bool {
    ALLOCATOR.is_locked()
}

/// Check if this CPU holds the allocator lock.
pub fn is_allocator_locked_by_cpu(cpu: u32) -> bool {
    ALLOCATOR.is_held_by_cpu(cpu)
}

/// Force-release the allocator lock unconditionally.
///
/// # Safety
/// Must only be called when `is_allocator_locked_by_cpu(cpu)` returns true
/// for the current CPU. The allocator's internal state may be inconsistent.
pub unsafe fn force_unlock_allocator() {
    ALLOCATOR.force_unlock();
}

/// Returns the number of free physical frames (alias for [`free_frame_count`]).
pub fn free_frames() -> usize {
    ALLOCATOR.lock().free_frames
}

/// Maximum physical address for contiguous allocations (128 MiB).
///
/// Contiguous allocations are used for DMA buffers and GPU framebuffers that are
/// accessed via identity mapping (physical address == virtual address). The kernel
/// identity-maps the first 128 MiB of physical memory, so contiguous allocations
/// must not exceed this boundary.
const CONTIGUOUS_MAX_FRAME: usize = (128 * 1024 * 1024) / FRAME_SIZE; // frame 32768

/// Allocate `count` physically contiguous 4 KiB frames.
///
/// Scans the bitmap for a run of `count` consecutive free frames (first-fit),
/// constrained to the identity-mapped region (< 128 MiB) so the result is
/// safely accessible via identity mapping from any CR3 context.
/// Returns the physical address of the first frame, or `None` if unavailable.
pub fn alloc_contiguous(count: usize) -> Option<PhysAddr> {
    if count == 0 {
        return None;
    }
    let mut alloc = ALLOCATOR.lock();
    let limit = alloc.total_frames.min(CONTIGUOUS_MAX_FRAME);
    let mut run_start = 0usize;
    let mut run_len = 0usize;
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

/// Returns the total number of physical frames tracked by the allocator.
pub fn total_frames() -> usize {
    ALLOCATOR.lock().total_frames
}

/// Reserve (mark as used) a specific physical frame.
///
/// Used on ARM64 where the kernel heap is pre-mapped by a 1 GiB block
/// and the backing frames must be excluded from allocation.
/// Has no effect if the frame is already marked as used.
pub fn reserve_frame(addr: PhysAddr) {
    let mut alloc = ALLOCATOR.lock();
    let frame = addr.frame_index();
    if frame < MAX_FRAMES && !alloc.is_used(frame) {
        alloc.set_used(frame);
        alloc.free_frames -= 1;
    }
}

/// Initialize the physical frame allocator for ARM64 (QEMU virt machine).
///
/// ARM64 does not have an E820 memory map — RAM layout comes from DTB or
/// is hardcoded for the QEMU virt platform (RAM at 0x4000_0000).
#[cfg(target_arch = "aarch64")]
pub fn init_arm64(ram_base: u64, ram_size: u64) {
    // Kernel virtual-to-physical offset (matching boot.S 1 GiB block mapping)
    const ARM64_PHYS_TO_VIRT: u64 = 0xFFFF_0000_4000_0000;

    let ram_end = ram_base + ram_size;
    let start_frame = (ram_base as usize) / FRAME_SIZE;
    let end_frame = (ram_end as usize) / FRAME_SIZE;

    let mut alloc = ALLOCATOR.lock();

    alloc.total_frames = end_frame;
    alloc.free_frames = 0;

    // Mark everything up to end_frame as used (0xFF = all bits set).
    let bitmap_bytes = (end_frame + 7) / 8;
    for byte in alloc.bitmap[..bitmap_bytes].iter_mut() {
        *byte = 0xFF;
    }

    // Free the RAM region.
    for frame in start_frame..end_frame {
        if frame < MAX_FRAMES {
            alloc.set_free(frame);
            alloc.free_frames += 1;
        }
    }

    // Reserve kernel region: from RAM base to _kernel_end (physical).
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

    crate::serial_println!(
        "Physical memory: {} MiB RAM ({:#010x}-{:#010x}), {} frames free ({} MiB)",
        ram_size / (1024 * 1024),
        ram_base,
        ram_end,
        alloc.free_frames,
        alloc.free_frames * FRAME_SIZE / (1024 * 1024)
    );
    crate::serial_println!(
        "  Kernel region reserved: {:#010x}-{:#010x}",
        ram_base,
        kernel_end_phys
    );
}

