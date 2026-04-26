//! Central definition of the user-space virtual address layout.
//!
//! Phase A migration: kernel-managed regions (stack, mmap, sigreturn
//! trampoline) live in the upper canonical-low half (à la Linux x86_64),
//! while program code and DLIBs are still loaded at their historical
//! low-half fixed addresses (programs at 0x0800_0000, DLIBs at
//! 0x0400_0000+). Moving those requires relinking every user binary
//! and is tracked as Phase B.
//!
//! ```text
//!   0x0000_0000_0000_0000  .. PROGRAM_LOAD_ADDR
//!                                       — reserved (low NULL guard)
//!   PROGRAM_LOAD_ADDR ..                — program code/data, brk grows up
//!   DLIB_REGION_START .. DLIB_REGION_END
//!                                       — shared DLIB pages (read-only,
//!                                          fixed-link load addresses)
//!   HEAP_LIMIT ..                       — guard gap, large unused span
//!   MMAP_BASE  .. MMAP_LIMIT            — anonymous mmap region
//!   USER_STACK_TOP - USER_STACK_PAGES*4K
//!     .. USER_STACK_TOP                 — user stack (grows down)
//!   SIGRETURN_TRAMPOLINE_ADDR           — kernel-written trampoline page
//!   0x0000_8000_0000_0000               — start of non-canonical hole
//! ```

/// Lowest user-space code load address (also the lowest legal entry point).
pub const PROGRAM_LOAD_ADDR: u64 = 0x0800_0000;

/// Start of the shared DLIB region (uisys, libimage, ...).
pub const DLIB_REGION_START: u64 = 0x0400_0000;
/// One past the last byte of the DLIB region. Heap is forbidden from
/// crossing into it.
pub const DLIB_REGION_END: u64 = PROGRAM_LOAD_ADDR;

/// Upper bound of the heap (sbrk) — must stay below [`MMAP_BASE`] with a
/// guard gap.
pub const HEAP_LIMIT: u64 = 0x6F00_0000;

/// Bottom of the 32-bit mmap region. Bump-allocator + first-fit gap
/// search both start here. Stays in the low half so that the existing
/// SYS_MMAP u32 ABI (and every user binary linked against libsyscall's
/// `mmap() -> u64` returning a sub-4-GiB address) keeps working.
pub const MMAP_BASE: u64 = 0x7000_0000;
/// One past the last byte of the 32-bit mmap region.
pub const MMAP_LIMIT: u64 = 0xBF00_0000;

/// Bottom of the 64-bit mmap region (above the historical 4 GiB ceiling).
pub const MMAP64_BASE: u64 = 0x0000_0001_0000_0000;

/// Number of 4 KiB pages reserved for the user stack.
pub const USER_STACK_PAGES: u64 = 2048;
/// Top of the user stack (exclusive). The stack grows down from here.
/// Top of canonical-low half minus one page (Linux x86_64 default).
pub const USER_STACK_TOP: u64 = 0x0000_7FFF_FFFF_F000;

/// Single-page kernel-written trampoline that ends a user signal handler.
/// Layout: `mov eax, 246 ; syscall ; nop` (8 bytes, page-aligned).
/// Placed one page below the stack region's bottom so it cannot collide
/// with the stack guard page or with the bottom of the mmap region.
pub const SIGRETURN_TRAMPOLINE_ADDR: u64 = 0x0000_7FFE_FFFF_F000;

/// Maximum number of 4 KiB pages of ASLR jitter applied to the stack top.
pub const ASLR_STACK_MAX_PAGES: u32 = 256;
/// Maximum number of 4 KiB pages of ASLR jitter applied to the mmap base.
pub const ASLR_MMAP_MAX_PAGES: u32 = 4096;

/// Address ranges used by the page-fault diagnostic in
/// `arch::x86::idt`. These are heuristics for log labels only — they do
/// not gate behavior.
pub mod fault_diag {
    use super::{DLIB_REGION_START, MMAP_BASE, PROGRAM_LOAD_ADDR, USER_STACK_TOP};

    /// "DLL/.dlib" range used for the fault-diag classifier.
    pub const DLL_RANGE: core::ops::Range<u64> = DLIB_REGION_START..PROGRAM_LOAD_ADDR;

    /// Approximate stack region for fault classification — last
    /// 256 MiB below the stack top (covers ASLR + the guard page).
    pub const STACK_RANGE: core::ops::Range<u64> = (USER_STACK_TOP - 0x1000_0000)..USER_STACK_TOP;

    pub use super::MMAP_BASE as MMAP_RANGE_LO;
}
