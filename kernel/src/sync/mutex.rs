//! Yielding mutex — gives up the current time slice instead of spinning
//! with interrupts disabled, so other threads (including the compositor)
//! keep running while the caller waits.
//!
//! Internally a brief [`Spinlock`] protects the `locked` flag.  When the
//! mutex is contended the thread briefly yields, then backs off to a one-tick
//! scheduler sleep if contention persists.  This avoids the lost-wakeup race
//! inherent in block/wake designs and requires zero heap allocation, while
//! preventing hot yield loops when no runnable thread can make progress.
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
        let mut attempts = 0u32;
        loop {
            {
                let mut locked = self.inner.lock();
                if !*locked {
                    *locked = true;
                    return MutexGuard { mutex: self };
                }
            } // Spinlock released — interrupts re-enabled

            crate::task::scheduler::contention_backoff(&mut attempts);
        }
    }

    /// Try to acquire the mutex without blocking.
    ///
    /// Returns `Some(guard)` if the mutex was free, `None` if it was already
    /// held. Never blocks or yields. Safe to call from any context including
    /// panic handlers where yielding is not possible.
    pub fn try_lock(&self) -> Option<MutexGuard<T>> {
        let mut locked = self.inner.lock();
        if !*locked {
            *locked = true;
            Some(MutexGuard { mutex: self })
        } else {
            None
        }
    }

    /// Check if this mutex is currently held (by any thread).
    pub fn is_locked(&self) -> bool {
        *self.inner.lock()
    }

    /// Force-release the mutex unconditionally.
    ///
    /// # Safety
    /// Only call from a fault/panic handler when the current thread is known
    /// to hold the mutex and is about to be killed. The protected data may be
    /// in a partially-modified state — no further access should occur.
    pub unsafe fn force_unlock(&self) {
        // Try to acquire the inner spinlock. If we can't (extremely unlikely —
        // the inner spinlock is held for microseconds), skip: the lock will be
        // released by its holder momentarily anyway.
        if let Some(mut locked) = self.inner.try_lock() {
            *locked = false;
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
