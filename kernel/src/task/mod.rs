//! Task management subsystem: threads, scheduling, program loading, and per-process resources.
//!
//! Provides a preemptive round-robin scheduler with priority support, ELF/flat binary
//! loading into isolated per-process address spaces, DLL mapping, and CPU utilization monitoring.

pub mod context;
pub mod cpu_monitor;
pub mod dll;
pub mod loader;
pub mod process;
pub mod scheduler;
pub mod thread;
