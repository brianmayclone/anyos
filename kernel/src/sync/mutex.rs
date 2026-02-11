//! Yielding mutex — gives up the current time slice instead of spinning
//! with interrupts disabled, so other threads (including the compositor)
//! keep running while the caller waits.
//!
//! Internally a brief [`Spinlock`] protects the `locked` flag.  When the
//! mutex is contended the thread does a voluntary `schedule()` (yield)
//! which puts it back in the ready queue.  The timer will reschedule it,
//! and it retries the lock.  This avoids the lost-wakeup race inherent
//! in block/wake designs and requires zero heap allocation.
//!
//! **Must NOT be used from interrupt handlers** — only from preemptible
//! kernel context (syscalls, kernel threads).

use crate::sync::spinlock::Spinlock;
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};

/// A yielding mutex that gives up its time slice when contended.
///
/// Interrupts stay enabled between retries, so timer/mouse/keyboard
/// events are processed normally even during long-held locks (e.g. VFS
/// disk I/O).
pub struct Mutex<T> {
    inner: Spinlock<bool>,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for Mutex<T> {}
unsafe impl<T: Send> Send for Mutex<T> {}

/// RAII guard for a held [`Mutex`].
///
/// Provides `Deref`/`DerefMut` access to the protected data.  Releases
/// the mutex when dropped.
pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> Mutex<T> {
    /// Create a new unlocked mutex wrapping the given data.
    pub const fn new(data: T) -> Self {
        Mutex {
            inner: Spinlock::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquire the mutex, yielding the current time slice if contended.
    pub fn lock(&self) -> MutexGuard<T> {
        loop {
            {
                let mut locked = self.inner.lock();
                if !*locked {
                    *locked = true;
                    return MutexGuard { mutex: self };
                }
            } // Spinlock released — interrupts re-enabled

            // Yield our time slice so other threads (including the lock
            // holder) can run.  We stay Ready and will be rescheduled.
            crate::task::scheduler::schedule();
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
        let mut locked = self.mutex.inner.lock();
        *locked = false;
    }
}
