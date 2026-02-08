//! DLL (Dynamic Link Library) loader and registry.
//!
//! DLLs are stateless shared code mapped read-only into every user process
//! at fixed virtual addresses. Physical frames are allocated once and shared.

use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;

const PAGE_SIZE: u32 = 4096;
const PAGE_USER: u32 = 0x04;

/// DLL virtual address range: 0x04000000 - 0x07FFFFFF.
/// Above identity-mapped region, below user programs (0x08000000).
pub const DLL_PDE_START: usize = 16;  // 0x04000000 >> 22
pub const DLL_PDE_END: usize = 31;    // 0x07FFFFFF >> 22

/// A loaded DLL: name, base virtual address, and backing physical frames.
///
/// Physical frames are allocated once at load time and shared (read-only)
/// across all user processes.
pub struct LoadedDll {
    /// Short filename (null-terminated) extracted from the load path.
    pub name: [u8; 32],
    /// Virtual address where this DLL is mapped in every user process.
    pub base_vaddr: u32,
    /// Physical frames holding the DLL code/data, in page order.
    pub pages: Vec<PhysAddr>,
}

static LOADED_DLLS: Spinlock<Vec<LoadedDll>> = Spinlock::new(Vec::new());

/// Load a DLL from the filesystem into physical memory.
/// Returns the number of pages loaded, or an error string.
pub fn load_dll(path: &str, base_vaddr: u32) -> Result<u32, &'static str> {
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

    let num_pages = (data.len() as u32 + PAGE_SIZE - 1) / PAGE_SIZE;
    let mut pages = Vec::with_capacity(num_pages as usize);

    // Allocate physical frames and copy DLL data page by page.
    // Use a temporary virtual address for each frame.
    let temp_virt = VirtAddr::new(0xC1F1_0000);

    for i in 0..num_pages {
        let frame = physical::alloc_frame()
            .ok_or("Out of memory allocating DLL frame")?;

        virtual_mem::map_page(temp_virt, frame, 0x02); // writable

        let offset = (i * PAGE_SIZE) as usize;
        let remaining = data.len() - offset;
        let copy_len = remaining.min(PAGE_SIZE as usize);

        unsafe {
            let dest = temp_virt.as_u32() as *mut u8;
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

    Ok(num_pages)
}

/// Map all loaded DLLs into a process page directory.
/// Pages are mapped as Present | User (read-only, executable).
pub fn map_all_dlls_into(pd_phys: PhysAddr) {
    let dlls = LOADED_DLLS.lock();
    for dll in dlls.iter() {
        for (i, &frame) in dll.pages.iter().enumerate() {
            let virt = VirtAddr::new(dll.base_vaddr + (i as u32) * PAGE_SIZE);
            virtual_mem::map_page_in_pd(pd_phys, virt, frame, PAGE_USER);
        }
    }
}

/// Get the base address of a loaded DLL by path name.
pub fn get_dll_base(path: &str) -> Option<u32> {
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

/// Check if a PDE index falls in the DLL region.
pub fn is_dll_pde(pde_idx: usize) -> bool {
    pde_idx >= DLL_PDE_START && pde_idx <= DLL_PDE_END
}
