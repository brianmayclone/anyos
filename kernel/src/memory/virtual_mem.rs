//! Virtual memory manager using four-level x86-64 paging with recursive mapping.
//!
//! The bootloader sets up initial 2 MiB page mappings. This module takes over,
//! creating fine-grained 4 KiB page management with PML4 entry 510 as a recursive
//! self-map for in-place page table manipulation.
//!
//! Kernel space: PML4[256..511] (upper canonical half, 0xFFFF800000000000+)
//! User space:   PML4[0..255]   (lower canonical half, 0x0000000000000000+)

use crate::boot_info::BootInfo;
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::physical;
use crate::memory::FRAME_SIZE;
use core::arch::asm;

/// Page table entry flag: page is present in physical memory.
const PAGE_PRESENT: u64 = 1 << 0;
/// Page table entry flag: page is writable.
const PAGE_WRITABLE: u64 = 1 << 1;
/// Page table entry flag: page is accessible from Ring 3 (user mode).
const PAGE_USER: u64 = 1 << 2;

/// Number of entries in a page table (512 for x86-64).
const ENTRIES_PER_TABLE: usize = 512;

/// Mask to extract the physical address from a page table entry (bits 12..51).
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// Kernel higher-half virtual base (must match link.ld).
const KERNEL_VIRT_BASE: u64 = 0xFFFF_FFFF_8000_0000;

/// Recursive mapping index in PML4 (entry 510).
/// PML4[510] points to the PML4 itself, providing access to all page tables.
const RECURSIVE_INDEX: usize = 510;

// ---- Recursive mapping virtual address computation ----
//
// With PML4[510] = self-reference, we can construct virtual addresses that
// map to any level of the page table hierarchy:
//
// To access PTE for vaddr:
//   recursive_pt_addr(vaddr) = sign_extend(510 << 39 | pml4i << 30 | pdpti << 21 | pdi << 12) + pti*8
//
// To access PDE for vaddr:
//   recursive_pd_addr(vaddr) = sign_extend(510 << 39 | 510 << 30 | pml4i << 21 | pdpti << 12) + pdi*8
//
// To access PDPTE for vaddr:
//   recursive_pdpt_addr(vaddr) = sign_extend(510 << 39 | 510 << 30 | 510 << 21 | pml4i << 12) + pdpti*8
//
// To access PML4E:
//   recursive_pml4_addr = sign_extend(510 << 39 | 510 << 30 | 510 << 21 | 510 << 12) = 0xFFFF_FF7F_BFDF_E000

/// Base address for accessing the PML4 table via recursive mapping.
const RECURSIVE_PML4_BASE: u64 = 0xFFFF_FF7F_BFDF_E000;

/// Sign-extend a 48-bit address to 64-bit canonical form.
fn sign_extend(addr: u64) -> u64 {
    // If bit 47 is set, fill bits 48-63 with 1s
    if addr & (1u64 << 47) != 0 {
        addr | 0xFFFF_0000_0000_0000
    } else {
        addr & 0x0000_FFFF_FFFF_FFFF
    }
}

/// Compute virtual address to access the page table (level 1) entry for `vaddr`.
fn recursive_pt_base(vaddr: VirtAddr) -> u64 {
    let pml4i = vaddr.pml4_index() as u64;
    let pdpti = vaddr.pdpt_index() as u64;
    let pdi = vaddr.pd_index() as u64;
    sign_extend(
        (RECURSIVE_INDEX as u64) << 39 | pml4i << 30 | pdpti << 21 | pdi << 12,
    )
}

/// Compute virtual address to access the page directory (level 2) entry for `vaddr`.
fn recursive_pd_base(vaddr: VirtAddr) -> u64 {
    let pml4i = vaddr.pml4_index() as u64;
    let pdpti = vaddr.pdpt_index() as u64;
    sign_extend(
        (RECURSIVE_INDEX as u64) << 39
            | (RECURSIVE_INDEX as u64) << 30
            | pml4i << 21
            | pdpti << 12,
    )
}

/// Compute virtual address to access the PDPT (level 3) entry for `vaddr`.
fn recursive_pdpt_base(vaddr: VirtAddr) -> u64 {
    let pml4i = vaddr.pml4_index() as u64;
    sign_extend(
        (RECURSIVE_INDEX as u64) << 39
            | (RECURSIVE_INDEX as u64) << 30
            | (RECURSIVE_INDEX as u64) << 21
            | pml4i << 12,
    )
}

/// Debug helper: get recursive PDPT base for a virtual address (used by page fault diagnostics).
pub fn debug_recursive_pdpt(vaddr: u64) -> u64 {
    let pml4i = ((vaddr >> 39) & 0x1FF) as u64;
    sign_extend(
        (RECURSIVE_INDEX as u64) << 39
            | (RECURSIVE_INDEX as u64) << 30
            | (RECURSIVE_INDEX as u64) << 21
            | pml4i << 12,
    )
}

/// Debug helper: get recursive PD base for a virtual address (used by page fault diagnostics).
pub fn debug_recursive_pd(vaddr: u64) -> u64 {
    let pml4i = ((vaddr >> 39) & 0x1FF) as u64;
    let pdpti = ((vaddr >> 30) & 0x1FF) as u64;
    sign_extend(
        (RECURSIVE_INDEX as u64) << 39
            | (RECURSIVE_INDEX as u64) << 30
            | pml4i << 21
            | pdpti << 12,
    )
}

/// Debug helper: get recursive PT base for a virtual address (used by page fault diagnostics).
pub fn debug_recursive_pt(vaddr: u64) -> u64 {
    let pml4i = ((vaddr >> 39) & 0x1FF) as u64;
    let pdpti = ((vaddr >> 30) & 0x1FF) as u64;
    let pdi = ((vaddr >> 21) & 0x1FF) as u64;
    sign_extend(
        (RECURSIVE_INDEX as u64) << 39 | pml4i << 30 | pdpti << 21 | pdi << 12,
    )
}

// PML4 physical address (set during init, used for kernel_cr3)
static mut PML4_PHYS: u64 = 0;

/// Initialize virtual memory: transition from bootloader's 2MB page tables to
/// fine-grained 4K pages with recursive mapping.
///
/// The bootloader already set up 4-level paging with 2MB pages. We:
/// 1. Allocate a new PML4 with recursive mapping at entry 510
/// 2. Re-map the kernel higher-half region with 4K pages
/// 3. Re-map identity-mapped low memory with 4K pages
/// 4. Map the framebuffer
/// 5. Switch CR3 to the new PML4
pub fn init(boot_info: &BootInfo) {
    // Allocate new PML4
    let pml4_phys = physical::alloc_frame().expect("Failed to allocate PML4");
    // We're running with the bootloader's page tables which identity-map low memory,
    // so we can access physical addresses directly (they're < 16 MiB).
    let pml4 = pml4_phys.as_u64() as *mut u64;

    // Zero the PML4
    for i in 0..ENTRIES_PER_TABLE {
        unsafe { pml4.add(i).write_volatile(0); }
    }

    // Identity-map first 128 MiB using 4K pages
    // Covers bootloader area, kernel, boot page tables, and DMA buffers
    for mb in 0..64u64 {
        let base = mb * 0x0020_0000; // 2 MiB per iteration
        // Each 2 MiB range needs: PDPT entry, PD entry, PT with 512 entries

        // Ensure PDPT exists for PML4[0]
        let pdpt_phys = ensure_table_entry(pml4, 0, PAGE_PRESENT | PAGE_WRITABLE);
        let pdpt = pdpt_phys as *mut u64;

        // PD index for this 2MB chunk
        let pdpt_idx = (base >> 30) as usize; // Should be 0 for < 1 GiB
        let pd_phys = ensure_table_entry(pdpt, pdpt_idx, PAGE_PRESENT | PAGE_WRITABLE);
        let pd = pd_phys as *mut u64;

        let pd_idx = ((base >> 21) & 0x1FF) as usize;
        let pt_phys = ensure_table_entry(pd, pd_idx, PAGE_PRESENT | PAGE_WRITABLE);
        let pt = pt_phys as *mut u64;

        // Fill PT with 512 4K page entries
        for pte in 0..ENTRIES_PER_TABLE {
            let phys = base + (pte as u64) * FRAME_SIZE as u64;
            unsafe {
                pt.add(pte).write_volatile(phys | PAGE_PRESENT | PAGE_WRITABLE);
            }
        }
    }

    // Map higher-half kernel: PML4[511] → same physical memory as identity map
    // Kernel is at virtual 0xFFFFFFFF80000000 → PML4[511], PDPT[510], PD[0..3]
    // (0xFFFFFFFF80000000: PML4 idx = 511, PDPT idx = 510, PD idx = 0)
    {
        // Ensure PDPT for PML4[511]
        let pdpt_phys = ensure_table_entry(pml4, 511, PAGE_PRESENT | PAGE_WRITABLE);
        let pdpt = pdpt_phys as *mut u64;

        // Ensure PD for PDPT[510]
        let pd_phys = ensure_table_entry(pdpt, 510, PAGE_PRESENT | PAGE_WRITABLE);
        let pd = pd_phys as *mut u64;

        // Map 16 MiB of kernel (8 PD entries, each covering 2 MiB via a page table)
        // Extra room for large BSS (e.g. 2 MiB physical allocator bitmap)
        for mb in 0..8u64 {
            let pt_phys_alloc = physical::alloc_frame().expect("Failed to allocate kernel PT");
            let pt = pt_phys_alloc.as_u64() as *mut u64;

            for pte in 0..ENTRIES_PER_TABLE {
                let phys = mb * 0x0020_0000 + (pte as u64) * FRAME_SIZE as u64;
                unsafe {
                    pt.add(pte).write_volatile(phys | PAGE_PRESENT | PAGE_WRITABLE);
                }
            }

            unsafe {
                pd.add(mb as usize)
                    .write_volatile(pt_phys_alloc.as_u64() | PAGE_PRESENT | PAGE_WRITABLE);
            }
        }
    }

    // Identity-map framebuffer region
    let fb_addr = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_addr).read_unaligned() } as u64;
    let fb_pitch = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_pitch).read_unaligned() } as u64;
    let fb_height = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_height).read_unaligned() } as u64;

    if fb_addr != 0 && fb_pitch != 0 && fb_height != 0 {
        // Map full 16 MiB VRAM
        let fb_size: u64 = 16 * 1024 * 1024;
        let fb_start = fb_addr & !0xFFF;
        let fb_end = (fb_addr + fb_size + 0xFFF) & !0xFFF;

        // Map each 4K page of the framebuffer
        let mut addr = fb_start;
        while addr < fb_end {
            let virt = VirtAddr::new(addr);
            // Ensure all 4 levels exist
            let pdpt_phys = ensure_table_entry(pml4, virt.pml4_index(), PAGE_PRESENT | PAGE_WRITABLE);
            let pdpt = pdpt_phys as *mut u64;
            let pd_phys = ensure_table_entry(pdpt, virt.pdpt_index(), PAGE_PRESENT | PAGE_WRITABLE);
            let pd = pd_phys as *mut u64;
            let pt_phys = ensure_table_entry(pd, virt.pd_index(), PAGE_PRESENT | PAGE_WRITABLE);
            let pt = pt_phys as *mut u64;

            unsafe {
                pt.add(virt.pt_index()).write_volatile(addr | PAGE_PRESENT | PAGE_WRITABLE);
            }

            addr += FRAME_SIZE as u64;
        }

        crate::serial_println!(
            "Framebuffer mapped: {:#010x}-{:#010x} ({} pages)",
            fb_start, fb_end, (fb_end - fb_start) / FRAME_SIZE as u64
        );
    }

    // Set up recursive mapping: PML4[510] → PML4 itself
    unsafe {
        pml4.add(RECURSIVE_INDEX)
            .write_volatile(pml4_phys.as_u64() | PAGE_PRESENT | PAGE_WRITABLE);
    }

    // Store PML4 physical address
    unsafe { PML4_PHYS = pml4_phys.as_u64(); }

    // Switch CR3 to new PML4
    unsafe {
        asm!(
            "mov cr3, {}",
            in(reg) pml4_phys.as_u64(),
            options(nostack, preserves_flags),
        );
    }

    crate::serial_println!("4-level paging enabled (identity + higher-half at {:#018x})", KERNEL_VIRT_BASE);
}

/// Ensure a page table entry at `index` in `table` exists.
/// If not present, allocates a new frame, zeros it, and installs it.
/// Returns the physical address of the child table.
fn ensure_table_entry(table: *mut u64, index: usize, flags: u64) -> u64 {
    unsafe {
        let entry = table.add(index).read_volatile();
        if entry & PAGE_PRESENT != 0 {
            return entry & ADDR_MASK;
        }

        let new_frame = physical::alloc_frame().expect("Failed to allocate page table frame");
        let new_addr = new_frame.as_u64();

        // Zero the new table
        let new_table = new_addr as *mut u64;
        for i in 0..ENTRIES_PER_TABLE {
            new_table.add(i).write_volatile(0);
        }

        table.add(index).write_volatile(new_addr | flags);
        new_addr
    }
}

/// Map a single 4K page: virtual -> physical.
///
/// Uses recursive mapping via PML4[510] to access page table structures.
pub fn map_page(virt: VirtAddr, phys: PhysAddr, flags: u64) {
    let pml4_ptr = RECURSIVE_PML4_BASE as *mut u64;
    let pml4i = virt.pml4_index();
    let pdpti = virt.pdpt_index();
    let pdi = virt.pd_index();
    let pti = virt.pt_index();

    unsafe {
        // Ensure PDPT exists
        let pml4e = pml4_ptr.add(pml4i).read_volatile();
        if pml4e & PAGE_PRESENT == 0 {
            let new_frame = physical::alloc_frame().expect("Failed to allocate PDPT");
            pml4_ptr.add(pml4i).write_volatile(new_frame.as_u64() | PAGE_PRESENT | PAGE_WRITABLE | (flags & PAGE_USER));
            // Zero the new PDPT via recursive mapping
            let pdpt_base = recursive_pdpt_base(virt) as *mut u8;
            // Flush TLB for the recursive address so we can access the new table
            asm!("invlpg [{}]", in(reg) pdpt_base, options(nostack, preserves_flags));
            core::ptr::write_bytes(pdpt_base, 0, FRAME_SIZE);
        }

        // Ensure PD exists
        let pdpt_ptr = recursive_pdpt_base(virt) as *mut u64;
        let pdpte = pdpt_ptr.add(pdpti).read_volatile();
        if pdpte & PAGE_PRESENT == 0 {
            let new_frame = physical::alloc_frame().expect("Failed to allocate PD");
            pdpt_ptr.add(pdpti).write_volatile(new_frame.as_u64() | PAGE_PRESENT | PAGE_WRITABLE | (flags & PAGE_USER));
            let pd_base = recursive_pd_base(virt) as *mut u8;
            asm!("invlpg [{}]", in(reg) pd_base, options(nostack, preserves_flags));
            core::ptr::write_bytes(pd_base, 0, FRAME_SIZE);
        }

        // Ensure PT exists
        let pd_ptr = recursive_pd_base(virt) as *mut u64;
        let pde = pd_ptr.add(pdi).read_volatile();
        if pde & PAGE_PRESENT == 0 {
            let new_frame = physical::alloc_frame().expect("Failed to allocate PT");
            pd_ptr.add(pdi).write_volatile(new_frame.as_u64() | PAGE_PRESENT | PAGE_WRITABLE | (flags & PAGE_USER));
            let pt_base = recursive_pt_base(virt) as *mut u8;
            asm!("invlpg [{}]", in(reg) pt_base, options(nostack, preserves_flags));
            core::ptr::write_bytes(pt_base, 0, FRAME_SIZE);
        }

        // Set the PTE
        let pt_ptr = recursive_pt_base(virt) as *mut u64;
        pt_ptr.add(pti).write_volatile(phys.as_u64() | flags | PAGE_PRESENT);

        // Invalidate TLB for the mapped page
        asm!("invlpg [{}]", in(reg) virt.as_u64(), options(nostack, preserves_flags));
    }
}

/// Unmap a single 4K page.
pub fn unmap_page(virt: VirtAddr) {
    let pml4i = virt.pml4_index();
    let pdpti = virt.pdpt_index();
    let pdi = virt.pd_index();
    let pti = virt.pt_index();

    unsafe {
        // Check PML4
        let pml4_ptr = RECURSIVE_PML4_BASE as *mut u64;
        let pml4e = pml4_ptr.add(pml4i).read_volatile();
        if pml4e & PAGE_PRESENT == 0 {
            return;
        }

        // Check PDPT
        let pdpt_ptr = recursive_pdpt_base(virt) as *mut u64;
        let pdpte = pdpt_ptr.add(pdpti).read_volatile();
        if pdpte & PAGE_PRESENT == 0 {
            return;
        }

        // Check PD
        let pd_ptr = recursive_pd_base(virt) as *mut u64;
        let pde = pd_ptr.add(pdi).read_volatile();
        if pde & PAGE_PRESENT == 0 {
            return;
        }

        // Clear PTE
        let pt_ptr = recursive_pt_base(virt) as *mut u64;
        pt_ptr.add(pti).write_volatile(0);

        asm!("invlpg [{}]", in(reg) virt.as_u64(), options(nostack, preserves_flags));
    }
}

/// Check if a virtual address is mapped in the current page directory.
/// Walks the 4-level page table via recursive mapping.
pub fn is_page_mapped(virt: VirtAddr) -> bool {
    let pml4i = virt.pml4_index();
    let pdpti = virt.pdpt_index();
    let pdi = virt.pd_index();
    let pti = virt.pt_index();

    unsafe {
        let pml4_ptr = RECURSIVE_PML4_BASE as *const u64;
        let pml4e = pml4_ptr.add(pml4i).read_volatile();
        if pml4e & PAGE_PRESENT == 0 {
            return false;
        }

        let pdpt_ptr = recursive_pdpt_base(virt) as *const u64;
        let pdpte = pdpt_ptr.add(pdpti).read_volatile();
        if pdpte & PAGE_PRESENT == 0 {
            return false;
        }

        let pd_ptr = recursive_pd_base(virt) as *const u64;
        let pde = pd_ptr.add(pdi).read_volatile();
        if pde & PAGE_PRESENT == 0 {
            return false;
        }

        let pt_ptr = recursive_pt_base(virt) as *const u64;
        let pte = pt_ptr.add(pti).read_volatile();
        pte & PAGE_PRESENT != 0
    }
}

/// Get the kernel PML4's physical address.
pub fn kernel_cr3() -> u64 {
    unsafe { PML4_PHYS }
}

/// Get the current page table root physical address (CR3).
pub fn current_cr3() -> u64 {
    let cr3: u64;
    unsafe { asm!("mov {}, cr3", out(reg) cr3); }
    cr3
}

/// Create a new PML4 for a user process.
/// Clones all kernel-space PML4 entries (256-511) from the current PML4.
/// User-space entries (0-255) are left empty for per-process mappings.
/// PML4[510] is set to the NEW PML4's own address for recursive mapping.
/// Returns the physical address of the new PML4.
pub fn create_user_page_directory() -> Option<PhysAddr> {
    let new_pml4_phys = physical::alloc_frame()?;
    let new_pdpt_phys = physical::alloc_frame()?; // PDPT for PML4[0]
    let new_pd_phys = physical::alloc_frame()?;   // PD for PML4[0]→PDPT[0]

    // Temp virtual addresses to write into the new page tables.
    // MUST be outside the heap range (HEAP_START + 512 MiB max) to avoid
    // clobbering heap page mappings when unmapping these temp pages.
    let temp_pml4 = VirtAddr::new(0xFFFF_FFFF_BFF0_0000);
    let temp_pdpt = VirtAddr::new(0xFFFF_FFFF_BFF0_1000);
    let temp_pd   = VirtAddr::new(0xFFFF_FFFF_BFF0_2000);

    map_page(temp_pml4, new_pml4_phys, PAGE_WRITABLE);
    map_page(temp_pdpt, new_pdpt_phys, PAGE_WRITABLE);
    map_page(temp_pd,   new_pd_phys,   PAGE_WRITABLE);

    let new_pml4 = temp_pml4.as_u64() as *mut u64;
    let new_pdpt_ptr = temp_pdpt.as_u64() as *mut u64;
    let new_pd_ptr = temp_pd.as_u64() as *mut u64;
    let cur_pml4 = RECURSIVE_PML4_BASE as *const u64;

    unsafe {
        // Zero the new PDPT and PD
        for i in 0..ENTRIES_PER_TABLE {
            new_pdpt_ptr.add(i).write_volatile(0);
            new_pd_ptr.add(i).write_volatile(0);
        }

        // Copy identity-map PD entries [0..31] from kernel (covers first 64 MiB).
        // These are kernel-only (no PAGE_USER), so Ring 3 can't access them.
        // Entries [32+] left empty for DLLs (0x04000000+) and user programs.
        let kernel_pd = recursive_pd_base(VirtAddr::new(0)) as *const u64;
        for i in 0..32 {
            new_pd_ptr.add(i).write_volatile(kernel_pd.add(i).read_volatile());
        }

        // Wire PDPT[0] → new PD (PAGE_USER so user program pages in PD[64+] work)
        new_pdpt_ptr.write_volatile(
            new_pd_phys.as_u64() | PAGE_PRESENT | PAGE_WRITABLE | PAGE_USER,
        );

        // Wire PML4[0] → new PDPT (PAGE_USER for same reason)
        new_pml4.write_volatile(
            new_pdpt_phys.as_u64() | PAGE_PRESENT | PAGE_WRITABLE | PAGE_USER,
        );

        // Clear remaining user-space entries (1-255)
        for i in 1..256 {
            new_pml4.add(i).write_volatile(0);
        }

        // Copy kernel-space entries (256-511) from current PML4.
        // Skip 510 (recursive mapping) — we'll set it to point to the new PML4.
        for i in 256..ENTRIES_PER_TABLE {
            if i == RECURSIVE_INDEX {
                continue;
            }
            new_pml4.add(i).write_volatile(cur_pml4.add(i).read_volatile());
        }

        // PML4[510]: recursive mapping points to the NEW PML4 itself
        new_pml4.add(RECURSIVE_INDEX)
            .write_volatile(new_pml4_phys.as_u64() | PAGE_PRESENT | PAGE_WRITABLE);
    }

    // Unmap temp pages
    unmap_page(temp_pml4);
    unmap_page(temp_pdpt);
    unmap_page(temp_pd);

    Some(new_pml4_phys)
}

/// Map a page in a specific page directory (not necessarily the current one).
/// Temporarily switches CR3 to the target PML4.
///
/// Interrupts are disabled for the duration: a context switch while CR3 is
/// temporarily switched would cause the scheduler to restore a different CR3,
/// making `map_page` silently modify the wrong process's page tables.
pub fn map_page_in_pd(pd_phys: PhysAddr, virt: VirtAddr, phys: PhysAddr, flags: u64) {
    unsafe {
        asm!("cli", options(nomem, nostack));
        let old_cr3 = current_cr3();
        asm!("mov cr3, {}", in(reg) pd_phys.as_u64());
        map_page(virt, phys, flags);
        asm!("mov cr3, {}", in(reg) old_cr3);
        asm!("sti", options(nomem, nostack));
    }
}

/// Check if a virtual address is mapped in a specific page directory.
/// Temporarily switches CR3 to the target PML4.
///
/// Interrupts are disabled for the duration: same race as `map_page_in_pd`.
pub fn is_mapped_in_pd(pd_phys: PhysAddr, virt: VirtAddr) -> bool {
    unsafe {
        asm!("cli", options(nomem, nostack));
        let old_cr3 = current_cr3();
        asm!("mov cr3, {}", in(reg) pd_phys.as_u64());

        let pml4_ptr = RECURSIVE_PML4_BASE as *const u64;
        let pml4e = pml4_ptr.add(virt.pml4_index()).read_volatile();
        let mapped = if pml4e & PAGE_PRESENT != 0 {
            let pdpt_ptr = recursive_pdpt_base(virt) as *const u64;
            let pdpte = pdpt_ptr.add(virt.pdpt_index()).read_volatile();
            if pdpte & PAGE_PRESENT != 0 {
                let pd_ptr = recursive_pd_base(virt) as *const u64;
                let pde = pd_ptr.add(virt.pd_index()).read_volatile();
                if pde & PAGE_PRESENT != 0 {
                    let pt_ptr = recursive_pt_base(virt) as *const u64;
                    pt_ptr.add(virt.pt_index()).read_volatile() & PAGE_PRESENT != 0
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        asm!("mov cr3, {}", in(reg) old_cr3);
        asm!("sti", options(nomem, nostack));
        mapped
    }
}

/// Destroy a user PML4: free all user-space pages, page tables, and the PML4.
/// Must NOT be the currently active page directory.
pub fn destroy_user_page_directory(pml4_phys: PhysAddr) {
    unsafe {
        let old_cr3 = current_cr3();

        // Save flags and disable interrupts — CRITICAL for SMP safety.
        // Without cli, a timer interrupt during the CR3 switch causes the scheduler
        // to save the wrong CR3, corrupting page tables of other processes.
        let rflags: u64;
        asm!("pushfq; pop {}", out(reg) rflags);
        asm!("cli");

        // Switch to the target PML4 so recursive mapping works on it
        asm!("mov cr3, {}", in(reg) pml4_phys.as_u64());

        let pml4_ptr = RECURSIVE_PML4_BASE as *const u64;

        // Walk user-space PML4 entries (0-255) and free mapped pages + tables.
        // DLL shared pages (vaddr 0x04000000-0x07FFFFFF, PML4[0] region)
        // have their frames managed by task::dll — free page tables but NOT frames.
        for pml4i in 0..256 {
            let pml4e = pml4_ptr.add(pml4i).read_volatile();
            if pml4e & PAGE_PRESENT == 0 {
                continue;
            }

            let pdpt_base = sign_extend(
                (RECURSIVE_INDEX as u64) << 39
                    | (RECURSIVE_INDEX as u64) << 30
                    | (RECURSIVE_INDEX as u64) << 21
                    | (pml4i as u64) << 12,
            );
            let pdpt_ptr = pdpt_base as *const u64;

            for pdpti in 0..ENTRIES_PER_TABLE {
                let pdpte = pdpt_ptr.add(pdpti).read_volatile();
                if pdpte & PAGE_PRESENT == 0 {
                    continue;
                }

                let pd_base = sign_extend(
                    (RECURSIVE_INDEX as u64) << 39
                        | (RECURSIVE_INDEX as u64) << 30
                        | (pml4i as u64) << 21
                        | (pdpti as u64) << 12,
                );
                let pd_ptr = pd_base as *const u64;

                for pdi in 0..ENTRIES_PER_TABLE {
                    let pde = pd_ptr.add(pdi).read_volatile();
                    if pde & PAGE_PRESENT == 0 {
                        continue;
                    }

                    // Check if this is in the DLL virtual address range
                    // DLLs at 0x04000000-0x07FFFFFF: PML4[0], PDPT[0], PD[32..63]
                    let is_dll = pml4i == 0 && pdpti == 0 && pdi >= 32 && pdi <= 63;

                    // Identity-map entries (PD[0..31]) share PTs with the kernel.
                    // Don't free their PT frames or the physical pages they map.
                    let is_identity_map = pml4i == 0 && pdpti == 0 && pdi < 32;

                    if is_identity_map {
                        continue; // Skip entirely — kernel owns these PTs
                    }

                    let pt_base = sign_extend(
                        (RECURSIVE_INDEX as u64) << 39
                            | (pml4i as u64) << 30
                            | (pdpti as u64) << 21
                            | (pdi as u64) << 12,
                    );
                    let pt_ptr = pt_base as *const u64;

                    for pti in 0..ENTRIES_PER_TABLE {
                        let pte = pt_ptr.add(pti).read_volatile();
                        if pte & PAGE_PRESENT != 0 && !is_dll {
                            physical::free_frame(PhysAddr::new(pte & ADDR_MASK));
                        }
                    }

                    // Free the page table frame
                    physical::free_frame(PhysAddr::new(pde & ADDR_MASK));
                }

                // Free the PD frame
                physical::free_frame(PhysAddr::new(pdpte & ADDR_MASK));
            }

            // Free the PDPT frame
            physical::free_frame(PhysAddr::new(pml4e & ADDR_MASK));
        }

        // Switch back to previous PML4
        asm!("mov cr3, {}", in(reg) old_cr3);

        // Restore interrupt flag
        asm!("push {}; popfq", in(reg) rflags);
    }

    // Free the PML4 frame itself
    physical::free_frame(pml4_phys);
}

/// Handle a demand-page fault for the kernel heap.
///
/// Called from the page fault handler (ISR 14) when a "not present" fault occurs.
/// If the faulting address is within the committed heap range, allocates a physical
/// frame, maps it, zeroes it, and returns `true` so the faulting instruction can retry.
///
/// Returns `false` if the address is not in the committed heap range (real fault).
pub fn handle_heap_demand_page(vaddr: u64) -> bool {
    let heap_start = 0xFFFF_FFFF_8200_0000u64;
    let committed = crate::memory::heap::HEAP_COMMITTED
        .load(core::sync::atomic::Ordering::Acquire);
    let heap_end = heap_start + committed as u64;

    if vaddr < heap_start || vaddr >= heap_end {
        return false;
    }

    // Allocate a physical frame
    let phys = match physical::alloc_frame() {
        Some(p) => p,
        None => return false,
    };

    // Map the page (Present + Writable, kernel-only)
    let page_addr = VirtAddr::new(vaddr & !0xFFF);
    map_page(page_addr, phys, 0x03);

    // Zero the page (demand-paged pages must be zeroed for security/correctness)
    unsafe {
        core::ptr::write_bytes(page_addr.as_u64() as *mut u8, 0, FRAME_SIZE);
    }

    true
}
