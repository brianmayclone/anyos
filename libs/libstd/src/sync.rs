//! std::sync compatible synchronization primitives.
//!
//! Provides Mutex, RwLock, Arc, Once, and mpsc channels.
//! Uses spinlocks since anyOS has no futex syscalls.

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

// Re-export Arc from alloc
pub use alloc::sync::Arc as ArcImpl;
pub use alloc::sync::Weak;

// ── Mutex ───────────────────────────────────────────────────────────────────

/// A mutual exclusion lock, mirroring std::sync::Mutex.
///
/// Uses a spinlock internally. The lock yields the CPU via
/// `anyos_std::process::yield_cpu()` while spinning.
pub struct Mutex<T: ?Sized> {
    locked: AtomicBool,
    poisoned: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T> Mutex<T> {
    pub const fn new(data: T) -> Self {
        Mutex {
            locked: AtomicBool::new(false),
            poisoned: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    pub fn into_inner(self) -> Result<T, PoisonError<T>> {
        if self.poisoned.load(Ordering::Relaxed) {
            let data = self.data.into_inner();
            Err(PoisonError::new(data))
        } else {
            Ok(self.data.into_inner())
        }
    }
}

impl<T: ?Sized> Mutex<T> {
    pub fn lock(&self) -> Result<MutexGuard<'_, T>, PoisonError<MutexGuard<'_, T>>> {
        self.raw_lock();
        let guard = MutexGuard { mutex: self };
        if self.poisoned.load(Ordering::Relaxed) {
            Err(PoisonError::new(guard))
        } else {
            Ok(guard)
        }
    }

    pub fn try_lock(&self) -> Result<MutexGuard<'_, T>, TryLockError<MutexGuard<'_, T>>> {
        if self.locked.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            let guard = MutexGuard { mutex: self };
            if self.poisoned.load(Ordering::Relaxed) {
                Err(TryLockError::Poisoned(PoisonError::new(guard)))
            } else {
                Ok(guard)
            }
        } else {
            Err(TryLockError::WouldBlock)
        }
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Relaxed)
    }

    pub fn get_mut(&mut self) -> Result<&mut T, PoisonError<&mut T>> {
        let data = self.data.get_mut();
        if self.poisoned.load(Ordering::Relaxed) {
            Err(PoisonError::new(data))
        } else {
            Ok(data)
        }
    }

    fn raw_lock(&self) {
        let mut spins = 0u32;
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            spins += 1;
            if spins > 100 {
                anyos_std::process::yield_cpu();
                spins = 0;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    fn raw_unlock(&self) {
        self.locked.store(false, Ordering::Release);
    }
}

/// RAII guard for Mutex.
pub struct MutexGuard<'a, T: ?Sized> {
    mutex: &'a Mutex<T>,
}

impl<T: ?Sized> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T: ?Sized> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T: ?Sized> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.raw_unlock();
    }
}

// ── RwLock ──────────────────────────────────────────────────────────────────

/// A reader-writer lock, mirroring std::sync::RwLock.
///
/// Multiple readers can hold the lock simultaneously, or one writer.
/// Uses atomics + spinning.
pub struct RwLock<T: ?Sized> {
    /// 0 = unlocked, u32::MAX = write-locked, 1..MAX-1 = reader count
    state: AtomicU32,
    poisoned: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: ?Sized + Send> Send for RwLock<T> {}
unsafe impl<T: ?Sized + Send + Sync> Sync for RwLock<T> {}

impl<T> RwLock<T> {
    pub const fn new(data: T) -> Self {
        RwLock {
            state: AtomicU32::new(0),
            poisoned: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    pub fn into_inner(self) -> Result<T, PoisonError<T>> {
        if self.poisoned.load(Ordering::Relaxed) {
            Err(PoisonError::new(self.data.into_inner()))
        } else {
            Ok(self.data.into_inner())
        }
    }
}

impl<T: ?Sized> RwLock<T> {
    pub fn read(&self) -> Result<RwLockReadGuard<'_, T>, PoisonError<RwLockReadGuard<'_, T>>> {
        let mut spins = 0u32;
        loop {
            let state = self.state.load(Ordering::Relaxed);
            if state != u32::MAX {
                if self.state.compare_exchange_weak(state, state + 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                    let guard = RwLockReadGuard { lock: self };
                    return if self.poisoned.load(Ordering::Relaxed) {
                        Err(PoisonError::new(guard))
                    } else {
                        Ok(guard)
                    };
                }
            }
            spins += 1;
            if spins > 100 {
                anyos_std::process::yield_cpu();
                spins = 0;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    pub fn write(&self) -> Result<RwLockWriteGuard<'_, T>, PoisonError<RwLockWriteGuard<'_, T>>> {
        let mut spins = 0u32;
        loop {
            if self.state.compare_exchange_weak(0, u32::MAX, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                let guard = RwLockWriteGuard { lock: self };
                return if self.poisoned.load(Ordering::Relaxed) {
                    Err(PoisonError::new(guard))
                } else {
                    Ok(guard)
                };
            }
            spins += 1;
            if spins > 100 {
                anyos_std::process::yield_cpu();
                spins = 0;
            } else {
                core::hint::spin_loop();
            }
        }
    }

    pub fn try_read(&self) -> Result<RwLockReadGuard<'_, T>, TryLockError<RwLockReadGuard<'_, T>>> {
        let state = self.state.load(Ordering::Relaxed);
        if state != u32::MAX {
            if self.state.compare_exchange(state, state + 1, Ordering::Acquire, Ordering::Relaxed).is_ok() {
                let guard = RwLockReadGuard { lock: self };
                return if self.poisoned.load(Ordering::Relaxed) {
                    Err(TryLockError::Poisoned(PoisonError::new(guard)))
                } else {
                    Ok(guard)
                };
            }
        }
        Err(TryLockError::WouldBlock)
    }

    pub fn try_write(&self) -> Result<RwLockWriteGuard<'_, T>, TryLockError<RwLockWriteGuard<'_, T>>> {
        if self.state.compare_exchange(0, u32::MAX, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            let guard = RwLockWriteGuard { lock: self };
            if self.poisoned.load(Ordering::Relaxed) {
                Err(TryLockError::Poisoned(PoisonError::new(guard)))
            } else {
                Ok(guard)
            }
        } else {
            Err(TryLockError::WouldBlock)
        }
    }

    pub fn is_poisoned(&self) -> bool {
        self.poisoned.load(Ordering::Relaxed)
    }

    pub fn get_mut(&mut self) -> Result<&mut T, PoisonError<&mut T>> {
        let data = self.data.get_mut();
        if self.poisoned.load(Ordering::Relaxed) {
            Err(PoisonError::new(data))
        } else {
            Ok(data)
        }
    }
}

pub struct RwLockReadGuard<'a, T: ?Sized> {
    lock: &'a RwLock<T>,
}

impl<T: ?Sized> Deref for RwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockReadGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.state.fetch_sub(1, Ordering::Release);
    }
}

pub struct RwLockWriteGuard<'a, T: ?Sized> {
    lock: &'a RwLock<T>,
}

impl<T: ?Sized> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for RwLockWriteGuard<'_, T> {
    fn drop(&mut self) {
        self.lock.state.store(0, Ordering::Release);
    }
}

// ── Once ────────────────────────────────────────────────────────────────────

/// A synchronization primitive for one-time initialization.
pub struct Once {
    state: AtomicU32, // 0=incomplete, 1=running, 2=complete, 3=poisoned
}

impl Once {
    pub const fn new() -> Self {
        Once { state: AtomicU32::new(0) }
    }

    pub fn call_once<F: FnOnce()>(&self, f: F) {
        if self.state.load(Ordering::Acquire) == 2 {
            return;
        }
        self.call_once_slow(f);
    }

    #[cold]
    fn call_once_slow<F: FnOnce()>(&self, f: F) {
        loop {
            match self.state.compare_exchange(0, 1, Ordering::Acquire, Ordering::Acquire) {
                Ok(_) => {
                    f();
                    self.state.store(2, Ordering::Release);
                    return;
                }
                Err(2) => return, // Already done
                Err(_) => {
                    // Someone else is running it, spin
                    while self.state.load(Ordering::Acquire) == 1 {
                        core::hint::spin_loop();
                    }
                    if self.state.load(Ordering::Acquire) == 2 {
                        return;
                    }
                }
            }
        }
    }

    pub fn is_completed(&self) -> bool {
        self.state.load(Ordering::Acquire) == 2
    }
}

// ── OnceLock ────────────────────────────────────────────────────────────────

/// A cell that can be written to only once, thread-safe.
pub struct OnceLock<T> {
    once: Once,
    value: UnsafeCell<Option<T>>,
}

unsafe impl<T: Send + Sync> Send for OnceLock<T> {}
unsafe impl<T: Send + Sync> Sync for OnceLock<T> {}

impl<T> OnceLock<T> {
    pub const fn new() -> Self {
        OnceLock {
            once: Once::new(),
            value: UnsafeCell::new(None),
        }
    }

    pub fn get(&self) -> Option<&T> {
        if self.once.is_completed() {
            unsafe { (*self.value.get()).as_ref() }
        } else {
            None
        }
    }

    pub fn set(&self, value: T) -> Result<(), T> {
        let mut val = Some(value);
        self.once.call_once(|| {
            unsafe { *self.value.get() = val.take(); }
        });
        match val {
            Some(v) => Err(v), // Already initialized
            None => Ok(()),
        }
    }

    pub fn get_or_init<F: FnOnce() -> T>(&self, f: F) -> &T {
        self.once.call_once(|| {
            unsafe { *self.value.get() = Some(f()); }
        });
        unsafe { (*self.value.get()).as_ref().unwrap() }
    }

    pub fn into_inner(self) -> Option<T> {
        self.value.into_inner()
    }
}

// ── Condvar ─────────────────────────────────────────────────────────────────

/// A condition variable (simplified spinning implementation).
pub struct Condvar {
    seq: AtomicU32,
}

impl Condvar {
    pub const fn new() -> Self {
        Condvar { seq: AtomicU32::new(0) }
    }

    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> Result<MutexGuard<'a, T>, PoisonError<MutexGuard<'a, T>>> {
        let seq = self.seq.load(Ordering::Acquire);
        let mutex = guard.mutex;
        drop(guard);

        // Spin until notified
        let mut spins = 0u32;
        while self.seq.load(Ordering::Acquire) == seq {
            spins += 1;
            if spins > 50 {
                anyos_std::process::yield_cpu();
                spins = 0;
            } else {
                core::hint::spin_loop();
            }
        }

        mutex.lock()
    }

    pub fn notify_one(&self) {
        self.seq.fetch_add(1, Ordering::Release);
    }

    pub fn notify_all(&self) {
        self.seq.fetch_add(1, Ordering::Release);
    }
}

// ── Barrier ─────────────────────────────────────────────────────────────────

/// A barrier for synchronizing multiple threads.
pub struct Barrier {
    n: usize,
    count: AtomicUsize,
    generation: AtomicUsize,
}

pub struct BarrierWaitResult {
    is_leader: bool,
}

impl BarrierWaitResult {
    pub fn is_leader(&self) -> bool {
        self.is_leader
    }
}

impl Barrier {
    pub fn new(n: usize) -> Self {
        Barrier {
            n,
            count: AtomicUsize::new(0),
            generation: AtomicUsize::new(0),
        }
    }

    pub fn wait(&self) -> BarrierWaitResult {
        let gen = self.generation.load(Ordering::Acquire);
        let prev = self.count.fetch_add(1, Ordering::AcqRel);

        if prev + 1 == self.n {
            // Last thread to arrive — reset and advance generation
            self.count.store(0, Ordering::Release);
            self.generation.fetch_add(1, Ordering::Release);
            BarrierWaitResult { is_leader: true }
        } else {
            // Wait for generation to advance
            while self.generation.load(Ordering::Acquire) == gen {
                anyos_std::process::yield_cpu();
            }
            BarrierWaitResult { is_leader: false }
        }
    }
}

// ── mpsc channel ────────────────────────────────────────────────────────────

/// Multi-producer, single-consumer channel.
pub mod mpsc {
    use super::*;

    /// Channel error types.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct SendError<T>(pub T);

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct RecvError;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum TryRecvError {
        Empty,
        Disconnected,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum TrySendError<T> {
        Full(T),
        Disconnected(T),
    }

    impl core::fmt::Display for RecvError {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("receiving on a closed channel")
        }
    }

    impl<T> core::fmt::Display for SendError<T> {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.write_str("sending on a closed channel")
        }
    }

    struct Inner<T> {
        queue: Mutex<Vec<T>>,
        closed: AtomicBool,
        has_data: AtomicBool,
    }

    /// Sending half of the channel.
    pub struct Sender<T> {
        inner: Arc<Inner<T>>,
    }

    impl<T> Clone for Sender<T> {
        fn clone(&self) -> Self {
            Sender { inner: Arc::clone(&self.inner) }
        }
    }

    impl<T> Sender<T> {
        pub fn send(&self, value: T) -> Result<(), SendError<T>> {
            if self.inner.closed.load(Ordering::Acquire) {
                return Err(SendError(value));
            }
            let mut queue = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.push(value);
            self.inner.has_data.store(true, Ordering::Release);
            Ok(())
        }
    }

    impl<T> Drop for Sender<T> {
        fn drop(&mut self) {
            // If this is the last sender, mark channel as closed
            if Arc::strong_count(&self.inner) <= 2 {
                self.inner.closed.store(true, Ordering::Release);
            }
        }
    }

    /// Receiving half of the channel.
    pub struct Receiver<T> {
        inner: Arc<Inner<T>>,
    }

    impl<T> Receiver<T> {
        pub fn recv(&self) -> Result<T, RecvError> {
            loop {
                match self.try_recv() {
                    Ok(val) => return Ok(val),
                    Err(TryRecvError::Disconnected) => return Err(RecvError),
                    Err(TryRecvError::Empty) => {
                        anyos_std::process::yield_cpu();
                    }
                }
            }
        }

        pub fn try_recv(&self) -> Result<T, TryRecvError> {
            let mut queue = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
            if !queue.is_empty() {
                let val = queue.remove(0);
                if queue.is_empty() {
                    self.inner.has_data.store(false, Ordering::Release);
                }
                Ok(val)
            } else if self.inner.closed.load(Ordering::Acquire) {
                Err(TryRecvError::Disconnected)
            } else {
                Err(TryRecvError::Empty)
            }
        }

        pub fn iter(&self) -> Iter<'_, T> {
            Iter { rx: self }
        }

        pub fn try_iter(&self) -> TryIter<'_, T> {
            TryIter { rx: self }
        }
    }

    impl<T> Drop for Receiver<T> {
        fn drop(&mut self) {
            self.inner.closed.store(true, Ordering::Release);
        }
    }

    impl<T> IntoIterator for Receiver<T> {
        type Item = T;
        type IntoIter = IntoIter<T>;

        fn into_iter(self) -> IntoIter<T> {
            IntoIter { rx: self }
        }
    }

    /// Blocking iterator.
    pub struct Iter<'a, T> {
        rx: &'a Receiver<T>,
    }

    impl<T> Iterator for Iter<'_, T> {
        type Item = T;
        fn next(&mut self) -> Option<T> {
            self.rx.recv().ok()
        }
    }

    /// Non-blocking iterator.
    pub struct TryIter<'a, T> {
        rx: &'a Receiver<T>,
    }

    impl<T> Iterator for TryIter<'_, T> {
        type Item = T;
        fn next(&mut self) -> Option<T> {
            self.rx.try_recv().ok()
        }
    }

    /// Consuming iterator.
    pub struct IntoIter<T> {
        rx: Receiver<T>,
    }

    impl<T> Iterator for IntoIter<T> {
        type Item = T;
        fn next(&mut self) -> Option<T> {
            self.rx.recv().ok()
        }
    }

    /// Create an unbounded channel.
    pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
        let inner = Arc::new(Inner {
            queue: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
            has_data: AtomicBool::new(false),
        });
        (
            Sender { inner: inner.clone() },
            Receiver { inner },
        )
    }

    /// Create a bounded synchronous channel.
    pub fn sync_channel<T>(bound: usize) -> (SyncSender<T>, Receiver<T>) {
        let inner = Arc::new(SyncInner {
            queue: Mutex::new(Vec::new()),
            bound,
            closed: AtomicBool::new(false),
            count: AtomicUsize::new(0),
        });
        (
            SyncSender { inner: inner.clone() },
            Receiver {
                inner: Arc::new(Inner {
                    queue: Mutex::new(Vec::new()),
                    closed: AtomicBool::new(false),
                    has_data: AtomicBool::new(false),
                }),
            },
        )
    }

    struct SyncInner<T> {
        queue: Mutex<Vec<T>>,
        bound: usize,
        closed: AtomicBool,
        count: AtomicUsize,
    }

    /// Synchronous sender (bounded channel).
    pub struct SyncSender<T> {
        inner: Arc<SyncInner<T>>,
    }

    impl<T> Clone for SyncSender<T> {
        fn clone(&self) -> Self {
            SyncSender { inner: Arc::clone(&self.inner) }
        }
    }

    impl<T> SyncSender<T> {
        pub fn send(&self, value: T) -> Result<(), SendError<T>> {
            if self.inner.closed.load(Ordering::Acquire) {
                return Err(SendError(value));
            }
            // Wait for space
            while self.inner.count.load(Ordering::Acquire) >= self.inner.bound {
                if self.inner.closed.load(Ordering::Acquire) {
                    return Err(SendError(value));
                }
                anyos_std::process::yield_cpu();
            }
            let mut queue = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.push(value);
            self.inner.count.fetch_add(1, Ordering::Release);
            Ok(())
        }

        pub fn try_send(&self, value: T) -> Result<(), TrySendError<T>> {
            if self.inner.closed.load(Ordering::Acquire) {
                return Err(TrySendError::Disconnected(value));
            }
            if self.inner.count.load(Ordering::Acquire) >= self.inner.bound {
                return Err(TrySendError::Full(value));
            }
            let mut queue = self.inner.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.push(value);
            self.inner.count.fetch_add(1, Ordering::Release);
            Ok(())
        }
    }
}

// ── Poison errors ───────────────────────────────────────────────────────────

/// Error indicating a mutex was poisoned.
#[derive(Debug)]
pub struct PoisonError<T> {
    guard: T,
}

impl<T> PoisonError<T> {
    pub fn new(guard: T) -> Self {
        PoisonError { guard }
    }

    pub fn into_inner(self) -> T {
        self.guard
    }

    pub fn get_ref(&self) -> &T {
        &self.guard
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

impl<T> core::fmt::Display for PoisonError<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("poisoned lock: another task failed inside")
    }
}

/// Error from try_lock.
#[derive(Debug)]
pub enum TryLockError<T> {
    Poisoned(PoisonError<T>),
    WouldBlock,
}

impl<T> core::fmt::Display for TryLockError<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TryLockError::Poisoned(_) => f.write_str("poisoned lock"),
            TryLockError::WouldBlock => f.write_str("try_lock failed because the operation would block"),
        }
    }
}

impl<T> From<PoisonError<T>> for TryLockError<T> {
    fn from(err: PoisonError<T>) -> Self {
        TryLockError::Poisoned(err)
    }
}

// ── Re-exports matching std::sync ───────────────────────────────────────────

pub mod atomic {
    pub use core::sync::atomic::*;
}
