//! Tests for the physmap (direct physical-memory mapping).
//!
//! What we want to verify:
//!   1. After boot, physmap is_ready().
//!   2. limit() exceeds the kernel image (i.e. it actually maps a
//!      meaningful chunk of RAM).
//!   3. phys_to_virt() returns a non-null pointer for valid phys
//!      addresses, None for out-of-range.
//!   4. The mapping is functional: writing through a freshly-allocated
//!      frame's physmap address and reading it back returns the same
//!      bytes (round-trip correctness — this is the property that the
//!      buddy allocator's intrusive free lists will rely on).
//!   5. Writes through physmap and through identity-map to the same
//!      page are observed identically (alias coherence — required so
//!      the kernel can mix the two during the migration).
//!
//! Tests deliberately do NOT touch reserved / MMIO / non-RAM regions:
//! physmap covers only USABLE E820 entries.

use crate::kunit::{TestCase, TestContext, TestSuite};
use crate::memory::address::PhysAddr;
use crate::memory::physical::{alloc_frame, free_frame};
use crate::memory::physmap;

pub static SUITE: TestSuite = TestSuite {
    name: "physmap",
    cases: &[
        TestCase {
            name: "ready_after_boot",
            run: test_ready_after_boot,
        },
        TestCase {
            name: "limit_covers_meaningful_ram",
            run: test_limit_covers_meaningful_ram,
        },
        TestCase {
            name: "phys_to_virt_returns_some_for_low_phys",
            run: test_phys_to_virt_low,
        },
        TestCase {
            name: "phys_to_virt_returns_none_for_out_of_range",
            run: test_phys_to_virt_oor,
        },
        TestCase {
            name: "round_trip_byte_through_physmap",
            run: test_round_trip_byte,
        },
        TestCase {
            name: "round_trip_word_through_physmap",
            run: test_round_trip_word,
        },
        TestCase {
            name: "alias_coherence_with_identity_map",
            run: test_alias_coherence,
        },
        TestCase {
            name: "many_frames_round_trip",
            run: test_many_frames_round_trip,
        },
    ],
};

fn test_ready_after_boot(ctx: &mut TestContext) {
    ctx.expect_true(physmap::is_ready(), "physmap::is_ready() after boot");
}

fn test_limit_covers_meaningful_ram(ctx: &mut TestContext) {
    // The kernel image alone occupies several MiB. If the limit is
    // less than 16 MiB the physmap is so small it's effectively
    // useless. QEMU defaults to at least 128 MiB for anyOS.
    let lim = physmap::limit();
    ctx.expect_gt(lim, 16 * 1024 * 1024, "physmap covers > 16 MiB");
}

fn test_phys_to_virt_low(ctx: &mut TestContext) {
    // 1 MiB (somewhere safely inside RAM, well above the IVT/BIOS
    // that ARM64 doesn't have anyway). On both architectures this
    // should resolve to a non-null kernel-virtual pointer.
    let v = physmap::phys_to_virt(PhysAddr::new(0x100_000));
    ctx.expect_some(v, "phys_to_virt(1 MiB) is Some");
}

fn test_phys_to_virt_oor(ctx: &mut TestContext) {
    // 1 PiB is far beyond any real RAM AND beyond physmap's
    // declared limit. Must return None — never a non-null pointer
    // into the kernel half (which would be aliased onto unrelated
    // virtual addresses).
    let v = physmap::phys_to_virt(PhysAddr::new(1u64 << 50));
    ctx.expect_none(v, "phys_to_virt(1 PiB) is None");
}

fn test_round_trip_byte(ctx: &mut TestContext) {
    // Allocate one frame, write a known byte through physmap, read
    // it back, free.
    let frame = match alloc_frame() {
        Some(f) => f,
        None => {
            ctx.expect_true(false, "alloc_frame failed");
            return;
        }
    };
    let v = match physmap::phys_to_virt(frame) {
        Some(v) => v,
        None => {
            free_frame(frame);
            ctx.expect_true(
                false,
                "phys_to_virt for freshly-allocated frame returned None",
            );
            return;
        }
    };
    let pattern: u8 = 0xA5;
    unsafe {
        core::ptr::write_volatile(v, pattern);
        let read_back = core::ptr::read_volatile(v);
        ctx.expect_eq(read_back, pattern, "u8 round trip");
    }
    free_frame(frame);
}

fn test_round_trip_word(ctx: &mut TestContext) {
    // Same idea but with a u32 link-word-sized value, the size the
    // upcoming buddy allocator will use.
    let frame = match alloc_frame() {
        Some(f) => f,
        None => {
            ctx.expect_true(false, "alloc_frame failed");
            return;
        }
    };
    let v = match physmap::phys_to_virt(frame) {
        Some(v) => v as *mut u32,
        None => {
            free_frame(frame);
            ctx.expect_true(
                false,
                "phys_to_virt for freshly-allocated frame returned None",
            );
            return;
        }
    };
    let pattern: u32 = 0xCAFE_BABE;
    unsafe {
        core::ptr::write_volatile(v, pattern);
        let read_back = core::ptr::read_volatile(v);
        ctx.expect_eq(read_back, pattern, "u32 round trip");
    }
    free_frame(frame);
}

fn test_alias_coherence(ctx: &mut TestContext) {
    // x86_64 only: identity-map exists for the lower 128 MiB. Pick
    // a frame from that region (alloc_contiguous from low memory),
    // write through identity, read through physmap, and vice versa.
    // ARM64's identity-map is the same as physmap so this test is
    // tautological there — skip with a single passing assertion.
    #[cfg(target_arch = "x86_64")]
    {
        use crate::memory::physical::alloc_contiguous;
        // alloc_contiguous returns a low-memory frame.
        let phys = match alloc_contiguous(1) {
            Some(p) => p,
            None => {
                ctx.expect_true(false, "alloc_contiguous failed");
                return;
            }
        };
        let id_ptr = phys.as_u64() as *mut u32;
        let pm_ptr = match physmap::phys_to_virt(phys) {
            Some(v) => v as *mut u32,
            None => {
                free_frame(phys);
                ctx.expect_true(false, "phys_to_virt(contiguous low) None");
                return;
            }
        };
        unsafe {
            // Write via identity, read via physmap.
            core::ptr::write_volatile(id_ptr, 0x1111_2222);
            let r1 = core::ptr::read_volatile(pm_ptr);
            ctx.expect_eq(r1, 0x1111_2222, "identity write → physmap read");

            // Write via physmap, read via identity.
            core::ptr::write_volatile(pm_ptr, 0x3333_4444);
            let r2 = core::ptr::read_volatile(id_ptr);
            ctx.expect_eq(r2, 0x3333_4444, "physmap write → identity read");
        }
        free_frame(phys);
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        ctx.expect_true(true, "alias coherence trivially holds on this arch");
    }
}

fn test_many_frames_round_trip(ctx: &mut TestContext) {
    // Sanity scale: allocate 64 frames, write a unique pattern to
    // each through physmap, read all back. Confirms the mapping
    // works across many distinct pages and that we don't have a
    // "first page works, rest don't" page-table-walking bug.
    const N: usize = 64;
    let mut frames: [crate::memory::address::PhysAddr; N] =
        [crate::memory::address::PhysAddr::new(0); N];
    let mut allocated = 0usize;
    for i in 0..N {
        match alloc_frame() {
            Some(f) => {
                frames[i] = f;
                allocated += 1;
            }
            None => break,
        }
    }
    ctx.expect_eq(allocated, N, "allocated N frames");

    // Write a pattern that depends on the frame index.
    for i in 0..allocated {
        let v = match physmap::phys_to_virt(frames[i]) {
            Some(v) => v as *mut u32,
            None => {
                ctx.expect_true(false, "phys_to_virt None mid-loop");
                continue;
            }
        };
        let pat: u32 = 0xC0DE_0000 | (i as u32);
        unsafe {
            core::ptr::write_volatile(v, pat);
        }
    }

    // Read back and verify.
    let mut mismatches = 0u32;
    for i in 0..allocated {
        let v = match physmap::phys_to_virt(frames[i]) {
            Some(v) => v as *const u32,
            None => continue,
        };
        let pat: u32 = 0xC0DE_0000 | (i as u32);
        let r = unsafe { core::ptr::read_volatile(v) };
        if r != pat {
            mismatches += 1;
        }
    }
    ctx.expect_eq(mismatches, 0, "all frames round-tripped correctly");

    for i in 0..allocated {
        free_frame(frames[i]);
    }
}
