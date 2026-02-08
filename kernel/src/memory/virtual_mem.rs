use crate::boot_info::BootInfo;
use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::physical;
use crate::memory::FRAME_SIZE;
use core::arch::asm;

const PAGE_PRESENT: u32 = 1 << 0;
const PAGE_WRITABLE: u32 = 1 << 1;
const PAGE_USER: u32 = 1 << 2;

const ENTRIES_PER_TABLE: usize = 1024;

// Kernel virtual base address (higher-half)
const KERNEL_VIRT_BASE: u32 = 0xC000_0000;

// Page directory physical address (allocated during init)
static mut PAGE_DIRECTORY_PHYS: u32 = 0;

/// Initialize virtual memory with identity mapping + higher-half kernel mapping.
///
/// Since the kernel is running at physical addresses when this is first called
/// (bootloader jumped to 0x100000), we need to:
/// 1. Create a page directory
/// 2. Identity-map the first 4 MiB (so current code keeps running)
/// 3. Map 0xC0000000-0xC03FFFFF -> physical 0x00000000-0x003FFFFF (higher-half)
/// 4. Enable paging
///
/// After paging is enabled, the kernel should be accessed via 0xC0100000+.
/// For Phase 1, we keep it simple: identity-map first 8 MiB and map them to 0xC0000000+ too.
pub fn init(boot_info: &BootInfo) {
    // Allocate page directory (must be 4K-aligned)
    let pd_phys = physical::alloc_frame().expect("Failed to allocate page directory");

    // Zero it out
    let pd = pd_phys.as_u32() as *mut u32;
    for i in 0..ENTRIES_PER_TABLE {
        unsafe { pd.add(i).write_volatile(0); }
    }

    // Identity-map first 8 MiB using 4K pages (2 page tables)
    for mb in 0..8u32 {
        let pt_phys = physical::alloc_frame().expect("Failed to allocate page table");
        let pt = pt_phys.as_u32() as *mut u32;

        // Fill page table: map each page to its physical address
        for i in 0..ENTRIES_PER_TABLE {
            let phys = mb * (4 * 1024 * 1024) + (i as u32) * FRAME_SIZE as u32;
            unsafe {
                pt.add(i).write_volatile(phys | PAGE_PRESENT | PAGE_WRITABLE);
            }
        }

        // Set PDE for identity mapping
        let pde_index = (mb * 4 * 1024 * 1024 / (4 * 1024 * 1024)) as usize;
        unsafe {
            pd.add(pde_index).write_volatile(pt_phys.as_u32() | PAGE_PRESENT | PAGE_WRITABLE);
        }
    }

    // Map higher-half: 0xC0000000 -> physical 0x00000000 (8 MiB)
    // The higher-half PDE indices start at 0xC0000000 >> 22 = 768
    let hh_start_pde = (KERNEL_VIRT_BASE >> 22) as usize;
    for mb in 0..8u32 {
        let pt_phys = physical::alloc_frame().expect("Failed to allocate page table for higher-half");
        let pt = pt_phys.as_u32() as *mut u32;

        for i in 0..ENTRIES_PER_TABLE {
            let phys = mb * (4 * 1024 * 1024) + (i as u32) * FRAME_SIZE as u32;
            unsafe {
                pt.add(i).write_volatile(phys | PAGE_PRESENT | PAGE_WRITABLE);
            }
        }

        unsafe {
            pd.add(hh_start_pde + mb as usize)
                .write_volatile(pt_phys.as_u32() | PAGE_PRESENT | PAGE_WRITABLE);
        }
    }

    // Identity-map framebuffer region if available (MMIO, not from physical allocator)
    let fb_addr = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_addr).read_unaligned() };
    let fb_pitch = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_pitch).read_unaligned() };
    let fb_height = unsafe { core::ptr::addr_of!((*boot_info).framebuffer_height).read_unaligned() };

    if fb_addr != 0 && fb_pitch != 0 && fb_height != 0 {
        // Map full 16 MiB VRAM to support runtime resolution changes with double-buffering.
        // Bochs VGA and VMware SVGA both have >= 16 MiB VRAM on QEMU.
        let fb_size = 16 * 1024 * 1024;
        let fb_start = fb_addr & !0xFFF; // Page-align down
        let fb_end = (fb_addr + fb_size + 0xFFF) & !0xFFF; // Page-align up

        let start_pde = (fb_start >> 22) as usize;
        let end_pde = ((fb_end - 1) >> 22) as usize;

        for pde_idx in start_pde..=end_pde {
            let pt_phys = physical::alloc_frame().expect("Failed to allocate FB page table");
            let pt = pt_phys.as_u32() as *mut u32;

            let pde_base = (pde_idx as u32) << 22;
            for pte_idx in 0..ENTRIES_PER_TABLE {
                let page_addr = pde_base + (pte_idx as u32) * FRAME_SIZE as u32;
                unsafe {
                    if page_addr >= fb_start && page_addr < fb_end {
                        pt.add(pte_idx).write_volatile(page_addr | PAGE_PRESENT | PAGE_WRITABLE);
                    } else {
                        pt.add(pte_idx).write_volatile(0);
                    }
                }
            }

            unsafe {
                pd.add(pde_idx).write_volatile(pt_phys.as_u32() | PAGE_PRESENT | PAGE_WRITABLE);
            }
        }

        crate::serial_println!(
            "Framebuffer mapped: {:#010x}-{:#010x} ({} pages)",
            fb_start, fb_end, (fb_end - fb_start) / FRAME_SIZE as u32
        );
    }

    // Set up recursive mapping: last PDE points to the page directory itself
    // This allows accessing page tables through virtual address 0xFFC00000
    unsafe {
        pd.add(1023).write_volatile(pd_phys.as_u32() | PAGE_PRESENT | PAGE_WRITABLE);
    }

    // Store page directory address
    unsafe { PAGE_DIRECTORY_PHYS = pd_phys.as_u32(); }

    // Load CR3 and enable paging
    unsafe {
        asm!(
            "mov cr3, {pd}",
            "mov {tmp}, cr0",
            "or {tmp}, 0x80000000",
            "mov cr0, {tmp}",
            pd = in(reg) pd_phys.as_u32(),
            tmp = out(reg) _,
        );
    }

    crate::serial_println!("Paging enabled (identity + higher-half at 0xC0000000)");
}

/// Map a single 4K page: virtual -> physical
pub fn map_page(virt: VirtAddr, phys: PhysAddr, flags: u32) {
    let pde_idx = virt.page_directory_index();
    let pte_idx = virt.page_table_index();

    // Access page directory via recursive mapping
    let pd = 0xFFFFF000 as *mut u32;

    unsafe {
        let pde = pd.add(pde_idx).read_volatile();

        // If page table doesn't exist, allocate one
        if pde & PAGE_PRESENT == 0 {
            let new_pt = physical::alloc_frame().expect("Failed to allocate page table");
            pd.add(pde_idx).write_volatile(new_pt.as_u32() | PAGE_PRESENT | PAGE_WRITABLE | flags);

            // Zero the new page table via recursive mapping
            let pt_virt = (0xFFC00000 + pde_idx * FRAME_SIZE) as *mut u32;
            for i in 0..ENTRIES_PER_TABLE {
                pt_virt.add(i).write_volatile(0);
            }
        }

        // Access page table via recursive mapping
        let pt = (0xFFC00000 + pde_idx * FRAME_SIZE) as *mut u32;
        pt.add(pte_idx).write_volatile(phys.as_u32() | flags | PAGE_PRESENT);

        // Invalidate TLB for this page
        asm!("invlpg [{}]", in(reg) virt.as_u32(), options(nostack, preserves_flags));
    }
}

/// Unmap a single 4K page
pub fn unmap_page(virt: VirtAddr) {
    let pde_idx = virt.page_directory_index();
    let pte_idx = virt.page_table_index();

    let pd = 0xFFFFF000 as *mut u32;

    unsafe {
        let pde = pd.add(pde_idx).read_volatile();
        if pde & PAGE_PRESENT == 0 {
            return;
        }

        let pt = (0xFFC00000 + pde_idx * FRAME_SIZE) as *mut u32;
        pt.add(pte_idx).write_volatile(0);

        asm!("invlpg [{}]", in(reg) virt.as_u32(), options(nostack, preserves_flags));
    }
}

/// Get the kernel page directory's physical address.
pub fn kernel_cr3() -> u32 {
    unsafe { PAGE_DIRECTORY_PHYS }
}

/// Get the current page directory's physical address.
pub fn current_cr3() -> u32 {
    let cr3: u32;
    unsafe { asm!("mov {}, cr3", out(reg) cr3); }
    cr3
}

/// Create a new page directory for a user process.
/// Clones all kernel-space mappings (identity map + higher-half + framebuffer).
/// User-space PDEs (8-767) are left empty for per-process mappings.
/// Returns the physical address of the new page directory.
pub fn create_user_page_directory() -> Option<PhysAddr> {
    let new_pd_phys = physical::alloc_frame()?;

    // Map the new PD at a temporary virtual address so we can write to it
    let temp_virt = VirtAddr::new(0xC1F0_0000);
    map_page(temp_virt, new_pd_phys, PAGE_WRITABLE);

    let new_pd = temp_virt.as_u32() as *mut u32;
    let cur_pd = 0xFFFFF000 as *const u32; // Current PD via recursive mapping

    unsafe {
        // Copy identity-mapping PDEs (0-7, covers first 32 MiB)
        for i in 0..8 {
            new_pd.add(i).write_volatile(cur_pd.add(i).read_volatile());
        }

        // Clear user-space PDEs (8-767)
        for i in 8..768 {
            new_pd.add(i).write_volatile(0);
        }

        // Copy kernel higher-half PDEs (768-1022, includes framebuffer)
        for i in 768..1023 {
            new_pd.add(i).write_volatile(cur_pd.add(i).read_volatile());
        }

        // PDE 1023: recursive mapping points to the NEW PD itself
        new_pd.add(1023).write_volatile(new_pd_phys.as_u32() | PAGE_PRESENT | PAGE_WRITABLE);
    }

    // Unmap the temporary page
    unmap_page(temp_virt);

    Some(new_pd_phys)
}

/// Map a page in a specific page directory (not necessarily the current one).
/// Temporarily switches CR3 to the target PD.
pub fn map_page_in_pd(pd_phys: PhysAddr, virt: VirtAddr, phys: PhysAddr, flags: u32) {
    unsafe {
        let old_cr3 = current_cr3();
        asm!("mov cr3, {}", in(reg) pd_phys.as_u32());
        map_page(virt, phys, flags);
        asm!("mov cr3, {}", in(reg) old_cr3);
    }
}

/// Check if a virtual address is mapped in a specific page directory.
/// Temporarily switches CR3.
pub fn is_mapped_in_pd(pd_phys: PhysAddr, virt: VirtAddr) -> bool {
    unsafe {
        let old_cr3 = current_cr3();
        asm!("mov cr3, {}", in(reg) pd_phys.as_u32());

        let pde_idx = (virt.as_u32() >> 22) as usize;
        let pte_idx = ((virt.as_u32() >> 12) & 0x3FF) as usize;

        let pd = 0xFFFFF000 as *const u32;
        let pde = pd.add(pde_idx).read_volatile();
        let mapped = if pde & PAGE_PRESENT != 0 {
            let pt = (0xFFC00000 + pde_idx * FRAME_SIZE) as *const u32;
            pt.add(pte_idx).read_volatile() & PAGE_PRESENT != 0
        } else {
            false
        };

        asm!("mov cr3, {}", in(reg) old_cr3);
        mapped
    }
}

/// Destroy a user page directory: free all user-space pages, page tables, and the PD.
/// Must NOT be the currently active page directory.
pub fn destroy_user_page_directory(pd_phys: PhysAddr) {
    unsafe {
        let old_cr3 = current_cr3();

        // Switch to the target PD so recursive mapping works on it
        asm!("mov cr3, {}", in(reg) pd_phys.as_u32());

        let pd = 0xFFFFF000 as *const u32;

        // Walk user-space PDEs (8-767) and free mapped pages + page tables.
        // DLL shared pages (PDEs 16-31, vaddr 0x04000000-0x07FFFFFF) are
        // managed by task::dll — free their page tables but NOT the frames.
        for pde_idx in 8..768 {
            let pde = pd.add(pde_idx).read_volatile();
            if pde & PAGE_PRESENT == 0 {
                continue;
            }

            let is_dll = pde_idx >= 16 && pde_idx <= 31;

            let pt = (0xFFC00000 + pde_idx * FRAME_SIZE) as *const u32;
            for pte_idx in 0..ENTRIES_PER_TABLE {
                let pte = pt.add(pte_idx).read_volatile();
                if pte & PAGE_PRESENT != 0 && !is_dll {
                    physical::free_frame(PhysAddr::new(pte & !0xFFF));
                }
            }

            // Free the page table frame itself (always per-process)
            physical::free_frame(PhysAddr::new(pde & !0xFFF));
        }

        // Switch back to the previous PD
        asm!("mov cr3, {}", in(reg) old_cr3);
    }

    // Free the page directory frame
    physical::free_frame(pd_phys);
}
