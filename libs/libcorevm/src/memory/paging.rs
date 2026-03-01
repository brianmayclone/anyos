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
) -> Result<u64> {
    if mmu.long_mode {
        walk_4level(linear, cr3, access, cpl, mmu, mem)
    } else if mmu.pae {
        walk_pae(linear, cr3, access, cpl, mmu, mem)
    } else {
        walk_2level(linear, cr3, access, cpl, mmu, mem)
    }
}

// ── Permission check ──

/// Check a page table entry for access violations.
///
/// Generates `VmError::PageFault` with an appropriate error code when the
/// entry is not present or when the requested access is forbidden.
///
/// # Parameters
///
/// - `pte`: The page table entry to check.
/// - `access`: Read, Write, or Execute.
/// - `cpl`: Current privilege level (0 = kernel, 3 = user).
/// - `linear`: The faulting linear address (for the #PF record).
/// - `wp`: Whether CR0.WP is set (supervisor write-protect).
/// - `nxe`: Whether EFER.NXE is set (no-execute supported).
fn check_pte(
    pte: u64,
    access: AccessType,
    cpl: u8,
    linear: u64,
    wp: bool,
    nxe: bool,
) -> Result<()> {
    let present = (pte & PTE_P) != 0;
    if !present {
        return Err(VmError::PageFault {
            address: linear,
            error_code: access.to_pf_error_code(cpl, false),
        });
    }

    let is_user = cpl == 3;
    let is_supervisor_page = (pte & PTE_US) == 0;
    let is_read_only = (pte & PTE_RW) == 0;

    // User access to supervisor page.
    if is_user && is_supervisor_page {
        return Err(VmError::PageFault {
            address: linear,
            error_code: access.to_pf_error_code(cpl, true),
        });
    }

    // Write access to read-only page.
    match access {
        AccessType::Write => {
            if is_read_only {
                // In supervisor mode, writes to RO pages fault only if CR0.WP=1.
                // In user mode, writes to RO pages always fault.
                if is_user || wp {
                    return Err(VmError::PageFault {
                        address: linear,
                        error_code: access.to_pf_error_code(cpl, true),
                    });
                }
            }
        }
        AccessType::Execute => {
            // NX check: if EFER.NXE is set and the NX bit is set, execution is
            // forbidden.
            if nxe && (pte & PTE_NX) != 0 {
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
) -> Result<u64> {
    let linear32 = linear as u32;

    // PD index: bits [31:22].
    let pd_index = (linear32 >> 22) as u64;
    let pd_base = cr3 & 0xFFFFF000;
    let pde_addr = pd_base + pd_index * 4;
    let pde = mem.read_u32(pde_addr)? as u64;

    check_pte(pde, access, cpl, linear, mmu.wp, mmu.nxe)?;

    // PSE 4 MiB huge page: PDE.PS=1 and CR4.PSE=1.
    if mmu.pse && (pde & PTE_PS) != 0 {
        // Physical address: PDE[31:22] || linear[21:0].
        // Bits [21:13] of the PDE are reserved in classic PSE (contribute to
        // the physical address in PSE-36, which we ignore for simplicity).
        let page_base = (pde & 0xFFC00000) as u64;
        let page_offset = (linear32 & 0x003FFFFF) as u64;
        return Ok(page_base | page_offset);
    }

    // PT index: bits [21:12].
    let pt_index = ((linear32 >> 12) & 0x3FF) as u64;
    let pt_base = pde & 0xFFFFF000;
    let pte_addr = pt_base + pt_index * 4;
    let pte = mem.read_u32(pte_addr)? as u64;

    check_pte(pte, access, cpl, linear, mmu.wp, mmu.nxe)?;

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
) -> Result<u64> {
    let linear32 = linear as u32;

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

    // PD index: bits [29:21] (9 bits -> 512 entries).
    let pd_index = ((linear32 >> 21) & 0x1FF) as u64;
    let pd_base = pdpte & 0x000FFFFF_FFFFF000;
    let pde_addr = pd_base + pd_index * 8;
    let pde = mem.read_u64(pde_addr)?;

    check_pte(pde, access, cpl, linear, mmu.wp, mmu.nxe)?;

    // 2 MiB huge page: PDE.PS=1.
    if (pde & PTE_PS) != 0 {
        let page_base = pde & 0x000FFFFF_FFE00000;
        let page_offset = (linear32 & 0x001FFFFF) as u64;
        return Ok(page_base | page_offset);
    }

    // PT index: bits [20:12] (9 bits -> 512 entries).
    let pt_index = ((linear32 >> 12) & 0x1FF) as u64;
    let pt_base = pde & 0x000FFFFF_FFFFF000;
    let pte_addr = pt_base + pt_index * 8;
    let pte = mem.read_u64(pte_addr)?;

    check_pte(pte, access, cpl, linear, mmu.wp, mmu.nxe)?;

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
) -> Result<u64> {
    // ── PML4 ──
    let pml4_index = (linear >> 39) & 0x1FF;
    let pml4_base = cr3 & 0x000FFFFF_FFFFF000;
    let pml4e_addr = pml4_base + pml4_index * 8;
    let pml4e = mem.read_u64(pml4e_addr)?;

    check_pte(pml4e, access, cpl, linear, mmu.wp, mmu.nxe)?;

    // ── PDPT ──
    let pdpt_index = (linear >> 30) & 0x1FF;
    let pdpt_base = pml4e & 0x000FFFFF_FFFFF000;
    let pdpte_addr = pdpt_base + pdpt_index * 8;
    let pdpte = mem.read_u64(pdpte_addr)?;

    check_pte(pdpte, access, cpl, linear, mmu.wp, mmu.nxe)?;

    // 1 GiB huge page: PDPTE.PS=1.
    if (pdpte & PTE_PS) != 0 {
        let page_base = pdpte & 0x000FFFFF_C0000000;
        let page_offset = linear & 0x3FFFFFFF;
        return Ok(page_base | page_offset);
    }

    // ── PD ──
    let pd_index = (linear >> 21) & 0x1FF;
    let pd_base = pdpte & 0x000FFFFF_FFFFF000;
    let pde_addr = pd_base + pd_index * 8;
    let pde = mem.read_u64(pde_addr)?;

    check_pte(pde, access, cpl, linear, mmu.wp, mmu.nxe)?;

    // 2 MiB huge page: PDE.PS=1.
    if (pde & PTE_PS) != 0 {
        let page_base = pde & 0x000FFFFF_FFE00000;
        let page_offset = linear & 0x1FFFFF;
        return Ok(page_base | page_offset);
    }

    // ── PT ──
    let pt_index = (linear >> 12) & 0x1FF;
    let pt_base = pde & 0x000FFFFF_FFFFF000;
    let pte_addr = pt_base + pt_index * 8;
    let pte = mem.read_u64(pte_addr)?;

    check_pte(pte, access, cpl, linear, mmu.wp, mmu.nxe)?;

    // Physical address: PTE[51:12] || linear[11:0].
    let page_base = pte & 0x000FFFFF_FFFFF000;
    let page_offset = linear & 0xFFF;
    Ok(page_base | page_offset)
}
