//! JIT acceleration for x86 instruction execution.
//!
//! This module provides a decode cache that stores pre-decoded basic blocks,
//! eliminating redundant instruction decoding for hot loops. Basic blocks are
//! identified by their physical address, CPU mode, and CS base, then cached
//! for reuse.
//!
//! Future phases will add native code generation (dynamic binary translation)
//! on top of this foundation.

pub mod block;
pub mod cache;
