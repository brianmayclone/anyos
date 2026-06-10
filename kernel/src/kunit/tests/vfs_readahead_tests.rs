//! Tests for VFS-level exFAT read-ahead policy.
//!
//! These assert the *policy* — minimum window, sequential doubling, random
//! reset, and per-tier memory caps — in terms of the named `EXFAT_READAHEAD_*`
//! constants, so retuning those constants keeps the suite green automatically
//! instead of requiring the magic numbers here to be hand-edited each time.

use crate::fs::file::ReadAheadState;
use crate::fs::vfs::{
    exfat_readahead_cap_for_memory_frames, exfat_readahead_decision, ExFatPrefetch,
    EXFAT_READAHEAD_HIGH_MAX_BYTES, EXFAT_READAHEAD_LARGE_MAX_BYTES, EXFAT_READAHEAD_LOW_MAX_BYTES,
    EXFAT_READAHEAD_MED_MAX_BYTES, EXFAT_READAHEAD_MIN_BYTES,
};
use crate::kunit::{TestCase, TestContext, TestSuite};

pub static SUITE: TestSuite = TestSuite {
    name: "vfs_readahead",
    cases: &[
        TestCase {
            name: "memory_cap_scales_with_free_ram",
            run: test_memory_cap_scales_with_free_ram,
        },
        TestCase {
            name: "sequential_reads_grow_window",
            run: test_sequential_reads_grow_window,
        },
        TestCase {
            name: "random_read_resets_window",
            run: test_random_read_resets_window,
        },
    ],
};

fn frames(mib: usize) -> usize {
    mib * 1024 * 1024 / crate::memory::FRAME_SIZE
}

fn test_memory_cap_scales_with_free_ram(ctx: &mut TestContext) {
    // Below the minimum free/total RAM thresholds, prefetch is disabled.
    ctx.expect_eq(
        exfat_readahead_cap_for_memory_frames(frames(8), frames(512)),
        0u32,
        "very low free memory disables prefetch",
    );
    // Free RAM selects progressively larger per-stream caps (asserted against
    // the named tier constants so this stays correct across retuning).
    ctx.expect_eq(
        exfat_readahead_cap_for_memory_frames(frames(32), frames(512)),
        EXFAT_READAHEAD_LOW_MAX_BYTES,
        "low memory selects LOW cap",
    );
    ctx.expect_eq(
        exfat_readahead_cap_for_memory_frames(frames(128), frames(512)),
        EXFAT_READAHEAD_MED_MAX_BYTES,
        "medium memory selects MED cap",
    );
    ctx.expect_eq(
        exfat_readahead_cap_for_memory_frames(frames(512), frames(1024)),
        EXFAT_READAHEAD_HIGH_MAX_BYTES,
        "high memory selects HIGH cap",
    );
    ctx.expect_eq(
        exfat_readahead_cap_for_memory_frames(frames(2048), frames(4096)),
        EXFAT_READAHEAD_LARGE_MAX_BYTES,
        "large memory selects LARGE cap",
    );
}

fn test_sequential_reads_grow_window(ctx: &mut TestContext) {
    const READ_LEN: usize = 4096;
    const FILE_SIZE: u32 = 16 * 1024 * 1024;
    // Cap well above MIN*2 so the window grows by pure doubling (cap never binds
    // for the two reads asserted here).
    let cap = EXFAT_READAHEAD_MIN_BYTES * 8;

    let first = exfat_readahead_decision(ReadAheadState::new(0), 0, FILE_SIZE, READ_LEN, cap);
    ctx.expect_eq(
        first.state.next_offset,
        READ_LEN as u32,
        "first read advances next offset",
    );
    ctx.expect_eq(
        first.state.window_bytes,
        EXFAT_READAHEAD_MIN_BYTES,
        "first window starts at minimum",
    );
    ctx.expect_eq(
        first.prefetch,
        Some(ExFatPrefetch {
            offset: READ_LEN as u32,
            len: EXFAT_READAHEAD_MIN_BYTES as usize,
        }),
        "first read schedules an initial prefetch of one minimum window",
    );

    let second = exfat_readahead_decision(first.state, READ_LEN as u32, FILE_SIZE, READ_LEN, cap);
    ctx.expect_eq(
        second.state.window_bytes,
        EXFAT_READAHEAD_MIN_BYTES * 2,
        "sequential read doubles the window",
    );
    // The second prefetch tops up from where the first one ended
    // (first.prefetched_until == READ_LEN + MIN) out to read_end + new window.
    ctx.expect_eq(
        second.prefetch,
        Some(ExFatPrefetch {
            offset: READ_LEN as u32 + EXFAT_READAHEAD_MIN_BYTES,
            len: READ_LEN + EXFAT_READAHEAD_MIN_BYTES as usize,
        }),
        "second read tops up beyond already-prefetched bytes",
    );
}

fn test_random_read_resets_window(ctx: &mut TestContext) {
    const READ_LEN: usize = 4096;
    const READ_POS: u32 = 128 * 1024;
    let cap = EXFAT_READAHEAD_MIN_BYTES * 8;

    // A stream that had grown its window, followed by a non-sequential jump.
    let mut state = ReadAheadState::new(0);
    state.next_offset = 4096; // != READ_POS → treated as random
    state.window_bytes = EXFAT_READAHEAD_MIN_BYTES * 4;
    state.prefetched_until = EXFAT_READAHEAD_MIN_BYTES * 8;

    let decision = exfat_readahead_decision(state, READ_POS, 16 * 1024 * 1024, READ_LEN, cap);
    ctx.expect_eq(
        decision.state.window_bytes,
        EXFAT_READAHEAD_MIN_BYTES,
        "random read resets window to the minimum",
    );
    ctx.expect_eq(
        decision.prefetch,
        Some(ExFatPrefetch {
            offset: READ_POS + READ_LEN as u32,
            len: EXFAT_READAHEAD_MIN_BYTES as usize,
        }),
        "random read starts prefetch at the new read end",
    );
}
