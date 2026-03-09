//! x86 page table walkers for all paging modes.
//!
//! Three paging modes are supported, matching the real hardware:
//!
//! - **2-level (32-bit):** CR3 -> PD[22:31] -> PT[12:21], 4 KiB pages.
//!   With PSE (CR4.PSE=1), PDE bit 7 enables 4 MiB huge pages.
//!
//! - **PAE (Physical Address Extension):** CR3 -> PDPT[30:31] (4 entries)
//!   -> PD[21:29] (512 entries) -> PT[12:20] (512 entries), 4 KiB pages.
//!   PDE bit 7 enables 2 MiB huge pages.
//!
//! - **4-level (IA-32e / long mode):** CR3 -> PML4[39:47] -> PDPT[30:38]
//!   -> PD[21:29] -> PT[12:20], 4 KiB pages. PDPTE bit 7 enables 1 GiB
//!   huge pages, PDE bit 7 enables 2 MiB huge pages.
//!
//! The `walk_page_tables` function dispatches to the correct walker based
//! on the `Mmu` configuration, which mirrors CR0/CR4/EFER state.

use crate::error::{Result, VmError};

use super::{AccessType, MemoryBus, Mmu};

// ── PTE bit definitions ──

/// Page present.
const PTE_P: u64 = 1 << 0;
/// Read/write: 0 = read-only, 1 = read-write.
const PTE_RW: u64 = 1 << 1;
/// User/supervisor: 0 = supervisor, 1 = user-accessible.
const PTE_US: u64 = 1 << 2;
/// Page size (huge page) in PDE/PDPTE.
const PTE_PS: u64 = 1 << 7;
/// No-execute (requires EFER.NXE=1).
const PTE_NX: u64 = 1u64 << 63;

/// Walk the guest page tables to translate a linear address to physical.
///
/// Dispatches to the correct page table walker based on the current paging
/// mode stored in `mmu`. Returns `VmError::PageFault` on access violations.
///
/// # Parameters
///
/// - `linear`: The linear (virtual) address to translate.
/// - `cr3`: The current page directory base register value.
/// - `access`: Whether this is a read, write, or instruction fetch.
/// - `cpl`: Current privilege level (0-3).
/// - `mmu`: Paging configuration (pae, long_mode, pse, wp, nxe).
/// - `mem`: Guest physical memory bus for reading page table entries.
pub fn walk_page_tables(
    linear: u64,
    cr3: u64,
    access: AccessType,
    cpl: u8,
    mmu: &Mmu,
    mem: &dyn MemoryBus,
    ram: *mut u8,
    ram_size: usize,
    rflags_ac: bool,
) -> Result<u64> {
    if mmu.long_mode {
        walk_4level(linear, cr3, access, cpl, mmu, mem, ram, ram_size, rflags_ac)
    } else if mmu.pae {
        walk_pae(linear, cr3, access, cpl, mmu, mem, ram, ram_size, rflags_ac)
    } else {
        walk_2level(linear, cr3, access, cpl, mmu, mem, ram, ram_size, rflags_ac)
    }
}

// ── Accessed/Dirty bit helpers ──
//
// Real x86 hardware sets the Accessed bit (bit 5) on every PTE/PDE it
// traverses, and the Dirty bit (bit 6) on the final PTE for writes.
// Operating systems (especially Windows) rely on these bits for virtual
// memory management (page replacement, copy-on-write, working set trimming).
//
// Page table entries always reside in flat guest RAM, so we write the
// bits directly via a raw pointer. The emulator is single-threaded.

const PTE_A: u64 = 1 << 5;
const PTE_D: u64 = 1 << 6;

/// Set the Accessed bit on a 32-bit PTE at `pte_addr` in guest RAM.
#[inline]
unsafe fn set_accessed_32(ram: *mut u8, ram_size: usize, pte_addr: u64, pte: u32) {
    if (pte & (1 << 5)) == 0 {
        raw_or_u32(ram, ram_size, pte_addr, 1 << 5);
    }
}

/// Set Accessed + Dirty bits on a 32-bit PTE.
#[inline]
unsafe fn set_accessed_dirty_32(ram: *mut u8, ram_size: usize, pte_addr: u64, pte: u32) {
    let needed = (1u32 << 5) | (1u32 << 6);
    if (pte & needed) != needed {
        raw_or_u32(ram, ram_size, pte_addr, needed);
    }
}

/// Set the Accessed bit on a 64-bit PTE.
#[inline]
unsafe fn set_accessed_64(ram: *mut u8, ram_size: usize, pte_addr: u64, pte: u64) {
    if (pte & PTE_A) == 0 {
        raw_or_u64(ram, ram_size, pte_addr, PTE_A);
    }
}

/// Set Accessed + Dirty bits on a 64-bit PTE.
#[inline]
unsafe fn set_accessed_dirty_64(ram: *mut u8, ram_size: usize, pte_addr: u64, pte: u64) {
    let needed = PTE_A | PTE_D;
    if (pte & needed) != needed {
        raw_or_u64(ram, ram_size, pte_addr, needed);
    }
}

/// OR bits into a 32-bit value in guest RAM.
#[inline]
unsafe fn raw_or_u32(ram: *mut u8, ram_size: usize, addr: u64, bits: u32) {
    let off = addr as usize;
    if off + 4 <= ram_size {
        let ptr = ram.add(off) as *mut u32;
        ptr.write_unaligned(ptr.read_unaligned() | bits);
    }
}

/// OR bits into a 64-bit value in guest RAM.
#[inline]
unsafe fn raw_or_u64(ram: *mut u8, ram_size: usize, addr: u64, bits: u64) {
    let off = addr as usize;
    if off + 8 <= ram_size {
        let ptr = ram.add(off) as *mut u64;
        ptr.write_unaligned(ptr.read_unaligned() | bits);
    }
}

// ── Permission check ──

/// Check that a paging-structure entry is present. For intermediate levels
/// only the Present bit needs to be verified; permission checks (U/S, R/W,
/// NX, SMAP, SMEP) must use the *combined* flags across all levels and are
/// deferred to [`check_final_permissions`].
#[inline]
fn check_present(pte: u64, access: AccessType, cpl: u8, linear: u64) -> Result<()> {
    if (pte & PTE_P) == 0 {
        return Err(VmError::PageFault {
            address: linear,
            error_code: access.to_pf_error_code(cpl, false),
        });
    }
    Ok(())
}

/// Permission check on the *effective* (combined) flags of the entire walk.
///
/// Intel SDM Vol. 3A §4.6: the effective U/S is the AND of U/S bits across
/// all levels; effective R/W is the AND of R/W bits; effective NX is the OR
/// of NX bits. SMEP/SMAP/WP operate on these effective permissions.
fn check_final_permissions(
    effective_us: bool,    // true = user-accessible (all levels had U/S=1)
    effective_rw: bool,    // true = writable (all levels had R/W=1)
    effective_nx: bool,    // true = NX (any level had NX=1)
    access: AccessType,
    cpl: u8,
    linear: u64,
    mmu: &Mmu,
    rflags_ac: bool,
) -> Result<()> {
    let is_user = cpl == 3;

    // User access to supervisor page.
    if is_user && !effective_us {
        return Err(VmError::PageFault {
            address: linear,
            error_code: access.to_pf_error_code(cpl, true),
        });
    }

    // SMEP: supervisor cannot execute from user-accessible pages.
    if !is_user && effective_us && mmu.smep && matches!(access, AccessType::Execute) {
        return Err(VmError::PageFault {
            address: linear,
            error_code: access.to_pf_error_code(cpl, true),
        });
    }

    // SMAP: supervisor cannot read/write user-accessible pages unless EFLAGS.AC=1.
    if !is_user && effective_us && mmu.smap && !rflags_ac {
        if matches!(access, AccessType::Read | AccessType::Write) {
            return Err(VmError::PageFault {
                address: linear,
                error_code: access.to_pf_error_code(cpl, true),
            });
        }
    }

    // Write access to read-only page.
    match access {
        AccessType::Write => {
            if !effective_rw {
                if is_user || mmu.wp {
                    return Err(VmError::PageFault {
                        address: linear,
                        error_code: access.to_pf_error_code(cpl, true),
                    });
                }
            }
        }
        AccessType::Execute => {
            if mmu.nxe && effective_nx {
                return Err(VmError::PageFault {
                    address: linear,
                    error_code: access.to_pf_error_code(cpl, true),
                });
            }
        }
        AccessType::Read => {
            // Reads are always allowed if the page is present and privilege is OK.
        }
    }

    Ok(())
}

/// Reserved bit mask for 64-bit PTEs (PAE/4-level): bits [51:MAXPHYADDR] are reserved.
/// For 40-bit physical address (Broadwell), bits [51:40] are reserved.
/// We use a conservative mask that checks bits [62:52] which are always reserved
/// in non-NX entries (bit 63 is NX, handled separately).
const RSVD_MASK_64: u64 = 0x7FF0_0000_0000_0000;

/// Check a 64-bit PTE for reserved bit violations.
/// Returns true if reserved bits are set (fault should use RSVD error code bit).
#[inline]
fn has_reserved_bits_64(pte: u64, is_large_page: bool) -> bool {
    // Bits [62:52] are always reserved in valid page table entries.
    // For large pages, additional bits in the address field may be reserved.
    if (pte & RSVD_MASK_64) != 0 {
        return true;
    }
    if is_large_page {
        // For 2MB pages: bits [20:13] of the PDE are reserved (must be 0).
        if (pte & 0x001F_E000) != 0 {
            return true;
        }
    }
    false
}

/// Generate a #PF with the RSVD bit (bit 3) set in the error code.
#[inline]
fn pf_rsvd(access: AccessType, cpl: u8, linear: u64) -> VmError {
    VmError::PageFault {
        address: linear,
        error_code: access.to_pf_error_code(cpl, true) | (1 << 3), // RSVD bit
    }
}

// ── 32-bit (2-level) paging ──

/// Walk a classic 32-bit two-level page table.
///
/// CR3 bits [31:12] point to the 4 KiB page directory. Each PDE maps 4 MiB
/// of linear address space. With PSE, PDE bit 7 creates a 4 MiB huge page;
/// otherwise the PDE points to a page table of 1024 PTEs covering 4 KiB each.
fn walk_2level(
    linear: u64,
    cr3: u64,
    access: AccessType,
    cpl: u8,
    mmu: &Mmu,
    mem: &dyn MemoryBus,
    ram: *mut u8,
    ram_size: usize,
    rflags_ac: bool,
) -> Result<u64> {
    let linear32 = linear as u32;
    let is_write = matches!(access, AccessType::Write);

    // PD index: bits [31:22].
    let pd_index = (linear32 >> 22) as u64;
    let pd_base = cr3 & 0xFFFFF000;
    let pde_addr = pd_base + pd_index * 4;
    let pde_raw = mem.read_u32(pde_addr)?;
    let pde = pde_raw as u64;

    check_present(pde, access, cpl, linear)?;

    // PSE 4 MiB huge page: PDE.PS=1 and CR4.PSE=1.
    if mmu.pse && (pde & PTE_PS) != 0 {
        // For huge pages the PDE is the final level — check permissions on it alone.
        let eff_us = (pde & PTE_US) != 0;
        let eff_rw = (pde & PTE_RW) != 0;
        // 32-bit non-PAE has no NX bit.
        check_final_permissions(eff_us, eff_rw, false, access, cpl, linear, mmu, rflags_ac)?;
        unsafe {
            if is_write {
                set_accessed_dirty_32(ram, ram_size, pde_addr, pde_raw);
            } else {
                set_accessed_32(ram, ram_size, pde_addr, pde_raw);
            }
        }
        let page_base = (pde & 0xFFC00000) as u64;
        let page_offset = (linear32 & 0x003FFFFF) as u64;
        return Ok(page_base | page_offset);
    }

    // Set Accessed on PDE (directory entries only get Accessed, not Dirty).
    unsafe { set_accessed_32(ram, ram_size, pde_addr, pde_raw); }

    // PT index: bits [21:12].
    let pt_index = ((linear32 >> 12) & 0x3FF) as u64;
    let pt_base = pde & 0xFFFFF000;
    let pte_addr = pt_base + pt_index * 4;
    let pte_raw = mem.read_u32(pte_addr)?;
    let pte = pte_raw as u64;

    // Combine effective permissions from PDE and PTE.
    let eff_us = (pde & PTE_US) != 0 && (pte & PTE_US) != 0;
    let eff_rw = (pde & PTE_RW) != 0 && (pte & PTE_RW) != 0;
    check_present(pte, access, cpl, linear)?;
    check_final_permissions(eff_us, eff_rw, false, access, cpl, linear, mmu, rflags_ac)?;

    // Set Accessed (+ Dirty for writes) on the final PTE.
    unsafe {
        if is_write {
            set_accessed_dirty_32(ram, ram_size, pte_addr, pte_raw);
        } else {
            set_accessed_32(ram, ram_size, pte_addr, pte_raw);
        }
    }

    // Physical address: PTE[31:12] || linear[11:0].
    let page_base = pte & 0xFFFFF000;
    let page_offset = (linear32 & 0xFFF) as u64;
    Ok(page_base | page_offset)
}

// ── PAE paging ──

/// Walk a PAE (Physical Address Extension) three-level page table.
///
/// CR3 bits [31:5] point to a 32-byte PDPT with 4 entries (bits [31:30]
/// index into it). Each PDPTE points to a 512-entry PD; each PDE either
/// maps a 2 MiB huge page (bit 7) or points to a 512-entry PT of 4 KiB pages.
/// PTEs and PDEs are 8 bytes wide, enabling physical addresses above 4 GiB.
fn walk_pae(
    linear: u64,
    cr3: u64,
    access: AccessType,
    cpl: u8,
    mmu: &Mmu,
    mem: &dyn MemoryBus,
    ram: *mut u8,
    ram_size: usize,
    rflags_ac: bool,
) -> Result<u64> {
    let linear32 = linear as u32;
    let is_write = matches!(access, AccessType::Write);

    // PDPT index: bits [31:30] (2 bits -> 4 entries).
    let pdpt_index = (linear32 >> 30) as u64;
    let pdpt_base = cr3 & 0xFFFFFFE0; // bits [31:5]
    let pdpte_addr = pdpt_base + pdpt_index * 8;
    let pdpte = mem.read_u64(pdpte_addr)?;

    // PDPTE present check (only bit 0 matters, no RW/US in PDPT).
    if (pdpte & PTE_P) == 0 {
        return Err(VmError::PageFault {
            address: linear,
            error_code: access.to_pf_error_code(cpl, false),
        });
    }
    // PAE PDPTE reserved bit check (Intel SDM Vol. 3A Table 4-7):
    // bits [63], [8:5], [2:1] are reserved. Only P(0), PWT(3), PCD(4),
    // and base address [51:12] are valid.
    const PAE_PDPTE_RSVD: u64 = (1u64 << 63)    // NX not valid in PDPTE
        | (0xF << 5)                              // bits [8:5]
        | (0x6);                                   // bits [2:1]
    if (pdpte & PAE_PDPTE_RSVD) != 0 || has_reserved_bits_64(pdpte, false) {
        return Err(pf_rsvd(access, cpl, linear));
    }
    // PAE PDPTEs do not have Accessed/Dirty bits per Intel spec.

    // PD index: bits [29:21] (9 bits -> 512 entries).
    let pd_index = ((linear32 >> 21) & 0x1FF) as u64;
    let pd_base = pdpte & 0x000FFFFF_FFFFF000;
    let pde_addr = pd_base + pd_index * 8;
    let pde = mem.read_u64(pde_addr)?;

    check_present(pde, access, cpl, linear)?;

    // 2 MiB huge page: PDE.PS=1 — PDE is the final level.
    if (pde & PTE_PS) != 0 {
        check_final_permissions(
            (pde & PTE_US) != 0,
            (pde & PTE_RW) != 0,
            (pde & PTE_NX) != 0,
            access, cpl, linear, mmu, rflags_ac,
        )?;
        unsafe {
            if is_write {
                set_accessed_dirty_64(ram, ram_size, pde_addr, pde);
            } else {
                set_accessed_64(ram, ram_size, pde_addr, pde);
            }
        }
        let page_base = pde & 0x000FFFFF_FFE00000;
        let page_offset = (linear32 & 0x001FFFFF) as u64;
        return Ok(page_base | page_offset);
    }

    // Set Accessed on PDE.
    unsafe { set_accessed_64(ram, ram_size, pde_addr, pde); }

    // PT index: bits [20:12] (9 bits -> 512 entries).
    let pt_index = ((linear32 >> 12) & 0x1FF) as u64;
    let pt_base = pde & 0x000FFFFF_FFFFF000;
    let pte_addr = pt_base + pt_index * 8;
    let pte = mem.read_u64(pte_addr)?;

    check_present(pte, access, cpl, linear)?;

    // Combined permissions: AND for U/S and R/W, OR for NX (Intel SDM §4.6).
    let eff_us = (pde & PTE_US) != 0 && (pte & PTE_US) != 0;
    let eff_rw = (pde & PTE_RW) != 0 && (pte & PTE_RW) != 0;
    let eff_nx = (pde & PTE_NX) != 0 || (pte & PTE_NX) != 0;
    check_final_permissions(eff_us, eff_rw, eff_nx, access, cpl, linear, mmu, rflags_ac)?;

    // Set Accessed (+ Dirty for writes) on final PTE.
    unsafe {
        if is_write {
            set_accessed_dirty_64(ram, ram_size, pte_addr, pte);
        } else {
            set_accessed_64(ram, ram_size, pte_addr, pte);
        }
    }

    // Physical address: PTE[51:12] || linear[11:0].
    let page_base = pte & 0x000FFFFF_FFFFF000;
    let page_offset = (linear32 & 0xFFF) as u64;
    Ok(page_base | page_offset)
}

// ── 4-level (IA-32e / long mode) paging ──

/// Walk a 4-level (IA-32e) page table hierarchy.
///
/// CR3 bits [51:12] point to the PML4 table. Each of the 4 levels indexes
/// 9 bits of the linear address:
///
/// - PML4: bits [47:39] (512 entries)
/// - PDPT: bits [38:30] (512 entries, 1 GiB huge pages with PS bit)
/// - PD: bits [29:21] (512 entries, 2 MiB huge pages with PS bit)
/// - PT: bits [20:12] (512 entries, 4 KiB pages)
///
/// Bits [63:48] of the linear address must be a sign-extension of bit 47
/// (canonical form), but that check is done at the segment/instruction level,
/// not here.
fn walk_4level(
    linear: u64,
    cr3: u64,
    access: AccessType,
    cpl: u8,
    mmu: &Mmu,
    mem: &dyn MemoryBus,
    ram: *mut u8,
    ram_size: usize,
    rflags_ac: bool,
) -> Result<u64> {
    let is_write = matches!(access, AccessType::Write);

    // ── PML4 ──
    let pml4_index = (linear >> 39) & 0x1FF;
    let pml4_base = cr3 & 0x000FFFFF_FFFFF000;
    let pml4e_addr = pml4_base + pml4_index * 8;
    let pml4e = mem.read_u64(pml4e_addr)?;

    check_present(pml4e, access, cpl, linear)?;
    // PML4E reserved bit check (no PS allowed at this level).
    if has_reserved_bits_64(pml4e, false) || (pml4e & PTE_PS) != 0 {
        return Err(pf_rsvd(access, cpl, linear));
    }
    unsafe { set_accessed_64(ram, ram_size, pml4e_addr, pml4e); }

    // ── PDPT ──
    let pdpt_index = (linear >> 30) & 0x1FF;
    let pdpt_base = pml4e & 0x000FFFFF_FFFFF000;
    let pdpte_addr = pdpt_base + pdpt_index * 8;
    let pdpte = mem.read_u64(pdpte_addr)?;

    check_present(pdpte, access, cpl, linear)?;

    // 1 GiB huge page: PDPTE.PS=1 — final level; combine PML4E + PDPTE.
    if (pdpte & PTE_PS) != 0 {
        if has_reserved_bits_64(pdpte, false) || (pdpte & 0x3FFE_0000) != 0 {
            return Err(pf_rsvd(access, cpl, linear));
        }
        let eff_us = (pml4e & PTE_US) != 0 && (pdpte & PTE_US) != 0;
        let eff_rw = (pml4e & PTE_RW) != 0 && (pdpte & PTE_RW) != 0;
        let eff_nx = (pml4e & PTE_NX) != 0 || (pdpte & PTE_NX) != 0;
        check_final_permissions(eff_us, eff_rw, eff_nx, access, cpl, linear, mmu, rflags_ac)?;
        unsafe {
            if is_write {
                set_accessed_dirty_64(ram, ram_size, pdpte_addr, pdpte);
            } else {
                set_accessed_64(ram, ram_size, pdpte_addr, pdpte);
            }
        }
        let page_base = pdpte & 0x000FFFFF_C0000000;
        let page_offset = linear & 0x3FFFFFFF;
        return Ok(page_base | page_offset);
    }

    if has_reserved_bits_64(pdpte, false) {
        return Err(pf_rsvd(access, cpl, linear));
    }
    unsafe { set_accessed_64(ram, ram_size, pdpte_addr, pdpte); }

    // ── PD ──
    let pd_index = (linear >> 21) & 0x1FF;
    let pd_base = pdpte & 0x000FFFFF_FFFFF000;
    let pde_addr = pd_base + pd_index * 8;
    let pde = mem.read_u64(pde_addr)?;

    check_present(pde, access, cpl, linear)?;

    // 2 MiB huge page: PDE.PS=1 — final level; combine PML4E + PDPTE + PDE.
    if (pde & PTE_PS) != 0 {
        if has_reserved_bits_64(pde, true) {
            return Err(pf_rsvd(access, cpl, linear));
        }
        let eff_us = (pml4e & PTE_US) != 0 && (pdpte & PTE_US) != 0 && (pde & PTE_US) != 0;
        let eff_rw = (pml4e & PTE_RW) != 0 && (pdpte & PTE_RW) != 0 && (pde & PTE_RW) != 0;
        let eff_nx = (pml4e & PTE_NX) != 0 || (pdpte & PTE_NX) != 0 || (pde & PTE_NX) != 0;
        check_final_permissions(eff_us, eff_rw, eff_nx, access, cpl, linear, mmu, rflags_ac)?;
        unsafe {
            if is_write {
                set_accessed_dirty_64(ram, ram_size, pde_addr, pde);
            } else {
                set_accessed_64(ram, ram_size, pde_addr, pde);
            }
        }
        let page_base = pde & 0x000FFFFF_FFE00000;
        let page_offset = linear & 0x1FFFFF;
        return Ok(page_base | page_offset);
    }

    if has_reserved_bits_64(pde, false) {
        return Err(pf_rsvd(access, cpl, linear));
    }
    unsafe { set_accessed_64(ram, ram_size, pde_addr, pde); }

    // ── PT ──
    let pt_index = (linear >> 12) & 0x1FF;
    let pt_base = pde & 0x000FFFFF_FFFFF000;
    let pte_addr = pt_base + pt_index * 8;
    let pte = mem.read_u64(pte_addr)?;

    check_present(pte, access, cpl, linear)?;
    if has_reserved_bits_64(pte, false) {
        return Err(pf_rsvd(access, cpl, linear));
    }

    // Combined permissions across all 4 levels.
    let eff_us = (pml4e & PTE_US) != 0 && (pdpte & PTE_US) != 0
        && (pde & PTE_US) != 0 && (pte & PTE_US) != 0;
    let eff_rw = (pml4e & PTE_RW) != 0 && (pdpte & PTE_RW) != 0
        && (pde & PTE_RW) != 0 && (pte & PTE_RW) != 0;
    let eff_nx = (pml4e & PTE_NX) != 0 || (pdpte & PTE_NX) != 0
        || (pde & PTE_NX) != 0 || (pte & PTE_NX) != 0;
    check_final_permissions(eff_us, eff_rw, eff_nx, access, cpl, linear, mmu, rflags_ac)?;

    // Set Accessed (+ Dirty for writes) on final PTE.
    unsafe {
        if is_write {
            set_accessed_dirty_64(ram, ram_size, pte_addr, pte);
        } else {
            set_accessed_64(ram, ram_size, pte_addr, pte);
        }
    }

    // Physical address: PTE[51:12] || linear[11:0].
    let page_base = pte & 0x000FFFFF_FFFFF000;
    let page_offset = linear & 0xFFF;
    Ok(page_base | page_offset)
}
