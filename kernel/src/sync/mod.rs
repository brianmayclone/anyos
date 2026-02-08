//! Synchronization primitives for the kernel.
//!
//! Provides an IRQ-safe [`spinlock::Spinlock`], a sleeping [`mutex::Mutex`],
//! and a counting [`semaphore::Semaphore`].

pub mod spinlock;
pub mod mutex;
pub mod semaphore;
