//! Counting semaphore with yield-based waiting.
//!
//! When the count is zero, [`wait`] briefly yields and then backs off to a
//! one-tick scheduler sleep under persistent contention.  No heap allocation
//! is needed.
//!
//! **Must NOT be used from interrupt handlers** — only from preemptible
//! kernel context (syscalls, kernel threads).

use crate::sync::spinlock::Spinlock;

/// Counting semaphore.
pub struct Semaphore {
    inner: Spinlock<i32>,
}

impl Semaphore {
    /// Create a new semaphore with the given initial count.
    pub const fn new(initial: i32) -> Self {
        Semaphore {
            inner: Spinlock::new(initial),
        }
    }

    /// Decrement (wait/P operation). Yields if count <= 0.
    pub fn wait(&self) {
        let mut attempts = 0u32;
        loop {
            {
                let mut count = self.inner.lock();
                if *count > 0 {
                    *count -= 1;
                    return;
                }
            }
            crate::task::scheduler::contention_backoff(&mut attempts);
        }
    }

    /// Increment (signal/V operation).
    pub fn signal(&self) {
        let mut count = self.inner.lock();
        *count += 1;
    }

    /// Try to decrement the semaphore without blocking.
    ///
    /// Returns `true` if the count was positive and was decremented, `false` otherwise.
    pub fn try_wait(&self) -> bool {
        let mut count = self.inner.lock();
        if *count > 0 {
            *count -= 1;
            true
        } else {
            false
        }
    }
}
