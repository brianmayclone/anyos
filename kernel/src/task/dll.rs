//! Dynamic library loader and registry.
//!
//! Supports two formats:
//! - **DLIB v3**: Proprietary format (4096-byte header + flat pages). Used by boot-time DLLs.
//! - **ELF64 ET_DYN**: Standard ELF shared objects linked by anyld. Used for new libraries.
//!
//! Both formats share the same runtime model:
//! - `.rodata` + `.text` pages are shared read-only across all processes.
//! - `.data` pages are per-process (copied from template on demand fault).
//! - `.bss` pages are per-process (zeroed on demand fault).
//!
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

/// Check if this CPU holds the LOADED_DLLS lock.
pub fn is_dll_locked_by_cpu(cpu: u32) -> bool {
    LOADED_DLLS.is_held_by_cpu(cpu)
}

/// Force-release the LOADED_DLLS lock unconditionally.
///
/// # Safety
/// Must only be called when `is_dll_locked_by_cpu(cpu)` returns true
/// for the current CPU. The DLL registry may be in an inconsistent state.
pub unsafe fn force_unlock_dlls() {
    LOADED_DLLS.force_unlock();
}

// ── Header parsing helpers ─────────────────────────────────

fn read_u32_le(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_u16_le(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
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

// ── ELF64 constants ──────────────────────────────────────────

const ELFCLASS64: u8 = 2;
const ELFDATA2LSB: u8 = 1;
const ET_DYN: u16 = 3;
const EM_X86_64: u16 = 62;
const PT_LOAD: u32 = 1;
const PF_W: u32 = 2;

// ELF64 header offsets
const EI_CLASS: usize = 4;
const EI_DATA: usize = 5;
const E_TYPE: usize = 16;
const E_MACHINE: usize = 18;
const E_PHOFF: usize = 32;
const E_PHENTSIZE: usize = 54;
const E_PHNUM: usize = 56;

// ELF64 Phdr offsets (each entry is 56 bytes)
const PH_TYPE: usize = 0;
const PH_FLAGS: usize = 4;
const PH_OFFSET: usize = 8;
const PH_VADDR: usize = 16;
const PH_FILESZ: usize = 32;
const PH_MEMSZ: usize = 40;

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

// ── ELF64 ET_DYN loader ──────────────────────────────────────

/// Load an ELF64 ET_DYN shared object into physical memory.
///
/// Parses PT_LOAD segments from the ELF file:
/// - RX segment (no PF_W) → shared read-only pages
/// - RW segment (PF_W) → per-process .data template + .bss
///
/// All relocations are pre-applied by anyld, so no runtime relocation is needed.
/// The virtual addresses in PT_LOAD headers are absolute — the kernel maps pages
/// at those exact addresses.
fn load_elf64_so(data: &[u8], path: &str) -> Option<u64> {
    // ── Validate ELF64 header ──
    if data.len() < 64 {
        crate::serial_println!("  dload: ELF too small");
        return None;
    }
    if data[EI_CLASS] != ELFCLASS64 {
        crate::serial_println!("  dload: not ELF64");
        return None;
    }
    if data[EI_DATA] != ELFDATA2LSB {
        crate::serial_println!("  dload: not little-endian");
        return None;
    }
    if read_u16_le(data, E_TYPE) != ET_DYN {
        crate::serial_println!("  dload: not ET_DYN");
        return None;
    }
    if read_u16_le(data, E_MACHINE) != EM_X86_64 {
        crate::serial_println!("  dload: not x86_64");
        return None;
    }

    let phoff = read_u64_le(data, E_PHOFF) as usize;
    let phentsize = read_u16_le(data, E_PHENTSIZE) as usize;
    let phnum = read_u16_le(data, E_PHNUM) as usize;

    if phentsize < 56 || phoff + phnum * phentsize > data.len() {
        crate::serial_println!("  dload: invalid program headers");
        return None;
    }

    // ── Collect PT_LOAD segments ──
    // anyld produces exactly 2 PT_LOAD segments: RX (code+metadata) and RW (data+dynamic+bss)
    let mut ro_vaddr: u64 = u64::MAX;
    let mut ro_offset: u64 = 0;
    let mut ro_filesz: u64 = 0;
    let mut rw_vaddr: u64 = 0;
    let mut rw_offset: u64 = 0;
    let mut rw_filesz: u64 = 0;
    let mut rw_memsz: u64 = 0;
    let mut has_ro = false;
    let mut has_rw = false;

    for i in 0..phnum {
        let ph = phoff + i * phentsize;
        let p_type = read_u32_le(data, ph + PH_TYPE);
        if p_type != PT_LOAD {
            continue;
        }

        let p_flags = read_u32_le(data, ph + PH_FLAGS);
        let p_offset = read_u64_le(data, ph + PH_OFFSET);
        let p_vaddr = read_u64_le(data, ph + PH_VADDR);
        let p_filesz = read_u64_le(data, ph + PH_FILESZ);
        let p_memsz = read_u64_le(data, ph + PH_MEMSZ);

        if (p_flags & PF_W) == 0 {
            // RX segment (read-only, executable)
            ro_vaddr = p_vaddr;
            ro_offset = p_offset;
            ro_filesz = p_filesz;
            has_ro = true;
        } else {
            // RW segment (data + bss)
            rw_vaddr = p_vaddr;
            rw_offset = p_offset;
            rw_filesz = p_filesz;
            rw_memsz = p_memsz;
            has_rw = true;
        }
    }

    if !has_ro {
        crate::serial_println!("  dload: no RX PT_LOAD segment");
        return None;
    }

    // ── Determine base virtual address ──
    let base = ro_vaddr; // Lowest PT_LOAD vaddr (anyld sets this to -b value)

    // If base is 0, allocate dynamically
    let base = if base == 0 {
        // Calculate total virtual size
        let total_vsize = if has_rw {
            let rw_end = rw_vaddr + rw_memsz;
            rw_end // vaddr is relative to 0 when base=0
        } else {
            let ro_end_page = (ro_filesz + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
            ro_end_page
        };
        let aligned_size = (total_vsize + PAGE_SIZE - 1) & !(PAGE_SIZE - 1);
        let b = NEXT_DYNAMIC_BASE.fetch_add(aligned_size, Ordering::SeqCst);
        if b + aligned_size > 0x0800_0000 {
            crate::serial_println!("  dload: address space exhausted");
            return None;
        }
        // Cannot relocate — anyld already resolved all symbols at link time.
        // Base=0 with anyld means all addresses are 0-based, which conflicts
        // with user address space. For now, reject base=0 .so files.
        crate::serial_println!("  dload: base=0 not supported for .so (anyld requires fixed base)");
        return None;
    } else {
        base
    };

    // Sanity: stay within DLIB range
    // If there's an RW segment, extend the RO region to cover the gap between
    // the end of file content and the start of the RW segment. This ensures
    // the page fault handler maps data pages at the correct virtual address.
    let ro_page_count = if has_rw && rw_vaddr > ro_vaddr {
        ((rw_vaddr - ro_vaddr + PAGE_SIZE - 1) / PAGE_SIZE) as u32
    } else {
        ((ro_filesz + PAGE_SIZE - 1) / PAGE_SIZE) as u32
    };
    let data_page_count = if has_rw {
        ((rw_filesz + PAGE_SIZE - 1) / PAGE_SIZE) as u32
    } else {
        0
    };
    let bss_size = if has_rw && rw_memsz > rw_filesz {
        rw_memsz - rw_filesz
    } else {
        0
    };
    // BSS immediately follows the .data page(s) in virtual space.
    // anyld may place .dynamic after .data within the same pages, so BSS starts
    // after the RW file content (page-aligned).
    let bss_page_count = ((bss_size + PAGE_SIZE - 1) / PAGE_SIZE) as u32;
    let total_pages = ro_page_count + data_page_count + bss_page_count;

    let end_vaddr = base + (total_pages as u64) * PAGE_SIZE;
    if end_vaddr > 0x0800_0000 {
        crate::serial_println!("  dload: .so at {:#x} exceeds DLIB range", base);
        return None;
    }

    // ── Check for address conflict ──
    {
        let dlls = LOADED_DLLS.lock();
        for dll in dlls.iter() {
            let dll_end = dll.base_vaddr + (dll.total_pages as u64) * PAGE_SIZE;
            if base < dll_end && end_vaddr > dll.base_vaddr {
                crate::serial_println!(
                    "  dload: address conflict: .so at {:#x} overlaps {} at {:#x}",
                    base,
                    core::str::from_utf8(&dll.name).unwrap_or("?"),
                    dll.base_vaddr
                );
                return None;
            }
        }
    }

    // ── Allocate and copy RO pages ──
    let temp_virt = VirtAddr::new(0xFFFF_FFFF_81F1_0000);
    let mut ro_pages = Vec::with_capacity(ro_page_count as usize);

    for i in 0..ro_page_count as usize {
        let frame = physical::alloc_frame().expect("OOM in .so RO page");
        virtual_mem::map_page(temp_virt, frame, PAGE_WRITABLE);

        let file_off = ro_offset as usize + i * PAGE_SIZE as usize;
        let byte_offset = i * PAGE_SIZE as usize;
        let copy_len = if byte_offset >= ro_filesz as usize {
            0 // Gap page between RO file content and RW segment — stays zeroed
        } else {
            core::cmp::min(PAGE_SIZE as usize, ro_filesz as usize - byte_offset)
        };
        unsafe {
            let dest = temp_virt.as_u64() as *mut u8;
            // Zero the page first (handles partial last page)
            core::ptr::write_bytes(dest, 0, PAGE_SIZE as usize);
            if copy_len > 0 && file_off + copy_len <= data.len() {
                core::ptr::copy_nonoverlapping(data.as_ptr().add(file_off), dest, copy_len);
            }
        }

        virtual_mem::unmap_page(temp_virt);
        ro_pages.push(frame);
    }

    // ── Allocate and copy .data template pages ──
    let mut data_template_pages = Vec::with_capacity(data_page_count as usize);

    for i in 0..data_page_count as usize {
        let frame = physical::alloc_frame().expect("OOM in .so data template page");
        virtual_mem::map_page(temp_virt, frame, PAGE_WRITABLE);

        let file_off = rw_offset as usize + i * PAGE_SIZE as usize;
        let copy_len = core::cmp::min(PAGE_SIZE as usize, rw_filesz as usize - i * PAGE_SIZE as usize);
        unsafe {
            let dest = temp_virt.as_u64() as *mut u8;
            // Zero the page first (handles .dynamic padding and partial pages)
            core::ptr::write_bytes(dest, 0, PAGE_SIZE as usize);
            if copy_len > 0 && file_off + copy_len <= data.len() {
                core::ptr::copy_nonoverlapping(data.as_ptr().add(file_off), dest, copy_len);
            }
        }

        virtual_mem::unmap_page(temp_virt);
        data_template_pages.push(frame);
    }

    // ── Update NEXT_DYNAMIC_BASE if this fixed-base .so consumed the space ──
    loop {
        let current = NEXT_DYNAMIC_BASE.load(Ordering::SeqCst);
        if end_vaddr <= current {
            break; // Already past this .so
        }
        if NEXT_DYNAMIC_BASE
            .compare_exchange(current, end_vaddr, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            break;
        }
    }

    // ── Register in loaded DLLs ──
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
        data_page_count,
        bss_page_count,
        total_pages,
    });

    crate::serial_println!(
        "[OK] dload ELF64 ET_DYN: '{}' at {:#010x} ({} RO + {} data + {} BSS pages)",
        name,
        base,
        ro_page_count,
        data_page_count,
        bss_page_count
    );

    Some(base)
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

/// Load a shared library dynamically at runtime from the filesystem.
/// Supports both DLIB v3 (.dlib) and ELF64 ET_DYN (.so) formats.
/// Returns the base virtual address on success.
pub fn load_dll_dynamic(path: &str) -> Option<u64> {
    // Validate extension
    if !path.ends_with(".dlib") && !path.ends_with(".so") {
        crate::serial_println!("  dload: unsupported extension: '{}'", path);
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

    // Dispatch based on file magic
    if data.len() >= 4 && &data[0..4] == b"\x7fELF" {
        return load_elf64_so(&data, path);
    }
    if data.len() >= 4 && &data[0..4] == b"DLIB" {
        return load_dlib_v3_dynamic(&data, path);
    }

    crate::serial_println!("  dload: unrecognized file format in '{}'", path);
    None
}

/// Load a DLIB v3 file dynamically. Called from load_dll_dynamic after magic check.
fn load_dlib_v3_dynamic(data: &[u8], path: &str) -> Option<u64> {
    let (base_vaddr, ro_count, data_count, bss_count, total) = match parse_dlib_header(data) {
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

    let ro_pages = match alloc_and_copy_pages(data, content_base, ro_count as usize, temp_virt) {
        Ok(p) => p,
        Err(_) => {
            crate::serial_println!("  dload: OOM allocating RO pages for '{}'", path);
            return None;
        }
    };

    let data_offset = content_base + ro_count as usize * PAGE_SIZE as usize;
    let data_template_pages =
        match alloc_and_copy_pages(data, data_offset, data_count as usize, temp_virt) {
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
