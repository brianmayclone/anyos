//! Inter-process communication primitives.
//!
//! Provides named pipes, a system/module event bus, POSIX-style signals,
//! shared memory regions, and message queues for kernel and user-space IPC.

pub mod event_bus;
pub mod message_queue;
pub mod pipe;
pub mod shared_memory;
pub mod signal;
