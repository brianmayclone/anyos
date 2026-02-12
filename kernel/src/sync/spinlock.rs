//! IRQ-safe spinlock with automatic interrupt disable/restore.
//!
//! Disables interrupts before acquiring the lock and restores the previous
//! interrupt state on drop, preventing deadlocks from IRQ handlers trying
//! to acquire an already-held lock on a single-core system.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

/// An IRQ-safe spinlock protecting data of type `T`.
///
/// Automatically disables interrupts while held and restores the previous
/// interrupt state when the guard is dropped. Safe to use from both normal
/// code and interrupt handlers (via [`try_lock`](Spinlock::try_lock)).
pub struct Spinlock<T> {
    lock: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for Spinlock<T> {}
unsafe impl<T: Send> Send for Spinlock<T> {}

/// RAII guard for a held [`Spinlock`].
///
/// Provides `Deref`/`DerefMut` access to the protected data. On drop, releases
/// the lock and restores the interrupt state that was saved at acquisition time.
pub struct SpinlockGuard<'a, T> {
    lock: &'a Spinlock<T>,
    irq_was_enabled: bool,
}

/// Check if interrupts are currently enabled (EFLAGS.IF bit 9).
#[inline(always)]
fn interrupts_enabled() -> bool {
    let flags: u64;
    unsafe {
        core::arch::asm!("pushfq; pop {}", out(reg) flags, options(nomem, preserves_flags));
    }
    flags & (1 << 9) != 0
}

/// Disable interrupts.
#[inline(always)]
fn cli() {
    unsafe {
        core::arch::asm!("cli", options(nomem, nostack));
    }
}

/// Enable interrupts.
#[inline(always)]
fn sti() {
    unsafe {
        core::arch::asm!("sti", options(nomem, nostack));
    }
}

impl<T> Spinlock<T> {
    /// Create a new unlocked spinlock wrapping the given data.
    pub const fn new(data: T) -> Self {
        Spinlock {
            lock: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquire the lock, spinning until it becomes available.
    ///
    /// Disables interrupts before spinning to prevent single-core deadlocks.
    pub fn lock(&self) -> SpinlockGuard<T> {
        // Save interrupt state and disable interrupts BEFORE acquiring the lock.
        // This prevents deadlock: if we hold the lock and a timer/device IRQ fires,
        // the IRQ handler can't try to acquire the same lock.
        let was_enabled = interrupts_enabled();
        cli();

        while self
            .lock
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // On single-core with interrupts disabled, spinning here means deadlock
            // (we can never release because we're the only core and IRQs are off).
            // On multi-core, the other core will eventually release.
            while self.lock.load(Ordering::Relaxed) {
                core::hint::spin_loop();
            }
        }

        SpinlockGuard { lock: self, irq_was_enabled: was_enabled }
    }

    /// Try to acquire the lock without blocking.
    ///
    /// Returns `Some(guard)` if the lock was acquired, `None` otherwise.
    /// Restores interrupt state on failure.
    pub fn try_lock(&self) -> Option<SpinlockGuard<T>> {
        let was_enabled = interrupts_enabled();
        cli();

        if self
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinlockGuard { lock: self, irq_was_enabled: was_enabled })
        } else {
            // Failed to acquire — restore interrupt state
            if was_enabled {
                sti();
            }
            None
        }
    }
}

impl<'a, T> Deref for SpinlockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        let ptr = self.lock.data.get();
        let addr = ptr as usize;
        let align = core::mem::align_of::<T>();
        if addr % align != 0 {
            crate::serial_println!(
                "BUG: Spinlock Deref misaligned ptr={:#x} align={} lock={:#x}",
                addr, align, self.lock as *const _ as usize,
            );
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        unsafe { &*ptr }
    }
}

impl<'a, T> DerefMut for SpinlockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        let ptr = self.lock.data.get();
        let addr = ptr as usize;
        let align = core::mem::align_of::<T>();
        if addr % align != 0 {
            crate::serial_println!(
                "BUG: Spinlock DerefMut misaligned ptr={:#x} align={} lock={:#x}",
                addr, align, self.lock as *const _ as usize,
            );
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        unsafe { &mut *ptr }
    }
}

impl<'a, T> SpinlockGuard<'a, T> {
    /// Release the lock WITHOUT restoring the saved interrupt state.
    /// Interrupts remain disabled after this call.
    /// Used by schedule() to keep IF=0 from lock acquisition through context_switch.
    pub fn release_no_irq_restore(self) {
        self.lock.lock.store(false, Ordering::Release);
        core::mem::forget(self); // Skip Drop (which would restore IF)
    }
}

impl<'a, T> Drop for SpinlockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.lock.store(false, Ordering::Release);
        // Restore interrupt state AFTER releasing the lock.
        // For nested locks: inner lock saves IF=0, restores IF=0 → still disabled.
        // Outermost lock restores the original state (usually IF=1).
        if self.irq_was_enabled {
            sti();
        }
    }
}
