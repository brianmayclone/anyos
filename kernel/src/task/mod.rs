//! Task management subsystem: threads, scheduling, program loading, and per-process resources.
//!
//! Provides a preemptive round-robin scheduler with priority support, ELF/flat binary
//! loading into isolated per-process address spaces, DLL mapping, and CPU utilization monitoring.

pub mod app_config;
pub mod capabilities;
pub mod context;
pub mod cpu_monitor;
pub mod dll;
pub mod env;
pub mod loader;
pub mod permissions;
pub mod process;
pub mod scheduler;
#[cfg(feature = "debug_verbose")]
pub mod stress_test;
pub mod thread;
pub mod users;
