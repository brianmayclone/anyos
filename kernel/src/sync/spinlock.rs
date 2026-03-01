//! IRQ-safe spinlock with automatic interrupt disable/restore.
//!
//! Disables interrupts before acquiring the lock and restores the previous
//! interrupt state on drop, preventing deadlocks from IRQ handlers trying
//! to acquire an already-held lock on a single-core system.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// No CPU owns this lock.
const NO_OWNER: u32 = u32::MAX;

/// After this many inner-loop iterations, print a deadlock diagnostic via
/// direct UART.  10M iterations × ~10-40 ns/PAUSE ≈ 100-400 ms — long enough
/// that normal contention never triggers it, short enough to fire before
/// the system appears frozen.
const SPIN_TIMEOUT: u32 = 10_000_000;

// ── Lock-free UART helpers (bypass ALL software locks) ──────────────────

/// Write one byte directly to the UART, polling for ready.
#[inline(never)]
fn diag_putc(c: u8) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        while crate::arch::x86::port::inb(0x3FD) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        crate::arch::x86::port::outb(0x3F8, c);
    }
    #[cfg(target_arch = "aarch64")]
    {
        crate::arch::arm64::serial::write_byte(c);
    }
}

fn diag_puts(s: &[u8]) {
    for &c in s {
        if c == b'\n' { diag_putc(b'\r'); }
        diag_putc(c);
    }
}

fn diag_hex(mut n: u64) {
    diag_puts(b"0x");
    if n == 0 { diag_putc(b'0'); return; }
    let mut buf = [0u8; 16];
    let mut i = 0usize;
    while n > 0 {
        let d = (n & 0xF) as u8;
        buf[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        n >>= 4;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        diag_putc(buf[i]);
    }
}

fn diag_dec(mut n: u32) {
    if n == 0 { diag_putc(b'0'); return; }
    let mut buf = [0u8; 10];
    let mut i = 0usize;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        diag_putc(buf[i]);
    }
}

/// An IRQ-safe spinlock protecting data of type `T`.
///
/// Automatically disables interrupts while held and restores the previous
/// interrupt state when the guard is dropped. Safe to use from both normal
/// code and interrupt handlers (via [`try_lock`](Spinlock::try_lock)).
///
/// Tracks the owning CPU ID for deadlock recovery: if a user thread faults
/// while this CPU holds the lock, the fault handler can force-release it
/// instead of deadlocking the entire system.
pub struct Spinlock<T> {
    lock: AtomicBool,
    owner_cpu: AtomicU32,
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

/// Check if interrupts are currently enabled.
#[inline(always)]
fn interrupts_enabled() -> bool {
    crate::arch::hal::interrupts_enabled()
}

/// Disable interrupts.
#[inline(always)]
fn cli() {
    crate::arch::hal::disable_interrupts();
}

/// Enable interrupts.
#[inline(always)]
fn sti() {
    crate::arch::hal::enable_interrupts();
}

/// Get the logical CPU index for spinlock ownership tracking.
#[inline(always)]
fn cpu_id() -> u32 {
    crate::arch::hal::cpu_id() as u32
}

impl<T> Spinlock<T> {
    /// Create a new unlocked spinlock wrapping the given data.
    pub const fn new(data: T) -> Self {
        Spinlock {
            lock: AtomicBool::new(false),
            owner_cpu: AtomicU32::new(NO_OWNER),
            data: UnsafeCell::new(data),
        }
    }

    /// Acquire the lock, spinning until it becomes available.
    ///
    /// Disables interrupts before spinning to prevent single-core deadlocks.
    /// If spinning exceeds `SPIN_TIMEOUT` iterations, prints a diagnostic
    /// via direct UART (lock-free) to identify the deadlocking lock and CPUs.
    pub fn lock(&self) -> SpinlockGuard<T> {
        // Save interrupt state and disable interrupts BEFORE acquiring the lock.
        // This prevents deadlock: if we hold the lock and a timer/device IRQ fires,
        // the IRQ handler can't try to acquire the same lock.
        let was_enabled = interrupts_enabled();
        cli();

        let mut spin_count: u32 = 0;
        let mut reported = false;

        while self
            .lock
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            // Exponential PAUSE backoff: 1, 2, 4, 8, 16, 32, 64 PAUSEs per check.
            // Reduces cache-line bouncing under contention and helps VMware's
            // "PAUSE Loop Exiting" (PLE) detect spin-waiting vCPUs.
            let mut backoff: u32 = 1;
            while self.lock.load(Ordering::Relaxed) {
                for _ in 0..backoff {
                    core::hint::spin_loop();
                }
                spin_count += backoff;
                if backoff < 64 {
                    backoff <<= 1;
                }

                if !reported && spin_count >= SPIN_TIMEOUT {
                    reported = true;
                    let lock_addr = self as *const _ as u64;
                    let me = cpu_id();
                    let owner = self.owner_cpu.load(Ordering::Relaxed);
                    diag_puts(b"\n!!! SPIN TIMEOUT lock=");
                    diag_hex(lock_addr);
                    diag_puts(b" cpu=");
                    diag_dec(me);
                    diag_puts(b" owner=");
                    if owner == NO_OWNER {
                        diag_puts(b"NONE");
                    } else {
                        diag_dec(owner);
                        // Print the SCHEDULER lock phase of the owner CPU so we
                        // know which function is holding the lock for 100-400 ms.
                        let phase = crate::sched_diag::get(owner as usize);
                        diag_puts(b" phase=");
                        diag_puts(crate::sched_diag::name(phase));
                    }
                    diag_putc(b'\n');
                }
            }
        }

        self.owner_cpu.store(cpu_id(), Ordering::Relaxed);
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
            self.owner_cpu.store(cpu_id(), Ordering::Relaxed);
            Some(SpinlockGuard { lock: self, irq_was_enabled: was_enabled })
        } else {
            // Failed to acquire — restore interrupt state
            if was_enabled {
                sti();
            }
            None
        }
    }

    /// Check if this lock is currently held by the given CPU.
    #[inline]
    pub fn is_held_by_cpu(&self, cpu: u32) -> bool {
        self.owner_cpu.load(Ordering::Relaxed) == cpu
    }

    /// Check if this lock is currently held (by any CPU).
    #[inline]
    pub fn is_locked(&self) -> bool {
        self.lock.load(Ordering::Relaxed)
    }

    /// Force-release the lock unconditionally. Used by the fault handler to
    /// recover from a user-thread fault that occurred while this CPU held the lock.
    ///
    /// # Safety
    /// Caller must ensure this CPU actually holds the lock (check `is_held_by_cpu` first).
    /// The protected data may be in a partially-modified state.
    pub unsafe fn force_unlock(&self) {
        self.owner_cpu.store(NO_OWNER, Ordering::Relaxed);
        self.lock.store(false, Ordering::Release);
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
            loop {
                crate::arch::hal::disable_interrupts();
                crate::arch::hal::halt();
            }
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
            loop {
                crate::arch::hal::disable_interrupts();
                crate::arch::hal::halt();
            }
        }
        unsafe { &mut *ptr }
    }
}

impl<'a, T> SpinlockGuard<'a, T> {
    /// Release the lock WITHOUT restoring the saved interrupt state.
    /// Interrupts remain disabled after this call.
    /// Used by schedule() to keep IF=0 from lock acquisition through context_switch.
    pub fn release_no_irq_restore(self) {
        self.lock.owner_cpu.store(NO_OWNER, Ordering::Relaxed);
        self.lock.lock.store(false, Ordering::Release);
        core::mem::forget(self); // Skip Drop (which would restore IF)
    }
}

impl<'a, T> Drop for SpinlockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.owner_cpu.store(NO_OWNER, Ordering::Relaxed);
        self.lock.lock.store(false, Ordering::Release);
        // Restore interrupt state AFTER releasing the lock.
        // For nested locks: inner lock saves IF=0, restores IF=0 → still disabled.
        // Outermost lock restores the original state (usually IF=1).
        if self.irq_was_enabled {
            sti();
        }
    }
}
