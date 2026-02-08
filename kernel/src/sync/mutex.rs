//! Sleeping mutex that blocks the calling thread instead of spinning.
//!
//! Currently falls back to spin-waiting. Future phases will integrate with the
//! scheduler to put blocked threads to sleep and wake them on unlock.

use crate::sync::spinlock::Spinlock;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};

/// A sleeping mutex that blocks the current thread if the lock is held.
/// In Phase 1, this falls back to spinning. In Phase 2+, it will integrate
/// with the scheduler to put threads to sleep.
pub struct Mutex<T> {
    inner: Spinlock<MutexInner>,
    data: UnsafeCell<T>,
}

struct MutexInner {
    locked: bool,
    // TODO Phase 2: wait queue of blocked thread IDs
}

unsafe impl<T: Send> Sync for Mutex<T> {}
unsafe impl<T: Send> Send for Mutex<T> {}

/// RAII guard for a held [`Mutex`].
///
/// Provides `Deref`/`DerefMut` access to the protected data. Releases the
/// mutex when dropped.
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> Mutex<T> {
    /// Create a new unlocked mutex wrapping the given data.
    pub const fn new(data: T) -> Self {
        Mutex {
            inner: Spinlock::new(MutexInner { locked: false }),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquire the mutex, blocking until it becomes available.
    pub fn lock(&self) -> MutexGuard<T> {
        loop {
            {
                let mut inner = self.inner.lock();
                if !inner.locked {
                    inner.locked = true;
                    return MutexGuard { mutex: self };
                }
            }
            // TODO Phase 2: yield to scheduler instead of spinning
            core::hint::spin_loop();
        }
    }
}

impl<'a, T> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<'a, T> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<'a, T> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        let mut inner = self.mutex.inner.lock();
        inner.locked = false;
        // TODO Phase 2: wake first thread in wait queue
    }
}
