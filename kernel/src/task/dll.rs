//! DLL (Dynamic Link Library) loader and registry.
//!
//! DLLs are stateless shared code mapped read-only into every user process
//! at fixed virtual addresses. Physical frames are allocated once and shared.

use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

const PAGE_SIZE: u64 = 4096;
const PAGE_USER: u64 = 0x04;

/// Next available virtual address for dynamically loaded DLLs.
/// Starts after the last boot-time DLL (0x0440_0000), incremented per load.
static NEXT_DYNAMIC_BASE: AtomicU64 = AtomicU64::new(0x0440_0000);

/// DLL virtual address range: 0x04000000 - 0x07FFFFFF.
/// In x86-64 4-level paging, these are PML4[0], PDPT[0], PD[32..63].
pub const DLL_PD_START: usize = 32;  // 0x04000000 >> 21 & 0x1FF
pub const DLL_PD_END: usize = 63;    // 0x07FFFFFF >> 21 & 0x1FF

/// A loaded DLL: name, base virtual address, and backing physical frames.
///
/// Physical frames are allocated once at load time and shared (read-only)
/// across all user processes.
pub struct LoadedDll {
    /// Short filename (null-terminated) extracted from the load path.
    pub name: [u8; 32],
    /// Virtual address where this DLL is mapped in every user process.
    pub base_vaddr: u64,
    /// Physical frames holding the DLL code/data, in page order.
    pub pages: Vec<PhysAddr>,
}

static LOADED_DLLS: Spinlock<Vec<LoadedDll>> = Spinlock::new(Vec::new());

/// Load a DLL from the filesystem into physical memory.
/// Returns the number of pages loaded, or an error string.
pub fn load_dll(path: &str, base_vaddr: u64) -> Result<u32, &'static str> {
    // Check if already loaded at this address
    {
        let dlls = LOADED_DLLS.lock();
        for dll in dlls.iter() {
            if dll.base_vaddr == base_vaddr {
                return Ok(dll.pages.len() as u32);
            }
        }
    }

    let data = crate::fs::vfs::read_file_to_vec(path)
        .map_err(|_| "Failed to read DLL file")?;

    if data.len() < 32 {
        return Err("DLL file too small");
    }

    // Validate DLIB magic at offset 0
    if &data[0..4] != b"DLIB" {
        return Err("Invalid DLL magic (expected DLIB)");
    }

    let num_pages = (data.len() as u64 + PAGE_SIZE - 1) / PAGE_SIZE;
    let mut pages = Vec::with_capacity(num_pages as usize);

    // Allocate physical frames and copy DLL data page by page.
    // Use a temporary virtual address in the higher-half kernel region.
    let temp_virt = VirtAddr::new(0xFFFF_FFFF_81F1_0000);

    for i in 0..num_pages {
        let frame = physical::alloc_frame()
            .ok_or("Out of memory allocating DLL frame")?;

        virtual_mem::map_page(temp_virt, frame, 0x02); // writable

        let offset = (i * PAGE_SIZE) as usize;
        let remaining = data.len() - offset;
        let copy_len = remaining.min(PAGE_SIZE as usize);

        unsafe {
            let dest = temp_virt.as_u64() as *mut u8;
            core::ptr::copy_nonoverlapping(data.as_ptr().add(offset), dest, copy_len);
            if copy_len < PAGE_SIZE as usize {
                core::ptr::write_bytes(dest.add(copy_len), 0, PAGE_SIZE as usize - copy_len);
            }
        }

        virtual_mem::unmap_page(temp_virt);
        pages.push(frame);
    }

    // Extract short name from path
    let mut name_buf = [0u8; 32];
    let name = path.rsplit('/').next().unwrap_or(path);
    let len = name.len().min(31);
    name_buf[..len].copy_from_slice(&name.as_bytes()[..len]);

    let mut dlls = LOADED_DLLS.lock();
    dlls.push(LoadedDll {
        name: name_buf,
        base_vaddr,
        pages,
    });

    crate::serial_println!(
        "[OK] DLL loaded: {} at {:#010x} ({} pages, {} bytes)",
        name, base_vaddr, num_pages, data.len()
    );

    Ok(num_pages as u32)
}

/// Map all loaded DLLs into a process page directory.
/// Pages are mapped as Present | User (read-only, executable).
///
/// The LOADED_DLLS spinlock is held only while collecting page info
/// (microseconds). Actual page mapping runs after the lock is dropped
/// so that timer/mouse/keyboard interrupts keep firing during spawn.
pub fn map_all_dlls_into(pd_phys: PhysAddr) {
    // Phase 1: Under lock — collect (virt, phys) pairs (no I/O, no CR3 switches)
    let page_maps: Vec<(VirtAddr, PhysAddr)> = {
        let dlls = LOADED_DLLS.lock();
        let mut v = Vec::new();
        for dll in dlls.iter() {
            for (i, &frame) in dll.pages.iter().enumerate() {
                v.push((VirtAddr::new(dll.base_vaddr + (i as u64) * PAGE_SIZE), frame));
            }
        }
        v
    }; // Lock dropped — interrupts re-enabled

    // Phase 2: Map pages without holding the lock
    for &(virt, phys) in &page_maps {
        virtual_mem::map_page_in_pd(pd_phys, virt, phys, PAGE_USER);
    }
}

/// Handle a demand-page fault for DLL pages.
///
/// Called from the page fault handler (ISR 14) when a user process accesses
/// an unmapped page in the DLL virtual range (0x04000000-0x07FFFFFF).
/// Looks up the shared physical frame and maps it into the current PD
/// (via recursive mapping — CR3 is already the faulting process's PD).
///
/// Returns `true` if the page was mapped (retry the instruction), `false`
/// if the address is not covered by any loaded DLL (real fault).
pub fn handle_dll_demand_page(vaddr: u64) -> bool {
    // Quick range check — DLL region is 0x04000000-0x07FFFFFF
    if vaddr < 0x0400_0000 || vaddr >= 0x0800_0000 {
        return false;
    }

    let page_base = vaddr & !0xFFF;

    let dlls = LOADED_DLLS.lock();
    for dll in dlls.iter() {
        let dll_end = dll.base_vaddr + (dll.pages.len() as u64) * PAGE_SIZE;
        if page_base >= dll.base_vaddr && page_base < dll_end {
            let page_idx = ((page_base - dll.base_vaddr) / PAGE_SIZE) as usize;
            let phys = dll.pages[page_idx];
            // Map with Present | User (read-only, executable).
            // We're already running with the faulting process's CR3,
            // so map_page uses recursive mapping on the correct PD.
            virtual_mem::map_page(VirtAddr::new(page_base), phys, PAGE_USER);
            return true;
        }
    }
    false
}

/// Load a DLL dynamically at runtime from the filesystem.
/// Allocates a virtual address from the DLL region, loads data, registers it.
/// Returns the base virtual address on success, or None.
pub fn load_dll_dynamic(path: &str) -> Option<u64> {
    // Check if already loaded (by filename)
    if let Some(base) = get_dll_base(path) {
        return Some(base);
    }

    // Read file from VFS
    let data = match crate::fs::vfs::read_file_to_vec(path) {
        Ok(d) => d,
        Err(_) => {
            crate::serial_println!("  dload: failed to read '{}'", path);
            return None;
        }
    };

    if data.len() < 32 {
        crate::serial_println!("  dload: file too small: '{}'", path);
        return None;
    }

    // Validate DLIB magic
    if &data[0..4] != b"DLIB" {
        crate::serial_println!("  dload: invalid magic in '{}'", path);
        return None;
    }

    let num_pages = (data.len() as u64 + PAGE_SIZE - 1) / PAGE_SIZE;

    // Allocate base address (atomic bump allocator, page-aligned)
    let aligned_size = num_pages * PAGE_SIZE;
    let base = NEXT_DYNAMIC_BASE.fetch_add(aligned_size, Ordering::SeqCst);

    // Sanity check: stay within DLL range (0x04000000 - 0x07FFFFFF)
    if base + aligned_size > 0x0800_0000 {
        crate::serial_println!("  dload: DLL address space exhausted at {:#x}", base);
        return None;
    }

    // Allocate physical frames and copy data
    let temp_virt = VirtAddr::new(0xFFFF_FFFF_81F1_0000);
    let mut pages = Vec::with_capacity(num_pages as usize);

    for i in 0..num_pages {
        let frame = match physical::alloc_frame() {
            Some(f) => f,
            None => {
                crate::serial_println!("  dload: OOM allocating frame for '{}'", path);
                return None;
            }
        };

        virtual_mem::map_page(temp_virt, frame, 0x02);

        let offset = (i * PAGE_SIZE) as usize;
        let remaining = data.len() - offset;
        let copy_len = remaining.min(PAGE_SIZE as usize);

        unsafe {
            let dest = temp_virt.as_u64() as *mut u8;
            core::ptr::copy_nonoverlapping(data.as_ptr().add(offset), dest, copy_len);
            if copy_len < PAGE_SIZE as usize {
                core::ptr::write_bytes(dest.add(copy_len), 0, PAGE_SIZE as usize - copy_len);
            }
        }

        virtual_mem::unmap_page(temp_virt);
        pages.push(frame);
    }

    // Register in loaded DLLs
    let mut name_buf = [0u8; 32];
    let name = path.rsplit('/').next().unwrap_or(path);
    let len = name.len().min(31);
    name_buf[..len].copy_from_slice(&name.as_bytes()[..len]);

    let mut dlls = LOADED_DLLS.lock();
    dlls.push(LoadedDll {
        name: name_buf,
        base_vaddr: base,
        pages,
    });

    crate::serial_println!(
        "[OK] dload: '{}' at {:#010x} ({} pages, {} bytes)",
        name, base, num_pages, data.len()
    );

    Some(base)
}

/// Get the base address of a loaded DLL by path name.
pub fn get_dll_base(path: &str) -> Option<u64> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let dlls = LOADED_DLLS.lock();
    for dll in dlls.iter() {
        let dll_name_len = dll.name.iter().position(|&b| b == 0).unwrap_or(32);
        if let Ok(dll_name) = core::str::from_utf8(&dll.name[..dll_name_len]) {
            if dll_name == name {
                return Some(dll.base_vaddr);
            }
        }
    }
    None
}

/// Check if a PD index (within PML4[0]/PDPT[0]) falls in the DLL region.
pub fn is_dll_pd(pd_idx: usize) -> bool {
    pd_idx >= DLL_PD_START && pd_idx <= DLL_PD_END
}
