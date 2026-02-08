use crate::sync::spinlock::Spinlock;

/// Counting semaphore.
/// In Phase 1 this spins; in Phase 2+ it will integrate with the scheduler.
pub struct Semaphore {
    inner: Spinlock<SemaphoreInner>,
}

struct SemaphoreInner {
    count: i32,
    // TODO Phase 2: wait queue
}

impl Semaphore {
    pub const fn new(initial: i32) -> Self {
        Semaphore {
            inner: Spinlock::new(SemaphoreInner { count: initial }),
        }
    }

    /// Decrement (wait/P operation). Blocks if count <= 0.
    pub fn wait(&self) {
        loop {
            {
                let mut inner = self.inner.lock();
                if inner.count > 0 {
                    inner.count -= 1;
                    return;
                }
            }
            // TODO Phase 2: block on scheduler
            core::hint::spin_loop();
        }
    }

    /// Increment (signal/V operation). Wakes one waiting thread.
    pub fn signal(&self) {
        let mut inner = self.inner.lock();
        inner.count += 1;
        // TODO Phase 2: wake one thread from wait queue
    }

    pub fn try_wait(&self) -> bool {
        let mut inner = self.inner.lock();
        if inner.count > 0 {
            inner.count -= 1;
            true
        } else {
            false
        }
    }
}
