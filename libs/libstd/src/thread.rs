//! std::thread compatible threading.

use alloc::boxed::Box;
use alloc::string::String;
use core::time::Duration;

/// A handle to a spawned thread.
pub struct JoinHandle<T> {
    inner: anyos_std::process::Thread,
    result: Option<Box<core::cell::UnsafeCell<Option<T>>>>,
}

unsafe impl<T> Send for JoinHandle<T> {}

impl<T> JoinHandle<T> {
    /// Wait for the thread to finish and return its result.
    pub fn join(self) -> Result<T, Box<dyn core::any::Any + Send + 'static>> {
        let result = self.result;
        self.inner.join();
        match result {
            Some(cell) => {
                // Safety: thread has joined, no more concurrent access
                let val = unsafe { &mut *cell.get() }.take();
                match val {
                    Some(v) => Ok(v),
                    None => Err(Box::new("thread did not produce a result")),
                }
            }
            None => Err(Box::new("no result available")),
        }
    }

    /// Get the thread handle.
    pub fn thread(&self) -> Thread {
        Thread { tid: self.inner.tid() }
    }

    /// Check if the thread is finished (non-blocking).
    pub fn is_finished(&self) -> bool {
        anyos_std::process::try_waitpid(self.inner.tid()) != anyos_std::process::STILL_RUNNING
    }
}

/// Thread identifier.
#[derive(Debug, Clone)]
pub struct Thread {
    tid: u32,
}

impl Thread {
    pub fn id(&self) -> ThreadId {
        ThreadId(self.tid as u64)
    }

    pub fn name(&self) -> Option<&str> {
        None
    }

    pub fn unpark(&self) {
        // No parking support, no-op
    }
}

/// A unique thread identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ThreadId(u64);

/// Thread builder.
pub struct Builder {
    name: Option<String>,
    stack_size: Option<usize>,
}

impl Builder {
    pub fn new() -> Self {
        Builder { name: None, stack_size: None }
    }

    pub fn name(mut self, name: String) -> Self {
        self.name = Some(name);
        self
    }

    pub fn stack_size(mut self, size: usize) -> Self {
        self.stack_size = Some(size);
        self
    }

    pub fn spawn<F, T>(self, f: F) -> crate::io::Result<JoinHandle<T>>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let stack_size = self.stack_size.unwrap_or(64 * 1024);
        let name = self.name.unwrap_or_else(|| String::from("thread"));

        // Store result in a shared cell
        let result_cell: Box<core::cell::UnsafeCell<Option<T>>> =
            Box::new(core::cell::UnsafeCell::new(None));
        let result_ptr = &*result_cell as *const core::cell::UnsafeCell<Option<T>> as usize;

        // Box the closure and leak it so the thread can pick it up
        let closure: Box<dyn FnOnce() + Send + 'static> = Box::new(move || {
            let val = f();
            unsafe {
                let cell = &*(result_ptr as *const core::cell::UnsafeCell<Option<T>>);
                *cell.get() = Some(val);
            }
        });
        let closure_ptr = Box::into_raw(Box::new(closure));

        // We need a plain fn() for anyos_std::process::Thread, so we use a trampoline
        // stored via a global. This is limited but functional for basic use.
        unsafe {
            THREAD_CLOSURE = Some(closure_ptr as usize);
        }

        let thread = anyos_std::process::Thread::spawn_with_stack(
            thread_trampoline,
            stack_size,
            &name,
        ).map_err(|e| crate::io::Error::from_anyos(e))?;

        Ok(JoinHandle {
            inner: thread,
            result: Some(result_cell),
        })
    }
}

// Global trampoline for passing closures to threads.
// This is a simplification — only one thread can be spawned at a time.
// A proper implementation would use thread-local storage or a per-thread mailbox.
static mut THREAD_CLOSURE: Option<usize> = None;

fn thread_trampoline() {
    let closure_ptr = unsafe {
        THREAD_CLOSURE.take()
    };
    if let Some(ptr) = closure_ptr {
        let closure = unsafe {
            Box::from_raw(ptr as *mut Box<dyn FnOnce() + Send + 'static>)
        };
        (*closure)();
    }
}

/// Spawn a new thread.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    Builder::new().spawn(f).expect("failed to spawn thread")
}

/// Sleep for a duration.
pub fn sleep(dur: Duration) {
    let ms = dur.as_millis() as u32;
    if ms > 0 {
        anyos_std::process::sleep(ms);
    }
}

/// Yield the current time slice.
pub fn yield_now() {
    anyos_std::process::yield_cpu();
}

/// Get the current thread.
pub fn current() -> Thread {
    Thread { tid: anyos_std::process::getpid() }
}

/// Park the current thread (simplified: just yields).
pub fn park() {
    anyos_std::process::yield_cpu();
}

/// Park with a timeout.
pub fn park_timeout(dur: Duration) {
    sleep(dur);
}

/// Get the number of available CPUs.
pub fn available_parallelism() -> crate::io::Result<core::num::NonZeroUsize> {
    // Query from sysinfo
    let mut buf = [0u8; 256];
    let ret = anyos_std::sys::sysinfo(2, &mut buf); // cmd 2 = cpu info
    let cpus = if ret > 0 { ret as usize } else { 1 };
    Ok(core::num::NonZeroUsize::new(cpus).unwrap_or(core::num::NonZeroUsize::new(1).unwrap()))
}
