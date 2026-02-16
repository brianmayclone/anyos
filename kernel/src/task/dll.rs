//! DLIB v3 (Dynamic Library) loader and registry.
//!
//! DLIBs are shared code mapped into every user process at fixed virtual addresses.
//! - `.rodata` + `.text` pages are shared read-only across all processes.
//! - `.data` pages are per-process (copied from template on demand fault).
//! - `.bss` pages are per-process (zeroed on demand fault).
//!
//! File format: 4096-byte header + RO content + .data template content.
//! The PAGE_WRITABLE bit on PTEs distinguishes per-process (free on destroy)
//! from shared (skip on destroy).

use crate::memory::address::{PhysAddr, VirtAddr};
use crate::memory::physical;
use crate::memory::virtual_mem;
use crate::sync::spinlock::Spinlock;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

const PAGE_SIZE: u64 = 4096;
const PAGE_USER: u64 = 0x04;
const PAGE_WRITABLE: u64 = 0x02;

/// Next available virtual address for dynamically loaded DLIBs.
/// Starts after the last boot-time DLIB (0x0440_0000), incremented per load.
static NEXT_DYNAMIC_BASE: AtomicU64 = AtomicU64::new(0x0440_0000);

/// DLIB virtual address range: 0x04000000 - 0x07FFFFFF.
/// In x86-64 4-level paging, these are PML4[0], PDPT[0], PD[32..63].
pub const DLL_PD_START: usize = 32; // 0x04000000 >> 21 & 0x1FF
pub const DLL_PD_END: usize = 63; // 0x07FFFFFF >> 21 & 0x1FF

/// Temp virtual addresses for demand-page copy operations.
/// Used only while LOADED_DLLS lock is held (serialized).
const TEMP_COPY_SRC: u64 = 0xFFFF_FFFF_81F2_0000;
const TEMP_COPY_DST: u64 = 0xFFFF_FFFF_81F2_1000;

/// A loaded DLIB: name, base virtual address, section pages, and metadata.
pub struct LoadedDll {
    /// Short filename (null-terminated) extracted from the load path.
    pub name: [u8; 32],
    /// Virtual address where this DLIB is mapped in every user process.
    pub base_vaddr: u64,
    /// Shared read-only physical frames (.rodata + .text), mapped into every process.
    pub ro_pages: Vec<PhysAddr>,
    /// .data template physical frames (kernel-private, never mapped into user space).
    /// On demand fault, kernel allocates a fresh frame, copies from template, maps writable.
    pub data_template_pages: Vec<PhysAddr>,
    /// Number of per-process .data pages.
    pub data_page_count: u32,
    /// Number of per-process .bss pages (zeroed on demand).
    pub bss_page_count: u32,
    /// Total virtual pages: ro_pages.len() + data_page_count + bss_page_count.
    pub total_pages: u32,
}

static LOADED_DLLS: Spinlock<Vec<LoadedDll>> = Spinlock::new(Vec::new());

// ── Header parsing helpers ─────────────────────────────────

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_u64_le(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

/// Parse and validate a DLIB v3 header.
/// Returns (base_vaddr, ro_pages, data_pages, bss_pages, total_pages).
fn parse_dlib_header(data: &[u8]) -> Result<(u64, u32, u32, u32, u32), &'static str> {
    if data.len() < PAGE_SIZE as usize {
        return Err("DLIB file too small for header");
    }
    if &data[0..4] != b"DLIB" {
        return Err("Invalid DLIB magic (expected DLIB)");
    }
    let version = read_u32_le(data, 0x04);
    if version != 3 {
        return Err("Unsupported DLIB version (expected 3)");
    }

    let base_vaddr = read_u64_le(data, 0x10);
    let ro_pages = read_u32_le(data, 0x18);
    let data_pages = read_u32_le(data, 0x1C);
    let bss_pages = read_u32_le(data, 0x20);
    let total_pages = read_u32_le(data, 0x24);

    if total_pages != ro_pages + data_pages + bss_pages {
        return Err("DLIB total_pages mismatch");
    }

    let expected_file_size =
        PAGE_SIZE as usize + (ro_pages as usize + data_pages as usize) * PAGE_SIZE as usize;
    if data.len() < expected_file_size {
        return Err("DLIB file truncated");
    }

    Ok((base_vaddr, ro_pages, data_pages, bss_pages, total_pages))
}

/// Allocate physical frames and copy page content from file data.
/// `file_offset` is the byte offset in `data` where the first page starts.
fn alloc_and_copy_pages(
    data: &[u8],
    file_offset: usize,
    count: usize,
    temp_virt: VirtAddr,
) -> Result<Vec<PhysAddr>, &'static str> {
    let mut pages = Vec::with_capacity(count);
    for i in 0..count {
        let frame = physical::alloc_frame().ok_or("Out of memory allocating DLIB frame")?;
        virtual_mem::map_page(temp_virt, frame, PAGE_WRITABLE);

        let offset = file_offset + i * PAGE_SIZE as usize;
        unsafe {
            let dest = temp_virt.as_u64() as *mut u8;
            core::ptr::copy_nonoverlapping(data.as_ptr().add(offset), dest, PAGE_SIZE as usize);
        }

        virtual_mem::unmap_page(temp_virt);
        pages.push(frame);
    }
    Ok(pages)
}

// ── Public API ─────────────────────────────────────────────

/// Load a DLIB from the filesystem into physical memory.
/// Returns the total number of virtual pages, or an error string.
pub fn load_dll(path: &str, expected_base: u64) -> Result<u32, &'static str> {
    // Validate .dlib extension
    if !path.ends_with(".dlib") {
        return Err("Invalid file extension (expected .dlib)");
    }

    // Check if already loaded at this address
    {
        let dlls = LOADED_DLLS.lock();
        for dll in dlls.iter() {
            if dll.base_vaddr == expected_base {
                return Ok(dll.total_pages);
            }
        }
    }

    let data = crate::fs::vfs::read_file_to_vec(path).map_err(|_| "Failed to read DLIB file")?;

    let (base_vaddr, ro_count, data_count, bss_count, total) = parse_dlib_header(&data)?;

    if base_vaddr != expected_base {
        return Err("DLIB base_vaddr does not match expected address");
    }

    let temp_virt = VirtAddr::new(0xFFFF_FFFF_81F1_0000);
    let content_base = PAGE_SIZE as usize; // Skip header page

    // Allocate shared RO pages (.rodata + .text)
    let ro_pages = alloc_and_copy_pages(&data, content_base, ro_count as usize, temp_virt)?;

    // Allocate .data template pages (kernel-private, used for per-process copy on demand)
    let data_offset = content_base + ro_count as usize * PAGE_SIZE as usize;
    let data_template_pages =
        alloc_and_copy_pages(&data, data_offset, data_count as usize, temp_virt)?;

    // Extract short name from path
    let mut name_buf = [0u8; 32];
    let name = path.rsplit('/').next().unwrap_or(path);
    let len = name.len().min(31);
    name_buf[..len].copy_from_slice(&name.as_bytes()[..len]);

    let mut dlls = LOADED_DLLS.lock();
    dlls.push(LoadedDll {
        name: name_buf,
        base_vaddr,
        ro_pages,
        data_template_pages,
        data_page_count: data_count,
        bss_page_count: bss_count,
        total_pages: total,
    });

    crate::serial_println!(
        "[OK] DLIB v3: {} at {:#010x} ({} RO + {} data + {} BSS pages)",
        name,
        base_vaddr,
        ro_count,
        data_count,
        bss_count
    );

    Ok(total)
}

/// Map all loaded DLIBs' shared RO pages into a process page directory.
/// Per-process .data/.bss pages are NOT pre-mapped — they are demand-paged
/// via handle_dll_demand_page() on first access.
pub fn map_all_dlls_into(pd_phys: PhysAddr) {
    // Phase 1: Under lock — collect (virt, phys) pairs for RO pages only
    let page_maps: Vec<(VirtAddr, PhysAddr)> = {
        let dlls = LOADED_DLLS.lock();
        let mut v = Vec::new();
        for dll in dlls.iter() {
            for (i, &frame) in dll.ro_pages.iter().enumerate() {
                v.push((
                    VirtAddr::new(dll.base_vaddr + (i as u64) * PAGE_SIZE),
                    frame,
                ));
            }
        }
        v
    }; // Lock dropped — interrupts re-enabled

    // Phase 2: Map RO pages without holding the lock
    for &(virt, phys) in &page_maps {
        virtual_mem::map_page_in_pd(pd_phys, virt, phys, PAGE_USER);
    }
}

/// Handle a demand-page fault for DLIB pages.
///
/// Called from the page fault handler (ISR 14) when a user process accesses
/// an unmapped page in the DLIB virtual range (0x04000000-0x07FFFFFF).
///
/// - RO pages: map shared physical frame (read-only, executable).
/// - .data pages: allocate fresh frame, copy from template, map writable.
/// - .bss pages: allocate fresh frame, zero it, map writable.
///
/// Returns `true` if the page was mapped (retry the instruction), `false`
/// if the address is not covered by any loaded DLIB (real fault).
pub fn handle_dll_demand_page(vaddr: u64) -> bool {
    // Quick range check — DLIB region is 0x04000000-0x07FFFFFF
    if vaddr < 0x0400_0000 || vaddr >= 0x0800_0000 {
        return false;
    }

    let page_base = vaddr & !0xFFF;

    let dlls = LOADED_DLLS.lock();
    for dll in dlls.iter() {
        let dll_end = dll.base_vaddr + (dll.total_pages as u64) * PAGE_SIZE;
        if page_base >= dll.base_vaddr && page_base < dll_end {
            let page_idx = ((page_base - dll.base_vaddr) / PAGE_SIZE) as usize;
            let ro_count = dll.ro_pages.len();
            let data_count = dll.data_page_count as usize;

            if page_idx < ro_count {
                // Shared RO page — map existing shared frame
                let phys = dll.ro_pages[page_idx];
                virtual_mem::map_page(VirtAddr::new(page_base), phys, PAGE_USER);
            } else if page_idx < ro_count + data_count {
                // Per-process .data page — copy from template
                let template_idx = page_idx - ro_count;
                let template_phys = dll.data_template_pages[template_idx];
                let new_frame = physical::alloc_frame().expect("OOM in DLIB .data demand page");

                // Copy template → new frame via temp kernel mappings.
                // Safe: LOADED_DLLS lock serializes access to these temp addresses.
                let src = VirtAddr::new(TEMP_COPY_SRC);
                let dst = VirtAddr::new(TEMP_COPY_DST);
                virtual_mem::map_page(src, template_phys, 0); // read-only
                virtual_mem::map_page(dst, new_frame, PAGE_WRITABLE);
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        src.as_u64() as *const u8,
                        dst.as_u64() as *mut u8,
                        PAGE_SIZE as usize,
                    );
                }
                virtual_mem::unmap_page(src);
                virtual_mem::unmap_page(dst);

                virtual_mem::map_page(
                    VirtAddr::new(page_base),
                    new_frame,
                    PAGE_USER | PAGE_WRITABLE,
                );
            } else {
                // Per-process .bss page — zero-fill
                let new_frame = physical::alloc_frame().expect("OOM in DLIB .bss demand page");

                let tmp = VirtAddr::new(TEMP_COPY_SRC);
                virtual_mem::map_page(tmp, new_frame, PAGE_WRITABLE);
                unsafe {
                    core::ptr::write_bytes(tmp.as_u64() as *mut u8, 0, PAGE_SIZE as usize);
                }
                virtual_mem::unmap_page(tmp);

                virtual_mem::map_page(
                    VirtAddr::new(page_base),
                    new_frame,
                    PAGE_USER | PAGE_WRITABLE,
                );
            }
            return true;
        }
    }
    false
}

/// Load a DLIB dynamically at runtime from the filesystem.
/// Reads base_vaddr from the DLIB v3 header. Returns the base on success.
pub fn load_dll_dynamic(path: &str) -> Option<u64> {
    // Validate .dlib extension
    if !path.ends_with(".dlib") {
        crate::serial_println!("  dload: invalid extension: '{}'", path);
        return None;
    }

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

    let (base_vaddr, ro_count, data_count, bss_count, total) = match parse_dlib_header(&data) {
        Ok(h) => h,
        Err(e) => {
            crate::serial_println!("  dload: header error in '{}': {}", path, e);
            return None;
        }
    };

    // For dynamic loading, use the header's base_vaddr (DLIBs are position-dependent).
    // If no explicit base, allocate from NEXT_DYNAMIC_BASE.
    let base = if base_vaddr != 0 {
        base_vaddr
    } else {
        let aligned_size = (total as u64) * PAGE_SIZE;
        let b = NEXT_DYNAMIC_BASE.fetch_add(aligned_size, Ordering::SeqCst);
        if b + aligned_size > 0x0800_0000 {
            crate::serial_println!("  dload: DLIB address space exhausted at {:#x}", b);
            return None;
        }
        b
    };

    // Sanity check: stay within DLIB range
    if base + (total as u64) * PAGE_SIZE > 0x0800_0000 {
        crate::serial_println!("  dload: DLIB at {:#x} exceeds range", base);
        return None;
    }

    let temp_virt = VirtAddr::new(0xFFFF_FFFF_81F1_0000);
    let content_base = PAGE_SIZE as usize;

    let ro_pages = match alloc_and_copy_pages(&data, content_base, ro_count as usize, temp_virt) {
        Ok(p) => p,
        Err(_) => {
            crate::serial_println!("  dload: OOM allocating RO pages for '{}'", path);
            return None;
        }
    };

    let data_offset = content_base + ro_count as usize * PAGE_SIZE as usize;
    let data_template_pages =
        match alloc_and_copy_pages(&data, data_offset, data_count as usize, temp_virt) {
            Ok(p) => p,
            Err(_) => {
                crate::serial_println!("  dload: OOM allocating data template for '{}'", path);
                return None;
            }
        };

    // Register in loaded DLIBs
    let mut name_buf = [0u8; 32];
    let name = path.rsplit('/').next().unwrap_or(path);
    let len = name.len().min(31);
    name_buf[..len].copy_from_slice(&name.as_bytes()[..len]);

    let mut dlls = LOADED_DLLS.lock();
    dlls.push(LoadedDll {
        name: name_buf,
        base_vaddr: base,
        ro_pages,
        data_template_pages,
        data_page_count: data_count,
        bss_page_count: bss_count,
        total_pages: total,
    });

    crate::serial_println!(
        "[OK] dload DLIB v3: '{}' at {:#010x} ({} RO + {} data + {} BSS pages)",
        name,
        base,
        ro_count,
        data_count,
        bss_count
    );

    Some(base)
}

/// Get the base address of a loaded DLIB by path name.
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

/// Check if a PD index (within PML4[0]/PDPT[0]) falls in the DLIB region.
pub fn is_dll_pd(pd_idx: usize) -> bool {
    pd_idx >= DLL_PD_START && pd_idx <= DLL_PD_END
}

/// Write a u32 value to a shared DLIB page at the specified offset.
/// Used by SYS_SET_DLL_U32 to allow processes (e.g., compositor) to write
/// to shared read-only DLIB pages (e.g., theme field in uisys exports).
///
/// Returns true on success, false if dll_base/offset is invalid.
pub fn set_dll_u32(dll_base: u64, offset: u64, value: u32) -> bool {
    let dlls = LOADED_DLLS.lock();
    for dll in dlls.iter() {
        if dll.base_vaddr == dll_base {
            // Validate offset is within the shared RO region
            let ro_size = (dll.ro_pages.len() as u64) * PAGE_SIZE;
            if offset + 4 > ro_size {
                return false;
            }

            let page_idx = (offset / PAGE_SIZE) as usize;
            let page_offset = (offset % PAGE_SIZE) as usize;
            let phys = dll.ro_pages[page_idx];

            // Temporarily map the shared frame and write the value
            let tmp = VirtAddr::new(TEMP_COPY_SRC);
            virtual_mem::map_page(tmp, phys, PAGE_WRITABLE);
            unsafe {
                let ptr = (tmp.as_u64() as *mut u8).add(page_offset) as *mut u32;
                core::ptr::write_volatile(ptr, value);
            }
            virtual_mem::unmap_page(tmp);
            return true;
        }
    }
    false
}
