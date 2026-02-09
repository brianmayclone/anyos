//! Physical frame allocator using a bitmap.
//!
//! Manages 4 KiB physical frames up to 1 GiB of RAM. The bitmap tracks
//! free/used state for each frame, initialized from the BIOS E820 memory map.

use crate::boot_info::{BootInfo, E820_TYPE_USABLE};
use crate::memory::address::PhysAddr;
use crate::memory::FRAME_SIZE;

/// Maximum supported physical memory (1 GiB).
const MAX_MEMORY: usize = 1024 * 1024 * 1024;
/// Total number of frames that can be tracked in the bitmap.
const MAX_FRAMES: usize = MAX_MEMORY / FRAME_SIZE;
/// Size of the bitmap in bytes (1 bit per frame).
const BITMAP_SIZE: usize = MAX_FRAMES / 8;

// Kernel virtual base (must match link.ld and boot.asm)
const KERNEL_VIRT_BASE: u64 = 0xFFFF_FFFF_8000_0000;

// Kernel stack placed above BSS (must match KERNEL_STACK_SIZE in boot.asm)
const KERNEL_STACK_SIZE: u64 = 0x10000; // 64 KiB

extern "C" {
    static _kernel_end: u8;
}

// Bitmap: 1 = used, 0 = free
static mut BITMAP: [u8; BITMAP_SIZE] = [0xFF; BITMAP_SIZE]; // All used initially
static mut TOTAL_FRAMES: usize = 0;
static mut FREE_FRAMES: usize = 0;

fn set_used(frame: usize) {
    unsafe {
        BITMAP[frame / 8] |= 1 << (frame % 8);
    }
}

fn set_free(frame: usize) {
    unsafe {
        BITMAP[frame / 8] &= !(1 << (frame % 8));
    }
}

fn is_used(frame: usize) -> bool {
    unsafe { BITMAP[frame / 8] & (1 << (frame % 8)) != 0 }
}

/// Initialize the physical frame allocator from the E820 memory map.
///
/// Marks usable regions as free, then reserves the first 2 MiB (legacy/bootloader area)
/// and the kernel's code, BSS, and stack regions.
pub fn init(boot_info: &BootInfo) {
    let memory_map = unsafe { boot_info.memory_map() };

    // First pass: find total memory
    let mut max_addr: u64 = 0;
    for entry in memory_map {
        let end = entry.base_addr + entry.length;
        if end > max_addr {
            max_addr = end;
        }
    }

    // Cap at MAX_MEMORY
    if max_addr > MAX_MEMORY as u64 {
        max_addr = MAX_MEMORY as u64;
    }

    unsafe {
        TOTAL_FRAMES = (max_addr as usize) / FRAME_SIZE;
        FREE_FRAMES = 0;
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
                set_free(frame);
                unsafe { FREE_FRAMES += 1; }
            }
        }
    }

    // Re-mark reserved regions as used:
    // - First 2 MiB (legacy area, bootloader, boot info, memory map,
    //   kernel at 1MB, and kernel stack growing down from physical 0x200000)
    let first_mb_frames = (2 * 1024 * 1024) / FRAME_SIZE;
    for frame in 0..first_mb_frames {
        if !is_used(frame) {
            set_used(frame);
            unsafe { FREE_FRAMES -= 1; }
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
    // Reserve BSS + kernel stack area (stack is placed above BSS in boot.asm)
    let kernel_end = PhysAddr::new(kernel_end_phys + KERNEL_STACK_SIZE).frame_align_up();
    crate::serial_println!(
        "Reserving kernel region: {:#010x} - {:#010x} (includes BSS + stack)",
        kernel_start.as_u64(), kernel_end.as_u64()
    );
    for frame in kernel_start.frame_index()..kernel_end.frame_index() {
        if frame < MAX_FRAMES && !is_used(frame) {
            set_used(frame);
            unsafe { FREE_FRAMES -= 1; }
        }
    }

    crate::serial_println!(
        "Physical memory: {} MiB total, {} frames free ({} MiB)",
        unsafe { TOTAL_FRAMES } * FRAME_SIZE / (1024 * 1024),
        unsafe { FREE_FRAMES },
        unsafe { FREE_FRAMES } * FRAME_SIZE / (1024 * 1024)
    );
}

/// Allocate a single 4 KiB physical frame, returning its physical address.
///
/// Uses a linear scan of the bitmap (first-fit). Returns `None` if no frames are available.
pub fn alloc_frame() -> Option<PhysAddr> {
    unsafe {
        for i in 0..TOTAL_FRAMES {
            if !is_used(i) {
                set_used(i);
                FREE_FRAMES -= 1;
                return Some(PhysAddr::new((i * FRAME_SIZE) as u64));
            }
        }
        None
    }
}

/// Free a previously allocated physical frame, returning it to the pool.
pub fn free_frame(addr: PhysAddr) {
    let frame = addr.frame_index();
    if is_used(frame) {
        set_free(frame);
        unsafe { FREE_FRAMES += 1; }
    }
}

/// Returns the number of free physical frames currently available.
pub fn free_frame_count() -> usize {
    unsafe { FREE_FRAMES }
}

/// Returns the number of free physical frames (alias for [`free_frame_count`]).
pub fn free_frames() -> usize {
    unsafe { FREE_FRAMES }
}

/// Allocate `count` physically contiguous 4 KiB frames.
///
/// Scans the bitmap for a run of `count` consecutive free frames (first-fit).
/// Returns the physical address of the first frame, or `None` if unavailable.
pub fn alloc_contiguous(count: usize) -> Option<PhysAddr> {
    if count == 0 {
        return None;
    }
    unsafe {
        let mut run_start = 0usize;
        let mut run_len = 0usize;
        for i in 0..TOTAL_FRAMES {
            if !is_used(i) {
                if run_len == 0 {
                    run_start = i;
                }
                run_len += 1;
                if run_len >= count {
                    for j in run_start..run_start + count {
                        set_used(j);
                        FREE_FRAMES -= 1;
                    }
                    return Some(PhysAddr::new((run_start * FRAME_SIZE) as u64));
                }
            } else {
                run_len = 0;
            }
        }
        None
    }
}

/// Returns the total number of physical frames tracked by the allocator.
pub fn total_frames() -> usize {
    unsafe { TOTAL_FRAMES }
}
