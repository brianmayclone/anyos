//! Memory management subsystem.
//!
//! Provides physical frame allocation, virtual memory paging, kernel heap,
//! and typed address wrappers for safe physical/virtual address manipulation.

pub mod address;
pub mod heap;
pub mod physical;
#[cfg(target_arch = "x86_64")]
pub mod virtual_mem;
#[cfg(target_arch = "aarch64")]
pub mod virtual_mem_stub;
#[cfg(target_arch = "aarch64")]
pub use virtual_mem_stub as virtual_mem;
pub mod vma;

/// Size of a single memory page/frame in bytes (4 KiB).
pub const FRAME_SIZE: usize = 4096;
