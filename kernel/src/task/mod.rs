//! Task management subsystem: threads, scheduling, program loading, and per-process resources.
//!
//! Provides a preemptive round-robin scheduler with priority support, ELF/flat binary
//! loading into isolated per-process address spaces, DLL mapping, and CPU utilization monitoring.

pub mod abi;
pub mod app_config;
pub mod capabilities;
pub mod context;
pub mod cpu_monitor;
pub mod crash_info;
pub mod dll;
pub mod env;
pub mod loader;
pub mod permissions;
pub mod process;
pub mod scheduler;
// Always compiled: `smp_stress_master` is gated at runtime by the `schedstress`
// boot param (Phase 4b safety net). The legacy `stress_master` is dead code
// without `debug_verbose`, hence the allow.
#[allow(dead_code)]
pub mod stress_test;
pub mod thread;
pub mod users;
