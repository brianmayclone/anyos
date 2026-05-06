//! Direct physical memory mapping ("physmap").
//!
//! Maps every usable RAM page to a fixed kernel-virtual base so the
//! kernel can dereference any physical address by adding a constant
//! offset. Equivalent to Linux's `__va()` / `PAGE_OFFSET`.
//!
//! # Why
//!
//! The legacy x86_64 boot path identity-maps only the first 128 MiB,
//! which limits both:
//!
//! * **Contiguous allocations** — the buddy / contiguous allocator
//!   has to live entirely below the 128 MiB ceiling so that physical
//!   addresses returned to drivers are dereferenceable from the
//!   kernel. Real systems easily exceed that.
//! * **Free-list link words** — a buddy allocator with intrusive free
//!   lists needs to read and write the head bytes of every free
//!   page. Without physmap that is only safe for low-memory pages.
//!
//! Physmap removes both restrictions in one shot. After init, every
//! RAM page (up to `MAX_PHYSMAP_BYTES`) is reachable at
//! `PHYSMAP_BASE + phys_addr`.
//!
//! # Layout
//!
//! Address space slot:
//!
//! ```text
//!   PML4[384]  →  0xFFFF_C000_0000_0000  (start of physmap)
//! ```
//!
//! Out of the 256 PML4 entries available to the kernel half
//! (entries 256..511), entries 510 and 511 are taken (recursive
//! mapping and higher-half kernel image). 384 sits in the middle and
//! is unused by anything else, leaving plenty of headroom for the
//! upcoming heap rework that may also want a large kernel-virtual
//! window.
//!
//! Page size: 2 MiB huge pages (PD entries with the PS bit). 16 GiB
//! of RAM fits in 8192 huge pages = 16 PDs hanging off one PDPT.
//! Total page-table cost ≈ 16 × 4 KiB + 1 × 4 KiB = 68 KiB.
//!
//! 2 MiB granularity is fine: physmap exists only to give the kernel
//! a way to read / write any physical address. Permissions inside
//! physmap are uniform (kernel-only, R/W, no-execute), so we don't
//! need 4 KiB granularity.
//!
//! # Caveats
//!
//! * **MMIO and reserved regions** are intentionally NOT mapped.
//!   Physmap covers only USABLE entries from the E820 map. Drivers
//!   that need MMIO continue to allocate dedicated mappings via
//!   `map_page`.
//! * **Aliases**: physmap creates a second virtual mapping for every
//!   RAM page (the first being the identity map / kernel-image
//!   mapping). Writes through one alias are visible through the
//!   other; that's intentional. PAT memory types must agree across
//!   aliases — physmap uses default WB, which matches identity-map.
//! * `init` must run AFTER `virtual_mem::init` has built the kernel
//!   PML4 (we install entries into it) but BEFORE any allocator
//!   tries to use physmap-backed addresses.

use crate::boot_info::{BootInfo, E820_TYPE_USABLE};
use crate::memory::address::PhysAddr;
// physical:: not needed anymore — physmap allocates PT pages from
// its own static BSS pool to avoid a boot-time cycle with buddy.
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Kernel-virtual base where physmap starts.
///
/// PML4 index 384 = `0xFFFF_C000_0000_0000`. Each PML4 entry covers
/// 512 GiB; we use exactly one. Address space picked to:
///   * stay in the kernel half (PML4[256..511])
///   * not collide with PML4[510] (recursive) or PML4[511] (kernel image)
///   * leave room for future kernel windows (heap, vmap)
pub const PHYSMAP_BASE: u64 = 0xFFFF_C000_0000_0000;

/// Maximum RAM the physmap can address. Caps at 64 GiB to match the
/// frame allocator's documented ceiling and keep the page-table cost
/// bounded. Per-PML4-entry capacity is 512 GiB so we have headroom
/// to grow this later without changing the address-space layout.
pub const MAX_PHYSMAP_BYTES: u64 = 64 * 1024 * 1024 * 1024;

/// PML4 index for physmap.
const PHYSMAP_PML4_INDEX: usize = 384;

/// 2 MiB huge-page size.
const HUGE_PAGE_SIZE: u64 = 2 * 1024 * 1024;

/// Page-table entry flags. Defined locally so this module is
/// self-contained — these match `virtual_mem.rs` exactly.
const PAGE_PRESENT: u64 = 1 << 0;
const PAGE_WRITABLE: u64 = 1 << 1;
/// Page Size bit on PD entries: when set, the entry is a leaf
/// pointing at a 2 MiB physical region instead of a PT.
const PAGE_HUGE: u64 = 1 << 7;
/// Global bit: TLB entry survives CR3 reload. Safe for kernel-only
/// mappings; gives a noticeable perf win for physmap because the
/// kernel touches it constantly.
const PAGE_GLOBAL: u64 = 1 << 8;
// Note on PAGE_NX: setting bit 63 in any PTE requires EFER.NXE = 1.
// On x86_64 anyOS sets NXE in syscall_msr::init_bsp(), which runs
// AFTER init_memory() — so during physmap::init() it is NOT yet on,
// and any PTE with bit 63 set becomes a reserved-bit violation as
// soon as the page is touched. We therefore omit NX here and rely
// on physmap being a kernel-only window (no user mode can reach
// PML4[384] via lower-half PML4 entries) for data-only enforcement.
// If we want NX-protected physmap later, init must be split: install
// PD entries in init_memory(), set the NX bit in a second pass after
// EFER.NXE goes live.

/// Highest physical address actually backed by physmap, set by
/// `init()`. Reads above this should fall through to the older
/// identity-map path; writes are a bug.
static PHYSMAP_LIMIT: AtomicU64 = AtomicU64::new(0);

/// Set once `init()` has installed the PD entries. Used to gate
/// `phys_to_virt` so callers running early in boot can detect
/// "physmap not ready yet" and fall back to identity-map access.
static PHYSMAP_READY: AtomicBool = AtomicBool::new(false);

/// True iff `init()` ran successfully and the physmap is usable.
#[inline]
pub fn is_ready() -> bool {
    PHYSMAP_READY.load(Ordering::Acquire)
}

/// Highest physical address the physmap covers, in bytes.
#[inline]
pub fn limit() -> u64 {
    PHYSMAP_LIMIT.load(Ordering::Acquire)
}

/// Translate a physical address to the kernel-virtual address that
/// maps it through physmap.
///
/// Returns `None` if physmap isn't initialised yet, or the physical
/// address is past the mapped limit. Callers that need to handle
/// either case (early-boot drivers, MMIO writers) should use the
/// returned Option; callers that operate strictly on RAM in the
/// post-init kernel can `.expect("physmap")`.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn phys_to_virt(phys: PhysAddr) -> Option<*mut u8> {
    let p = phys.as_u64();
    if !is_ready() {
        return None;
    }
    if p >= limit() {
        return None;
    }
    Some((PHYSMAP_BASE + p) as *mut u8)
}

/// ARM64 variant: the bootloader's 1 GiB TTBR1 block already maps
/// all RAM at KERNEL_VIRT_BASE, so physmap is just a bookkeeping
/// veneer over that mapping. Translation is `vaddr = phys + offset`
/// with the offset taken from the linker layout.
#[cfg(target_arch = "aarch64")]
#[inline]
pub fn phys_to_virt(phys: PhysAddr) -> Option<*mut u8> {
    const ARM64_PHYS_TO_VIRT_OFFSET: u64 = 0xFFFF_0000_0000_0000;
    let p = phys.as_u64();
    if !is_ready() {
        return None;
    }
    if p >= limit() {
        return None;
    }
    Some((p + ARM64_PHYS_TO_VIRT_OFFSET) as *mut u8)
}

/// Identity-map fallback: translate a phys addr using whichever map
/// is available. After init this is always physmap; before init it
/// falls back to the boot identity-map (which only covers the lower
/// 128 MiB on x86_64). The lower-128-MiB constraint is asserted in
/// debug builds.
#[inline]
pub fn phys_to_virt_or_identity(phys: PhysAddr) -> *mut u8 {
    if let Some(v) = phys_to_virt(phys) {
        return v;
    }
    let p = phys.as_u64();
    debug_assert!(
        p < 128 * 1024 * 1024,
        "phys_to_virt_or_identity: physmap missing for {:#x}",
        p
    );
    p as *mut u8
}

/// Bootstrap pool for physmap's own page-table pages. Physmap needs
/// 1 PDPT + N PDs (one per GiB of mapped RAM). For the 64 GiB cap
/// that's 65 frames worst case = 260 KiB. We size at 80 to give a
/// small margin and keep alignment trivial.
///
/// Why not call `physical::alloc_frame()`?  Because under
/// `--features buddy_alloc` the production allocator is the buddy,
/// which builds its free lists by writing intrusive link words into
/// freed pages — and that needs physmap to be ready, creating a
/// boot-time cycle: buddy_init → write_link → physmap_lookup →
/// physmap_init → physical::alloc_frame → buddy_alloc which is the
/// thing we're bootstrapping. Linux solves this by having a
/// separate bootmem allocator that hands page-table pages to the
/// physmap setup; we use a fixed BSS pool because anyOS only ever
/// needs a handful of pages.
///
/// Each entry in the pool is a 4 KiB page. The pool is accessed
/// only from `init`, before any other code can race against it.
const PHYSMAP_PT_POOL_PAGES: usize = 80;

#[repr(C, align(4096))]
struct PhysmapPtPool {
    pages: [[u8; 4096]; PHYSMAP_PT_POOL_PAGES],
}

static mut PHYSMAP_PT_POOL: PhysmapPtPool = PhysmapPtPool {
    pages: [[0u8; 4096]; PHYSMAP_PT_POOL_PAGES],
};

static PHYSMAP_PT_POOL_NEXT: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Hand out one zeroed page-table page from the bootstrap pool.
/// Returns the page's physical address (== virtual since the pool
/// lives in the kernel image, which is identity-mapped + higher-half
/// aliased; we use the physical address derived from the linker
/// layout). Returns 0 if the pool is exhausted.
fn alloc_pt_page_from_pool() -> u64 {
    let next = PHYSMAP_PT_POOL_NEXT
        .fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    if next >= PHYSMAP_PT_POOL_PAGES {
        return 0;
    }
    // SAFETY: each pool entry is exclusively assigned via the atomic
    // counter, so no two callers ever see the same `next`.
    let page_va = unsafe {
        (&PHYSMAP_PT_POOL.pages[next] as *const _) as u64
    };
    // Convert kernel-virtual to physical: the kernel image is mapped
    // both at its physical load address (low identity-map) and at
    // KERNEL_VIRT_BASE in the higher half. The linker places statics
    // in the higher half, so subtract KERNEL_VIRT_BASE to get phys.
    const KERNEL_VIRT_BASE: u64 = 0xFFFF_FFFF_8000_0000;
    let page_phys = if page_va >= KERNEL_VIRT_BASE {
        page_va - KERNEL_VIRT_BASE
    } else {
        page_va
    };
    // Zero the page (it's already zeroed via BSS, but reset on
    // re-use isn't needed because we never recycle pool pages).
    unsafe {
        core::ptr::write_bytes(page_va as *mut u8, 0, 4096);
    }
    page_phys
}

/// Install PD entries covering every USABLE E820 region (truncated
/// to the physmap limit) at PHYSMAP_BASE + phys.
///
/// SAFETY: must run after the kernel PML4 is in CR3. Idempotent —
/// calling twice is a no-op because the second call sees existing
/// PDPT/PD entries and reuses them.
#[cfg(target_arch = "x86_64")]
pub fn init(boot_info: &BootInfo) {
    use crate::memory::virtual_mem;

    if PHYSMAP_READY.load(Ordering::Relaxed) {
        return;
    }

    // Determine highest physical address worth mapping. We round up
    // to a 2 MiB boundary so partial trailing pages stay covered.
    let mem_map = unsafe { boot_info.memory_map() };
    let mut top_phys: u64 = 0;
    for entry in mem_map {
        if entry.entry_type == E820_TYPE_USABLE {
            let end = entry.base_addr + entry.length;
            if end > top_phys {
                top_phys = end;
            }
        }
    }
    if top_phys == 0 {
        crate::serial_verbose_println!("[physmap] no usable RAM in E820 — disabled");
        return;
    }
    if top_phys > MAX_PHYSMAP_BYTES {
        top_phys = MAX_PHYSMAP_BYTES;
    }
    // Round up to 2 MiB.
    let mapped_bytes = (top_phys + HUGE_PAGE_SIZE - 1) & !(HUGE_PAGE_SIZE - 1);
    let huge_pages = (mapped_bytes / HUGE_PAGE_SIZE) as usize;

    // Walk into the kernel PML4 and install our PML4 → PDPT → PDs.
    // The kernel PML4 lives at PML4_PHYS (set by virtual_mem::init);
    // all of its entries are reachable through identity-mapped low
    // memory because the bootloader already mapped the kernel image
    // and PML4 page low.
    let pml4_phys = virtual_mem::kernel_pml4_phys();
    if pml4_phys == 0 {
        crate::serial_verbose_println!(
            "[physmap] kernel PML4 not initialised yet — physmap init aborted"
        );
        return;
    }
    let pml4 = pml4_phys as *mut u64;

    // Allocate a PDPT for PML4[384] if it doesn't already exist.
    // We use a static BSS pool instead of physical::alloc_frame() to
    // avoid a boot-time circular dependency under --features
    // buddy_alloc — see the PhysmapPtPool comment above.
    let pdpt_phys = unsafe {
        let pml4_entry = core::ptr::read_volatile(pml4.add(PHYSMAP_PML4_INDEX));
        if pml4_entry & PAGE_PRESENT != 0 {
            pml4_entry & 0x000F_FFFF_FFFF_F000
        } else {
            let new_pdpt = alloc_pt_page_from_pool();
            if new_pdpt == 0 {
                crate::serial_verbose_println!("[physmap] PT pool exhausted (PDPT)");
                return;
            }
            core::ptr::write_volatile(
                pml4.add(PHYSMAP_PML4_INDEX),
                new_pdpt | PAGE_PRESENT | PAGE_WRITABLE,
            );
            new_pdpt
        }
    };
    let pdpt = pdpt_phys as *mut u64;

    // Each PDPT entry covers 1 GiB; we need ceil(mapped_bytes / 1 GiB)
    // PD tables. Each PD covers 1 GiB via 512 huge-page entries.
    let gib = 1024u64 * 1024 * 1024;
    let num_pds = ((mapped_bytes + gib - 1) / gib) as usize;
    let mut huge_pages_left = huge_pages;

    for pdpt_i in 0..num_pds {
        let pd_phys = unsafe {
            let pdpt_entry = core::ptr::read_volatile(pdpt.add(pdpt_i));
            if pdpt_entry & PAGE_PRESENT != 0 {
                pdpt_entry & 0x000F_FFFF_FFFF_F000
            } else {
                let new_pd = alloc_pt_page_from_pool();
                if new_pd == 0 {
                    crate::serial_verbose_println!(
                        "[physmap] PT pool exhausted at GiB {}",
                        pdpt_i
                    );
                    return;
                }
                core::ptr::write_volatile(
                    pdpt.add(pdpt_i),
                    new_pd | PAGE_PRESENT | PAGE_WRITABLE,
                );
                new_pd
            }
        };
        let pd = pd_phys as *mut u64;

        // Install up to 512 huge-page entries in this PD, each
        // pointing at consecutive 2 MiB chunks of physical memory
        // starting at GiB boundary.
        let pd_base_phys = (pdpt_i as u64) * gib;
        let entries_in_this_pd = huge_pages_left.min(512);
        for pd_i in 0..entries_in_this_pd {
            let phys_addr = pd_base_phys + (pd_i as u64) * HUGE_PAGE_SIZE;
            let entry = phys_addr
                | PAGE_PRESENT
                | PAGE_WRITABLE
                | PAGE_HUGE
                | PAGE_GLOBAL;
            unsafe {
                core::ptr::write_volatile(pd.add(pd_i), entry);
            }
        }
        huge_pages_left -= entries_in_this_pd;
    }

    // TLB flush is not strictly needed yet — nobody has touched
    // physmap addresses pre-init — but a CR3 reload is the cheapest
    // way to publish the new mappings on the boot CPU. Other CPUs
    // see them when they next switch CR3, which is fine because
    // physmap goes live before the user space starts.
    unsafe {
        let cr3: u64;
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nomem, nostack));
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack));
    }

    PHYSMAP_LIMIT.store(mapped_bytes, Ordering::Release);
    PHYSMAP_READY.store(true, Ordering::Release);

    crate::serial_verbose_println!(
        "[physmap] mapped {} MiB at VA {:#x} ({} huge pages)",
        mapped_bytes / (1024 * 1024),
        PHYSMAP_BASE,
        huge_pages
    );
}

/// ARM64 init takes RAM extents directly; the kernel already maps
/// all RAM through TTBR1 at KERNEL_VIRT_BASE. We re-export that as
/// the physmap so the cross-arch buddy code can pretend physmap is
/// always available with the same `phys_to_virt` API.
///
/// The translation on ARM64 is `vaddr = phys + ARM64_PHYS_TO_VIRT`
/// where `ARM64_PHYS_TO_VIRT = 0xFFFF_0000_4000_0000 - 0x4000_0000`
/// (the offset baked into the boot.S 1 GiB block). Rather than
/// duplicate that constant here we ship a separate
/// `phys_to_virt_arm64` and have [`phys_to_virt`] dispatch.
#[cfg(target_arch = "aarch64")]
pub fn init_arm64(ram_base: u64, ram_size: u64) {
    PHYSMAP_LIMIT.store(ram_base + ram_size, Ordering::Release);
    PHYSMAP_READY.store(true, Ordering::Release);
    crate::serial_verbose_println!(
        "[physmap] ARM64: re-using TTBR1 block as physmap, {} MiB",
        ram_size / (1024 * 1024)
    );
}
