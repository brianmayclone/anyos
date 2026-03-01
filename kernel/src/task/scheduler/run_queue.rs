//! Bitmap-indexed multi-level FIFO priority queue for O(1) thread selection.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

use super::NUM_PRIORITIES;

/// Per-CPU multi-level priority queue with O(1) highest-priority lookup.
///
/// 128 priority levels (0 = lowest / idle, 127 = highest / real-time).
/// A 2×u64 bitmap tracks which levels have queued threads. Finding the
/// highest non-empty level is a single `leading_zeros` operation.
pub(super) struct RunQueue {
    /// One FIFO queue per priority level.
    levels: Vec<VecDeque<u32>>,
    /// Bitmap: bit `p` set ⟺ `levels[p]` is non-empty.
    /// `bits[0]` covers priorities 0–63, `bits[1]` covers 64–127.
    bits: [u64; 2],
    /// Cached total count — avoids O(128) sum on every `total_count()` call.
    count: usize,
}

impl RunQueue {
    pub(super) fn new() -> Self {
        let mut levels = Vec::with_capacity(NUM_PRIORITIES);
        for _ in 0..NUM_PRIORITIES {
            levels.push(VecDeque::new());
        }
        RunQueue { levels, bits: [0; 2], count: 0 }
    }

    /// Enqueue a TID at the given priority level (back of FIFO).
    /// Caller must ensure no duplicates (the scheduler guarantees this via
    /// state transitions: only Ready threads are enqueued, and they transition
    /// to Running immediately on pick).
    pub(super) fn enqueue(&mut self, tid: u32, priority: u8) {
        let p = (priority as usize).min(NUM_PRIORITIES - 1);
        self.levels[p].push_back(tid);
        self.bits[p / 64] |= 1u64 << (p % 64);
        self.count += 1;
    }

    /// Dequeue the highest-priority thread (front of its FIFO). O(1) via bitmap.
    pub(super) fn dequeue_highest(&mut self) -> Option<u32> {
        let p = self.highest_priority()?;
        let tid = self.levels[p].pop_front()?;
        if self.levels[p].is_empty() {
            self.bits[p / 64] &= !(1u64 << (p % 64));
        }
        self.count -= 1;
        Some(tid)
    }

    /// Dequeue the lowest-priority thread (used for work stealing).
    pub(super) fn dequeue_lowest(&mut self) -> Option<u32> {
        let p = self.lowest_priority()?;
        let tid = self.levels[p].pop_front()?;
        if self.levels[p].is_empty() {
            self.bits[p / 64] &= !(1u64 << (p % 64));
        }
        self.count -= 1;
        Some(tid)
    }

    /// Remove a specific TID from all priority levels.
    pub(super) fn remove(&mut self, tid: u32) {
        for p in 0..NUM_PRIORITIES {
            if let Some(pos) = self.levels[p].iter().position(|&t| t == tid) {
                self.levels[p].remove(pos);
                if self.levels[p].is_empty() {
                    self.bits[p / 64] &= !(1u64 << (p % 64));
                }
                self.count -= 1;
                return;
            }
        }
    }

    /// Total number of queued threads across all priority levels. O(1).
    #[inline]
    pub(super) fn total_count(&self) -> usize {
        self.count
    }

    pub(super) fn is_empty(&self) -> bool {
        self.bits[0] == 0 && self.bits[1] == 0
    }

    /// Highest priority level that has queued threads.
    fn highest_priority(&self) -> Option<usize> {
        if self.bits[1] != 0 {
            Some(127 - self.bits[1].leading_zeros() as usize)
        } else if self.bits[0] != 0 {
            Some(63 - self.bits[0].leading_zeros() as usize)
        } else {
            None
        }
    }

    /// Lowest priority level that has queued threads.
    fn lowest_priority(&self) -> Option<usize> {
        if self.bits[0] != 0 {
            Some(self.bits[0].trailing_zeros() as usize)
        } else if self.bits[1] != 0 {
            Some(64 + self.bits[1].trailing_zeros() as usize)
        } else {
            None
        }
    }
}
