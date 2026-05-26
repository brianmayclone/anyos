//! Tests for VFS-level exFAT read-ahead policy.

use crate::fs::file::ReadAheadState;
use crate::fs::vfs::{
    exfat_readahead_cap_for_memory_frames, exfat_readahead_decision, ExFatPrefetch,
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
    ctx.expect_eq(
        exfat_readahead_cap_for_memory_frames(frames(8), frames(512)),
        0u32,
        "very low free memory disables prefetch",
    );
    ctx.expect_eq(
        exfat_readahead_cap_for_memory_frames(frames(128), frames(512)),
        256 * 1024u32,
        "medium memory selects 256 KiB cap",
    );
    ctx.expect_eq(
        exfat_readahead_cap_for_memory_frames(frames(512), frames(1024)),
        512 * 1024u32,
        "high memory selects 512 KiB cap",
    );
    ctx.expect_eq(
        exfat_readahead_cap_for_memory_frames(frames(2048), frames(4096)),
        1024 * 1024u32,
        "large memory selects 1 MiB cap",
    );
}

fn test_sequential_reads_grow_window(ctx: &mut TestContext) {
    let first = exfat_readahead_decision(ReadAheadState::new(0), 0, 1024 * 1024, 4096, 256 * 1024);
    ctx.expect_eq(
        first.state.next_offset,
        4096u32,
        "first read advances next offset",
    );
    ctx.expect_eq(
        first.state.window_bytes,
        32 * 1024u32,
        "first window starts at minimum",
    );
    ctx.expect_eq(
        first.prefetch,
        Some(ExFatPrefetch {
            offset: 4096,
            len: 32 * 1024,
        }),
        "first read schedules initial prefetch",
    );

    let second = exfat_readahead_decision(first.state, 4096, 1024 * 1024, 4096, 256 * 1024);
    ctx.expect_eq(
        second.state.window_bytes,
        64 * 1024u32,
        "sequential read doubles window",
    );
    ctx.expect_eq(
        second.prefetch,
        Some(ExFatPrefetch {
            offset: 4096 + 32 * 1024,
            len: 36 * 1024,
        }),
        "second read tops up beyond already-prefetched bytes",
    );
}

fn test_random_read_resets_window(ctx: &mut TestContext) {
    let mut state = ReadAheadState::new(0);
    state.next_offset = 4096;
    state.window_bytes = 128 * 1024;
    state.prefetched_until = 256 * 1024;

    let decision = exfat_readahead_decision(state, 128 * 1024, 1024 * 1024, 4096, 256 * 1024);
    ctx.expect_eq(
        decision.state.window_bytes,
        32 * 1024u32,
        "random read resets window",
    );
    ctx.expect_eq(
        decision.prefetch,
        Some(ExFatPrefetch {
            offset: 128 * 1024 + 4096,
            len: 32 * 1024,
        }),
        "random read starts prefetch at the new read end",
    );
}
