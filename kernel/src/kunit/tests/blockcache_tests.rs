//! Tests for the global block-cache data structure.

use alloc::vec::Vec;

use crate::fs::blockcache::BlockCache;
use crate::kunit::{TestCase, TestContext, TestSuite};

pub static SUITE: TestSuite = TestSuite {
    name: "blockcache",
    cases: &[
        TestCase {
            name: "hash_holes_preserve_lookup",
            run: test_hash_holes_preserve_lookup,
        },
        TestCase {
            name: "dirty_update_after_hash_hole_is_unique",
            run: test_dirty_update_after_hash_hole_is_unique,
        },
    ],
};

fn hash_key(key: u64) -> usize {
    (key.wrapping_mul(11400714819323198485)) as usize >> (64 - 15)
}

fn cache_key(disk_id: u8, lba: u32) -> u64 {
    ((disk_id as u64) << 32) | lba as u64
}

fn colliding_lbas(disk_id: u8, need: usize) -> Vec<u32> {
    let target = hash_key(cache_key(disk_id, 1));
    let mut out = Vec::new();
    let mut lba = 1u32;
    while out.len() < need && lba < 2_000_000 {
        if hash_key(cache_key(disk_id, lba)) == target {
            out.push(lba);
        }
        lba += 1;
    }
    out
}

fn sector(fill: u8) -> [u8; 512] {
    [fill; 512]
}

fn test_hash_holes_preserve_lookup(ctx: &mut TestContext) {
    let disk_id = 0;
    let lbas = colliding_lbas(disk_id, 6);
    ctx.expect_eq(lbas.len(), 6usize, "found enough colliding lbas");
    if lbas.len() < 6 {
        return;
    }

    let mut cache = BlockCache::new();
    cache.init();

    for (i, lba) in lbas.iter().enumerate() {
        cache.insert(disk_id, *lba, &sector((i + 1) as u8));
    }

    cache.invalidate(disk_id, lbas[1]);

    let mut buf = [0u8; 512];
    let hit = cache.lookup(disk_id, lbas[5], &mut buf);
    ctx.expect_true(hit, "lookup finds entry beyond invalidated hash slot");
    ctx.expect_eq(buf[0], 6u8, "lookup returned the later colliding sector");
}

fn test_dirty_update_after_hash_hole_is_unique(ctx: &mut TestContext) {
    let disk_id = 0;
    let lbas = colliding_lbas(disk_id, 6);
    ctx.expect_eq(lbas.len(), 6usize, "found enough colliding lbas");
    if lbas.len() < 6 {
        return;
    }

    let mut cache = BlockCache::new();
    cache.init();

    for (i, lba) in lbas.iter().enumerate() {
        cache.insert_dirty(disk_id, *lba, &sector((i + 1) as u8));
    }

    cache.invalidate(disk_id, lbas[1]);
    cache.insert_dirty(disk_id, lbas[5], &sector(0xAB));

    let mut target_writes = 0u32;
    let mut target_first_byte = 0u8;
    let ok = cache.flush_dirty(disk_id, |lba, data| {
        if lba == lbas[5] {
            target_writes += 1;
            target_first_byte = data[0];
        }
        true
    });

    ctx.expect_true(ok, "dirty flush succeeded");
    ctx.expect_eq(target_writes, 1u32, "updated colliding lba flushed once");
    ctx.expect_eq(target_first_byte, 0xABu8, "dirty update kept newest data");
}
