//! Memory management subsystem.
//!
//! Provides physical frame allocation, virtual memory paging, kernel heap,
//! and typed address wrappers for safe physical/virtual address manipulation.

pub mod address;
pub mod heap;
pub mod physical;
pub mod virtual_mem;

/// Size of a single memory page/frame in bytes (4 KiB).
pub const FRAME_SIZE: usize = 4096;
